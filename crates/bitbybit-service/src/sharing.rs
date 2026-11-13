//! Sharing inside the service: the DHCP server and DNS forwarder of a WinNAT session run here (threads on UDP
//! sockets bound to the LAN gateway address only), every other step goes to the platform backend. A stopped
//! service stops its servers; their journal records then read as gone.

use bitbybit_core::{Error, ErrorCode, Result};
use bitbybit_journal::ChangeRecord;
use bitbybit_network::dhcp::{Destination, DhcpServer, Lease, PoolConfig};
use bitbybit_network::dns::{Action, Forwarder};
use bitbybit_network::session::{SharingBackend, StepAction, StepStates};
use bitbybit_recovery::{LiveState, RevertHandler};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use time::OffsetDateTime;

struct Running {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Running {
    fn halt(self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.thread.join();
    }
}

/// The DHCP and DNS servers of the current session (at most one of each).
#[derive(Default)]
pub struct Hosts {
    dhcp: Mutex<Option<Running>>,
    dns: Mutex<Option<Running>>,
    /// Leases of the running DHCP server (for the list of connected devices).
    leases: Arc<Mutex<Vec<Lease>>>,
    /// Fixed addresses ("IP fixo") the running DHCP server honours, by MAC.
    reservations: Arc<Mutex<std::collections::BTreeMap<String, Ipv4Addr>>>,
    /// Ports, so tests can avoid the privileged 67/53.
    pub dhcp_port: u16,
    pub dns_port: u16,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Hosts {
    pub fn new() -> Self {
        Self { dhcp_port: 67, dns_port: 53, ..Self::default() }
    }

    pub fn dhcp_running(&self) -> bool {
        lock(&self.dhcp).as_ref().is_some_and(|r| !r.thread.is_finished())
    }

    pub fn leases(&self) -> Vec<Lease> {
        lock(&self.leases).clone()
    }

    /// Sets (`Some`) or clears one device's fixed address; the DHCP server applies it at its next request.
    pub fn set_reservation(&self, mac: &str, ip: Option<Ipv4Addr>) {
        let mut r = lock(&self.reservations);
        match ip {
            Some(ip) => r.insert(mac.to_owned(), ip),
            None => r.remove(mac),
        };
    }

    pub fn dns_running(&self) -> bool {
        lock(&self.dns).as_ref().is_some_and(|r| !r.thread.is_finished())
    }

    pub fn start_dhcp(&self, pool: PoolConfig) -> Result<()> {
        let mut slot = lock(&self.dhcp);
        if slot.is_some() {
            return Err(Error::new(ErrorCode::Conflict, "A DHCP server is already running."));
        }
        let socket = UdpSocket::bind((pool.server, self.dhcp_port))?;
        socket.set_broadcast(true)?;
        socket.set_read_timeout(Some(Duration::from_millis(300)))?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let shared = self.leases.clone();
        lock(&shared).clear();
        let reservations = self.reservations.clone();
        lock(&reservations).clear();
        let thread = std::thread::spawn(move || {
            let mut server = DhcpServer::new(pool, vec![]);
            let mut buf = [0u8; 1500];
            while !flag.load(Ordering::SeqCst) {
                let Ok((n, _)) = socket.recv_from(&mut buf) else { continue };
                server.set_reservations(&lock(&reservations));
                let reply = server.handle(&buf[..n], OffsetDateTime::now_utc());
                *lock(&shared) = server.leases().cloned().collect();
                if let Some(reply) = reply {
                    let to = match reply.destination {
                        Destination::Broadcast => Ipv4Addr::BROADCAST,
                        Destination::Unicast(ip) => ip,
                    };
                    if let Err(e) = socket.send_to(&reply.message.encode(), (to, 68)) {
                        log::warn!("DHCP reply to {to} failed: {e}");
                    }
                }
            }
        });
        *slot = Some(Running { stop, thread });
        Ok(())
    }

    pub fn start_dns(&self, listen: Ipv4Addr, upstreams: &[SocketAddr]) -> Result<()> {
        let mut slot = lock(&self.dns);
        if slot.is_some() {
            return Err(Error::new(ErrorCode::Conflict, "A DNS forwarder is already running."));
        }
        let mut forwarder = Forwarder::new(upstreams, &[IpAddr::V4(listen)]);
        if !forwarder.has_upstream() {
            return Err(Error::new(ErrorCode::PreconditionFailed, "No usable upstream DNS server."));
        }
        let clients = UdpSocket::bind((listen, self.dns_port))?;
        let upstream = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        for s in [&clients, &upstream] {
            s.set_read_timeout(Some(Duration::from_millis(100)))?;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let send = |a: Action| match a {
                Action::Reply { to, datagram } => drop(clients.send_to(&datagram, to)),
                Action::Forward { to, datagram } => drop(upstream.send_to(&datagram, to)),
                Action::Drop => {}
            };
            while !flag.load(Ordering::SeqCst) {
                if let Ok((n, from)) = clients.recv_from(&mut buf) {
                    send(forwarder.handle_query(&buf[..n], from, OffsetDateTime::now_utc()));
                }
                if let Ok((n, from)) = upstream.recv_from(&mut buf) {
                    send(forwarder.handle_response(&buf[..n], from, OffsetDateTime::now_utc()));
                }
                forwarder.poll_timeouts(OffsetDateTime::now_utc()).into_iter().for_each(send);
            }
        });
        *slot = Some(Running { stop, thread });
        Ok(())
    }

