//! Endpoint device formats (AUD-07): the "Default Format" of the Windows Sound panel. Pure WAVEFORMATEX /
//! WAVEFORMATEXTENSIBLE encoding and decoding, the candidate list the platform tests with
//! `IAudioClient::IsFormatSupported(EXCLUSIVE)` (as the Sound panel does), and the journaled change, which the
//! platform applies through the UNDOCUMENTED `IPolicyConfig::SetDeviceFormat`. Every write is read back.

use bitbybit_core::{Error, ErrorCode, Result};
use bitbybit_journal::{ChangeCategory, ChangeRecord, ChangeStatus, Journal};
use bitbybit_recovery::{LiveState, RevertHandler};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub const DEVICE_FORMAT_KIND: &str = "audio.deviceFormat";

const TAG_PCM: u16 = 1;
const TAG_FLOAT: u16 = 3;
const TAG_EXTENSIBLE: u16 = 0xFFFE;
/// KSDATAFORMAT_SUBTYPE_PCM / _IEEE_FLOAT share this tail after their first four bytes.
const SUBFORMAT_TAIL: [u8; 12] = [0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71];

/// Sample rates the Sound panel offers.
pub const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
/// (container, valid) bits tried per rate: 16, 24 packed, 24 in a 32-bit container, 32.
const DEPTHS: [(u16, u16); 4] = [(16, 16), (24, 24), (32, 24), (32, 32)];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFormat {
    pub channels: u16,
    pub sample_rate: u32,
    /// Bits per sample of the container (`wBitsPerSample`).
    pub container_bits: u16,
    /// Meaningful bits (`wValidBitsPerSample`; equal to the container for plain WAVEFORMATEX).
    pub valid_bits: u16,
    pub float: bool,
    /// Speaker positions (`dwChannelMask`); 0 for plain WAVEFORMATEX.
    pub channel_mask: u32,
}

impl AudioFormat {
    /// WAVEFORMATEXTENSIBLE (40 bytes, little endian).
    pub fn encode(&self) -> Vec<u8> {
        let block = self.channels * (self.container_bits / 8);
        let mut b = Vec::with_capacity(40);
        b.extend_from_slice(&TAG_EXTENSIBLE.to_le_bytes());
        b.extend_from_slice(&self.channels.to_le_bytes());
        b.extend_from_slice(&self.sample_rate.to_le_bytes());
        b.extend_from_slice(&(self.sample_rate * u32::from(block)).to_le_bytes());
        b.extend_from_slice(&block.to_le_bytes());
        b.extend_from_slice(&self.container_bits.to_le_bytes());
        b.extend_from_slice(&22u16.to_le_bytes());
        b.extend_from_slice(&self.valid_bits.to_le_bytes());
        b.extend_from_slice(&self.channel_mask.to_le_bytes());
        b.extend_from_slice(&u32::from(if self.float { TAG_FLOAT } else { TAG_PCM }).to_le_bytes());
        b.extend_from_slice(&SUBFORMAT_TAIL);
        b
    }

    /// WAVEFORMATEX or WAVEFORMATEXTENSIBLE; `None` for anything that is not PCM / IEEE float (e.g. a
    /// pass-through format) or malformed input.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let u16_at = |i: usize| Some(u16::from_le_bytes(bytes.get(i..i + 2)?.try_into().ok()?));
        let u32_at = |i: usize| Some(u32::from_le_bytes(bytes.get(i..i + 4)?.try_into().ok()?));
        let (tag, channels, sample_rate, container_bits) = (u16_at(0)?, u16_at(2)?, u32_at(4)?, u16_at(14)?);
        if channels == 0 || sample_rate == 0 || container_bits == 0 || !container_bits.is_multiple_of(8) {
            return None;
        }
        let (valid_bits, float, channel_mask) = match tag {
            TAG_PCM | TAG_FLOAT => (container_bits, tag == TAG_FLOAT, 0),
            TAG_EXTENSIBLE if u16_at(16)? >= 22 => {
                let sub = u32_at(24)?;
                if bytes.get(28..40)? != SUBFORMAT_TAIL || !(sub == 1 || sub == 3) {
                    return None;
                }
                let valid = u16_at(18)?;
                (if valid == 0 { container_bits } else { valid }, sub == 3, u32_at(20)?)
            }
            _ => return None,
        };
        Some(Self { channels, sample_rate, container_bits, valid_bits, float, channel_mask })
    }

    /// The same layout as float 32: what the shared-mode engine mixes in.
    pub fn mix(&self) -> Self {
        Self { container_bits: 32, valid_bits: 32, float: true, ..*self }
    }
}

