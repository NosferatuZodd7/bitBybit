//! "Diagnóstico" panel of the network page: ping, traceroute, DNS lookup and gateway check. Read-only and
//! unelevated, so they run here in the app (not through the broker) on a worker thread; rows stream to the UI as
//! each probe answers. One test at a time; "Cancelar" stops it before the next probe (a probe already sent ends at
//! its own timeout). Probes go only to what the user typed or to the local default gateways.

use crate::ui::{AppWindow, NetDiag, NetDiagRow};
use bitbybit_core::{Error, LocalizedText, Result, text};
use bitbybit_localization::TextCatalog;
use bitbybit_network::diag::{
    DnsStatus, GatewayReachability, HostName, NetworkDiagnostics, PingRequest, PingResult, ProbeStatus,
    TracerouteRequest, overall, parse_target, summarize, trace,
};
use bitbybit_network::model::NetworkSnapshot;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::net::IpAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const PING: i32 = 0;
pub const TRACE: i32 = 1;
pub const DNS: i32 = 2;
pub const GATEWAY: i32 = 3;
/// Echo requests per ping, like ping.exe.
const PING_COUNT: u32 = 4;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    pub item: String,
    pub address: String,
    pub time: String,
    pub detail: String,
    pub status: String,
    /// As the Badge: 0 neutral, 1 warning, 2 problem, 3 info, 4 ok.
    pub tone: i32,
}

/// One run of a test: the platform, where texts come from, the cancel flag and the pause between echoes.
pub struct Job<'a> {
    pub diag: &'a dyn NetworkDiagnostics,
    pub catalog: &'a TextCatalog,
    pub cancel: &'a AtomicBool,
    pub pause: Duration,
}

fn ms(d: Duration) -> String {
    match d.as_millis() {
        0 => "<1 ms".to_owned(),
        n => format!("{n} ms"),
    }
}

fn opt_ms(d: Option<Duration>) -> String {
    d.map(ms).unwrap_or_default()
}

fn probe_status(s: ProbeStatus) -> (LocalizedText, i32) {
    match s {
        ProbeStatus::Success => (text!("network.diag.status.ok", "Replied"), 4),
        ProbeStatus::TimedOut => (text!("network.diag.status.timedOut", "No reply"), 1),
        ProbeStatus::TtlExpired => (text!("network.diag.status.ttlExpired", "Router (TTL expired)"), 3),
        ProbeStatus::DestinationUnreachable => (text!("network.diag.status.unreachable", "Destination unreachable"), 2),
        ProbeStatus::PacketTooBig => (text!("network.diag.status.tooBig", "Packet too big"), 2),
        ProbeStatus::BadRoute => (text!("network.diag.status.badRoute", "No route"), 2),
        ProbeStatus::SourceAddressInvalid => (text!("network.diag.status.badSource", "Invalid source address"), 2),
        ProbeStatus::NotSupported => (text!("network.diag.status.notSupported", "Not supported"), 2),
        ProbeStatus::Error => (text!("network.diag.status.error", "Error"), 2),
    }
}

fn dns_status(s: DnsStatus) -> LocalizedText {
    match s {
        DnsStatus::Resolved => text!("network.diag.dns.resolvedShort", "Resolved"),
        DnsStatus::NameNotFound => text!("network.diag.dns.nxdomain", "the name does not exist (NXDOMAIN)"),
        DnsStatus::NoAddresses => text!("network.diag.dns.noAddresses", "the name exists but has no IP address"),
        DnsStatus::TimedOut => text!("network.diag.dns.timedOut", "the DNS servers did not answer in time"),
        DnsStatus::NoServers => text!("network.diag.dns.noServers", "no DNS server is configured"),
        DnsStatus::ServerFailure => text!("network.diag.dns.serverFailure", "the DNS server failed (SERVFAIL)"),
        DnsStatus::Refused => text!("network.diag.dns.refused", "the DNS server refused the query"),
        DnsStatus::Error => text!("network.diag.dns.error", "the lookup failed"),
    }
}

