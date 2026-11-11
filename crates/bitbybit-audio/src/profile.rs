//! "DisplayPort Audio Preferred": keep one output as the default device. The endpoint is identified by a durable
//! fingerprint (its id changes when the driver is reinstalled), the reconciler is a bounded state machine that
//! never loops, respects manual choices, refuses ambiguous matches and only ever proposes the low-risk
//! "set default" action. Pure logic: the host feeds it Core Audio events and executes its decisions.

use crate::hwid;
use bitbybit_core::{LocalizedText, text};
use bitbybit_evidence::EvidenceItem;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use time::{Duration, OffsetDateTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum AudioDataFlow {
    #[default]
    Render,
    Capture,
}

/// `DEVICE_STATE_*` of one endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EndpointStatus {
    Active,
    Disabled,
    #[default]
    NotPresent,
    Unplugged,
}

impl EndpointStatus {
    /// Preference among candidates: Active > Unplugged > Disabled > NotPresent.
    pub fn availability_rank(self) -> u8 {
        match self {
            Self::Active => 3,
            Self::Unplugged => 2,
            Self::Disabled => 1,
            Self::NotPresent => 0,
        }
    }
}

/// `EndpointFormFactor` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FormFactor {
    RemoteNetworkDevice = 0,
    Speakers = 1,
    LineLevel = 2,
    Headphones = 3,
    Microphone = 4,
    Headset = 5,
    Handset = 6,
    UnknownDigitalPassthrough = 7,
    Spdif = 8,
    /// HDMI or DisplayPort audio sink.
    DigitalAudioDisplayDevice = 9,
    UnknownFormFactor = 10,
}

impl FormFactor {
    pub fn from_raw(value: u32) -> Option<Self> {
        use FormFactor::*;
        [
            RemoteNetworkDevice,
            Speakers,
            LineLevel,
            Headphones,
            Microphone,
            Headset,
            Handset,
            UnknownDigitalPassthrough,
            Spdif,
            DigitalAudioDisplayDevice,
            UnknownFormFactor,
        ]
        .into_iter()
        .find(|f| *f as u32 == value)
    }

    pub fn is_analogue_transducer(self) -> bool {
        matches!(
            self,
            Self::Speakers | Self::LineLevel | Self::Headphones | Self::Microphone | Self::Headset | Self::Handset
        )
    }
}

/// Connector class from `PKEY_AudioEndpoint_JackSubType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum JackKind {
    #[default]
    Unknown,
    Hdmi,
    Spdif,
    /// A known subtype that is neither HDMI nor S/PDIF.
    Other,
}

/// Kind of hardware behind an endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ControllerKind {
    #[default]
    Unknown,
    OnboardHdaCodec,
    GpuDisplayAudio,
    UsbAudio,
    Bluetooth,
    Virtual,
}

impl ControllerKind {
    /// From the hardware id of the device that owns the endpoint.
    pub fn from_hardware_id(id: &str) -> Self {
        let upper = id.to_uppercase();
        if hwid::is_gpu_display_audio(id) {
            Self::GpuDisplayAudio
        } else if hwid::is_hd_audio_function(id) {
            Self::OnboardHdaCodec
        } else if upper.starts_with(r"USB\") {
            Self::UsbAudio
        } else if upper.starts_with(r"BTH") {
            Self::Bluetooth
        } else if upper.starts_with(r"ROOT\") || upper.starts_with(r"SWD\") {
            Self::Virtual
        } else {
            Self::Unknown
        }
    }
}

/// Default-device roles; "Default device" in the Sound panel = console + multimedia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct AudioRoles {
    pub console: bool,
    pub multimedia: bool,
    pub communications: bool,
}

impl AudioRoles {
    pub const DEFAULT_DEVICE: Self = Self { console: true, multimedia: true, communications: false };
    pub const ALL: Self = Self { console: true, multimedia: true, communications: true };
    pub const NONE: Self = Self { console: false, multimedia: false, communications: false };

    /// Every role in `self` is also in `held`.
    pub fn covered_by(self, held: Self) -> bool {
        (!self.console || held.console)
            && (!self.multimedia || held.multimedia)
            && (!self.communications || held.communications)
    }
}

