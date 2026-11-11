//! bitBybit only changes objects it owns: names with its prefix, its firewall group, its WFP provider and
//! sublayer (fixed GUIDs), or objects recorded in the journal.

/// Tag stored with owned objects and journal records.
pub const OWNER_TAG: &str = "bitBybit";

/// Prefix of every WinNAT instance and firewall rule bitBybit creates.
pub const OWNED_NAME_PREFIX: &str = "bitBybit-";

/// Windows Firewall rule group. Rules outside it are never modified or removed.
pub const FIREWALL_GROUP: &str = "bitBybit";

/// WFP provider key `{6B1F5A2E-3C47-4D8B-9E21-B17B1B170001}`.
pub const WFP_PROVIDER_KEY: u128 = 0x6B1F5A2E_3C47_4D8B_9E21_B17B1B170001;

/// WFP sublayer key `{6B1F5A2E-3C47-4D8B-9E21-B17B1B170002}`.
pub const WFP_SUBLAYER_KEY: u128 = 0x6B1F5A2E_3C47_4D8B_9E21_B17B1B170002;

/// True for a name bitBybit may modify or delete: its prefix followed by a non-empty, tame suffix.
pub fn is_owned_name(name: &str) -> bool {
    name.strip_prefix(OWNED_NAME_PREFIX).is_some_and(|rest| {
        !rest.is_empty() && rest.len() <= 64 && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_prefixed_tame_names_are_owned() {
        assert!(is_owned_name("bitBybit-nat-1"));
        assert!(!is_owned_name("bitBybit-"));
        assert!(!is_owned_name("DockerNAT"));
        assert!(!is_owned_name("bitbybit-nat")); // case matters: the prefix is exact
        assert!(!is_owned_name("bitBybit-a b"));
        assert!(!is_owned_name("bitBybit-*"));
    }
}
