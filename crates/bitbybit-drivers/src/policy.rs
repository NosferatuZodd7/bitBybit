//! Device-specific installation restrictions (`HKLM\SOFTWARE\Policies\Microsoft\Windows\DeviceInstall\Restrictions`).
//! Only precise hardware ids (bus + vendor + device), never class-wide, compatible-style or wildcard ids; never
//! ids that would block GPU HDMI/DisplayPort audio (NVIDIA, AMD/ATI, Intel HD Audio functions). Entries are
//! added under the next free numeric value names; existing (possibly GPO-managed) entries are never removed or
//! renumbered; `DenyDeviceIDsRetroactive` is never touched. The revert removes only bitBybit's entries whose
//! data still match and clears the flag only if bitBybit set it and no other entries remain.

use bitbybit_core::{Error, LocalizedText, Result, text};
use bitbybit_domain::snapshot::{DenyDeviceIdEntry, DeviceInstallRestrictionsState};
use bitbybit_recovery::LiveState;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

pub const MAX_HARDWARE_ID_LENGTH: usize = 200;
pub const MAX_IDS_PER_PLAN: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyOperationKind {
    SetDenyFlag,
    AddEntry,
    RemoveEntry,
    ClearDenyFlag,
}

/// One explicit registry operation; the Windows store executes exactly these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyOperation {
    pub kind: PolicyOperationKind,
    pub value_name: Option<String>,
    pub hardware_id: Option<String>,
}

impl PolicyOperation {
    fn flag(kind: PolicyOperationKind) -> Self {
        Self { kind, value_name: None, hardware_id: None }
    }

    fn entry(kind: PolicyOperationKind, entry: &DenyDeviceIdEntry) -> Self {
        Self { kind, value_name: Some(entry.value_name.clone()), hardware_id: Some(entry.hardware_id.clone()) }
    }
}

/// What bitBybit added; stored in the journal so the revert removes only bitBybit's own entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedDenyPolicy {
    pub added_entries: Vec<DenyDeviceIdEntry>,
    /// True when bitBybit set `DenyDeviceIDs=1` (it was absent or 0 before).
    #[serde(default)]
    pub set_deny_flag: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyPolicyPlan {
    pub operations: Vec<PolicyOperation>,
    pub owned: OwnedDenyPolicy,
}

fn re(pattern: &str) -> Regex {
    #[allow(clippy::expect_used)] // literal patterns, covered by tests
    Regex::new(pattern).expect("policy pattern compiles")
}

static HD_AUDIO: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)^(?:HDAUDIO|INTELAUDIO)\\FUNC_[0-9A-F]{2}&VEN_(?P<ven>[0-9A-F]{4})&DEV_[0-9A-F]{4}(?:&[A-Z0-9_]+)*$")
});
static PRECISE: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        re(r"(?i)^PCI\\VEN_[0-9A-F]{4}&DEV_[0-9A-F]{4}(?:&[A-Z0-9_]+)*$"),
        re(r"(?i)^USB\\VID_[0-9A-F]{4}&PID_[0-9A-F]{4}(?:&[A-Z0-9_]+)*$"),
        re(r"(?i)^SWC\\VEN_[0-9A-F]{4}(?:&[A-Z0-9_]+)+$"),
    ]
});

/// GPU vendors whose HD Audio function carries HDMI/DisplayPort audio.
const DISPLAY_AUDIO_VENDORS: [&str; 3] = ["10DE", "1002", "8086"];

fn invalid(id: &str, user_text: LocalizedText) -> Error {
    Error::invalid(format!("Hardware id rejected: {id}")).with_user_text(user_text)
}

