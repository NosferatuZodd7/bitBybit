//! Kernel-PnP configuration events (device and driver installs) as the diagnostic snapshot keeps them: the event
//! XML rendered by the Windows event log is reduced to time, id, provider and the formatted message, only for the
//! audio / display devices of the case.

use bitbybit_domain::snapshot::PnpEventSummary;
use regex::Regex;
use std::sync::LazyLock;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const CHANNEL: &str = "Microsoft-Windows-Kernel-PnP/Configuration";
/// Device configured / started / install problems / driver selected (the C# collector's query).
pub const QUERY: &str = "*[System[(EventID=400 or EventID=410 or EventID=411 or EventID=420 or EventID=430 or EventID=441 or EventID=442)]]";

fn re(pattern: &str) -> Regex {
    #[allow(clippy::expect_used)] // literal patterns, covered by tests
    Regex::new(pattern).expect("event pattern compiles")
}

static EVENT_ID: LazyLock<Regex> = LazyLock::new(|| re(r"<EventID(?:\s[^>]*)?>(\d+)</EventID>"));
static TIME: LazyLock<Regex> = LazyLock::new(|| re(r#"<TimeCreated\s+SystemTime=['"]([^'"]+)['"]"#));
static PROVIDER: LazyLock<Regex> = LazyLock::new(|| re(r#"<Provider\s+Name=['"]([^'"]+)['"]"#));
static FIRST_DATA: LazyLock<Regex> = LazyLock::new(|| re(r"<Data(?:\s[^>]*)?>([^<]*)</Data>"));

pub fn is_audio_relevant_instance(instance: &str) -> bool {
    let upper = instance.to_ascii_uppercase();
    ["HDAUDIO\\", "INTELAUDIO\\", "SWD\\MMDEVAPI", "DISPLAY\\"].iter().any(|p| upper.starts_with(p))
        || upper.contains("VEN_10EC")
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// `None` for events of other devices or without an instance id. `message` is the formatted event text when the
/// publisher's messages could be loaded; otherwise the instance id stands in.
pub fn from_event_xml(xml: &str, message: Option<String>) -> Option<PnpEventSummary> {
    let instance = unescape(FIRST_DATA.captures(xml)?.get(1)?.as_str().trim());
    if !is_audio_relevant_instance(&instance) {
        return None;
    }
    let message = message.filter(|m| !m.trim().is_empty()).unwrap_or(instance);
    Some(PnpEventSummary {
        time: TIME.captures(xml).and_then(|c| OffsetDateTime::parse(&c[1], &Rfc3339).ok()),
        event_id: EVENT_ID.captures(xml).and_then(|c| c[1].parse().ok()).unwrap_or_default(),
        provider: PROVIDER.captures(xml).map(|c| c[1].to_owned()),
        message: message.chars().take(600).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Microsoft-Windows-Kernel-PnP' Guid='{9c205a39-1250-487d-abd7-e831c6290539}'/><EventID>400</EventID><TimeCreated SystemTime='2026-10-10T09:15:02.1234567Z'/></System><EventData><Data Name='DeviceInstanceId'>HDAUDIO\FUNC_01&amp;VEN_10EC&amp;DEV_0897\4&amp;1</Data><Data Name='DriverName'>oem45.inf</Data></EventData></Event>"#;

    #[test]
    fn keeps_audio_events_with_time_id_provider_and_message() {
        let e = from_event_xml(XML, Some("Device HDAUDIO\\FUNC_01&VEN_10EC... Driver Name: oem45.inf".into())).unwrap();
        assert_eq!(e.event_id, 400);
        assert_eq!(e.provider.as_deref(), Some("Microsoft-Windows-Kernel-PnP"));
        assert_eq!(e.time.map(|t| (t.year(), t.hour(), t.second())), Some((2026, 9, 2)));
        assert!(e.message.contains("oem45.inf"));
        let fallback = from_event_xml(XML, None).unwrap();
        assert_eq!(fallback.message, r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897\4&1");
        let usb = XML.replace(r"HDAUDIO\FUNC_01&amp;VEN_10EC&amp;DEV_0897", r"USB\VID_046D&amp;PID_C52B");
        assert!(from_event_xml(&usb, None).is_none());
        assert_eq!(from_event_xml(XML, Some("é".repeat(700))).unwrap().message.chars().count(), 600);
    }
}