    pub fn stop_dhcp(&self) {
        if let Some(r) = lock(&self.dhcp).take() {
            r.halt();
        }
    }

    pub fn stop_dns(&self) {
        if let Some(r) = lock(&self.dns).take() {
            r.halt();
        }
    }
}

/// Platform steps plus the service-hosted DHCP and DNS.
pub struct ServiceSharing {
    pub platform: Arc<dyn SharingBackend>,
    pub hosts: Arc<Hosts>,
}

impl SharingBackend for ServiceSharing {
    fn prepare(&self, step: &StepAction) -> Result<StepStates> {
        match step {
            StepAction::StartDhcp { .. } | StepAction::StartDns { .. } => {
                Ok(StepStates { previous: None, applied: Some(serde_json::to_string(step)?) })
            }
            _ => self.platform.prepare(step),
        }
    }

    fn apply(&self, step: &StepAction) -> Result<()> {
        match step {
            StepAction::StartDhcp { pool } => self.hosts.start_dhcp(pool.clone()),
            StepAction::StartDns { listen, upstreams } => self.hosts.start_dns(*listen, upstreams),
            _ => self.platform.apply(step),
        }
    }

    fn verify(&self, step: &StepAction) -> Result<bool> {
        match step {
            StepAction::StartDhcp { .. } => Ok(self.hosts.dhcp_running()),
            StepAction::StartDns { .. } => Ok(self.hosts.dns_running()),
            _ => self.platform.verify(step),
        }
    }
}

/// Stops the session's DHCP server (gone once the service restarted).
pub struct DhcpRevert(pub Arc<Hosts>);
impl RevertHandler for DhcpRevert {
    fn target_kind(&self) -> &str {
        "sharing.dhcp"
    }
    fn compare(&self, _: &ChangeRecord) -> Result<LiveState> {
        Ok(if self.0.dhcp_running() { LiveState::MatchesApplied } else { LiveState::Gone })
    }
    fn revert(&self, _: &ChangeRecord) -> Result<()> {
        self.0.stop_dhcp();
        Ok(())
    }
}

/// Stops the session's DNS forwarder.
pub struct DnsRevert(pub Arc<Hosts>);
impl RevertHandler for DnsRevert {
    fn target_kind(&self) -> &str {
        "sharing.dns"
    }
    fn compare(&self, _: &ChangeRecord) -> Result<LiveState> {
        Ok(if self.0.dns_running() { LiveState::MatchesApplied } else { LiveState::Gone })
    }
    fn revert(&self, _: &ChangeRecord) -> Result<()> {
        self.0.stop_dns();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_network::prefix::IpPrefix;

    #[test]
    fn servers_start_once_on_the_lan_address_and_stop_cleanly() {
        let hosts = Hosts { dhcp_port: 0, dns_port: 0, ..Hosts::default() };
        let lan = IpPrefix::parse("127.0.0.0/24", true).unwrap();
        let pool = PoolConfig::for_lan(lan, 3600).unwrap();
        hosts.start_dhcp(pool.clone()).unwrap();
        assert!(hosts.dhcp_running());
        assert!(hosts.start_dhcp(pool).is_err(), "one DHCP server per session");
        let upstream: SocketAddr = "9.9.9.9:53".parse().unwrap();
        hosts.start_dns(Ipv4Addr::LOCALHOST, &[upstream]).unwrap();
        assert!(hosts.dns_running());
        hosts.stop_dhcp();
        hosts.stop_dns();
        assert!(!hosts.dhcp_running() && !hosts.dns_running());
        assert!(hosts.start_dns(Ipv4Addr::LOCALHOST, &["127.0.0.1:53".parse().unwrap()]).is_err(), "loop refused");
    }
}

/// Leak-guard watchdog tick: for every active `sharing.leakGuard` record, the planned filters are compared with
/// what WFP holds now (`installed`). When protection is no longer guaranteed (filters missing or changed, or WFP
/// unreadable) the whole session is stopped, so shared devices lose the Internet instead of leaking. Returns the
/// sessions stopped.
pub fn leak_watchdog_tick(
    journal: &dyn bitbybit_journal::Journal,
    installed: &dyn Fn() -> Result<Vec<bitbybit_network::leak::FilterSpec>>,
    stop: &dyn Fn(uuid::Uuid) -> Result<()>,
) -> Vec<uuid::Uuid> {
    use bitbybit_network::leak::{FilterSpec, LeakGuardObservation, ProtectionState, assess};
    #[derive(serde::Deserialize)]
    struct Guard {
        filters: Vec<FilterSpec>,
    }
    let Ok(records) = journal.all() else { return vec![] };
    let mut stopped = Vec::new();
    for r in records.iter().filter(|r| r.target_kind == "sharing.leakGuard" && r.is_active()) {
        let (Some(session), Some(plan)) =
            (r.correlation_id, r.applied_state.as_deref().and_then(|s| serde_json::from_str::<Guard>(s).ok()))
        else {
            continue;
        };
        let observed = LeakGuardObservation {
            installed: installed().ok(),
            vpn_up: true,
            vpn_index_current: true,
            forwarding_enabled_on_downstream: true,
        };
        let state = assess(&plan.filters, &observed).state;
        if matches!(state, ProtectionState::NotGuaranteed | ProtectionState::Unknown) && !stopped.contains(&session) {
            log::error!("Leak protection of session {session} is {state:?}; stopping the session");
            match stop(session) {
                Ok(()) => stopped.push(session),
                Err(e) => log::error!("Stopping session {session} failed: {e}"),
            }
        }
    }
    stopped
}

#[cfg(test)]
mod watchdog_tests {
    use super::*;
    use bitbybit_journal::{ChangeCategory, ChangeRecord, Journal, JsonLinesJournal};
    use bitbybit_network::leak::{LeakGuardInput, plan};