pub fn validate_hardware_id(id: &str) -> Result<()> {
    if id.trim().is_empty()
        || id.len() > MAX_HARDWARE_ID_LENGTH
        || id != id.trim()
        || id.chars().any(|c| c.is_control() || matches!(c, '*' | '?' | '"' | '/'))
    {
        return Err(invalid(id, text!("drivers.policy.invalidId", "'{0}' is not a valid, specific hardware ID.", id)));
    }
    if let Some(m) = HD_AUDIO.captures(id) {
        if DISPLAY_AUDIO_VENDORS.iter().any(|v| v.eq_ignore_ascii_case(&m["ven"])) {
            return Err(invalid(
                id,
                text!(
                    "drivers.policy.displayAudio",
                    "'{0}' belongs to graphics-card HDMI/DisplayPort audio (NVIDIA/AMD/Intel). bitBybit never blocks it.",
                    id
                ),
            ));
        }
        return Ok(());
    }
    if PRECISE.iter().any(|r| r.is_match(id)) {
        return Ok(());
    }
    Err(invalid(
        id,
        text!(
            "drivers.policy.tooGeneric",
            "'{0}' is too generic (it must name the bus, vendor and device) and could block other devices.",
            id
        ),
    ))
}

pub fn plan_deny(current: &DeviceInstallRestrictionsState, hardware_ids: &[String]) -> Result<DenyPolicyPlan> {
    if hardware_ids.is_empty() || hardware_ids.len() > MAX_IDS_PER_PLAN {
        return Err(Error::invalid("Between 1 and 16 hardware ids are required."));
    }
    for id in hardware_ids {
        validate_hardware_id(id)?;
    }
    let mut used: Vec<String> = current.entries.iter().map(|e| e.value_name.to_lowercase()).collect();
    let existing: Vec<String> = current.entries.iter().map(|e| e.hardware_id.to_lowercase()).collect();
    let (mut operations, mut added, mut seen) = (Vec::new(), Vec::new(), Vec::<String>::new());
    let mut next = 1u32;
    for id in hardware_ids {
        let lower = id.to_lowercase();
        if seen.contains(&lower) || existing.contains(&lower) {
            continue; // already denied (by bitBybit earlier or by an administrator/GPO): nothing to add or own
        }
        seen.push(lower);
        while used.contains(&next.to_string()) {
            next += 1;
        }
        used.push(next.to_string());
        let entry = DenyDeviceIdEntry { value_name: next.to_string(), hardware_id: id.clone() };
        operations.push(PolicyOperation::entry(PolicyOperationKind::AddEntry, &entry));
        added.push(entry);
    }
    let set_deny_flag = !current.policy_enabled();
    if set_deny_flag {
        operations.insert(0, PolicyOperation::flag(PolicyOperationKind::SetDenyFlag));
    }
    Ok(DenyPolicyPlan { operations, owned: OwnedDenyPolicy { added_entries: added, set_deny_flag } })
}

fn same(a: &DenyDeviceIdEntry, b: &DenyDeviceIdEntry) -> bool {
    a.value_name.eq_ignore_ascii_case(&b.value_name) && a.hardware_id.eq_ignore_ascii_case(&b.hardware_id)
}

pub fn plan_revert(current: &DeviceInstallRestrictionsState, owned: &OwnedDenyPolicy) -> Vec<PolicyOperation> {
    let mut remaining = current.entries.clone();
    let mut operations = Vec::new();
    for mine in &owned.added_entries {
        if let Some(i) = remaining.iter().position(|e| same(e, mine)) {
            operations.push(PolicyOperation::entry(PolicyOperationKind::RemoveEntry, &remaining.remove(i)));
        }
    }
    if owned.set_deny_flag && current.policy_enabled() && remaining.is_empty() {
        operations.push(PolicyOperation::flag(PolicyOperationKind::ClearDenyFlag));
    }
    operations
}

/// Drift check before reverting.
pub fn compare(current: &DeviceInstallRestrictionsState, owned: &OwnedDenyPolicy) -> LiveState {
    let present = owned.added_entries.iter().filter(|mine| current.entries.iter().any(|e| same(e, mine))).count();
    match present {
        0 if owned.added_entries.is_empty() && owned.set_deny_flag && current.policy_enabled() => {
            LiveState::MatchesApplied
        }
        0 => LiveState::Gone,
        n if n == owned.added_entries.len() => LiveState::MatchesApplied,
        _ => LiveState::Drifted,
    }
}