/// Integer PCM candidates at the current channel layout, for the platform to test.
pub fn candidates(current: &AudioFormat) -> Vec<AudioFormat> {
    RATES
        .iter()
        .flat_map(|&sample_rate| {
            DEPTHS.iter().map(move |&(container_bits, valid_bits)| AudioFormat {
                channels: current.channels,
                sample_rate,
                container_bits,
                valid_bits,
                float: false,
                channel_mask: current.channel_mask,
            })
        })
        .collect()
}

/// What the user can pick: one entry per (rate, bits), the current format always included and preferred over
/// another container with the same bits, ordered as the Sound panel (rate, then bits).
pub fn choices(current: &AudioFormat, supported: &[AudioFormat]) -> Vec<AudioFormat> {
    let mut out: Vec<AudioFormat> = vec![*current];
    for f in supported {
        if !out.iter().any(|o| (o.sample_rate, o.valid_bits, o.float) == (f.sample_rate, f.valid_bits, f.float)) {
            out.push(*f);
        }
    }
    out.sort_by_key(|f| (f.sample_rate, f.valid_bits, f.float));
    out
}

/// Core Audio port for formats, implemented by the platform.
pub trait AudioFormats: Send + Sync {
    /// The endpoint's device format; `NotFound` when the endpoint is gone.
    fn device_format(&self, endpoint_id: &str) -> Result<AudioFormat>;
    /// Which of `candidates(current)` the device accepts in exclusive mode (read-only, bounded).
    fn supported(&self, endpoint_id: &str, current: &AudioFormat) -> Result<Vec<AudioFormat>>;
    /// `IPolicyConfig::SetDeviceFormat` (undocumented). `AccessDenied` when Windows refuses for this user.
    fn set_device_format(&self, endpoint_id: &str, format: &AudioFormat) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatState {
    pub endpoint_id: String,
    pub format: AudioFormat,
}

/// "24 bit, 48000 Hz" for journal descriptions (English; the UI has catalog text).
pub fn label(f: &AudioFormat) -> String {
    format!("{} bit, {} Hz", f.valid_bits, f.sample_rate)
}

fn usable(dev: &dyn AudioFormats, endpoint_id: &str, format: &AudioFormat) -> Result<()> {
    let current = dev.device_format(endpoint_id)?;
    if *format == current || dev.supported(endpoint_id, &current)?.contains(format) {
        Ok(())
    } else {
        Err(Error::new(ErrorCode::NotSupported, "The device no longer supports this format."))
    }
}

/// Journaled change of one endpoint's format: read, check it is still offered, journal, apply, verify, record.
/// `Ok(true)` when Windows then reports the new format.
pub fn set_format(
    dev: &dyn AudioFormats,
    journal: &dyn Journal,
    endpoint_id: &str,
    name: &str,
    requested: &AudioFormat,
    now: OffsetDateTime,
) -> Result<bool> {
    let current = dev.device_format(endpoint_id)?;
    if current == *requested {
        return Ok(true);
    }
    usable(dev, endpoint_id, requested)?;
    let state = |format: &AudioFormat| -> Result<Option<String>> {
        Ok(Some(serde_json::to_string(&FormatState { endpoint_id: endpoint_id.to_owned(), format: *format })?))
    };
    let mut record = ChangeRecord::new(
        ChangeCategory::Audio,
        DEVICE_FORMAT_KIND,
        endpoint_id,
        format!("Format of \"{name}\" set to {} (was {})", label(requested), label(&current)),
        now,
    )
    .with_states(state(&current)?, state(requested)?);
    record.revert_on_uninstall = false;
    let id = record.id;
    journal.append(record)?;
    let outcome =
        dev.set_device_format(endpoint_id, requested).and_then(|()| Ok(dev.device_format(endpoint_id)? == *requested));
    match &outcome {
        Err(e) => {
            journal.update_status(id, ChangeStatus::RevertFailed, Some(format!("Apply failed: {}", e.message)))?
        }
        Ok(false) => journal.update_status(id, ChangeStatus::RevertFailed, Some("Applied but not verified.".into()))?,
        Ok(true) => {}
    }
    outcome
}

/// Reverts a format change only while the endpoint still has the format bitBybit set.
pub struct DeviceFormatRevert<'a>(pub &'a dyn AudioFormats);