    #[test]
    fn missing_filters_stop_the_session_and_intact_ones_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let filters =
            plan(&LeakGuardInput { downstream_index: 7, vpn_index: 9, vpn_luid: 1, guard_dns_forwarder: false });
        let session = uuid::Uuid::new_v4();
        let record = ChangeRecord::new(
            ChangeCategory::PacketFilter,
            "sharing.leakGuard",
            "sharing.leakGuard",
            "guard",
            time::OffsetDateTime::UNIX_EPOCH,
        )
        .with_states(None, Some(serde_json::json!({ "filters": filters }).to_string()))
        .in_operation(session, true);
        journal.append(record).unwrap();
        let stops = std::cell::RefCell::new(vec![]);
        let stop = |s| {
            stops.borrow_mut().push(s);
            Ok(())
        };
        let intact = filters.clone();
        assert!(leak_watchdog_tick(&journal, &|| Ok(intact.clone()), &stop).is_empty());
        let mut broken = filters.clone();
        broken.retain(|f| f.action == bitbybit_network::leak::WfpAction::Permit); // block filters removed
        assert_eq!(leak_watchdog_tick(&journal, &|| Ok(broken.clone()), &stop), vec![session]);
        assert_eq!(leak_watchdog_tick(&journal, &|| Err(Error::platform("WFP unreadable", 5)), &stop), vec![session]);
    }
}
