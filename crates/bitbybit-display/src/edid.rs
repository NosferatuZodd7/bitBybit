//! Minimal EDID 1.x + CEA-861 parser: manufacturer, product code, monitor name, native mode and the audio the
//! monitor *announces* (basic-audio flag, Short Audio Descriptors, HDMI vendor block). Serial numbers are never
//! extracted. Malformed input never panics.

#[derive(Debug, Clone, PartialEq)]
pub struct EdidInfo {
    pub manufacturer_id: Option<String>,
    pub product_code: u16,
    pub monitor_name: Option<String>,
    pub has_cea_extension: bool,
    pub basic_audio: bool,
    pub audio_formats: Vec<String>,
    pub checksum_valid: bool,
    /// Bytes available (128 = base block only).
    pub length: usize,
    /// Extension blocks declared by the base block (byte 126).
    pub extension_count: u8,
    /// A CEA-861 HDMI Vendor-Specific Data Block (OUI 00-0C-03) is present: the monitor declares HDMI (not DVI).
    /// Without it, graphics drivers drive the link in DVI mode, which carries no audio.
    pub has_hdmi_vsdb: bool,
    pub native_width: Option<u32>,
    pub native_height: Option<u32>,
    pub native_pixel_clock_mhz: Option<f64>,
}

impl EdidInfo {
    /// What the monitor announces; not proof that speakers exist or work.
    pub fn announces_audio(&self) -> bool {
        self.basic_audio || !self.audio_formats.is_empty()
    }
}

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
const AUDIO_FORMATS: [&str; 16] = [
    "Reserved",
    "LPCM",
    "AC-3",
    "MPEG-1",
    "MP3",
    "MPEG-2",
    "AAC LC",
    "DTS",
    "ATRAC",
    "One Bit Audio",
    "E-AC-3",
    "DTS-HD",
    "Dolby MAT/TrueHD",
    "DST",
    "WMA Pro",
    "Extended",
];

fn checksum_ok(block: &[u8]) -> bool {
    block.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) == 0
}

fn manufacturer(hi: u8, lo: u8) -> Option<String> {
    let v = (u16::from(hi) << 8) | u16::from(lo);
    [(v >> 10) & 0x1F, (v >> 5) & 0x1F, v & 0x1F]
        .iter()
        .map(|c| (1..=26).contains(c).then(|| char::from(b'A' + (*c as u8) - 1)))
        .collect()
}

fn descriptor_text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|b| **b != 0x0A && **b != 0x00)
        .map(|b| if (0x20..0x7F).contains(b) { char::from(*b) } else { '?' })
        .collect::<String>()
        .trim()
        .to_owned()
}