fn read(json: Option<&str>) -> Option<FormatState> {
    serde_json::from_str(json?).ok()
}

impl RevertHandler for DeviceFormatRevert<'_> {
    fn target_kind(&self) -> &str {
        DEVICE_FORMAT_KIND
    }

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let Some(applied) = read(record.applied_state.as_deref()) else { return Ok(LiveState::Unknown) };
        Ok(match self.0.device_format(&applied.endpoint_id) {
            Ok(live) if live == applied.format => LiveState::MatchesApplied,
            Ok(_) => LiveState::Drifted,
            Err(e) if e.code == ErrorCode::NotFound => LiveState::Gone,
            Err(_) => LiveState::Unknown,
        })
    }

    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let previous = read(record.previous_state.as_deref())
            .ok_or_else(|| Error::invalid("The journal record has no readable previous state."))?;
        usable(self.0, &previous.endpoint_id, &previous.format)?;
        self.0.set_device_format(&previous.endpoint_id, &previous.format)?;
        if self.0.device_format(&previous.endpoint_id)? == previous.format {
            Ok(())
        } else {
            Err(Error::new(ErrorCode::VerificationFailed, "Windows did not report the previous format."))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_journal::JsonLinesJournal;
    use bitbybit_recovery::RecoveryCoordinator;
    use std::sync::Mutex;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-11-24 10:00 UTC);
    const ID: &str = "{0.0.0.00000000}.{realtek-speakers}";
    const STEREO: u32 = 0x3;
    const F16_48: AudioFormat = AudioFormat {
        channels: 2,
        sample_rate: 48_000,
        container_bits: 16,
        valid_bits: 16,
        float: false,
        channel_mask: STEREO,
    };
    const F24_96: AudioFormat = AudioFormat { sample_rate: 96_000, container_bits: 32, valid_bits: 24, ..F16_48 };
    const F24_192: AudioFormat = AudioFormat { sample_rate: 192_000, ..F24_96 };

    #[test]
    fn extensible_round_trips_byte_for_byte() {
        let bytes = F24_96.encode();
        assert_eq!(bytes.len(), 40);
        assert_eq!(&bytes[..4], &[0xFE, 0xFF, 2, 0]);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 96_000 * 8, "avg bytes/s");
        assert_eq!(u16::from_le_bytes(bytes[12..14].try_into().unwrap()), 8, "block align");
        assert_eq!(&bytes[24..28], &[1, 0, 0, 0], "KSDATAFORMAT_SUBTYPE_PCM");
        assert_eq!(AudioFormat::decode(&bytes), Some(F24_96));
        assert_eq!(AudioFormat::decode(&F24_96.mix().encode()), Some(F24_96.mix()));
        assert!(F24_96.mix().float && F24_96.mix().encode()[24] == 3);
    }

    #[test]
    fn plain_waveformatex_and_garbage() {
        // WAVE_FORMAT_PCM, 2 ch, 44100 Hz, 176400 B/s, block 4, 16 bit, cbSize 0 (fixture bytes).
        let pcm = [1, 0, 2, 0, 0x44, 0xAC, 0, 0, 0x10, 0xB1, 2, 0, 4, 0, 16, 0, 0, 0];
        let f = AudioFormat::decode(&pcm).unwrap();
        assert_eq!((f.sample_rate, f.valid_bits, f.float, f.channel_mask), (44_100, 16, false, 0));
        assert_eq!(AudioFormat::decode(&pcm[..10]), None);
        let mut ac3 = F16_48.encode();
        ac3[24] = 0x92; // a pass-through subformat
        assert_eq!(AudioFormat::decode(&ac3), None);
        let mut seed = 7u64;
        for _ in 0..2000 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let data: Vec<u8> = (0..(seed >> 58) as usize).map(|i| (seed >> (i % 56)) as u8).collect();
            let _ = AudioFormat::decode(&data);
        }
    }

    #[test]
    fn candidates_and_choices() {
        let all = candidates(&F16_48);
        assert_eq!(all.len(), 24);
        assert!(all.iter().all(|f| f.channels == 2 && f.channel_mask == STEREO && !f.float));
        // The device takes 24 bits both packed and in 32; the current (32-bit container) one is the one listed.
        let packed = AudioFormat { container_bits: 24, ..F24_96 };
        let list = choices(&F24_96, &[F16_48, packed, F24_96, F24_192]);
        assert_eq!(list, [F16_48, F24_96, F24_192]);
        assert_eq!(choices(&F16_48, &[]), [F16_48], "nothing readable: the current format alone");
    }

    /// In-memory stand-in for Core Audio formats (unit tests only; no real device).
    #[derive(Default)]
    struct Fake {
        s: Mutex<(Option<AudioFormat>, Vec<AudioFormat>, bool, u32)>,
    }

    impl AudioFormats for Fake {
        fn device_format(&self, _: &str) -> Result<AudioFormat> {
            self.s.lock().unwrap().0.ok_or_else(|| Error::not_found("gone"))
        }
        fn supported(&self, _: &str, _: &AudioFormat) -> Result<Vec<AudioFormat>> {
            Ok(self.s.lock().unwrap().1.clone())
        }
        fn set_device_format(&self, _: &str, f: &AudioFormat) -> Result<()> {
            let mut s = self.s.lock().unwrap();
            s.3 += 1;
            if s.2 {
                return Err(Error::new(ErrorCode::AccessDenied, "simulated E_ACCESSDENIED"));
            }
            s.0 = Some(*f);
            Ok(())
        }
    }

    fn setup() -> (tempfile::TempDir, JsonLinesJournal, Fake) {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let fake = Fake::default();
        *fake.s.lock().unwrap() = (Some(F16_48), vec![F16_48, F24_96], false, 0);
        (dir, journal, fake)
    }

    fn revert(journal: &JsonLinesJournal, fake: &Fake) -> ChangeStatus {
        let h = DeviceFormatRevert(fake);
        RecoveryCoordinator::new(journal, &[&h]).revert(|_| true, false).unwrap()[0].new_status
    }

    #[test]
    fn a_format_change_is_journaled_verified_and_reverted() {
        let (_d, journal, fake) = setup();
        assert!(set_format(&fake, &journal, ID, "Speakers", &F24_96, NOW).unwrap());
        let r = journal.all().unwrap().remove(0);
        assert_eq!((r.target_kind.as_str(), r.status), (DEVICE_FORMAT_KIND, ChangeStatus::Applied));
        assert_eq!(read(r.previous_state.as_deref()).unwrap().format, F16_48);
        assert_eq!(revert(&journal, &fake), ChangeStatus::Reverted);
        assert_eq!(fake.s.lock().unwrap().0, Some(F16_48));
    }

    #[test]
    fn unsupported_or_unchanged_formats_journal_nothing() {
        let (_d, journal, fake) = setup();
        assert!(set_format(&fake, &journal, ID, "Speakers", &F16_48, NOW).unwrap());
        assert_eq!(
            set_format(&fake, &journal, ID, "Speakers", &F24_192, NOW).unwrap_err().code,
            ErrorCode::NotSupported
        );
        fake.s.lock().unwrap().0 = None;
        assert_eq!(set_format(&fake, &journal, ID, "Speakers", &F24_96, NOW).unwrap_err().code, ErrorCode::NotFound);
        assert!(journal.all().unwrap().is_empty());
        assert_eq!(fake.s.lock().unwrap().3, 0);
    }

    #[test]
    fn access_denied_keeps_the_record_active_with_the_error() {
        let (_d, journal, fake) = setup();
        fake.s.lock().unwrap().2 = true;
        assert_eq!(
            set_format(&fake, &journal, ID, "Speakers", &F24_96, NOW).unwrap_err().code,
            ErrorCode::AccessDenied
        );
        let r = journal.all().unwrap().remove(0);
        assert_eq!(r.status, ChangeStatus::RevertFailed);
        assert!(r.status_detail.unwrap().starts_with("Apply failed"));
    }

    #[test]
    fn revert_is_refused_after_an_external_change_and_gone_without_the_device() {
        let (_d, journal, fake) = setup();
        set_format(&fake, &journal, ID, "Speakers", &F24_96, NOW).unwrap();
        fake.s.lock().unwrap().0 = Some(F24_192); // changed in the Sound panel since
        assert_eq!(revert(&journal, &fake), ChangeStatus::DriftDetected);
        assert_eq!(fake.s.lock().unwrap().0, Some(F24_192));

        let (_d, journal, fake) = setup();
        set_format(&fake, &journal, ID, "Speakers", &F24_96, NOW).unwrap();
        fake.s.lock().unwrap().0 = None;
        assert_eq!(revert(&journal, &fake), ChangeStatus::TargetGone);
    }
}