impl Job<'_> {
    fn t(&self, t: &LocalizedText) -> String {
        self.catalog.resolve(t)
    }

    fn problem(&self, e: &Error) -> String {
        self.t(&e.user_text.clone().unwrap_or_else(|| text!("network.diag.failed", "The test failed: {0}", e.message)))
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Runs one test, emitting rows as they come; returns the summary line.
    pub fn run(
        &self,
        kind: i32,
        target: &str,
        snapshot: &dyn Fn() -> Result<NetworkSnapshot>,
        emit: &mut dyn FnMut(Row),
    ) -> String {
        let target = target.trim();
        if target.is_empty() && kind != GATEWAY {
            return self.t(&text!("network.diag.needTarget", "Enter an IP address or a host name."));
        }
        let summary = match kind {
            PING => self.ping(target, emit),
            TRACE => self.trace(target, emit),
            DNS => self.dns(target, snapshot, emit),
            _ => self.gateway(snapshot, emit),
        };
        let summary = summary.unwrap_or_else(|e| self.problem(&e));
        if self.cancelled() {
            self.t(&text!("network.diag.cancelled", "{0} · Stopped by the user.", summary))
        } else {
            summary
        }
    }

    /// An address as typed, or the first address of a host name (shown as "name (address)").
    fn target(&self, typed: &str) -> Result<(IpAddr, String)> {
        if let Ok(ip) = parse_target(typed) {
            return Ok((ip, ip.to_string()));
        }
        let name = HostName::parse(typed).map_err(|e| {
            e.with_user_text(text!("network.diag.invalidTarget", "Enter an IP address or a host name."))
        })?;
        let answer = self.diag.resolve(&name)?;
        match answer.addresses.first() {
            Some(ip) => Ok((*ip, format!("{} ({ip})", name.as_str()))),
            None => Err(Error::not_found("host name not resolved").with_user_text(text!(
                "network.diag.dns.failed",
                "{0} was not resolved after {1}: {2}",
                name.as_str().to_owned(),
                ms(answer.elapsed),
                self.t(&dns_status(answer.status))
            ))),
        }
    }

    fn ping(&self, typed: &str, emit: &mut dyn FnMut(Row)) -> Result<String> {
        let (ip, label) = self.target(typed)?;
        let request = PingRequest::new(ip);
        let mut replies: Vec<PingResult> = Vec::new();
        for n in 1..=PING_COUNT {
            if self.cancelled() {
                break;
            }
            if n > 1 {
                std::thread::sleep(self.pause);
            }
            let r = self.diag.ping(&request)?;
            let (status, tone) = probe_status(r.status);
            emit(Row {
                item: n.to_string(),
                address: r.responded_from.map(|a| a.to_string()).unwrap_or_default(),
                time: opt_ms(r.round_trip.filter(|_| r.status == ProbeStatus::Success)),
                detail: r
                    .reply_ttl
                    .filter(|_| r.status == ProbeStatus::Success)
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
                status: self.t(&status),
                tone,
            });
            replies.push(r);
        }
        let s = summarize(&replies);
        Ok(match (s.min, s.avg, s.max) {
            (Some(min), Some(avg), Some(max)) => self.t(&text!(
                "network.diag.ping.summary",
                "{0}: {1} sent, {2} received, {3} % lost · min {4} / avg {5} / max {6}",
                label,
                s.sent,
                s.received,
                s.loss_percent,
                ms(min),
                ms(avg),
                ms(max)
            )),
            _ => self.t(&text!(
                "network.diag.ping.noReply",
                "{0}: {1} sent, no reply ({2} % lost). Many devices and firewalls do not answer ping.",
                label,
                s.sent,
                s.loss_percent
            )),
        })
    }

    fn trace(&self, typed: &str, emit: &mut dyn FnMut(Row)) -> Result<String> {
        let (ip, label) = self.target(typed)?;
        let hops = trace(
            &TracerouteRequest::new(ip),
            |r| self.diag.ping(r),
            |h| {
                let (status, tone) = probe_status(h.status);
                emit(Row {
                    item: h.hop.to_string(),
                    address: h.address.map_or_else(|| "*".to_owned(), |a| a.to_string()),
                    time: opt_ms(h.round_trip.filter(|_| h.status != ProbeStatus::TimedOut)),
                    status: self.t(&status),
                    tone,
                    ..Row::default()
                });
                !self.cancelled()
            },
        )?;
        let reached = hops.last().is_some_and(|h| h.address == Some(ip));
        Ok(if reached {
            self.t(&text!("network.diag.trace.reached", "{0} reached in {1} hops.", label, hops.len()))
        } else {
            self.t(&text!("network.diag.trace.notReached", "{0} was not reached ({1} hops tried).", label, hops.len()))
        })
    }

    fn dns(
        &self,
        typed: &str,
        snapshot: &dyn Fn() -> Result<NetworkSnapshot>,
        emit: &mut dyn FnMut(Row),
    ) -> Result<String> {
        let name = HostName::parse(typed)?;
        let answer = self.diag.resolve(&name)?;
        let ok = answer.status == DnsStatus::Resolved;
        for a in &answer.addresses {
            emit(Row {
                item: if a.is_ipv4() { "IPv4 (A)" } else { "IPv6 (AAAA)" }.to_owned(),
                address: a.to_string(),
                status: self.t(&dns_status(DnsStatus::Resolved)),
                tone: 4,
                ..Row::default()
            });
        }
        // Which servers Windows asks (the interfaces' configuration; nothing is sent to them from here).
        let servers = snapshot().map(|s| {
            s.interfaces
                .iter()
                .filter(|i| i.is_up())
                .flat_map(|i| i.dns_servers.iter().map(move |d| (*d, i.alias.clone())))
                .collect::<Vec<_>>()
        });
        let server = self.t(&text!("network.diag.dns.server", "DNS server"));
        for (address, alias) in servers.as_deref().unwrap_or_default() {
            emit(Row { item: server.clone(), address: address.to_string(), detail: alias.clone(), ..Row::default() });
        }
        let mut summary = if ok {
            self.t(&text!(
                "network.diag.dns.resolved",
                "{0} resolved in {1}: {2} addresses.",
                name.as_str().to_owned(),
                ms(answer.elapsed),
                answer.addresses.len()
            ))
        } else {
            let mut reason = self.t(&dns_status(answer.status));
            if let Some(code) = answer.native_status {
                reason = format!("{reason} ({code})");
            }
            self.t(&text!(
                "network.diag.dns.failed",
                "{0} was not resolved after {1}: {2}",
                name.as_str().to_owned(),
                ms(answer.elapsed),
                reason
            ))
        };
        if servers.is_ok_and(|s| s.is_empty()) {
            summary = format!(
                "{summary} {}",
                self.t(&text!("network.diag.dns.noneConfigured", "No interface has a DNS server configured."))
            );
        }
        Ok(summary)
    }

    fn gateway(&self, snapshot: &dyn Fn() -> Result<NetworkSnapshot>, emit: &mut dyn FnMut(Row)) -> Result<String> {
        let snapshot = snapshot()?;
        let mut probes = Vec::new();
        for i in snapshot.interfaces.iter().filter(|i| i.is_up() && !i.gateways.is_empty()) {
            if self.cancelled() {
                break;
            }
            match self.diag.check_gateway(i.luid) {
                Ok(found) => {
                    for p in found {
                        let (status, tone) = match p.reachability {
                            GatewayReachability::Reachable => {
                                (text!("network.diag.gateway.statusReachable", "Reachable"), 4)
                            }
                            GatewayReachability::Unreachable => {
                                (text!("network.diag.gateway.statusUnreachable", "Not answering"), 2)
                            }
                            _ => (text!("network.diag.gateway.statusNotTested", "Not tested"), 0),
                        };
                        emit(Row {
                            item: i.alias.clone(),
                            address: p.gateway.to_string(),
                            time: opt_ms(p.latency),
                            detail: p.mac.clone().unwrap_or_default(),
                            status: self.t(&status),
                            tone,
                        });
                        probes.push(p);
                    }
                }
                Err(e) => emit(Row {
                    item: i.alias.clone(),
                    address: i.gateways.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "),
                    detail: self.problem(&e),
                    status: self.t(&probe_status(ProbeStatus::Error).0),
                    tone: 2,
                    ..Row::default()
                }),
            }
        }
        let answered = probes.iter().filter(|p| p.reachability == GatewayReachability::Reachable).count();
        Ok(self.t(&match overall(&probes) {
            GatewayReachability::NoGateway => text!(
                "network.diag.gateway.none",
                "No connected interface has a default gateway (normal without a network and for some VPNs)."
            ),
            GatewayReachability::Reachable => {
                text!("network.diag.gateway.reachable", "{0} of {1} gateways answered.", answered, probes.len())
            }
            GatewayReachability::Unreachable => text!(
                "network.diag.gateway.unreachable",
                "No gateway answered: check the cable or the Wi-Fi connection and the router."
            ),
            GatewayReachability::NotTested => text!("network.diag.gateway.notTested", "The gateways were not tested."),
        }))
    }
}

