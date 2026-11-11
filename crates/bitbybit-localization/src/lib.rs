//! Resolves [`LocalizedText`] keys against the embedded catalogs (`catalogs/<culture>/<module>.json`, flat
//! `{"key": "text"}` objects, one file per module). Resolution: catalog text, else the English fallback carried
//! by the text itself. Portuguese exists only in the catalog files.

use bitbybit_core::text::{LocalizedText, format_template};
use std::collections::HashMap;
use std::sync::OnceLock;

pub const DEFAULT_CULTURE: &str = "pt-PT";

/// Every catalog file. A test checks this list against the directory, so a new module cannot be forgotten.
const PT_PT: &[(&str, &str)] = &[
    ("app", include_str!("../catalogs/pt-PT/app.json")),
    ("audio", include_str!("../catalogs/pt-PT/audio.json")),
    ("cleanup", include_str!("../catalogs/pt-PT/cleanup.json")),
    ("common", include_str!("../catalogs/pt-PT/common.json")),
    ("devices", include_str!("../catalogs/pt-PT/devices.json")),
    ("drivers", include_str!("../catalogs/pt-PT/drivers.json")),
    ("ipc", include_str!("../catalogs/pt-PT/ipc.json")),
    ("network", include_str!("../catalogs/pt-PT/network.json")),
    ("profiles", include_str!("../catalogs/pt-PT/profiles.json")),
    ("quickdiag", include_str!("../catalogs/pt-PT/quickdiag.json")),
    ("report", include_str!("../catalogs/pt-PT/report.json")),
];

pub struct TextCatalog {
    entries: HashMap<String, String>,
}

impl TextCatalog {
    /// The pt-PT catalog. Built once; a malformed or duplicated entry is a build defect caught by the tests, so
    /// here such entries are skipped instead of failing at run time.
    pub fn default_catalog() -> &'static TextCatalog {
        static CATALOG: OnceLock<TextCatalog> = OnceLock::new();
        CATALOG.get_or_init(|| TextCatalog::from_sources(PT_PT).0)
    }

    /// Returns the catalog and the problems found (invalid JSON, duplicate keys).
    fn from_sources(sources: &[(&str, &str)]) -> (Self, Vec<String>) {
        let mut entries = HashMap::new();
        let mut problems = Vec::new();
        for (module, json) in sources {
            match serde_json::from_str::<HashMap<String, String>>(json) {
                Ok(map) => {
                    for (key, value) in map {
                        if entries.insert(key.clone(), value).is_some() {
                            problems.push(format!("duplicate key '{key}' (module {module})"));
                        }
                    }
                }
                Err(e) => problems.push(format!("module {module}: {e}")),
            }
        }
        (Self { entries }, problems)
    }

    pub fn template(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    pub fn resolve(&self, text: &LocalizedText) -> String {
        format_template(self.template(&text.key).unwrap_or(&text.fallback), &text.args)
    }

    /// Text for a key with arguments; the key itself when missing (visible, so a missing entry is noticed).
    pub fn get(&self, key: &str, args: &[String]) -> String {
        format_template(self.template(key).unwrap_or(key), args)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// Resolves with the default catalog.
pub fn resolve(text: &LocalizedText) -> String {
    TextCatalog::default_catalog().resolve(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_core::text;

    #[test]
    fn catalogs_are_valid_and_have_no_duplicate_keys() {
        let (catalog, problems) = TextCatalog::from_sources(PT_PT);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(catalog.entries().count() > 500);
    }

    #[test]
    fn every_catalog_file_is_registered() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("catalogs").join(DEFAULT_CULTURE);
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().trim_end_matches(".json").to_owned())
            .collect();
        on_disk.sort();
        let registered: Vec<&str> = PT_PT.iter().map(|(m, _)| *m).collect();
        assert_eq!(on_disk, registered);
    }

    #[test]
    fn resolves_key_with_arguments() {
        assert_eq!(resolve(&text!("error.notFound", "Not found: {0}", "Ethernet 2")), "Não encontrado: Ethernet 2");
    }

    #[test]
    fn falls_back_to_english_for_unknown_keys() {
        assert_eq!(resolve(&text!("does.not.exist", "Fallback {0}", 42)), "Fallback 42");
    }

    #[test]
    fn entries_are_non_empty_and_keys_well_formed() {
        for (key, value) in TextCatalog::default_catalog().entries() {
            assert!(!value.trim().is_empty(), "empty text for {key}");
            let mut segments = key.split('.');
            let first = segments.next().unwrap();
            assert!(first.starts_with(|c: char| c.is_ascii_lowercase()), "{key}");
            assert!(key.split('.').count() >= 2, "{key}");
            assert!(key.split('.').all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())), "{key}");
        }
    }
}
