//! `sharing.start` (WinNAT mode: optional Mobile Hotspot, LAN address, firewall rules, DHCP, DNS, forwarding, NAT,
//! and — when sharing a VPN — the WFP leak guard first) and `sharing.stop`. The plan comes from the live network
//! state and the topology validator; a failed step rolls back everything already applied.

use crate::NetworkInventory;
use bitbybit_core::{Error, ErrorCode, LocalizedText, Result, RiskLevel, text};
use bitbybit_ipc::operation::{BrokerOperation, CallContext, Category, Descriptor, RiskTier};
use bitbybit_ipc::plan::OperationPlan;
use bitbybit_journal::Journal;
use bitbybit_network::classify::{Classification, classify};
use bitbybit_network::dhcp::PoolConfig;
use bitbybit_network::leak::{self, LeakGuardInput};
use bitbybit_network::prefix::IpPrefix;
use bitbybit_network::session::{PlanInput, SharingBackend, SharingMode, SharingSessions, StepAction, build_plan};
use bitbybit_network::topology::{self, IssueSeverity, SharingPlanInput};
use bitbybit_recovery::RevertHandler;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct SharingContext {
    pub network: Arc<dyn NetworkInventory>,
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
    pub downstream_luid: u64,
    /// e.g. "192.168.137.0/24"; none = the first free subnet.
    pub lan: Option<String>,
    pub use_hotspot: bool,
    /// Only for a VPN upstream: shared traffic may leave only through the VPN (WFP, fail closed).
    pub leak_protection: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    pub session_id: uuid::Uuid,
    pub lan: String,
}

fn step_text(step: &StepAction) -> LocalizedText {
    match step {
        StepAction::StartHotspot => text!("sharing.step.hotspot", "Turn on the Mobile Hotspot"),
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

/// Steps from the live state; refusals carry the reason shown to the user.
fn build(context: &SharingContext, r: &StartRequest) -> Result<(Vec<StepAction>, IpPrefix, Vec<LocalizedText>)> {
    let snapshot = context.network.collect()?;
    let classifications: Vec<Classification> =
        snapshot.interfaces.iter().map(|i| classify(i, snapshot.ras_connection(i))).collect();
    let routes = bitbybit_network::routes::analyze(&snapshot, &classifications);
    let lan = match &r.lan {
        Some(text) => Some(IpPrefix::parse(text, true).ok_or_else(|| Error::invalid("Invalid LAN subnet."))?),
        None => None,
    };
    let report = topology::validate(
        &SharingPlanInput { upstream_luid: r.upstream_luid, downstream_luid: r.downstream_luid, lan },
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
        snapshot.interface(r.downstream_luid).ok_or_else(|| Error::not_found("Shared interface not found."))?;
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
        use_hotspot: r.use_hotspot,
        upstream_luid: r.upstream_luid,
        downstream_luid: r.downstream_luid,
        lan,
        gateway,
        pool: PoolConfig::for_lan(lan, 3600),
        dns_upstreams,
        firewall_rules: bitbybit_network::firewall::sharing_rules(&downstream.alias, lan),
        leak_filters,
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
        if r.upstream_luid == 0 || r.downstream_luid == 0 || r.upstream_luid == r.downstream_luid {
            return Err(Error::invalid("Choose two different interfaces."));
        }
        match &r.lan {
            Some(t) if t.len() > 43 || IpPrefix::parse(t, true).is_none() => Err(Error::invalid("Invalid LAN subnet.")),
            _ => Ok(()),
        }
    }
    fn plan(&self, r: &StartRequest, _: &CallContext) -> Result<Option<OperationPlan>> {
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
        let (steps, lan, _) = build(&self.1, r)?;
        let handlers: Vec<&dyn RevertHandler> =
            self.1.handlers.iter().map(|h| h.as_ref() as &dyn RevertHandler).collect();
        match self.1.sessions(&handlers).start(&steps, time::OffsetDateTime::now_utc()) {
            Ok(session_id) => Ok(Started { session_id, lan: lan.to_string() }),
            Err(failure) => Err(Error::new(
                failure.error.code,
                format!(
                    "Step {} failed: {}; {} applied step(s) were rolled back.",
                    failure.failed_step + 1,
                    failure.error.message,
                    failure.rollback.len()
                ),
            )),
        }
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
        Ok(Stopped { reverted: self.1.sessions(&handlers).stop(r.session_id)?.len() })
    }
}