/// Journal kind shared with the C# app (applied state = `OwnedDenyPolicy` JSON).
pub const DENY_POLICY_KIND: &str = "policy.deviceInstallDeny";

/// The restriction policy key (Windows: HKLM registry, 64-bit view). `apply` executes exactly the operations.
pub trait DenyPolicyStore: Send + Sync {
    fn read(&self) -> Result<DeviceInstallRestrictionsState>;
    fn apply(&self, operations: &[PolicyOperation]) -> Result<()>;
}

/// Denies installing drivers for `hardware_ids`: plans from the live policy, journals what bitBybit will own,
/// applies, and verifies that every added entry is present. Returns the journal record id (none if nothing changed).
pub fn deny(
    store: &dyn DenyPolicyStore,
    journal: &dyn bitbybit_journal::Journal,
    hardware_ids: &[String],
    now: time::OffsetDateTime,
) -> Result<Option<uuid::Uuid>> {
    use bitbybit_journal::{ChangeCategory, ChangeRecord, ChangeStatus};
    let plan = plan_deny(&store.read()?, hardware_ids)?;
    if plan.operations.is_empty() {
        return Ok(None);
    }
    let mut record = ChangeRecord::new(
        ChangeCategory::DevicePolicy,
        DENY_POLICY_KIND,
        "DeviceInstall\\Restrictions",
        format!("Block driver installation for {}", hardware_ids.join(", ")),
        now,
    )
    .with_states(None, Some(serde_json::to_string(&plan.owned)?));
    record.risk = bitbybit_core::RiskLevel::Medium;
    let id = record.id;
    journal.append(record)?;
    if let Err(e) = store.apply(&plan.operations) {
        journal.update_status(id, ChangeStatus::RevertFailed, Some(format!("Apply failed: {}", e.message)))?;
        return Err(e);
    }
    if compare(&store.read()?, &plan.owned) != LiveState::MatchesApplied {
        journal.update_status(id, ChangeStatus::RevertFailed, Some("Applied but not verified.".into()))?;
        return Err(Error::new(
            bitbybit_core::ErrorCode::VerificationFailed,
            "The policy entries are not all present.",
        ));
    }
    Ok(Some(id))
}

/// Removes only bitBybit's own entries (and the flag it set) while they are still what it added.
pub struct DenyPolicyRevert<'a>(pub &'a dyn DenyPolicyStore);

fn owned(record: &bitbybit_journal::ChangeRecord) -> Option<OwnedDenyPolicy> {
    serde_json::from_str(record.applied_state.as_deref()?).ok()
}

