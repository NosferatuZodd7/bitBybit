//! Who is on the other end of a broker connection, as established by the transport from OS facts (pipe client
//! PID, process image, token). Image-based checks have PID-reuse / same-user-injection limits (docs/SECURITY.md).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Mandatory integrity levels (RID of S-1-16-RID).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum IntegrityLevel {
    Untrusted = 0x0000,
    Low = 0x1000,
    Medium = 0x2000,
    MediumPlus = 0x2100,
    High = 0x3000,
    System = 0x4000,
    Protected = 0x5000,
}

impl IntegrityLevel {
    /// The highest defined level not above `rid` (unknown RIDs round down).
    pub fn from_rid(rid: u32) -> Self {
        use IntegrityLevel::*;
        [Protected, System, High, MediumPlus, Medium, Low].into_iter().find(|l| *l as u32 <= rid).unwrap_or(Untrusted)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIdentity {
    pub process_id: u32,
    /// Full Win32 path of the client image.
    pub image_path: Option<String>,
    pub user_sid: Option<String>,
    pub is_elevated: bool,
    pub integrity: IntegrityLevel,
    pub session_id: Option<u32>,
    /// False for remote clients or tokens carrying the NETWORK SID.
    pub is_local: bool,
    /// Why the identity is incomplete (English, for the audit log).
    pub resolution_error: Option<String>,
}

impl ClientIdentity {
    pub fn is_resolved(&self) -> bool {
        self.resolution_error.is_none()
            && self.process_id > 0
            && self.image_path.as_deref().is_some_and(|p| !p.is_empty())
            && self.user_sid.as_deref().is_some_and(|s| !s.is_empty())
    }

    /// Every tier denies it.
    pub fn unresolved(reason: &str, process_id: u32) -> Self {
        Self {
            process_id,
            image_path: None,
            user_sid: None,
            is_elevated: false,
            integrity: IntegrityLevel::Untrusted,
            session_id: None,
            is_local: false,
            resolution_error: Some(if reason.trim().is_empty() {
                "Identity could not be resolved.".into()
            } else {
                reason.to_owned()
            }),
        }
    }
}

impl fmt::Display for ClientIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "pid={} user={} image={} elevated={} integrity={:?} session={} local={}",
            self.process_id,
            self.user_sid.as_deref().unwrap_or("?"),
            self.image_path.as_deref().unwrap_or("?"),
            self.is_elevated,
            self.integrity,
            self.session_id.map_or_else(|| "?".into(), |s| s.to_string()),
            self.is_local
        )?;
        if let Some(e) = &self.resolution_error {
            write!(f, " error={e}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrity_from_rid_rounds_down() {
        assert_eq!(IntegrityLevel::from_rid(0x2000), IntegrityLevel::Medium);
        assert_eq!(IntegrityLevel::from_rid(0x2fff), IntegrityLevel::MediumPlus);
        assert_eq!(IntegrityLevel::from_rid(0x0fff), IntegrityLevel::Untrusted);
        assert_eq!(IntegrityLevel::from_rid(0x9000), IntegrityLevel::Protected);
        assert!(!ClientIdentity::unresolved("", 5).is_resolved());
    }
}
