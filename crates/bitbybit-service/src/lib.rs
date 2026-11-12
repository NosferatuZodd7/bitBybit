//! The privileged broker (`bitBybitService`, LocalSystem). This library assembles the closed operation allowlist,
//! the authorization policy and start-up recovery; `main.rs` hosts it as a Windows service (or in a console for
//! development). Every operation re-validates its input; mutating operations go through the dispatcher's
//! two-phase confirmation and the change journal.

use bitbybit_core::Result;
use bitbybit_ipc::auth::AuthorizationPolicy;
use bitbybit_ipc::builtin::{Ping, PollEvents};
use bitbybit_ipc::confirm::TokenService;
use bitbybit_ipc::dispatch::{Dispatcher, DispatcherOptions};
use bitbybit_ipc::events::EventHub;
use bitbybit_ipc::operation::{BrokerOperation, CallContext, Category, Descriptor, RegistryBuilder, RiskTier};
use bitbybit_journal::Journal;
use bitbybit_network::classify::{Classification, classify};
use bitbybit_network::connectivity::{ConnectivityLevel, global};
use bitbybit_network::model::NetworkSnapshot;
use bitbybit_network::routes::{DefaultRoute, VpnRouting, analyze};
use bitbybit_recovery::{RecoveryCoordinator, RevertHandler, RevertOutcome};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// `%ProgramData%\bitBybit\config\broker-developer-mode.json`. Off unless the file says so.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeveloperModeConfig {
    pub enabled: bool,
    /// Client images under this directory are accepted besides the installed ones.
    pub directory: Option<String>,
}

/// The developer directory when developer mode is enabled; a missing or malformed file means off.
/// shortcut: trusts the config folder ACL set by the installer (admins only); verify the owner if that changes.
pub fn developer_directory(config_file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(config_file).ok()?;
    let config: DeveloperModeConfig =
        serde_json::from_str(&text).map_err(|e| log::warn!("Ignoring malformed developer mode config: {e}")).ok()?;
    config.enabled.then_some(config.directory).flatten()
}