/// One audio endpoint as the platform reports it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioEndpoint {
    pub id: String,
    pub flow: AudioDataFlow,
    pub state: EndpointStatus,
    pub friendly_name: Option<String>,
    pub device_description: Option<String>,
    pub interface_friendly_name: Option<String>,
    pub form_factor: Option<FormFactor>,
    #[serde(default)]
    pub jack: JackKind,
    /// `PKEY_Device_ContainerId` as a lower-case GUID string.
    pub container_id: Option<String>,
    /// Hardware id of the device that owns the endpoint (from the device topology).
    pub controller_hardware_id: Option<String>,
    pub default_roles: AudioRoles,
    /// Master volume 0.0–1.0 and mute, when readable.
    pub volume: Option<f32>,
    pub muted: Option<bool>,
}

impl AudioEndpoint {
    pub fn display_name(&self) -> String {
        self.friendly_name
            .clone()
            .or_else(|| self.device_description.clone())
            .or_else(|| self.interface_friendly_name.clone())
            .unwrap_or_else(|| self.id.clone())
    }
}

/// How an endpoint connects; derived from the controller kind, form factor and jack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Transport {
    #[default]
    Unknown,
    OnboardAnalogue,
    DisplayAudio,
    Spdif,
    Usb,
    Bluetooth,
    Virtual,
    Remote,
}

pub fn transport_of(endpoint: &AudioEndpoint, controller: ControllerKind) -> Transport {
    match controller {
        ControllerKind::Bluetooth => return Transport::Bluetooth,
        ControllerKind::UsbAudio => return Transport::Usb,
        ControllerKind::Virtual => return Transport::Virtual,
        _ => {}
    }
    let ff = endpoint.form_factor;
    if ff == Some(FormFactor::DigitalAudioDisplayDevice) || endpoint.jack == JackKind::Hdmi {
        return Transport::DisplayAudio;
    }
    if ff == Some(FormFactor::RemoteNetworkDevice) {
        return Transport::Remote;
    }
    if ff == Some(FormFactor::Spdif) || endpoint.jack == JackKind::Spdif {
        return Transport::Spdif;
    }
    if controller == ControllerKind::GpuDisplayAudio && endpoint.flow == AudioDataFlow::Render {
        return Transport::DisplayAudio;
    }
    if controller == ControllerKind::OnboardHdaCodec && ff.is_none_or(FormFactor::is_analogue_transducer) {
        return Transport::OnboardAnalogue;
    }
    Transport::Unknown
}

/// Container of devices built into the PC (weak evidence: every onboard device shares it).
pub const LOCAL_MACHINE_CONTAINER: &str = "00000000-0000-0000-ffff-ffffffffffff";

/// Durable identity of an endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub endpoint_id: String,
    pub flow: AudioDataFlow,
    pub form_factor: Option<FormFactor>,
    #[serde(default)]
    pub jack: JackKind,
    pub transport: Transport,
    pub interface_friendly_name: Option<String>,
    pub friendly_name: Option<String>,
    pub device_description: Option<String>,
    pub container_id: Option<String>,
    pub controller_kind: ControllerKind,
    /// `VEN_xxxx` of the controller.
    pub controller_vendor: Option<String>,
    /// `DEV_xxxx` of the controller.
    pub controller_device: Option<String>,
}

fn tag_value(id: &str, tags: &[&str]) -> Option<String> {
    let upper = id.to_uppercase();
    tags.iter().find_map(|tag| {
        let at = upper.find(tag)? + tag.len();
        upper.get(at..at + 4).filter(|v| v.chars().all(|c| c.is_ascii_hexdigit())).map(str::to_owned)
    })
}

impl Fingerprint {
    pub fn of(endpoint: &AudioEndpoint) -> Self {
        let controller = endpoint.controller_hardware_id.as_deref();
        let kind = controller.map_or(ControllerKind::Unknown, ControllerKind::from_hardware_id);
        Self {
            endpoint_id: endpoint.id.clone(),
            flow: endpoint.flow,
            form_factor: endpoint.form_factor,
            jack: endpoint.jack,
            transport: transport_of(endpoint, kind),
            interface_friendly_name: endpoint.interface_friendly_name.clone(),
            friendly_name: endpoint.friendly_name.clone(),
            device_description: endpoint.device_description.clone(),
            container_id: endpoint.container_id.as_deref().map(str::to_lowercase),
            controller_kind: kind,
            controller_vendor: controller.and_then(|c| tag_value(c, &["VEN_", "VID_"])),
            controller_device: controller.and_then(|c| tag_value(c, &["DEV_", "PID_"])),
        }
    }
}

