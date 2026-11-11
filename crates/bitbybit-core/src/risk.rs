//! Risk of an action. Decides whether explicit confirmation is required.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum RiskLevel {
    #[default]
    None = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

/// Medium risk and above, irreversible actions and actions needing a restart always need explicit confirmation.
pub fn requires_explicit_confirmation(risk: RiskLevel, reversible: bool, requires_reboot: bool) -> bool {
    risk >= RiskLevel::Medium || !reversible || requires_reboot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_rule() {
        assert!(!requires_explicit_confirmation(RiskLevel::Low, true, false));
        assert!(requires_explicit_confirmation(RiskLevel::Medium, true, false));
        assert!(requires_explicit_confirmation(RiskLevel::Low, false, false));
        assert!(requires_explicit_confirmation(RiskLevel::None, true, true));
        assert!(RiskLevel::High > RiskLevel::Medium);
    }
}
