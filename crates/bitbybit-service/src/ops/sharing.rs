//! `sharing.start` (WinNAT mode: optional Mobile Hotspot, LAN address, firewall rules, DHCP, DNS, forwarding, NAT,
//! and — when sharing a VPN — the WFP leak guard first) and `sharing.stop`. The plan comes from the live network
//! state and the topology validator; a failed step rolls back everything already applied.

use crate::NetworkInventory;
use bitbybit_core::{Error, ErrorCode, LocalizedText, Result, RiskLevel, text};
use bitbybit_ipc::operation::{BrokerOperation, CallContext, Category, Descriptor, RiskTier};
use bitbybit_ipc::plan::OperationPlan;
use bitbybit_journal::Journal;
use bitbybit_network::classify::{Classification, InterfaceKind, classify};
use bitbybit_network::clients::{Client, NeighborEntry};
use bitbybit_network::dhcp::PoolConfig;
use bitbybit_network::leak::{self, LeakGuardInput};
use bitbybit_network::prefix::IpPrefix;
use bitbybit_network::session::{
    ApProvider, HotspotSettings, Passphrase, PlanInput, SharingBackend, SharingMode, SharingSessions, StepAction,
    build_plan,
};
use bitbybit_network::topology::{self, IssueSeverity, SharingPlanInput};
use bitbybit_recovery::RevertHandler;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct SharingContext {
    pub network: Arc<dyn NetworkInventory>,
    /// Access points of bitBybit's own this PC's Wi-Fi driver may offer, best first (probed each time).
    pub access_points: Arc<dyn Fn() -> Vec<ApProvider> + Send + Sync>,
    /// The IPv4 neighbour (ARP) table, for the devices on the shared network.
    pub neighbors: Arc<dyn Fn() -> Vec<NeighborEntry> + Send + Sync>,
    /// DHCP leases of the session's server.
    pub hosts: Arc<crate::sharing::Hosts>,
    /// The share this service started (gone after a restart: start-up recovery reverts dead sessions).
    pub active: Arc<std::sync::Mutex<Option<ActiveShare>>>,
    pub backend: Arc<dyn SharingBackend>,
    pub handlers: Vec<Arc<dyn RevertHandler>>,
    pub journal: Arc<dyn Journal>,
}

impl SharingContext {
    fn sessions<'a>(&'a self, handlers: &'a [&'a dyn RevertHandler]) -> SharingSessions<'a> {
        SharingSessions::new(self.journal.as_ref(), self.backend.as_ref(), handlers)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRequest {
    pub upstream_luid: u64,
    /// Adapter mode: the interface the connection is shared on (unused in hotspot mode).
    #[serde(default)]
    pub downstream_luid: u64,
    /// e.g. "192.168.137.0/24"; none = the first free subnet (adapter mode).
    pub lan: Option<String>,
    /// Hotspot mode: the Windows Mobile Hotspot shares the upstream under this name and password.
    #[serde(default)]
    pub hotspot_name: Option<String>,
    /// Never logged (the audit masks it) nor journaled.
    #[serde(default)]
    pub hotspot_passphrase: Option<String>,
    /// Only for a VPN upstream: shared traffic may leave only through the VPN (WFP, fail closed).
    pub leak_protection: bool,
    /// false ("no Internet sharing"): the devices get a local network only; `upstream_luid` may be 0.
    #[serde(default = "yes")]
    pub allow_internet: bool,
}

fn yes() -> bool {
    true
}