/// Upper case, whitespace collapsed, Windows' duplicate prefix ("2- ") removed.
pub fn normalize_name(name: Option<&str>) -> Option<String> {
    let collapsed = name?.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let strip = |s: &str| -> String {
        let digits = s.chars().take_while(char::is_ascii_digit).count();
        match s[digits..].strip_prefix(" - ").or_else(|| s[digits..].strip_prefix("- ")) {
            Some(rest) if (1..=3).contains(&digits) => rest.to_owned(),
            _ => s.to_owned(),
        }
    };
    let mut out = strip(&collapsed);
    if let Some(open) = out.find('(') {
        out = format!("{}({}", &out[..open], strip(out[open + 1..].trim_start()));
    }
    Some(out.trim().to_uppercase())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateScore {
    pub endpoint_id: String,
    pub state: EndpointStatus,
    pub score: u32,
    pub exact: bool,
    pub disqualified: bool,
    /// Matching attributes (not counting the id and the weak local-machine container).
    pub attributes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchResult {
    pub matched: Option<CandidateScore>,
    pub ambiguous: bool,
    pub candidates: Vec<CandidateScore>,
}

const MIN_SCORE: u32 = 50;
const MIN_ATTRIBUTES: u32 = 2;
const MIN_EVIDENCE_WEIGHT: u32 = 60;
const MAX_FALLBACK_SCORE: u32 = 95;

/// Finds the endpoint a fingerprint designates. Hard mismatches (flow, controller vendor/kind, display vs
/// analogue, connector, transport) disqualify; otherwise weighted attributes score the candidates. Equal best
/// scores are ambiguous and never acted upon.
pub fn match_endpoint(want: &Fingerprint, candidates: &[AudioEndpoint]) -> MatchResult {
    let mut scores: Vec<CandidateScore> = candidates.iter().map(|c| score(want, c, &Fingerprint::of(c))).collect();
    scores.sort_by(|a, b| {
        let key = |s: &CandidateScore| (if s.disqualified { -1 } else { s.score as i64 }, s.state.availability_rank());
        key(b).cmp(&key(a))
    });
    let accepted: Vec<&CandidateScore> = scores
        .iter()
        .filter(|s| !s.disqualified && (s.exact || (s.score >= MIN_SCORE && s.attributes >= MIN_ATTRIBUTES)))
        .collect();
    let Some(best) = accepted.first() else {
        return MatchResult { matched: None, ambiguous: false, candidates: scores };
    };
    if best.exact {
        return MatchResult { matched: Some((*best).clone()), ambiguous: false, candidates: scores.clone() };
    }
    let ambiguous = accepted.get(1).is_some_and(|second| {
        second.score == best.score && second.state.availability_rank() == best.state.availability_rank()
    });
    MatchResult { matched: (!ambiguous).then(|| (*best).clone()), ambiguous, candidates: scores.clone() }
}

fn score(want: &Fingerprint, endpoint: &AudioEndpoint, have: &Fingerprint) -> CandidateScore {
    let exact = want.endpoint_id.eq_ignore_ascii_case(&endpoint.id);
    let both =
        |a: &Option<String>, b: &Option<String>| matches!((a, b), (Some(a), Some(b)) if !a.eq_ignore_ascii_case(b));
    let display_vs_analogue = matches!((want.form_factor, have.form_factor), (Some(a), Some(b))
        if (a == FormFactor::DigitalAudioDisplayDevice && b.is_analogue_transducer()) || (b == FormFactor::DigitalAudioDisplayDevice && a.is_analogue_transducer()));
    let digital = |j: JackKind| matches!(j, JackKind::Hdmi | JackKind::Spdif);
    let disqualified = want.flow != have.flow
        || both(&want.controller_vendor, &have.controller_vendor)
        || (want.controller_kind != ControllerKind::Unknown
            && have.controller_kind != ControllerKind::Unknown
            && want.controller_kind != have.controller_kind)
        || display_vs_analogue
        || (digital(want.jack) && digital(have.jack) && want.jack != have.jack)
        || (want.transport != Transport::Unknown
            && have.transport != Transport::Unknown
            && want.transport != have.transport);
    if disqualified {
        return CandidateScore {
            endpoint_id: endpoint.id.clone(),
            state: endpoint.state,
            score: 0,
            exact,
            disqualified: true,
            attributes: 0,
        };
    }
    let (mut points, mut available, mut attributes) = (0u32, 0u32, 0u32);
    if let Some(vendor) = &want.controller_vendor {
        let same_vendor = have.controller_vendor.as_deref().is_some_and(|v| v.eq_ignore_ascii_case(vendor));
        let with_device = want.controller_device.is_some();
        let same_device = same_vendor && want.controller_device == have.controller_device;
        available += if with_device { 20 } else { 10 };
        // Vendor alone earns half the controller weight when a device id was recorded.
        points += match (same_vendor, with_device && !same_device) {
            (false, _) => 0,
            (true, true) => 10,
            (true, false) => {
                if with_device {
                    20
                } else {
                    10
                }
            }
        };
        attributes += u32::from(same_vendor);
    }
    let mut weigh = |weight: u32, applicable: bool, matched: bool, counts: bool| {
        if applicable {
            available += weight;
            if matched {
                points += weight;
                attributes += u32::from(counts);
            }
        }
    };
    if let Some(container) =
        want.container_id.as_deref().filter(|c| !c.is_empty() && *c != "00000000-0000-0000-0000-000000000000")
    {
        let local = container == LOCAL_MACHINE_CONTAINER;
        weigh(if local { 5 } else { 30 }, true, have.container_id.as_deref() == Some(container), !local);
    }
    for (w, h, weight) in [
        (&want.interface_friendly_name, &have.interface_friendly_name, 20),
        (&want.friendly_name, &have.friendly_name, 15),
        (&want.device_description, &have.device_description, 10),
    ] {
        let (w, h) = (normalize_name(w.as_deref()), normalize_name(h.as_deref()));
        weigh(weight, w.is_some(), w.is_some() && w == h, true);
    }
    weigh(
        10,
        want.form_factor.is_some_and(|f| f != FormFactor::UnknownFormFactor),
        have.form_factor == want.form_factor,
        true,
    );
    weigh(5, want.controller_kind != ControllerKind::Unknown, have.controller_kind == want.controller_kind, true);
    weigh(5, want.jack != JackKind::Unknown && want.jack != JackKind::Other, have.jack == want.jack, true);
    let score = if exact {
        100
    } else {
        (MAX_FALLBACK_SCORE * points).div_ceil(available.max(MIN_EVIDENCE_WEIGHT)).min(MAX_FALLBACK_SCORE)
    };
    CandidateScore {
        endpoint_id: endpoint.id.clone(),
        state: endpoint.state,
        score,
        exact,
        disqualified: false,
        attributes,
    }
}

/// A persistent audio preference. Only the minimum needed is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioPreferenceProfile {
    pub schema_version: u32,
    pub name: String,
    pub enabled: bool,
    pub preferred_endpoint: Fingerprint,
    pub roles: AudioRoles,
    /// After a manual choice of another default, stop re-applying until the preferred endpoint comes back.
    pub respect_manual_changes: bool,
    /// Upper bound of automatic changes per session (prevents fighting other software).
    pub max_automatic_applies_per_session: u32,
    pub cooldown_seconds: u32,
    pub auto_set_default: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl AudioPreferenceProfile {
    pub const DISPLAYPORT_PREFERRED: &'static str = "DisplayPort Audio Preferred";

    pub fn for_display_port(endpoint: &AudioEndpoint, now: OffsetDateTime) -> Self {
        Self {
            schema_version: 1,
            name: Self::DISPLAYPORT_PREFERRED.into(),
            enabled: true,
            preferred_endpoint: Fingerprint::of(endpoint),
            roles: AudioRoles::DEFAULT_DEVICE,
            respect_manual_changes: true,
            max_automatic_applies_per_session: 5,
            cooldown_seconds: 20,
            auto_set_default: true,
            created_at: now,
        }
    }
}

/// `%ProgramData%\bitBybit\profiles\audio-profile.json`.
pub struct ProfileStore {
    path: PathBuf,
}

impl ProfileStore {
    pub fn new(directory: &Path) -> Self {
        Self { path: directory.join("audio-profile.json") }
    }

    pub fn load(&self) -> Option<AudioPreferenceProfile> {
        bitbybit_core::fsio::read_json(&self.path)
    }

    pub fn save(&self, profile: &AudioPreferenceProfile) -> bitbybit_core::Result<()> {
        bitbybit_core::fsio::write_json(&self.path, profile)
    }

    pub fn delete(&self) -> bitbybit_core::Result<()> {
        for path in [self.path.clone(), PathBuf::from(format!("{}.bak", self.path.display()))] {
            match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileTrigger {
    Startup,
    EndpointChanged,
    DefaultChanged,
    Periodic,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionKind {
    Satisfied,
    /// Make `endpoint_id` the default; the host executes then calls [`Reconciler::record_applied`].
    SetDefault,
    WaitForEndpoint,
    Ambiguous,
    PausedByUser,
    LimitReached,
    CoolingDown,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub kind: DecisionKind,
    pub endpoint_id: Option<String>,
    pub reason: LocalizedText,
    pub evidence: Vec<EvidenceItem>,
}

const SOURCE: &str = "bitBybit.profiles.audio";

pub struct Reconciler {
    profile: AudioPreferenceProfile,
    applies: u32,
    last_apply: Option<OffsetDateTime>,
    expected_default: Option<String>,
    paused_by_user: bool,
    preferred_was_active: bool,
}

impl Reconciler {
    pub fn new(profile: AudioPreferenceProfile) -> Self {
        Self {
            profile,
            applies: 0,
            last_apply: None,
            expected_default: None,
            paused_by_user: false,
            preferred_was_active: false,
        }
    }

    pub fn profile(&self) -> &AudioPreferenceProfile {
        &self.profile
    }

    pub fn is_paused_by_user(&self) -> bool {
        self.paused_by_user
    }

    pub fn evaluate(
        &mut self,
        trigger: ReconcileTrigger,
        endpoints: &[AudioEndpoint],
        now: OffsetDateTime,
        default_changed_to: Option<&str>,
    ) -> Decision {
        let decide =
            |kind, endpoint_id: Option<String>, reason, evidence| Decision { kind, endpoint_id, reason, evidence };
        if !self.profile.enabled || !self.profile.auto_set_default {
            return decide(
                DecisionKind::Disabled,
                None,
                text!("profiles.audio.disabled", "The profile is disabled."),
                vec![],
            );
        }
        let same_flow: Vec<AudioEndpoint> =
            endpoints.iter().filter(|e| e.flow == self.profile.preferred_endpoint.flow).cloned().collect();
        let found = match_endpoint(&self.profile.preferred_endpoint, &same_flow);
        if found.ambiguous {
            let equal = found.candidates.iter().filter(|c| !c.disqualified).count();
            return decide(
                DecisionKind::Ambiguous,
                None,
                text!(
                    "profiles.audio.ambiguous",
                    "Several audio devices match the preferred device equally; bitBybit will not choose automatically."
                ),
                vec![EvidenceItem::observation(SOURCE, format!("{equal} equally scored candidates."))],
            );
        }
        let Some(endpoint) =
            found.matched.and_then(|m| endpoints.iter().find(|e| e.id.eq_ignore_ascii_case(&m.endpoint_id)))
        else {
            self.preferred_was_active = false;
            let fp = &self.profile.preferred_endpoint;
            return decide(
                DecisionKind::WaitForEndpoint,
                None,
                text!(
                    "profiles.audio.missing",
                    "The preferred device '{0}' is not available yet (monitor off or not connected, or the graphics audio driver has not created it).",
                    fp.friendly_name.clone().unwrap_or_else(|| fp.endpoint_id.clone())
                ),
                vec![EvidenceItem::observation(
                    "CoreAudio:EnumAudioEndpoints",
                    "No endpoint matches the profile fingerprint.",
                )],
            );
        };
        let name = endpoint.display_name();
        if endpoint.state != EndpointStatus::Active {
            self.preferred_was_active = false;
            let reason = match endpoint.state {
                EndpointStatus::Unplugged => text!(
                    "profiles.audio.unplugged",
                    "'{0}' is reported as unplugged (monitor off, asleep or not sending audio).",
                    name
                ),
                EndpointStatus::Disabled => {
                    text!("profiles.audio.disabledEndpoint", "'{0}' is disabled in the Sound settings.", name)
                }
                _ => text!("profiles.audio.notPresent", "'{0}' is not present now.", name),
            };
            return decide(
                DecisionKind::WaitForEndpoint,
                Some(endpoint.id.clone()),
                reason,
                vec![EvidenceItem::observation(
                    "CoreAudio:IMMDevice.GetState",
                    format!("Preferred endpoint state: {:?}", endpoint.state),
                )],
            );
        }
        if !self.preferred_was_active {
            // The preferred endpoint (re-)appeared: a new episode starts; earlier manual overrides no longer apply.
            self.paused_by_user = false;
            self.preferred_was_active = true;
        }
        let is_default = self.profile.roles.covered_by(endpoint.default_roles);
        if trigger == ReconcileTrigger::DefaultChanged
            && self.profile.respect_manual_changes
            && !is_default
            && let Some(changed_to) = default_changed_to
            && !self.expected_default.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(changed_to))
        {
            self.paused_by_user = true;
            return decide(
                DecisionKind::PausedByUser,
                Some(endpoint.id.clone()),
                text!(
                    "profiles.audio.userChanged",
                    "You selected another default device; bitBybit will respect it until '{0}' is reconnected.",
                    name
                ),
                vec![
                    EvidenceItem::observation(
                        "CoreAudio:OnDefaultDeviceChanged",
                        "Default changed by someone other than bitBybit.",
                    )
                    .with_value(changed_to),
                ],
            );
        }
        if is_default {
            self.expected_default = None;
            return decide(
                DecisionKind::Satisfied,
                Some(endpoint.id.clone()),
                text!("profiles.audio.satisfied", "'{0}' is the default device.", name),
                vec![],
            );
        }
        if self.paused_by_user {
            return decide(
                DecisionKind::PausedByUser,
                Some(endpoint.id.clone()),
                text!("profiles.audio.paused", "Paused: you chose another default device."),
                vec![],
            );
        }
        if self.applies >= self.profile.max_automatic_applies_per_session {
            return decide(
                DecisionKind::LimitReached,
                Some(endpoint.id.clone()),
                text!(
                    "profiles.audio.limit",
                    "bitBybit already changed the default device {0} times in this session; another program may be changing it back. No further automatic changes.",
                    self.applies
                ),
                vec![],
            );
        }
        if self.last_apply.is_some_and(|last| now - last < Duration::seconds(i64::from(self.profile.cooldown_seconds)))
        {
            return decide(
                DecisionKind::CoolingDown,
                Some(endpoint.id.clone()),
                text!("profiles.audio.cooldown", "Waiting a few seconds before changing the default device again."),
                vec![],
            );
        }
        decide(
            DecisionKind::SetDefault,
            Some(endpoint.id.clone()),
            text!("profiles.audio.apply", "Making '{0}' the default device.", name),
            vec![EvidenceItem::observation(
                "CoreAudio:GetDefaultAudioEndpoint",
                "The preferred endpoint is active but not the default.",
            )],
        )
    }

    /// Records an automatic change made by the host (counts toward the session limit).
    pub fn record_applied(&mut self, endpoint_id: &str, at: OffsetDateTime, verified: bool) {
        self.applies += 1;
        self.last_apply = Some(at);
        self.expected_default = verified.then(|| endpoint_id.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    const DP_ID: &str = "{0.0.0.00000000}.{8e3c1a2b-1111-4a4a-9b9b-000000000001}";
    const DP_ID_AFTER_REINSTALL: &str = "{0.0.0.00000000}.{8e3c1a2b-2222-4a4a-9b9b-000000000002}";
    const REALTEK_ID: &str = "{0.0.0.00000000}.{8e3c1a2b-3333-4a4a-9b9b-000000000003}";
    const T0: OffsetDateTime = datetime!(2026-10-17 09:00 UTC);

    fn dp(id: &str, state: EndpointStatus, is_default: bool) -> AudioEndpoint {
        AudioEndpoint {
            id: id.into(),
            state,
            friendly_name: Some("DELL P2720DC (NVIDIA High Definition Audio)".into()),
            device_description: Some("DELL P2720DC".into()),
            interface_friendly_name: Some("NVIDIA High Definition Audio".into()),
            form_factor: Some(FormFactor::DigitalAudioDisplayDevice),
            container_id: Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()),
            default_roles: if is_default { AudioRoles::ALL } else { AudioRoles::NONE },
            ..AudioEndpoint::default()
        }
    }

    fn realtek(is_default: bool) -> AudioEndpoint {
        AudioEndpoint {
            id: REALTEK_ID.into(),
            state: EndpointStatus::Active,
            friendly_name: Some("Altifalantes (Realtek(R) Audio)".into()),
            device_description: Some("Altifalantes".into()),
            interface_friendly_name: Some("Realtek(R) Audio".into()),
            form_factor: Some(FormFactor::Speakers),
            container_id: Some(LOCAL_MACHINE_CONTAINER.into()),
            default_roles: if is_default { AudioRoles::ALL } else { AudioRoles::NONE },
            ..AudioEndpoint::default()
        }
    }

    fn active_dp() -> AudioEndpoint {
        dp(DP_ID, EndpointStatus::Active, false)
    }

    fn reconciler(max: u32, cooldown: u32) -> Reconciler {
        let mut profile = AudioPreferenceProfile::for_display_port(&active_dp(), T0);
        profile.max_automatic_applies_per_session = max;
        profile.cooldown_seconds = cooldown;
        Reconciler::new(profile)
    }

    fn eval(
        r: &mut Reconciler,
        trigger: ReconcileTrigger,
        at: OffsetDateTime,
        changed: Option<&str>,
        endpoints: &[AudioEndpoint],
    ) -> Decision {
        r.evaluate(trigger, endpoints, at, changed)
    }

    #[test]
    fn sets_displayport_as_default_when_realtek_took_over() {
        let d = eval(&mut reconciler(5, 20), ReconcileTrigger::Startup, T0, None, &[realtek(true), active_dp()]);
        assert_eq!((d.kind, d.endpoint_id.as_deref()), (DecisionKind::SetDefault, Some(DP_ID)));
    }

    #[test]
    fn never_matches_the_realtek_speaker_when_displayport_is_missing() {
        let d = eval(&mut reconciler(5, 20), ReconcileTrigger::Startup, T0, None, &[realtek(true)]);
        assert_eq!((d.kind, d.endpoint_id), (DecisionKind::WaitForEndpoint, None));
    }

    #[test]
    fn follows_the_endpoint_after_a_driver_reinstall_changed_its_id() {
        let d = eval(
            &mut reconciler(5, 20),
            ReconcileTrigger::EndpointChanged,
            T0,
            None,
            &[realtek(true), dp(DP_ID_AFTER_REINSTALL, EndpointStatus::Active, false)],
        );
        assert_eq!((d.kind, d.endpoint_id.as_deref()), (DecisionKind::SetDefault, Some(DP_ID_AFTER_REINSTALL)));
    }

    #[test]
    fn waits_and_explains_when_the_preferred_endpoint_is_not_active() {
        for (state, key) in [
            (EndpointStatus::Unplugged, "profiles.audio.unplugged"),
            (EndpointStatus::Disabled, "profiles.audio.disabledEndpoint"),
            (EndpointStatus::NotPresent, "profiles.audio.notPresent"),
        ] {
            let d = eval(
                &mut reconciler(5, 20),
                ReconcileTrigger::Startup,
                T0,
                None,
                &[realtek(true), dp(DP_ID, state, false)],
            );
            assert_eq!((d.kind, d.reason.key.as_str()), (DecisionKind::WaitForEndpoint, key));
        }
    }

    #[test]
    fn satisfied_when_displayport_is_default() {
        let d = eval(
            &mut reconciler(5, 20),
            ReconcileTrigger::Periodic,
            T0,
            None,
            &[realtek(false), dp(DP_ID, EndpointStatus::Active, true)],
        );
        assert_eq!(d.kind, DecisionKind::Satisfied);
    }

    #[test]
    fn respects_a_manual_change_until_the_monitor_is_reconnected() {
        let mut r = reconciler(5, 20);
        let m = |minutes: i64| T0 + Duration::minutes(minutes);
        eval(&mut r, ReconcileTrigger::Startup, T0, None, &[realtek(false), dp(DP_ID, EndpointStatus::Active, true)]);
        assert_eq!(
            eval(&mut r, ReconcileTrigger::DefaultChanged, m(1), Some(REALTEK_ID), &[realtek(true), active_dp()]).kind,
            DecisionKind::PausedByUser
        );
        assert_eq!(
            eval(&mut r, ReconcileTrigger::Periodic, m(5), None, &[realtek(true), active_dp()]).kind,
            DecisionKind::PausedByUser
        );
        eval(
            &mut r,
            ReconcileTrigger::EndpointChanged,
            m(6),
            None,
            &[realtek(true), dp(DP_ID, EndpointStatus::Unplugged, false)],
        );
        assert_eq!(
            eval(&mut r, ReconcileTrigger::EndpointChanged, m(7), None, &[realtek(true), active_dp()]).kind,
            DecisionKind::SetDefault
        );
    }

    #[test]
    fn its_own_change_is_not_mistaken_for_a_manual_one() {
        let mut r = reconciler(5, 20);
        let d = eval(&mut r, ReconcileTrigger::Startup, T0, None, &[realtek(true), active_dp()]);
        r.record_applied(d.endpoint_id.as_deref().unwrap(), T0, true);
        let echo = eval(
            &mut r,
            ReconcileTrigger::DefaultChanged,
            T0 + Duration::seconds(1),
            Some(DP_ID),
            &[realtek(false), dp(DP_ID, EndpointStatus::Active, true)],
        );
        assert_eq!(echo.kind, DecisionKind::Satisfied);
        assert!(!r.is_paused_by_user());
    }

    #[test]
    fn is_rate_limited_and_capped_per_session() {
        let mut r = reconciler(2, 30);
        let mut now = T0;
        for i in 0..2 {
            assert_eq!(
                eval(&mut r, ReconcileTrigger::Periodic, now, None, &[realtek(true), active_dp()]).kind,
                DecisionKind::SetDefault
            );
            r.record_applied(DP_ID, now, false);
            let expected = if i == 0 { DecisionKind::CoolingDown } else { DecisionKind::LimitReached };
            assert_eq!(
                eval(
                    &mut r,
                    ReconcileTrigger::Periodic,
                    now + Duration::seconds(5),
                    None,
                    &[realtek(true), active_dp()]
                )
                .kind,
                expected
            );
            now += Duration::minutes(1);
        }
        assert_eq!(
            eval(&mut r, ReconcileTrigger::Periodic, now, None, &[realtek(true), active_dp()]).kind,
            DecisionKind::LimitReached
        );
    }

    #[test]
    fn disabled_profile_does_nothing() {
        let mut profile = AudioPreferenceProfile::for_display_port(&active_dp(), T0);
        profile.enabled = false;
        assert_eq!(
            Reconciler::new(profile).evaluate(ReconcileTrigger::Startup, &[realtek(true), active_dp()], T0, None).kind,
            DecisionKind::Disabled
        );
    }

    #[test]
    fn all_reason_keys_exist_in_catalog() {
        let catalog = bitbybit_localization::TextCatalog::default_catalog();
        for key in [
            "disabled",
            "ambiguous",
            "missing",
            "unplugged",
            "disabledEndpoint",
            "notPresent",
            "userChanged",
            "satisfied",
            "paused",
            "limit",
            "cooldown",
            "apply",
        ] {
            assert!(catalog.template(&format!("profiles.audio.{key}")).is_some(), "{key}");
        }
    }

    #[test]
    fn identical_twins_are_ambiguous_and_never_chosen() {
        let a = dp("{0.0.0.00000000}.{a}", EndpointStatus::Active, false);
        let b = dp("{0.0.0.00000000}.{b}", EndpointStatus::Active, false);
        let mut profile =
            AudioPreferenceProfile::for_display_port(&dp("{0.0.0.00000000}.{gone}", EndpointStatus::Active, false), T0);
        profile.max_automatic_applies_per_session = 5;
        assert_eq!(
            Reconciler::new(profile).evaluate(ReconcileTrigger::Startup, &[a, b], T0, None).kind,
            DecisionKind::Ambiguous
        );
    }

    #[test]
    fn profile_store_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path());
        let profile = AudioPreferenceProfile::for_display_port(&active_dp(), T0);
        store.save(&profile).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!((loaded.preferred_endpoint.endpoint_id.as_str(), loaded.roles), (DP_ID, AudioRoles::DEFAULT_DEVICE));
        store.delete().unwrap();
        assert!(store.load().is_none());
    }

    #[test]
    fn names_are_normalized() {
        assert_eq!(normalize_name(Some("  2- DELL   P2720DC ")), Some("DELL P2720DC".into()));
        assert_eq!(normalize_name(Some("Speakers (2- Realtek(R) Audio)")), Some("SPEAKERS (REALTEK(R) AUDIO)".into()));
        assert_eq!(normalize_name(Some("  ")), None);
    }
}