/// Read-only network state source (Windows: IP Helper).
pub trait NetworkInventory: Send + Sync {
    fn collect(&self) -> Result<NetworkSnapshot>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoParameters {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkOverview {
    pub snapshot: NetworkSnapshot,
    pub classifications: Vec<Classification>,
    pub connectivity: ConnectivityLevel,
    pub ipv4_default: Option<DefaultRoute>,
    pub ipv6_default: Option<DefaultRoute>,
    pub vpns: Vec<VpnRouting>,
    pub ipv6_bypass: Option<u64>,
}

/// `network.overview`: interfaces, classification, connectivity and route ownership. Changes nothing.
pub struct NetworkOverviewOperation<I> {
    descriptor: Descriptor,
    inventory: I,
}

impl<I: NetworkInventory> NetworkOverviewOperation<I> {
    pub fn new(inventory: I) -> Self {
        Self {
            descriptor: Descriptor {
                name: "network.overview",
                tier: RiskTier::ReadOnly,
                category: Category::NONE,
                requires_confirmation: false,
                timeout: None,
            },
            inventory,
        }
    }
}

impl<I: NetworkInventory> BrokerOperation for NetworkOverviewOperation<I> {
    type Request = NoParameters;
    type Response = NetworkOverview;
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn validate(&self, _: &NoParameters) -> Result<()> {
        Ok(())
    }
    fn execute(&self, _: &NoParameters, _: &CallContext) -> Result<NetworkOverview> {
        let snapshot = self.inventory.collect()?;
        let classifications: Vec<Classification> =
            snapshot.interfaces.iter().map(|i| classify(i, snapshot.ras_connection(i))).collect();
        let routes = analyze(&snapshot, &classifications);
        Ok(NetworkOverview {
            connectivity: global(&snapshot),
            classifications,
            ipv4_default: routes.ipv4_default,
            ipv6_default: routes.ipv6_default,
            vpns: routes.vpns,
            ipv6_bypass: routes.ipv6_bypass,
            snapshot,
        })
    }
}

/// The frozen allowlist and the dispatcher. `allowed_images`: full paths of the installed clients.
pub fn build_dispatcher(
    version: &str,
    allowed_images: &[&str],
    developer_dir: Option<&str>,
    inventory: impl NetworkInventory + 'static,
) -> std::result::Result<(Dispatcher, Arc<EventHub>), String> {
    let policy = AuthorizationPolicy::new(allowed_images, developer_dir)?;
    let events = Arc::new(EventHub::new(1024));
    let ping = Ping::new(version, policy.developer_mode());
    let names = ping.operations.clone();
    let registry = RegistryBuilder::default()
        .register(ping)?
        .register(PollEvents::new(events.clone()))?
        .register(NetworkOverviewOperation::new(inventory))?
        .freeze();
    let _ = names.set(registry.names());
    let tokens = TokenService::new().map_err(|e| format!("no OS random source: {e}"))?;
    Ok((Dispatcher::new(registry, policy, tokens, DispatcherOptions::default()), events))
}

/// At broker start: reverts changes flagged for recovery (no operation survives a restart), but only those a
/// registered handler knows; other records are left untouched for a later version or the user.
pub fn recover_at_start(journal: &dyn Journal, handlers: &[&dyn RevertHandler]) -> Result<Vec<RevertOutcome>> {
    let kinds: Vec<String> = handlers.iter().map(|h| h.target_kind().to_owned()).collect();
    RecoveryCoordinator::new(journal, handlers)
        .revert(|r| r.revert_on_recovery && kinds.contains(&r.target_kind), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_ipc::envelope::{BrokerRequest, ResponseStatus};
    use bitbybit_ipc::identity::{ClientIdentity, IntegrityLevel};
    use bitbybit_journal::{ChangeCategory, ChangeRecord, ChangeStatus, JsonLinesJournal};
    use bitbybit_recovery::LiveState;
    use std::sync::atomic::AtomicBool;

    const APP: &str = r"C:\Program Files\bitBybit\bitBybit.exe";

    struct Fixed;
    impl NetworkInventory for Fixed {
        fn collect(&self) -> Result<NetworkSnapshot> {
            Ok(NetworkSnapshot {
                captured_at: time::OffsetDateTime::UNIX_EPOCH,
                interfaces: vec![],
                routes: vec![],
                ras_connections: vec![],
                os_connectivity: None,
                limitations: vec![],
            })
        }
    }

    fn client(image: &str) -> ClientIdentity {
        ClientIdentity {
            process_id: 7,
            image_path: Some(image.into()),
            user_sid: Some("S-1-5-21-1".into()),
            is_elevated: false,
            integrity: IntegrityLevel::Medium,
            session_id: Some(1),
            is_local: true,
            resolution_error: None,
        }
    }

    #[test]
    fn allowlist_contains_exactly_the_registered_operations() {
        let (d, _) = build_dispatcher("0.7.0", &[APP], None, Fixed).unwrap();
        assert_eq!(d.registry().names(), ["broker.events.poll", "broker.ping", "network.overview"]);
        let call = |op: &str, image: &str| {
            let r = BrokerRequest {
                protocol_version: 1,
                request_id: uuid::Uuid::new_v4(),
                operation: op.into(),
                parameters: serde_json::json!({}),
                confirmation_token: None,
            };
            d.dispatch(&r, &client(image), Arc::new(AtomicBool::new(false)))
        };
        let ok = call("network.overview", APP);
        assert_eq!(ok.status, ResponseStatus::Ok);
        assert_eq!(ok.result.unwrap()["connectivity"], "Unknown");
        assert_eq!(call("network.overview", r"C:\Temp\bitBybit.exe").status, ResponseStatus::Error);
        assert_eq!(call("system.runCommand", APP).status, ResponseStatus::Error);
    }

    #[test]
    fn developer_mode_needs_an_explicit_enabled_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("broker-developer-mode.json");
        assert_eq!(developer_directory(&f), None);
        std::fs::write(&f, r#"{"enabled":false,"directory":"C:\\src"}"#).unwrap();
        assert_eq!(developer_directory(&f), None);
        std::fs::write(&f, r#"{"enabled":true,"directory":"C:\\src","extra":1}"#).unwrap();
        assert_eq!(developer_directory(&f), None, "malformed means off");
        std::fs::write(&f, r#"{"enabled":true,"directory":"C:\\src\\bitBybit"}"#).unwrap();
        assert_eq!(developer_directory(&f).as_deref(), Some(r"C:\src\bitBybit"));
        assert!(build_dispatcher("0.7.0", &[APP], Some(r"C:\"), Fixed).is_err(), "a drive root is refused");
    }

    struct Handler;
    impl RevertHandler for Handler {
        fn target_kind(&self) -> &str {
            "sharing.nat"
        }
        fn compare(&self, _: &ChangeRecord) -> Result<LiveState> {
            Ok(LiveState::MatchesApplied)
        }
        fn revert(&self, _: &ChangeRecord) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn recovery_at_start_leaves_unknown_kinds_alone() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let now = time::OffsetDateTime::UNIX_EPOCH;
        let session = uuid::Uuid::new_v4();
        let known = ChangeRecord::new(ChangeCategory::Network, "sharing.nat", "bitBybit-share", "nat", now)
            .in_operation(session, true);
        let unknown =
            ChangeRecord::new(ChangeCategory::Network, "future.kind", "x", "x", now).in_operation(session, true);
        let (known_id, unknown_id) = (known.id, unknown.id);
        journal.append(known).unwrap();
        journal.append(unknown).unwrap();
        let outcomes = recover_at_start(&journal, &[&Handler]).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(journal.get(known_id).unwrap().unwrap().status, ChangeStatus::Reverted);
        assert_eq!(journal.get(unknown_id).unwrap().unwrap().status, ChangeStatus::Applied);
    }
}
