//! Who may call which tier: only resolved, local clients of at least Medium integrity whose image is an installed
//! bitBybit binary; the Elevated tier also needs an elevated token. Developer mode (off by default, enabled only by
//! an admin-writable machine config) additionally allows images under one explicit, non-root directory.

use crate::identity::{ClientIdentity, IntegrityLevel};
use crate::operation::RiskTier;
use crate::winpath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    /// For the audit log.
    pub reason: String,
    pub elevation_would_help: bool,
}

fn deny(reason: impl Into<String>, elevation_would_help: bool) -> Decision {
    Decision { allowed: false, reason: reason.into(), elevation_would_help }
}

pub struct AuthorizationPolicy {
    allowed_images: Vec<String>,
    developer_directory: Option<String>,
}

impl AuthorizationPolicy {
    /// `allowed_images`: full paths of the installed client binaries.
    pub fn new(allowed_images: &[&str], developer_directory: Option<&str>) -> Result<Self, String> {
        let allowed_images = allowed_images
            .iter()
            .map(|p| winpath::normalize(p).ok_or_else(|| format!("invalid allowed image path '{p}'")))
            .collect::<Result<_, _>>()?;
        let developer_directory = match developer_directory {
            None => None,
            Some(d) => Some(
                winpath::normalize(d)
                    .filter(|n| !winpath::is_drive_root(n))
                    .ok_or("developer mode directory must be a normalized, non-root directory")?,
            ),
        };
        Ok(Self { allowed_images, developer_directory })
    }

    pub fn developer_mode(&self) -> bool {
        self.developer_directory.is_some()
    }

    pub fn authorize(&self, client: &ClientIdentity, tier: RiskTier) -> Decision {
        if !client.is_resolved() {
            return deny(
                format!("client identity unresolved: {}", client.resolution_error.as_deref().unwrap_or("incomplete")),
                false,
            );
        }
        if !client.is_local {
            return deny("remote clients are not allowed", false);
        }
        if client.integrity < IntegrityLevel::Medium {
            return deny(format!("integrity level {:?} is below Medium", client.integrity), false);
        }
        let Some(image) = client.image_path.as_deref().and_then(winpath::normalize) else {
            return deny("client image path is not valid", false);
        };
        let how = if self.allowed_images.iter().any(|a| winpath::equals(a, &image)) {
            "installed image"
        } else if self.developer_directory.as_deref().is_some_and(|d| winpath::is_under(&image, d)) {
            "developer mode"
        } else {
            return deny("client image is not an installed bitBybit binary", false);
        };
        if tier == RiskTier::Elevated && !client.is_elevated {
            return deny("elevated tier requires an elevated client token", true);
        }
        Decision { allowed: true, reason: format!("{tier:?} allowed ({how})"), elevation_would_help: false }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const APP: &str = r"C:\Program Files\bitBybit\bitBybit.exe";

    pub fn client(image: &str, elevated: bool) -> ClientIdentity {
        ClientIdentity {
            process_id: 4242,
            image_path: Some(image.into()),
            user_sid: Some("S-1-5-21-1-2-3-1001".into()),
            is_elevated: elevated,
            integrity: if elevated { IntegrityLevel::High } else { IntegrityLevel::Medium },
            session_id: Some(1),
            is_local: true,
            resolution_error: None,
        }
    }

    fn policy() -> AuthorizationPolicy {
        AuthorizationPolicy::new(&[APP, r"C:\Program Files\bitBybit\bitbybit-cli.exe"], None).unwrap()
    }

    #[test]
    fn allows_installed_app_for_read_only_and_standard() {
        let p = policy();
        assert!(p.authorize(&client(APP, false), RiskTier::ReadOnly).allowed);
        assert!(p.authorize(&client(r"c:\program files\BITBYBIT\bitbybit.EXE", false), RiskTier::Standard).allowed);
    }

    #[test]
    fn elevated_tier_requires_elevated_token() {
        let d = policy().authorize(&client(APP, false), RiskTier::Elevated);
        assert!(!d.allowed && d.elevation_would_help);
        assert!(policy().authorize(&client(APP, true), RiskTier::Elevated).allowed);
    }

    #[test]
    fn denies_other_images() {
        for image in [
            r"C:\Users\x\Downloads\bitBybit.exe",
            r"C:\Program Files\bitBybit\..\evil\bitBybit.exe",
            r"\\server\share\bitBybit.exe",
            r"C:\Program Files\bitBybit\bitBybit.exe.evil",
        ] {
            assert!(!policy().authorize(&client(image, true), RiskTier::ReadOnly).allowed, "{image}");
        }
    }

    #[test]
    fn denies_unresolved_remote_and_low_integrity_clients() {
        let p = policy();
        assert!(!p.authorize(&ClientIdentity::unresolved("no token", 9), RiskTier::ReadOnly).allowed);
        assert!(!p.authorize(&ClientIdentity { is_local: false, ..client(APP, false) }, RiskTier::ReadOnly).allowed);
        assert!(
            !p.authorize(&ClientIdentity { integrity: IntegrityLevel::Low, ..client(APP, false) }, RiskTier::ReadOnly)
                .allowed
        );
    }

    #[test]
    fn developer_mode_allows_only_its_directory() {
        assert!(AuthorizationPolicy::new(&[APP], Some(r"C:\")).is_err());
        let p = AuthorizationPolicy::new(&[APP], Some(r"C:\src\bitBybit\target")).unwrap();
        assert!(p.developer_mode());
        assert!(p.authorize(&client(r"C:\src\bitBybit\target\debug\bitBybit.exe", false), RiskTier::Standard).allowed);
        assert!(!p.authorize(&client(r"C:\src\bitBybit\targetx\bitBybit.exe", false), RiskTier::Standard).allowed);
    }
}