impl bitbybit_recovery::RevertHandler for DenyPolicyRevert<'_> {
    fn target_kind(&self) -> &str {
        DENY_POLICY_KIND
    }

    fn compare(&self, record: &bitbybit_journal::ChangeRecord) -> Result<LiveState> {
        let (Some(owned), Ok(current)) = (owned(record), self.0.read()) else { return Ok(LiveState::Unknown) };
        Ok(compare(&current, &owned))
    }

    fn revert(&self, record: &bitbybit_journal::ChangeRecord) -> Result<()> {
        let owned = owned(record).ok_or_else(|| Error::invalid("Journal record has no owned policy state."))?;
        self.0.apply(&plan_revert(&self.0.read()?, &owned))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REALTEK: &str = r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280A2E";

    fn entry(name: &str, id: &str) -> DenyDeviceIdEntry {
        DenyDeviceIdEntry { value_name: name.into(), hardware_id: id.into() }
    }

    fn state(flag: Option<u32>, entries: Vec<DenyDeviceIdEntry>) -> DeviceInstallRestrictionsState {
        DeviceInstallRestrictionsState {
            key_exists: true,
            deny_device_ids_flag: flag,
            deny_device_ids_retroactive_flag: None,
            entries,
        }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn accepts_precise_ids() {
        for id in [
            r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256",
            r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280A2E&REV_1000",
            r"INTELAUDIO\FUNC_01&VEN_10EC&DEV_0295",
            r"PCI\VEN_10EC&DEV_8168",
            r"USB\VID_0BDA&PID_4014",
            r"SWC\VEN_10EC&AID_0001",
        ] {
            assert!(validate_hardware_id(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn rejects_generic_or_display_audio_ids() {
        for (id, key) in [
            (r"HDAUDIO\FUNC_01&VEN_10DE&DEV_0083", "drivers.policy.displayAudio"),
            (r"HDAUDIO\FUNC_01&VEN_1002&DEV_AA01", "drivers.policy.displayAudio"),
            (r"HDAUDIO\FUNC_01&VEN_8086&DEV_280B", "drivers.policy.displayAudio"),
            (r"HDAUDIO\FUNC_01", "drivers.policy.tooGeneric"),
            (r"HDAUDIO\FUNC_01&VEN_10EC", "drivers.policy.tooGeneric"),
            (r"PCI\CC_0403", "drivers.policy.tooGeneric"),
            (r"HDAUDIO\FUNC_01&VEN_10EC&DEV_*", "drivers.policy.invalidId"),
            (r" HDAUDIO\FUNC_01&VEN_10EC&DEV_0256", "drivers.policy.invalidId"),
            ("", "drivers.policy.invalidId"),
        ] {
            assert_eq!(validate_hardware_id(id).unwrap_err().user_text.unwrap().key, key, "{id}");
        }
    }

    #[test]
    fn plans_flag_and_first_free_names_without_touching_foreign_entries() {
        let current = state(Some(1), vec![entry("1", r"USB\VID_1234&PID_5678"), entry("3", r"PCI\VEN_1111&DEV_2222")]);
        let plan = plan_deny(&current, &ids(&[REALTEK, r"SWC\VEN_10EC&AID_0001"])).unwrap();
        assert_eq!(
            plan.operations,
            [
                PolicyOperation::entry(PolicyOperationKind::AddEntry, &entry("2", REALTEK)),
                PolicyOperation::entry(PolicyOperationKind::AddEntry, &entry("4", r"SWC\VEN_10EC&AID_0001")),
            ]
        );
        assert!(!plan.owned.set_deny_flag);
    }

    #[test]
    fn sets_the_flag_only_when_needed_and_skips_already_denied_ids() {
        let plan =
            plan_deny(&DeviceInstallRestrictionsState::default(), &ids(&[REALTEK, &REALTEK.to_lowercase()])).unwrap();
        assert_eq!(plan.operations[0].kind, PolicyOperationKind::SetDenyFlag);
        assert_eq!(plan.owned.added_entries.len(), 1);
        assert!(plan.owned.set_deny_flag);
        let already = state(Some(1), vec![entry("1", REALTEK)]);
        assert!(plan_deny(&already, &ids(&[REALTEK])).unwrap().operations.is_empty());
    }

    #[test]
    fn refuses_the_whole_plan_if_any_id_is_unsafe() {
        assert!(
            plan_deny(
                &DeviceInstallRestrictionsState::default(),
                &ids(&[REALTEK, r"HDAUDIO\FUNC_01&VEN_10DE&DEV_0083"])
            )
            .is_err()
        );
        assert!(plan_deny(&DeviceInstallRestrictionsState::default(), &[]).is_err());
    }

    #[test]
    fn revert_removes_only_owned_entries_and_clears_flag_only_if_set_by_bitbybit_and_empty() {
        let owned = OwnedDenyPolicy { added_entries: vec![entry("2", REALTEK)], set_deny_flag: true };
        let with_foreign = state(Some(1), vec![entry("1", r"USB\VID_1234&PID_5678"), entry("2", REALTEK)]);
        assert_eq!(
            plan_revert(&with_foreign, &owned),
            [PolicyOperation::entry(PolicyOperationKind::RemoveEntry, &entry("2", REALTEK))]
        );
        let only_mine = state(Some(1), vec![entry("2", REALTEK)]);
        assert_eq!(plan_revert(&only_mine, &owned).last().unwrap().kind, PolicyOperationKind::ClearDenyFlag);
        let not_our_flag = OwnedDenyPolicy { set_deny_flag: false, ..owned.clone() };
        assert!(!plan_revert(&only_mine, &not_our_flag).iter().any(|o| o.kind == PolicyOperationKind::ClearDenyFlag));
        let replaced = state(Some(1), vec![entry("2", r"PCI\VEN_9999&DEV_9999")]);
        assert!(!plan_revert(&replaced, &owned).iter().any(|o| o.kind == PolicyOperationKind::RemoveEntry));
    }

    #[test]
    fn detects_drift() {
        let owned = OwnedDenyPolicy {
            added_entries: vec![entry("2", REALTEK), entry("3", r"SWC\VEN_10EC&AID_0001")],
            set_deny_flag: false,
        };
        assert_eq!(compare(&state(Some(1), owned.added_entries.clone()), &owned), LiveState::MatchesApplied);
        assert_eq!(compare(&state(Some(1), vec![owned.added_entries[0].clone()]), &owned), LiveState::Drifted);
        assert_eq!(compare(&DeviceInstallRestrictionsState::default(), &owned), LiveState::Gone);
    }

    /// In-memory policy key (unit tests only).
    struct Memory(std::sync::Mutex<DeviceInstallRestrictionsState>);
    impl DenyPolicyStore for Memory {
        fn read(&self) -> Result<DeviceInstallRestrictionsState> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn apply(&self, operations: &[PolicyOperation]) -> Result<()> {
            let mut s = self.0.lock().unwrap();
            s.key_exists = true;
            for op in operations {
                match op.kind {
                    PolicyOperationKind::SetDenyFlag => s.deny_device_ids_flag = Some(1),
                    PolicyOperationKind::ClearDenyFlag => s.deny_device_ids_flag = Some(0),
                    PolicyOperationKind::AddEntry => s.entries.push(DenyDeviceIdEntry {
                        value_name: op.value_name.clone().unwrap(),
                        hardware_id: op.hardware_id.clone().unwrap(),
                    }),
                    PolicyOperationKind::RemoveEntry => {
                        s.entries.retain(|e| Some(&e.value_name) != op.value_name.as_ref())
                    }
                }
            }
            Ok(())
        }
    }

    #[test]
    fn deny_is_journaled_verified_and_reverted_keeping_foreign_entries() {
        use bitbybit_journal::JsonLinesJournal;
        use bitbybit_recovery::{RecoveryCoordinator, RevertHandler};
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let gpo = entry("1", r"PCI\VEN_8086&DEV_1234");
        let store = Memory(std::sync::Mutex::new(state(Some(1), vec![gpo.clone()])));
        let id = deny(&store, &journal, &ids(&[REALTEK]), time::OffsetDateTime::UNIX_EPOCH).unwrap().unwrap();
        assert_eq!(store.read().unwrap().entries.len(), 2);
        let handlers: [&dyn RevertHandler; 1] = [&DenyPolicyRevert(&store)];
        RecoveryCoordinator::new(&journal, &handlers).revert(|r| r.id == id, false).unwrap();
        let after = store.read().unwrap();
        assert_eq!(after.entries, vec![gpo], "the administrator's entry stays");
        assert_eq!(after.deny_device_ids_flag, Some(1), "bitBybit did not set the flag, so it keeps it");
        assert!(
            deny(&store, &journal, &ids(&[r"HDAUDIO\FUNC_01&VEN_10DE&DEV_0080"]), time::OffsetDateTime::UNIX_EPOCH)
                .is_err()
        );
    }
}