/// The diagnostics port and the (read-only) inventory read used for gateways and DNS servers.
type Platform = (Box<dyn NetworkDiagnostics>, fn() -> Result<NetworkSnapshot>);

#[cfg(windows)]
fn platform() -> Platform {
    (
        Box::new(bitbybit_platform_windows::net::diag::WindowsDiagnostics),
        bitbybit_platform_windows::net::inventory::collect,
    )
}

#[cfg(not(windows))]
fn platform() -> Platform {
    struct Unsupported;
    fn no<T>() -> Result<T> {
        Err(Error::new(bitbybit_core::ErrorCode::NotImplementedOnThisPlatform, "network diagnostics need Windows")
            .with_user_text(text!("network.diag.windowsOnly", "Network diagnostics are only available on Windows.")))
    }
    impl NetworkDiagnostics for Unsupported {
        fn ping(&self, _: &PingRequest) -> Result<PingResult> {
            no()
        }
        fn traceroute(&self, _: &TracerouteRequest) -> Result<Vec<bitbybit_network::diag::TracerouteHop>> {
            no()
        }
        fn check_gateway(&self, _: u64) -> Result<Vec<bitbybit_network::diag::GatewayProbe>> {
            no()
        }
        fn best_route(&self, _: IpAddr, _: Option<IpAddr>) -> Result<bitbybit_network::diag::BestRouteResult> {
            no()
        }
        fn resolve(&self, _: &HostName) -> Result<bitbybit_network::diag::DnsResult> {
            no()
        }
    }
    (Box::new(Unsupported), no)
}

