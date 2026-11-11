//! Semantic versions as bitBybit publishes them: `0.7.0` (stable) or `0.7.0-dev.12` (testing build, N = CI run).
//! A leading `v` and build metadata (`+abc`) are ignored. `0.7.0-dev.12 < 0.7.0`, so a testing user gets the
//! stable when it is published and a stable user is never moved to a testing build.

use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre_release: Vec<String>,
}

fn number(s: &str) -> Option<u32> {
    (!s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

impl SemVersion {
    pub fn parse(text: &str) -> Option<Self> {
        let s = text.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let s = s.split_once('+').map_or(s, |(core, _)| core);
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let parts: Vec<u32> = core.split('.').map(number).collect::<Option<_>>()?;
        let [major, minor, patch] = parts[..] else { return None };
        let pre_release = match pre {
            None => vec![],
            Some(p) => {
                let ids: Vec<String> = p.split('.').map(str::to_owned).collect();
                if ids.iter().any(|i| i.is_empty() || !i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
                    return None;
                }
                ids
            }
        };
        Some(Self { major, minor, patch, pre_release })
    }

    pub fn is_pre_release(&self) -> bool {
        !self.pre_release.is_empty()
    }
}

impl Ord for SemVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let core = (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch));
        if core != Ordering::Equal {
            return core;
        }
        // No pre-release ranks above any pre-release of the same core version.
        match (self.pre_release.is_empty(), other.pre_release.is_empty()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Greater,
            (false, true) => return Ordering::Less,
            _ => {}
        }
        for (a, b) in self.pre_release.iter().zip(&other.pre_release) {
            let numeric = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
            let c = match (numeric(a), numeric(b)) {
                (true, true) => a.parse::<u64>().unwrap_or(u64::MAX).cmp(&b.parse::<u64>().unwrap_or(u64::MAX)),
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => a.cmp(b),
            };
            if c != Ordering::Equal {
                return c;
            }
        }
        self.pre_release.len().cmp(&other.pre_release.len())
    }
}

impl PartialOrd for SemVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for SemVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if self.is_pre_release() {
            write!(f, "-{}", self.pre_release.join("."))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> SemVersion {
        SemVersion::parse(s).unwrap()
    }

    #[test]
    fn versions_are_ordered_as_semver() {
        for (lower, higher) in [
            ("0.5.0-dev.2", "0.5.0-dev.10"),
            ("0.5.0-dev.10", "0.5.0"),
            ("0.4.0", "0.5.0-dev.1"),
            ("v0.5.0", "0.5.1"),
            ("0.9.9", "0.10.0"),
        ] {
            assert!(v(lower) < v(higher), "{lower} < {higher}");
            assert!(v(higher) > v(lower));
        }
    }

    #[test]
    fn malformed_versions_are_rejected() {
        for bad in ["0.5", "0.5.0-", "0.5.0-dev..1", "x.y.z", "", "0.5.0.1", "1234567890.0.0"] {
            assert!(SemVersion::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn build_metadata_is_ignored() {
        assert_eq!(v("0.5.0-dev.3"), v("0.5.0-dev.3+abc123"));
        assert_eq!(v("V0.7.0-dev.4").to_string(), "0.7.0-dev.4");
    }
}