pub fn parse(edid: &[u8]) -> Option<EdidInfo> {
    if edid.len() < 128 || edid[..8] != HEADER {
        return None;
    }
    let mut checksum_valid = checksum_ok(&edid[..128]);
    let mut monitor_name = None;
    for offset in (54..=108).step_by(18) {
        let d = &edid[offset..offset + 18];
        if d[..4] == [0, 0, 0, 0xFC] {
            monitor_name = Some(descriptor_text(&d[5..18]));
        }
    }
    // First detailed timing descriptor = preferred (native) mode; a zero pixel clock marks a display descriptor.
    let clock = u16::from(edid[54]) | (u16::from(edid[55]) << 8);
    let (native_width, native_height, native_pixel_clock_mhz) = if clock == 0 {
        (None, None, None)
    } else {
        (
            Some(u32::from(edid[56]) | ((u32::from(edid[58]) & 0xF0) << 4)),
            Some(u32::from(edid[59]) | ((u32::from(edid[61]) & 0xF0) << 4)),
            Some(f64::from(clock) / 100.0),
        )
    };

    let (mut has_cea, mut has_hdmi_vsdb, mut basic_audio, mut formats) = (false, false, false, Vec::<String>::new());
    let extension_count = edid[126];
    for i in 1..=usize::from(extension_count) {
        let Some(block) = edid.get(i * 128..i * 128 + 128) else { break };
        checksum_valid &= checksum_ok(block);
        if block[0] != 0x02 {
            continue;
        }
        has_cea = true;
        if block[1] >= 2 {
            basic_audio |= block[3] & 0x40 != 0;
        }
        let dtd_offset = usize::from(block[2]);
        if block[1] < 3 || dtd_offset <= 4 || dtd_offset > 127 {
            continue;
        }
        let mut p = 4;
        while p < dtd_offset {
            let (tag, len) = (block[p] >> 5, usize::from(block[p] & 0x1F));
            if p + 1 + len > dtd_offset {
                break;
            }
            if tag == 3 && len >= 3 && block[p + 1..p + 4] == [0x03, 0x0C, 0x00] {
                has_hdmi_vsdb = true;
            }
            if tag == 1 {
                let mut s = p + 1;
                while s + 2 <= p + len {
                    let code = usize::from((block[s] >> 3) & 0x0F);
                    let name = format!("{} {}ch", AUDIO_FORMATS[code], (block[s] & 0x07) + 1);
                    if code != 0 && !formats.contains(&name) {
                        formats.push(name);
                    }
                    s += 3;
                }
            }
            p += 1 + len;
        }
    }
    Some(EdidInfo {
        manufacturer_id: manufacturer(edid[8], edid[9]),
        product_code: u16::from(edid[10]) | (u16::from(edid[11]) << 8),
        monitor_name,
        has_cea_extension: has_cea,
        basic_audio,
        audio_formats: formats,
        checksum_valid,
        length: edid.len(),
        extension_count,
        has_hdmi_vsdb,
        native_width,
        native_height,
        native_pixel_clock_mhz,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn checksum(edid: &mut [u8], offset: usize) {
        let sum = edid[offset..offset + 127].iter().fold(0u8, |s, b| s.wrapping_add(*b));
        edid[offset + 127] = 0u8.wrapping_sub(sum);
    }

    /// Synthetic EDID (fixture, not a real monitor dump) with valid checksums.
    pub fn build(manufacturer: &str, product: u16, name: &str, cea: bool, basic_audio: bool, lpcm: bool) -> Vec<u8> {
        let blocks = if cea { 2 } else { 1 };
        let mut edid = vec![0u8; 128 * blocks];
        edid[..8].copy_from_slice(&HEADER);
        let m: Vec<u16> = manufacturer.bytes().map(|c| u16::from(c - b'A' + 1)).collect();
        let packed = (m[0] << 10) | (m[1] << 5) | m[2];
        edid[8] = (packed >> 8) as u8;
        edid[9] = packed as u8;
        edid[10] = product as u8;
        edid[11] = (product >> 8) as u8;
        edid[18] = 1;
        edid[19] = 4;
        edid[75] = 0xFC;
        let name = format!("{name}\n");
        for i in 0..13 {
            edid[77 + i] = *name.as_bytes().get(i).unwrap_or(&0x20);
        }
        edid[126] = (blocks - 1) as u8;
        checksum(&mut edid, 0);
        if cea {
            let b = 128;
            edid[b] = 0x02;
            edid[b + 1] = 3;
            edid[b + 3] = if basic_audio { 0x40 } else { 0 };
            let mut p = b + 4;
            if lpcm {
                edid[p..p + 4].copy_from_slice(&[(1 << 5) | 3, (1 << 3) | 1, 0x07, 0x07]);
                p += 4;
            }
            edid[b + 2] = (p - b) as u8;
            checksum(&mut edid, b);
        }
        edid
    }

    #[test]
    fn parses_displayport_monitor_with_cea_audio() {
        let info = parse(&build("DEL", 0xA0B1, "DELL P2720DC", true, true, true)).unwrap();
        assert_eq!(info.manufacturer_id.as_deref(), Some("DEL"));
        assert_eq!(info.product_code, 0xA0B1);
        assert_eq!(info.monitor_name.as_deref(), Some("DELL P2720DC"));
        assert!(info.has_cea_extension && info.announces_audio() && info.checksum_valid);
        assert!(info.audio_formats.contains(&"LPCM 2ch".to_owned()));
    }

    #[test]
    fn monitor_without_cea_extension_does_not_announce_audio() {
        let info = parse(&build("SAM", 0x1234, "Office", false, false, false)).unwrap();
        assert!(!info.has_cea_extension && !info.announces_audio() && !info.has_hdmi_vsdb);
        assert_eq!((info.length, info.extension_count, info.native_width), (128, 0, None));
    }

    #[test]
    fn basic_audio_flag_alone_counts_as_announced() {
        let info = parse(&build("GSM", 0x5B7F, "LG HDR", true, true, false)).unwrap();
        assert!(info.announces_audio() && info.audio_formats.is_empty());
    }

    #[test]
    fn detects_bad_checksum_and_rejects_garbage() {
        let mut edid = build("DEL", 1, "X", true, true, true);
        edid[20] ^= 0xFF;
        assert!(!parse(&edid).unwrap().checksum_valid);
        assert!(parse(&[0u8; 64]).is_none() && parse(&[0u8; 128]).is_none());
    }

    #[test]
    fn random_bytes_never_panic() {
        let mut seed = 1234u64;
        let mut next = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        for _ in 0..2000 {
            let mut data: Vec<u8> = (0..next() % 600).map(|_| next() as u8).collect();
            if data.len() >= 8 && next() % 2 == 0 {
                data[..8].copy_from_slice(&HEADER);
            }
            let _ = parse(&data);
        }
    }

    #[test]
    fn reads_native_mode_extension_count_and_hdmi_data_block() {
        let mut edid = build("GSM", 0x5B00, "WIDE", true, true, true);
        edid[54] = 0x7E;
        edid[55] = 0x48;
        edid[56] = 0x00;
        edid[58] = 0xA0;
        edid[59] = 0x38;
        edid[61] = 0x40;
        checksum(&mut edid, 0);
        let b = 128;
        let p = b + usize::from(edid[b + 2]);
        edid[p..p + 6].copy_from_slice(&[(3 << 5) | 5, 0x03, 0x0C, 0x00, 0x10, 0x00]);
        edid[b + 2] = (p + 6 - b) as u8;
        checksum(&mut edid, b);
        let info = parse(&edid).unwrap();
        assert!(info.checksum_valid && info.has_hdmi_vsdb);
        assert_eq!((info.length, info.extension_count), (256, 1));
        assert_eq!((info.native_width, info.native_height), (Some(2560), Some(1080)));
        assert!((info.native_pixel_clock_mhz.unwrap() - 185.58).abs() < 0.001);
    }
}