thread_local! {
    static CANCEL: std::cell::RefCell<Arc<AtomicBool>> = std::cell::RefCell::new(Arc::default());
}

fn to_ui(r: Row) -> NetDiagRow {
    NetDiagRow {
        item: r.item.into(),
        address: r.address.into(),
        time: r.time.into(),
        detail: r.detail.into(),
        status: r.status.into(),
        tone: r.tone,
    }
}

pub fn wire(ui: &AppWindow) {
    let g = ui.global::<NetDiag>();
    g.on_cancel({
        let weak = ui.as_weak();
        move || {
            CANCEL.with(|c| c.borrow().store(true, Ordering::Relaxed));
            if let Some(ui) = weak.upgrade() {
                ui.global::<NetDiag>().set_cancelling(true);
            }
        }
    });
    let weak = ui.as_weak();
    g.on_run(move |kind| {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<NetDiag>();
        if g.get_running() {
            return; // one test at a time
        }
        let target = g.get_target().to_string();
        let rows: Rc<VecModel<NetDiagRow>> = Rc::new(VecModel::default());
        g.set_rows(ModelRc::from(rows));
        g.set_kind(kind);
        g.set_summary("".into());
        g.set_running(true);
        g.set_cancelling(false);
        let cancel = Arc::new(AtomicBool::new(false));
        CANCEL.with(|c| *c.borrow_mut() = cancel.clone());
        let weak = weak.clone();
        std::thread::spawn(move || {
            let (diag, snapshot) = platform();
            let job = Job {
                diag: diag.as_ref(),
                catalog: TextCatalog::default_catalog(),
                cancel: &cancel,
                pause: Duration::from_secs(1),
            };
            let mut emit = |row: Row| {
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let rows = ui.global::<NetDiag>().get_rows();
                    if let Some(rows) = rows.as_any().downcast_ref::<VecModel<NetDiagRow>>() {
                        rows.push(to_ui(row));
                    }
                });
            };
            let summary = job.run(kind, &target, &snapshot, &mut emit);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                let g = ui.global::<NetDiag>();
                g.set_summary(summary.into());
                g.set_running(false);
                g.set_cancelling(false);
            });
        });
    });
}