impl StartRequest {
    fn hotspot(&self) -> Option<HotspotSettings> {
        self.hotspot_name.as_ref().map(|ssid| HotspotSettings {
            ssid: ssid.clone(),
            passphrase: Passphrase(self.hotspot_passphrase.clone().unwrap_or_default()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    pub session_id: uuid::Uuid,
    pub lan: String,
}

fn step_text(step: &StepAction) -> LocalizedText {
    match step {
        StepAction::StartAccessPoint { ssid, .. } => {
            text!(
                "sharing.step.accessPoint",
                "Create the Wi-Fi network \"{0}\" with bitBybit's own access point",
                ssid.clone()
            )
        }
        StepAction::StartHotspot { ssid, .. } => {
            text!("sharing.step.hotspot", "Turn on the Wi-Fi hotspot \"{0}\" (Windows Mobile Hotspot)", ssid.clone())
        }
        StepAction::SetLanAddress { address, prefix_len, .. } => {
            text!(
                "sharing.step.lanAddress",
                "Give the shared network the address {0}/{1}",
                address.to_string(),
                prefix_len.to_string()
            )
        }
        StepAction::InstallLeakGuard { filters } => text!(
            "sharing.step.leakGuard",
            "Install {0} bitBybit packet filters: shared devices can only use the VPN; if it drops they are blocked",
            filters.len().to_string()
        ),
        StepAction::AddFirewallRules { rules } => {
            text!(
                "sharing.step.firewall",
                "Add {0} bitBybit firewall rules for the shared network",
                rules.len().to_string()
            )
        }
        StepAction::StartDhcp { pool } => text!(
            "sharing.step.dhcp",
            "Hand out addresses {0}–{1} to connected devices",
            pool.first.to_string(),
            pool.last.to_string()
        ),
        StepAction::StartDns { .. } => text!("sharing.step.dns", "Answer DNS queries of connected devices"),
        StepAction::EnableForwarding { .. } => {
            text!("sharing.step.forwarding", "Turn on IPv4 forwarding on both interfaces")
        }
        StepAction::CreateNat { name, internal } => {
            text!("sharing.step.nat", "Create the NAT {0} for {1}", name.clone(), internal.to_string())
        }
        StepAction::EnableIcs { .. } => text!("sharing.step.ics", "Turn on Internet Connection Sharing"),
    }
}

/// The fixed subnet Windows gives its Mobile Hotspot (it brings its own DHCP and NAT).
const WINDOWS_HOTSPOT_LAN: IpPrefix = IpPrefix::v4_const([192, 168, 137, 0], 24);

/// "No Internet sharing": a local network for the devices (address, firewall rules, DHCP), nothing forwarded.
fn local_only(
    snapshot: &bitbybit_network::model::NetworkSnapshot,
    downstream_luid: u64,
    lan_text: Option<&str>,
) -> Result<(Vec<StepAction>, IpPrefix, Vec<LocalizedText>)> {
    let downstream =
        snapshot.interface(downstream_luid).ok_or_else(|| Error::not_found("Shared interface not found."))?;
    let lan = match lan_text {
        Some(text) => IpPrefix::parse(text, true).ok_or_else(|| Error::invalid("Invalid LAN subnet."))?,
        None => topology::first_free_subnet(snapshot, downstream_luid)
            .ok_or_else(|| Error::new(ErrorCode::PreconditionFailed, "No free LAN subnet."))?,
    };
    let Some(std::net::IpAddr::V4(gateway)) = lan.first_host() else {
        return Err(Error::invalid("The shared network must be IPv4."));
    };
    let steps = build_plan(&PlanInput {
        mode: SharingMode::WinNat,
        hotspot: None,
        upstream_luid: 0,
        downstream_luid,
        lan,
        gateway,
        pool: PoolConfig::for_lan(lan, 3600),
        dns_upstreams: Vec::new(),
        firewall_rules: bitbybit_network::firewall::sharing_rules(&downstream.alias, lan),
        leak_filters: None,
        share_internet: false,
    });
    Ok((steps, lan, Vec::new()))
}

/// The Windows Mobile Hotspot fallback: Windows shares the source itself (its own addresses and DHCP).
fn windows_hotspot(r: &StartRequest, hotspot: HotspotSettings) -> Vec<StepAction> {
    build_plan(&PlanInput {
        mode: SharingMode::WinNat,
        hotspot: Some(hotspot),
        upstream_luid: r.upstream_luid,
        downstream_luid: 0,
        lan: WINDOWS_HOTSPOT_LAN,
        gateway: std::net::Ipv4Addr::new(192, 168, 137, 1),
        pool: None,
        dns_upstreams: Vec::new(),
        firewall_rules: Vec::new(),
        leak_filters: None,
        share_internet: true,
    })
}

/// Steps from the live state; refusals carry the reason shown to the user.
fn build(context: &SharingContext, r: &StartRequest) -> Result<(Vec<StepAction>, IpPrefix, Vec<LocalizedText>)> {
    build_lan(context, r, r.downstream_luid, r.lan.as_deref())
}

/// Adapter mode, or the network behind one of bitBybit's own access points (`downstream` = its interface).
fn build_lan(
    context: &SharingContext,
    r: &StartRequest,
    downstream_luid: u64,
    lan_text: Option<&str>,
) -> Result<(Vec<StepAction>, IpPrefix, Vec<LocalizedText>)> {
    let snapshot = context.network.collect()?;
    let classifications: Vec<Classification> =
        snapshot.interfaces.iter().map(|i| classify(i, snapshot.ras_connection(i))).collect();
    if !r.allow_internet {
        return local_only(&snapshot, downstream_luid, lan_text);
    }
    let routes = bitbybit_network::routes::analyze(&snapshot, &classifications);
    let lan = match lan_text {
        Some(text) => Some(IpPrefix::parse(text, true).ok_or_else(|| Error::invalid("Invalid LAN subnet."))?),
        None => None,
    };
    let report = topology::validate(
        &SharingPlanInput { upstream_luid: r.upstream_luid, downstream_luid, lan },
        &snapshot,
        &classifications,
        &routes,
    );
    if let Some(blocker) = report.issues.iter().find(|i| i.severity == IssueSeverity::Blocker) {
        return Err(
            Error::new(ErrorCode::PreconditionFailed, blocker.code.clone()).with_user_text(blocker.text.clone())
        );
    }
    let (Some(lan), Some(gateway)) = (report.lan, report.gateway) else {
        return Err(Error::new(ErrorCode::PreconditionFailed, "No free LAN subnet."));
    };
    let upstream =
        snapshot.interface(r.upstream_luid).ok_or_else(|| Error::not_found("Upstream interface not found."))?;
    let downstream =
        snapshot.interface(downstream_luid).ok_or_else(|| Error::not_found("Shared interface not found."))?;
    let leak_filters = if r.leak_protection {
        let vpn =
            classifications.iter().find(|c| c.interface_luid == r.upstream_luid).is_some_and(Classification::is_vpn);
        if !vpn {
            return Err(Error::invalid("Leak protection needs a VPN as the Internet source.").with_user_text(text!(
                "sharing.leak.needsVpn",
                "Leak protection is only for sharing a VPN connection."
            )));
        }
        Some(leak::plan(&LeakGuardInput {
            downstream_index: downstream.index,
            vpn_index: upstream.index,
            vpn_luid: upstream.luid,
            guard_dns_forwarder: true,
        }))
    } else {
        None
    };
    let dns_upstreams: Vec<SocketAddr> =
        upstream.dns_servers.iter().map(|ip: &IpAddr| SocketAddr::new(*ip, 53)).collect();
    let steps = build_plan(&PlanInput {
        mode: SharingMode::WinNat,
        hotspot: None,
        upstream_luid: r.upstream_luid,
        downstream_luid,
        lan,
        gateway,
        pool: PoolConfig::for_lan(lan, 3600),
        dns_upstreams,
        firewall_rules: bitbybit_network::firewall::sharing_rules(&downstream.alias, lan),
        leak_filters,
        share_internet: true,
    });
    let warnings = report.issues.into_iter().filter(|i| i.severity == IssueSeverity::Warning).map(|i| i.text).collect();
    Ok((steps, lan, warnings))
}

pub struct StartOperation(Descriptor, SharingContext);

impl StartOperation {
    pub fn new(context: SharingContext) -> Self {
        Self(
            Descriptor {
                name: "sharing.start",
                tier: RiskTier::Standard,
                category: Category::NETWORK,
                requires_confirmation: true,
                timeout: Some(Duration::from_secs(300)),
            },
            context,
        )
    }
}

impl BrokerOperation for StartOperation {
    type Request = StartRequest;
    type Response = Started;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, r: &StartRequest) -> Result<()> {
        if !r.allow_internet && r.leak_protection {
            return Err(Error::invalid("Leak protection needs Internet sharing."));
        }
        if let Some(hotspot) = r.hotspot() {
            if r.upstream_luid == 0 && r.allow_internet {
                return Err(Error::invalid("Choose the Internet source."));
            }
            return hotspot.validate();
        }
        if r.hotspot_passphrase.is_some() {
            return Err(Error::invalid("A hotspot password needs a hotspot name."));
        }
        if (r.allow_internet && r.upstream_luid == 0) || r.downstream_luid == 0 || r.upstream_luid == r.downstream_luid
        {
            return Err(Error::invalid("Choose two different interfaces."));
        }
        match &r.lan {
            Some(t) if t.len() > 43 || IpPrefix::parse(t, true).is_none() => Err(Error::invalid("Invalid LAN subnet.")),
            _ => Ok(()),
        }
    }
    fn plan(&self, r: &StartRequest, _: &CallContext) -> Result<Option<OperationPlan>> {
        if let Some(hotspot) = r.hotspot() {
            return Ok(Some(hotspot_plan(&self.1, r, &hotspot)?));
        }
        let (steps, lan, warnings) = build(&self.1, r)?;
        let mut plan = OperationPlan::new(
            text!("sharing.plan.summary", "Share the Internet connection on the network {0}", lan.to_string()),
            steps.iter().map(step_text).collect(),
            RiskLevel::Medium,
        );
        plan.may_interrupt_connectivity = true;
        plan.warnings = warnings;
        Ok(Some(plan))
    }
    fn execute(&self, r: &StartRequest, _: &CallContext) -> Result<Started> {
        let handlers: Vec<&dyn RevertHandler> =
            self.1.handlers.iter().map(|h| h.as_ref() as &dyn RevertHandler).collect();
        let sessions = self.1.sessions(&handlers);
        let now = time::OffsetDateTime::now_utc();
        if let Some(hotspot) = r.hotspot() {
            return start_hotspot(&self.1, &sessions, r, hotspot, now);
        }
        let (steps, lan, _) = build(&self.1, r)?;
        let session_id = sessions.start(&steps, now).map_err(|f| failure(&f))?;
        set_active(
            &self.1,
            Some(ActiveShare {
                session_id,
                upstream_luid: r.upstream_luid,
                downstream_luid: r.downstream_luid,
                lan: lan.to_string(),
                mode: "adapter".into(),
                ssid: None,
                leak_protection: r.leak_protection,
            }),
        );
        Ok(Started { session_id, lan: lan.to_string() })
    }
}

fn failure(f: &bitbybit_network::session::StartFailure) -> Error {
    let mut e = Error::new(
        f.error.code,
        format!(
            "Step {} failed: {}; {} applied step(s) were rolled back.",
            f.failed_step + 1,
            f.error.message,
            f.rollback.len()
        ),
    );
    e.user_text.clone_from(&f.error.user_text);
    e
}

/// What the confirmation shows for the hotspot: bitBybit's own access point and network first, the Windows Mobile
/// Hotspot only as the fallback (never with leak protection, which needs bitBybit's own network).
fn hotspot_plan(context: &SharingContext, r: &StartRequest, hotspot: &HotspotSettings) -> Result<OperationPlan> {
    let snapshot = context.network.collect()?;
    let vpn = match snapshot.interface(r.upstream_luid) {
        Some(upstream) => classify(upstream, snapshot.ras_connection(upstream)).is_vpn(),
        None if !r.allow_internet => false,
        None => return Err(Error::not_found("Internet source not found.")),
    };
    if r.leak_protection && !vpn {
        return Err(Error::invalid("Leak protection needs a VPN as the Internet source.")
            .with_user_text(text!("sharing.leak.needsVpn", "Leak protection is only for sharing a VPN connection.")));
    }
    let mut steps = vec![
        text!(
            "sharing.step.accessPoint",
            "Create the Wi-Fi network \"{0}\" with bitBybit's own access point",
            hotspot.ssid.clone()
        ),
        text!(
            "sharing.step.apEngine",
            "Give connected devices addresses, DNS and Internet through bitBybit's own network (NAT, DHCP, firewall rules)"
        ),
    ];
    if !r.allow_internet {
        steps[1] = text!(
            "sharing.step.apLocalOnly",
            "Give connected devices addresses on a local network of their own, without Internet access"
        );
    } else if r.leak_protection {
        steps.push(text!(
            "sharing.step.apLeakGuard",
            "Install bitBybit packet filters: devices on the hotspot can only use the VPN; if it drops they are blocked"
        ));
    }
    if r.allow_internet && !r.leak_protection {
        steps.push(text!(
            "sharing.step.apFallback",
            "If this Wi-Fi card offers no access point of its own, use the Windows Mobile Hotspot instead"
        ));
    }
    let mut plan = OperationPlan::new(
        text!(
            "sharing.plan.hotspotSummary",
            "Share the Internet connection through the Wi-Fi hotspot \"{0}\"",
            hotspot.ssid.clone()
        ),
        steps,
        RiskLevel::Medium,
    );
    plan.may_interrupt_connectivity = true;
    plan.warnings.push(text!(
        "sharing.hotspot.keepsSettings",
        "The name and password become the Windows Mobile Hotspot settings; stopping turns the hotspot off and keeps them."
    ));
    if vpn && !r.leak_protection {
        plan.warnings.push(text!(
            "sharing.hotspot.noLeakGuard",
            "Leak protection is not available with the hotspot: if the VPN drops, devices on the hotspot use the normal connection."
        ));
    }
    Ok(plan)
}

/// The access point's interface once it is up (bounded wait: the virtual adapter appears a moment after start).
fn ap_interface(context: &SharingContext) -> Result<u64> {
    use bitbybit_network::classify::InterfaceKind;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = context.network.collect()?;
        let found = snapshot.interfaces.iter().find(|i| {
            i.is_up()
                && matches!(
                    classify(i, snapshot.ras_connection(i)).kind,
                    InterfaceKind::WifiDirectVirtual | InterfaceKind::HostedNetworkVirtual
                )
        });
        if let Some(i) = found {
            return Ok(i.luid);
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new(ErrorCode::Timeout, "The access point's network interface did not appear."));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn start_hotspot(
    context: &SharingContext,
    sessions: &SharingSessions<'_>,
    r: &StartRequest,
    hotspot: HotspotSettings,
    now: time::OffsetDateTime,
) -> Result<Started> {
    let mut last = None;
    for provider in (context.access_points)() {
        let step = StepAction::StartAccessPoint {
            provider,
            ssid: hotspot.ssid.clone(),
            passphrase: hotspot.passphrase.clone(),
        };
        let session = match sessions.start(std::slice::from_ref(&step), now) {
            Ok(session) => session,
            Err(f) => {
                log::warn!("Access point {provider:?} did not start: {}", f.error.message);
                last = Some(f.error);
                continue;
            }
        };
        // The network behind the AP; any failure from here rolls the whole session back (AP included).
        let lan = ap_interface(context).and_then(|downstream| {
            let request = StartRequest { downstream_luid: downstream, ..r.clone() };
            let (steps, lan, _) = build_lan(context, &request, downstream, None)?;
            sessions.extend(session, &steps, now).map_err(|f| failure(&f))?;
            Ok((lan, downstream))
        });
        return match lan {
            Ok((lan, downstream)) => {
                set_active(
                    context,
                    Some(ActiveShare {
                        session_id: session,
                        upstream_luid: r.upstream_luid,
                        downstream_luid: downstream,
                        lan: lan.to_string(),
                        mode: "accessPoint".into(),
                        ssid: Some(hotspot.ssid.clone()),
                        leak_protection: r.leak_protection,
                    }),
                );
                Ok(Started { session_id: session, lan: lan.to_string() })
            }
            Err(e) => {
                let _ = sessions.stop(session);
                Err(e)
            }
        };
    }
    if !r.allow_internet {
        return Err(last.unwrap_or_else(|| Error::new(ErrorCode::NotSupported, "No access point of bitBybit's own.")).with_user_text(text!(
            "sharing.hotspot.localNeedsOwnAp",
            "This Wi-Fi card offers no access point bitBybit can run itself; the Windows Mobile Hotspot always shares the Internet, so a local-only network is not possible."
        )));
    }
    if r.leak_protection {
        return Err(last.unwrap_or_else(|| Error::new(ErrorCode::NotSupported, "No access point of bitBybit's own.")).with_user_text(text!(
            "sharing.hotspot.needsOwnAp",
            "This Wi-Fi card offers no access point bitBybit can run itself, so leak protection is not possible; the Windows Mobile Hotspot cannot block traffic when the VPN drops."
        )));
    }
    let ssid = hotspot.ssid.clone();
    let steps = windows_hotspot(r, hotspot);
    let session_id = sessions.start(&steps, now).map_err(|f| failure(&f))?;
    set_active(
        context,
        Some(ActiveShare {
            session_id,
            upstream_luid: r.upstream_luid,
            downstream_luid: 0,
            lan: WINDOWS_HOTSPOT_LAN.to_string(),
            mode: "windowsHotspot".into(),
            ssid: Some(ssid),
            leak_protection: false,
        }),
    );
    Ok(Started { session_id, lan: WINDOWS_HOTSPOT_LAN.to_string() })
}

/// What is being shared right now: the diagram's chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveShare {
    pub session_id: uuid::Uuid,
    pub upstream_luid: u64,
    /// Interface the devices are on (0: the Windows Mobile Hotspot's, found when asked).
    pub downstream_luid: u64,
    pub lan: String,
    /// "accessPoint" (bitBybit's own AP), "windowsHotspot" or "adapter".
    pub mode: String,
    pub ssid: Option<String>,
    pub leak_protection: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareStatus {
    pub active: Option<ActiveShare>,
    pub clients: Vec<Client>,
}

fn set_active(context: &SharingContext, share: Option<ActiveShare>) {
    *context.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = share;
}

/// `sharing.status`: the active share and the devices on it (our DHCP leases + the neighbour table). Read-only.
pub struct StatusOperation(Descriptor, SharingContext);

impl StatusOperation {
    pub fn new(context: SharingContext) -> Self {
        Self(
            Descriptor {
                name: "sharing.status",
                tier: RiskTier::ReadOnly,
                category: Category::NONE,
                requires_confirmation: false,
                timeout: Some(Duration::from_secs(30)),
            },
            context,
        )
    }
}

impl BrokerOperation for StatusOperation {
    type Request = crate::NoParameters;
    type Response = ShareStatus;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, _: &Self::Request) -> Result<()> {
        Ok(())
    }
    fn execute(&self, _: &Self::Request, _: &CallContext) -> Result<ShareStatus> {
        let active = self.1.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let Some(share) = active else { return Ok(ShareStatus { active: None, clients: Vec::new() }) };
        let mut downstream = share.downstream_luid;
        if downstream == 0 {
            let snapshot = self.1.network.collect()?;
            downstream = snapshot
                .interfaces
                .iter()
                .find(|i| classify(i, snapshot.ras_connection(i)).kind == InterfaceKind::WifiDirectVirtual)
                .map_or(0, |i| i.luid);
        }
        let lan = IpPrefix::parse(&share.lan, true).unwrap_or(WINDOWS_HOTSPOT_LAN);
        let clients =
            bitbybit_network::clients::merge(lan, downstream, &self.1.hosts.leases(), &(self.1.neighbors)(), &[]);
        Ok(ShareStatus { active: Some(share), clients })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StopRequest {
    pub session_id: uuid::Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stopped {
    pub reverted: usize,
}

pub struct StopOperation(Descriptor, SharingContext);

impl StopOperation {
    pub fn new(context: SharingContext) -> Self {
        Self(
            Descriptor {
                name: "sharing.stop",
                tier: RiskTier::Standard,
                category: Category::NETWORK,
                requires_confirmation: false,
                timeout: Some(Duration::from_secs(300)),
            },
            context,
        )
    }
}

impl BrokerOperation for StopOperation {
    type Request = StopRequest;
    type Response = Stopped;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, _: &StopRequest) -> Result<()> {
        Ok(())
    }
    fn execute(&self, r: &StopRequest, _: &CallContext) -> Result<Stopped> {
        let handlers: Vec<&dyn RevertHandler> =
            self.1.handlers.iter().map(|h| h.as_ref() as &dyn RevertHandler).collect();
        let reverted = self.1.sessions(&handlers).stop(r.session_id)?.len();
        let mut active = self.1.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.as_ref().is_some_and(|a| a.session_id == r.session_id) {
            *active = None;
        }
        Ok(Stopped { reverted })
    }
}
