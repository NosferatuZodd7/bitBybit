//! Reads the `[Version]` section of an INF (resolving `%token%` values from `[Strings]`) and the hardware ids of
//! its models sections. Text parsing only; enough to list Driver Store packages without pnputil (whose output is
//! localized).

use std::collections::HashMap;
use time::{Date, Month, OffsetDateTime, Time};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InfVersionInfo {
    pub provider: Option<String>,
    pub class_name: Option<String>,
    pub date: Option<OffsetDateTime>,
    pub version: Option<String>,
    pub catalog_file: Option<String>,
}

fn strip_comment(line: &str) -> &str {
    let mut in_quotes = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(value: &str) -> &str {
    value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value)
}

fn section_name(line: &str) -> Option<&str> {
    line.strip_prefix('[')?.strip_suffix(']').map(str::trim)
}

/// Lines of every section, by lower-case section name.
fn sections(text: &str) -> HashMap<String, Vec<String>> {
    let mut sections: HashMap<String, Vec<String>> = HashMap::new();
    let mut current: Option<String> = None;
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = section_name(line) {
            let key = name.to_lowercase();
            sections.entry(key.clone()).or_default();
            current = Some(key);
        } else if let Some(section) = &current {
            sections.entry(section.clone()).or_default().push(line.to_owned());
        }
    }
    sections
}

fn key_values(lines: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once('=').filter(|(k, _)| !k.trim().is_empty()) {
            map.entry(k.trim().to_lowercase()).or_insert_with(|| unquote(v.trim()).to_owned());
        }
    }
    map
}

pub fn parse(text: &str) -> InfVersionInfo {
    let all = sections(text);
    let empty = Vec::new();
    let version = key_values(all.get("version").unwrap_or(&empty));
    let strings = key_values(all.get("strings").unwrap_or(&empty));
    let get = |key: &str| {
        version.get(key).map(|v| {
            v.strip_prefix('%')
                .and_then(|t| t.strip_suffix('%'))
                .filter(|t| !t.is_empty())
                .and_then(|t| strings.get(&t.to_lowercase()).cloned())
                .unwrap_or_else(|| v.clone())
        })
    };
    let (date, ver) = parse_driver_ver(get("driverver").as_deref());
    InfVersionInfo {
        provider: get("provider"),
        class_name: get("class"),
        date,
        version: ver,
        catalog_file: get("catalogfile").or_else(|| get("catalogfile.ntamd64")),
    }
}

/// `"10/19/2023,10.0.19041.3636"` → date (UTC midnight) and version.
pub fn parse_driver_ver(value: Option<&str>) -> (Option<OffsetDateTime>, Option<String>) {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else { return (None, None) };
    let (date, version) = value.split_once(',').map_or((value, ""), |(d, v)| (d.trim(), v.trim()));
    let parsed = (|| {
        let mut parts = date.split('/');
        let month: u8 = parts.next()?.parse().ok()?;
        let day: u8 = parts.next()?.parse().ok()?;
        let year: i32 = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(
            Date::from_calendar_date(year, Month::try_from(month).ok()?, day)
                .ok()?
                .with_time(Time::MIDNIGHT)
                .assume_utc(),
        )
    })();
    (parsed, Some(version.to_owned()).filter(|v| !v.is_empty()))
}

/// Hardware ids the package installs on: ids listed in the models sections named by `[Manufacturer]`
/// (`Intel = Intel, NTamd64.10.0` → `[Intel]` and `[Intel.NTamd64.10.0]`; each line `desc = install, id1, id2…`).
/// Upper case, distinct, at most `max`.
pub fn parse_hardware_ids(text: &str, max: usize) -> Vec<String> {
    let all = sections(text);
    let mut models = Vec::new();
    for entry in all.get("manufacturer").into_iter().flatten() {
        let Some((_, value)) = entry.split_once('=') else { continue };
        let parts: Vec<&str> = value.split(',').map(|p| unquote(p.trim())).filter(|p| !p.is_empty()).collect();
        let Some(first) = parts.first() else { continue };
        models.push(first.to_lowercase());
        models.extend(parts[1..].iter().map(|target| format!("{first}.{target}").to_lowercase()));
    }
    let mut ids: Vec<String> = Vec::new();
    for model in models {
        for entry in all.get(&model).into_iter().flatten() {
            let Some((_, value)) = entry.split_once('=') else { continue };
            for id in value.split(',').skip(1).map(|p| unquote(p.trim())) {
                let upper = id.to_uppercase();
                if id.len() >= 4 && !id.contains('%') && !ids.contains(&upper) {
                    ids.push(upper);
                    if ids.len() >= max {
                        return ids;
                    }
                }
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    const INF: &str = r#"; Copyright (c) Example Corp. ; not a section
[Version]
Signature   = "$WINDOWS NT$"
Class       = MEDIA ; device class
ClassGuid   = {4d36e96c-e325-11ce-bfc1-08002be10318}
Provider    = %ProviderName%
DriverVer   = 09/09/2014,6.16.0.3154
CatalogFile = example.cat

[Strings]
ProviderName = "Example Corp; Audio"
"#;

    #[test]
    fn reads_provider_class_date_and_version_and_resolves_string_tokens() {
        let info = parse(INF);
        assert_eq!(info.provider.as_deref(), Some("Example Corp; Audio"));
        assert_eq!(info.class_name.as_deref(), Some("MEDIA"));
        assert_eq!(info.date, Some(datetime!(2014-09-09 0:00 UTC)));
        assert_eq!(info.version.as_deref(), Some("6.16.0.3154"));
        assert_eq!(info.catalog_file.as_deref(), Some("example.cat"));
    }

    #[test]
    fn tolerates_missing_sections_and_odd_dates() {
        assert_eq!(parse("[Manufacturer]\nx=y"), InfVersionInfo::default());
        assert_eq!(parse_driver_ver(Some("2014-09-09,1.0")), (None, Some("1.0".into())));
        assert_eq!(parse_driver_ver(Some("13/40/2014,1.0")).0, None);
    }

    #[test]
    fn hardware_ids_come_from_the_models_sections() {
        let inf = "[Version]\nClass=MEDIA\n[Manufacturer]\n%Intel% = Intel, NTamd64.10.0\n[Intel.NTamd64.10.0]\n%Desc% = Install, HDAUDIO\\FUNC_01&VEN_8086&DEV_2806, hdaudio\\func_01&ven_8086&dev_2807\n[Install]\nCopyFiles = x ; not a models line\n[Strings]\nIntel = \"Intel\"\n";
        assert_eq!(
            parse_hardware_ids(inf, 300),
            [r"HDAUDIO\FUNC_01&VEN_8086&DEV_2806", r"HDAUDIO\FUNC_01&VEN_8086&DEV_2807"]
        );
        assert_eq!(parse_hardware_ids(inf, 1).len(), 1);
    }
}