#[cfg(test)]
mod tests {
    //! Unit tests with a fake platform (no packets are sent).
    use super::*;
    use bitbybit_network::diag::{BestRouteResult, DnsResult, GatewayProbe, TracerouteHop};
    use bitbybit_network::model::{NetworkInterface, OperStatus};
    use std::sync::Mutex;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// Fake network: 10.0.0.9 is three hops away and answers every other echo; "router.lan" resolves to it.
    #[derive(Default)]
    struct Fake {
        sent: Mutex<Vec<IpAddr>>,
        gateway_calls: Mutex<Vec<u64>>,
    }

    impl NetworkDiagnostics for Fake {
        fn ping(&self, r: &PingRequest) -> Result<PingResult> {
            let mut sent = self.sent.lock().unwrap();
            sent.push(r.target);
            let (status, from) = if r.ttl < 3 {
                (ProbeStatus::TtlExpired, Some(ip(&format!("192.168.{}.1", r.ttl))))
            } else if sent.len().is_multiple_of(2) {
                (ProbeStatus::TimedOut, None)
            } else {
                (ProbeStatus::Success, Some(r.target))
            };
            Ok(PingResult {
                target: r.target,
                status,
                round_trip: Some(Duration::from_millis(10 * sent.len() as u64)),
                responded_from: from,
                reply_ttl: Some(60),
                native_status: None,
            })
        }
        fn traceroute(&self, _: &TracerouteRequest) -> Result<Vec<TracerouteHop>> {
            unreachable!("the page traces through ping")
        }
        fn check_gateway(&self, luid: u64) -> Result<Vec<GatewayProbe>> {
            self.gateway_calls.lock().unwrap().push(luid);
            Ok(vec![GatewayProbe {
                gateway: ip("192.168.1.1"),
                reachability: GatewayReachability::Reachable,
                by_neighbor_resolution: true,
                latency: Some(Duration::from_millis(2)),
                mac: Some("AA:BB:CC:DD:EE:FF".into()),
            }])
        }
        fn best_route(&self, _: IpAddr, _: Option<IpAddr>) -> Result<BestRouteResult> {
            unreachable!()
        }
        fn resolve(&self, name: &HostName) -> Result<DnsResult> {
            let found = name.as_str() == "router.lan";
            Ok(DnsResult {
                name: name.as_str().to_owned(),
                status: if found { DnsStatus::Resolved } else { DnsStatus::NameNotFound },
                addresses: if found { vec![ip("10.0.0.9"), ip("fd00::9")] } else { vec![] },
                elapsed: Duration::from_millis(12),
                native_status: (!found).then_some(9003),
            })
        }
    }

    fn snapshot() -> Result<NetworkSnapshot> {
        let iface = |luid, alias: &str, up, gateways: Vec<IpAddr>, dns: Vec<IpAddr>| NetworkInterface {
            luid,
            alias: alias.into(),
            oper_status: if up { OperStatus::Up } else { OperStatus::Down },
            gateways,
            dns_servers: dns,
            ..NetworkInterface::default()
        };
        Ok(NetworkSnapshot {
            captured_at: time::OffsetDateTime::UNIX_EPOCH,
            interfaces: vec![
                iface(1, "Ethernet", true, vec![ip("192.168.1.1")], vec![ip("192.168.1.1")]),
                iface(2, "Wi-Fi", false, vec![ip("192.168.5.1")], vec![ip("192.168.5.1")]),
                iface(3, "VPN", true, vec![], vec![ip("10.8.0.1")]),
            ],
            routes: vec![],
            ras_connections: vec![],
            os_connectivity: None,
            limitations: vec![],
        })
    }

    fn run(fake: &Fake, cancel: &AtomicBool, kind: i32, target: &str) -> (Vec<Row>, String) {
        let job = Job { diag: fake, catalog: TextCatalog::default_catalog(), cancel, pause: Duration::ZERO };
        let mut rows = Vec::new();
        let summary = job.run(kind, target, &snapshot, &mut |r| rows.push(r));
        (rows, summary)
    }

    #[test]
    fn ping_reports_each_reply_and_the_summary() {
        let fake = Fake::default();
        let (rows, summary) = run(&fake, &AtomicBool::new(false), PING, " 10.0.0.9 ");
        assert_eq!(rows.len(), 4);
        assert_eq!((rows[0].time.as_str(), rows[0].detail.as_str(), rows[0].tone), ("10 ms", "60", 4));
        assert_eq!((rows[1].time.as_str(), rows[1].tone), ("", 1), "timeout shows no time");
        assert!(summary.contains("4") && summary.contains("50 %") && summary.contains("30 ms"), "{summary}");
    }

    #[test]
    fn host_names_are_resolved_first_and_bad_input_sends_nothing() {
        let fake = Fake::default();
        let (rows, summary) = run(&fake, &AtomicBool::new(false), TRACE, "Router.LAN");
        assert_eq!(rows.iter().map(|r| r.item.as_str()).collect::<Vec<_>>(), ["1", "2", "3"]);
        assert_eq!(rows[2].address, "10.0.0.9");
        assert!(summary.contains("router.lan (10.0.0.9)"), "{summary}");
        let before = fake.sent.lock().unwrap().len();
        for (kind, target) in [(PING, ""), (TRACE, "   "), (PING, "bad name!"), (DNS, "-x.com"), (PING, "nowhere.lan")]
        {
            let (rows, summary) = run(&fake, &AtomicBool::new(false), kind, target);
            assert!(rows.is_empty() && !summary.is_empty(), "{target:?}");
        }
        assert_eq!(fake.sent.lock().unwrap().len(), before, "no echo for empty, invalid or unresolved targets");
    }

    #[test]
    fn dns_lists_addresses_servers_and_errors() {
        let fake = Fake::default();
        let (rows, summary) = run(&fake, &AtomicBool::new(false), DNS, "router.lan");
        let addresses: Vec<&str> = rows.iter().map(|r| r.address.as_str()).collect();
        // Answers, then the servers of the connected interfaces only (not the Wi-Fi that is down).
        assert_eq!(addresses, ["10.0.0.9", "fd00::9", "192.168.1.1", "10.8.0.1"]);
        assert_eq!(rows[2].detail, "Ethernet");
        assert!(summary.contains("12 ms") && summary.contains('2'), "{summary}");
        let (rows, summary) = run(&fake, &AtomicBool::new(false), DNS, "nowhere.lan");
        assert_eq!(rows.len(), 2, "servers only");
        assert!(summary.contains("NXDOMAIN") && summary.contains("9003"), "{summary}");
        assert!(fake.sent.lock().unwrap().is_empty(), "DNS sends no echo");
    }

    #[test]
    fn gateway_checks_only_connected_interfaces_with_a_gateway() {
        let fake = Fake::default();
        let (rows, summary) = run(&fake, &AtomicBool::new(false), GATEWAY, "");
        assert_eq!(*fake.gateway_calls.lock().unwrap(), [1]);
        assert_eq!(
            (rows[0].item.as_str(), rows[0].detail.as_str(), rows[0].time.as_str(), rows[0].tone),
            ("Ethernet", "AA:BB:CC:DD:EE:FF", "2 ms", 4)
        );
        assert!(summary.contains("1") && !summary.is_empty());
    }

    #[test]
    fn cancel_stops_before_the_next_probe() {
        let fake = Fake::default();
        let cancel = AtomicBool::new(true);
        let (rows, summary) = run(&fake, &cancel, PING, "10.0.0.9");
        assert!(rows.is_empty() && fake.sent.lock().unwrap().is_empty());
        let stopped = TextCatalog::default_catalog().get("network.diag.cancelled", &["x".into()]);
        assert!(summary.ends_with(stopped.trim_start_matches('x')), "{summary}");
        let (rows, _) = run(&fake, &cancel, TRACE, "10.0.0.9");
        assert_eq!(rows.len(), 1, "the hop in flight is shown, then the trace stops");
    }
}
