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

/// Where an EDID mode comes from (it decides how exact its pixel clock is).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSource {
    /// Established timings (bytes 35–37): VESA DMT modes by definition.
    Established,
    /// Standard timings (bytes 38–53): resolution and refresh only.
    Standard,
    /// A detailed timing descriptor of the base block: exact timing.
    Detailed,
    /// A CEA-861 Short Video Descriptor: the VIC defines the exact timing.
    CeaVic,
    /// A detailed timing descriptor of the CEA-861 extension.
    CeaDetailed,
}

/// One mode the EDID announces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdidMode {
    pub width: u32,
    pub height: u32,
    /// Exact for detailed timings (pixel clock / totals), nominal otherwise.
    pub refresh_hz: f64,
    /// From the timing itself (detailed timing or VIC), never estimated here.
    pub pixel_clock_khz: Option<u32>,
    pub source: ModeSource,
    /// The first detailed timing (the preferred, normally native, mode) or a VIC flagged native.
    pub preferred: bool,
}

/// Display Range Limits descriptor (tag 0xFD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeLimits {
    pub min_vertical_hz: u16,
    pub max_vertical_hz: u16,
    pub min_horizontal_khz: u16,
    pub max_horizontal_khz: u16,
    pub max_pixel_clock_mhz: Option<u16>,
}

/// Everything the EDID declares about modes and the link. Never extracts serial numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct EdidCapabilities {
    pub version: (u8, u8),
    pub digital: bool,
    /// EDID 1.4 only.
    pub bits_per_color: Option<u8>,
    /// EDID 1.4 only: "DVI", "HDMI-a", "HDMI-b", "MDDI", "DisplayPort".
    pub interface: Option<&'static str>,
    pub modes: Vec<EdidMode>,
    pub range: Option<RangeLimits>,
    /// HDMI VSDB "Max_TMDS_Clock" (×5 MHz), when the monitor declares it.
    pub hdmi_max_tmds_mhz: Option<u16>,
    pub has_cea_extension: bool,
    pub has_hdmi_vsdb: bool,
    pub announces_audio: bool,
    pub length: usize,
}

impl EdidCapabilities {
    /// The preferred (native) mode: the first detailed timing, else a native VIC.
    pub fn preferred(&self) -> Option<&EdidMode> {
        self.modes.iter().find(|m| m.preferred)
    }
}

const ESTABLISHED: [(usize, u8, u32, u32, u32); 16] = [
    (35, 7, 720, 400, 70),
    (35, 6, 720, 400, 88),
    (35, 5, 640, 480, 60),
    (35, 4, 640, 480, 67),
    (35, 3, 640, 480, 72),
    (35, 2, 640, 480, 75),
    (35, 1, 800, 600, 56),
    (35, 0, 800, 600, 60),
    (36, 7, 800, 600, 72),
    (36, 6, 800, 600, 75),
    (36, 5, 832, 624, 75),
    // (36, 4) is 1024×768 interlaced: never offered.
    (36, 3, 1024, 768, 60),
    (36, 2, 1024, 768, 70),
    (36, 1, 1024, 768, 75),
    (36, 0, 1280, 1024, 75),
    (37, 7, 1152, 870, 75),
];

/// Common CEA-861 VICs (progressive only): (vic, width, height, refresh Hz, pixel clock kHz).
pub const VICS: [(u8, u32, u32, u32, u32); 18] = [
    (1, 640, 480, 60, 25_175),
    (2, 720, 480, 60, 27_000),
    (3, 720, 480, 60, 27_000),
    (4, 1280, 720, 60, 74_250),
    (16, 1920, 1080, 60, 148_500),
    (17, 720, 576, 50, 27_000),
    (18, 720, 576, 50, 27_000),
    (19, 1280, 720, 50, 74_250),
    (31, 1920, 1080, 50, 148_500),
    (32, 1920, 1080, 24, 74_250),
    (33, 1920, 1080, 25, 74_250),
    (34, 1920, 1080, 30, 74_250),
    (63, 1920, 1080, 120, 297_000),
    (64, 1920, 1080, 100, 297_000),
    (93, 3840, 2160, 24, 297_000),
    (95, 3840, 2160, 30, 297_000),
    (96, 3840, 2160, 50, 594_000),
    (97, 3840, 2160, 60, 594_000),
];

/// An 18-byte detailed timing; `None` for a display descriptor, an interlaced or a malformed timing.
fn detailed(d: &[u8], source: ModeSource, preferred: bool) -> Option<EdidMode> {
    let clock_khz = (u32::from(d[0]) | (u32::from(d[1]) << 8)) * 10;
    if clock_khz == 0 || d[17] & 0x80 != 0 {
        return None;
    }
    let h_active = u32::from(d[2]) | ((u32::from(d[4]) & 0xF0) << 4);
    let h_blank = u32::from(d[3]) | ((u32::from(d[4]) & 0x0F) << 8);
    let v_active = u32::from(d[5]) | ((u32::from(d[7]) & 0xF0) << 4);
    let v_blank = u32::from(d[6]) | ((u32::from(d[7]) & 0x0F) << 8);
    let total = f64::from(h_active + h_blank) * f64::from(v_active + v_blank);
    if h_active == 0 || v_active == 0 || total == 0.0 {
        return None;
    }
    Some(EdidMode {
        width: h_active,
        height: v_active,
        refresh_hz: (f64::from(clock_khz) * 1000.0 / total * 100.0).round() / 100.0,
        pixel_clock_khz: Some(clock_khz),
        source,
        preferred,
    })
}

fn standard(b1: u8, b2: u8, version_minor: u8) -> Option<EdidMode> {
    if (b1 == 0x01 && b2 == 0x01) || b1 == 0x00 {
        return None;
    }
    let width = (u32::from(b1) + 31) * 8;
    let height = match b2 >> 6 {
        0 if version_minor < 3 => width,
        0 => width * 10 / 16,
        1 => width * 3 / 4,
        2 => width * 4 / 5,
        _ => width * 9 / 16,
    };
    let refresh_hz = f64::from(u32::from(b2 & 0x3F) + 60);
    Some(EdidMode { width, height, refresh_hz, pixel_clock_khz: None, source: ModeSource::Standard, preferred: false })
}

/// Every mode, the range limits and the link declarations of an EDID (base block + CEA-861 extensions).
pub fn capabilities(edid: &[u8]) -> Option<EdidCapabilities> {
    let info = parse(edid)?;
    let version = (edid[18], edid[19]);
    let digital = edid[20] & 0x80 != 0;
    let (bits_per_color, interface) = if digital && version.1 >= 4 {
        let depth = match (edid[20] >> 4) & 0x07 {
            d @ 1..=6 => Some(4 + 2 * d),
            _ => None,
        };
        let interface = match edid[20] & 0x0F {
            1 => Some("DVI"),
            2 => Some("HDMI-a"),
            3 => Some("HDMI-b"),
            4 => Some("MDDI"),
            5 => Some("DisplayPort"),
            _ => None,
        };
        (depth, interface)
    } else {
        (None, None)
    };
    let mut modes = Vec::new();
    for (offset, i) in (54..=108).step_by(18).zip(0..) {
        modes.extend(detailed(&edid[offset..offset + 18], ModeSource::Detailed, i == 0));
    }
    for (byte, bit, width, height, hz) in ESTABLISHED {
        if edid[byte] & (1 << bit) != 0 {
            modes.push(EdidMode {
                width,
                height,
                refresh_hz: f64::from(hz),
                pixel_clock_khz: None,
                source: ModeSource::Established,
                preferred: false,
            });
        }
    }
    for i in (38..54).step_by(2) {
        modes.extend(standard(edid[i], edid[i + 1], version.1));
    }
    let mut range = None;
    for offset in (54..=108).step_by(18) {
        let d = &edid[offset..offset + 18];
        if d[..4] == [0, 0, 0, 0xFD] {
            let add = |bit: u8| if d[4] & bit != 0 { 255 } else { 0 };
            range = Some(RangeLimits {
                min_vertical_hz: u16::from(d[5]) + add(0x01),
                max_vertical_hz: u16::from(d[6]) + add(0x02),
                min_horizontal_khz: u16::from(d[7]) + add(0x04),
                max_horizontal_khz: u16::from(d[8]) + add(0x08),
                max_pixel_clock_mhz: (d[9] != 0).then(|| u16::from(d[9]) * 10),
            });
        }
    }
    let mut hdmi_max_tmds_mhz = None;
    for i in 1..=usize::from(edid[126]) {
        let Some(block) = edid.get(i * 128..i * 128 + 128) else { break };
        if block[0] != 0x02 || block[1] < 3 {
            continue;
        }
        let dtd_offset = usize::from(block[2]);
        if dtd_offset > 4 && dtd_offset <= 127 {
            let mut p = 4;
            while p < dtd_offset {
                let (tag, len) = (block[p] >> 5, usize::from(block[p] & 0x1F));
                if p + 1 + len > dtd_offset {
                    break;
                }
                let payload = &block[p + 1..p + 1 + len];
                if tag == 2 {
                    for &svd in payload {
                        // VIC 1–64 may carry the "native" flag in bit 7 (CEA-861-F).
                        let (vic, native) = if (129..=192).contains(&svd) { (svd & 0x7F, true) } else { (svd, false) };
                        if let Some(&(_, width, height, hz, khz)) = VICS.iter().find(|v| v.0 == vic) {
                            modes.push(EdidMode {
                                width,
                                height,
                                refresh_hz: f64::from(hz),
                                pixel_clock_khz: Some(khz),
                                source: ModeSource::CeaVic,
                                preferred: native && !modes.iter().any(|m: &EdidMode| m.preferred),
                            });
                        }
                    }
                }
                if tag == 3 && len >= 7 && payload[..3] == [0x03, 0x0C, 0x00] && payload[6] != 0 {
                    hdmi_max_tmds_mhz = Some(u16::from(payload[6]) * 5);
                }
                p += 1 + len;
            }
        }
        if dtd_offset >= 4 {
            let mut o = dtd_offset;
            while o + 18 <= 127 {
                match detailed(&block[o..o + 18], ModeSource::CeaDetailed, false) {
                    Some(m) => modes.push(m),
                    None if block[o] == 0 && block[o + 1] == 0 => break,
                    None => {}
                }
                o += 18;
            }
        }
    }
    Some(EdidCapabilities {
        version,
        digital,
        bits_per_color,
        interface,
        modes,
        range,
        hdmi_max_tmds_mhz,
        has_cea_extension: info.has_cea_extension,
        has_hdmi_vsdb: info.has_hdmi_vsdb,
        announces_audio: info.announces_audio(),
        length: info.length,
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

    /// Synthetic EDID 1.3 (fixture): `clock` in 10 kHz units as the preferred detailed timing, 640×480@60
    /// established, 1920×1080@60 standard, range limits 50–75 Hz / 30–90 kHz / 190 MHz; with `hdmi` a CEA-861
    /// block holding an HDMI VSDB (max TMDS 165 MHz) and VIC 16 flagged native.
    pub fn with_detailed(hdmi: bool, clock: u16, h: u32, h_blank: u32, v: u32, v_blank: u32) -> Vec<u8> {
        let mut edid = build("GSM", 0x5A3E, "LG ULTRAWIDE", hdmi, true, false);
        let d = 54;
        edid[d] = clock as u8;
        edid[d + 1] = (clock >> 8) as u8;
        edid[d + 2] = h as u8;
        edid[d + 3] = h_blank as u8;
        edid[d + 4] = (((h >> 8) as u8) << 4) | ((h_blank >> 8) as u8 & 0x0F);
        edid[d + 5] = v as u8;
        edid[d + 6] = v_blank as u8;
        edid[d + 7] = (((v >> 8) as u8) << 4) | ((v_blank >> 8) as u8 & 0x0F);
        edid[d + 17] = 0x1E;
        edid[20] = 0x80;
        edid[35] = 1 << 5;
        edid[38] = (1920 / 8 - 31) as u8;
        edid[39] = 0xC0;
        for i in (40..54).step_by(2) {
            edid[i] = 1;
            edid[i + 1] = 1;
        }
        edid[90..108]
            .copy_from_slice(&[0, 0, 0, 0xFD, 0, 50, 75, 30, 90, 19, 0, 0x0A, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20]);
        checksum(&mut edid, 0);
        if hdmi {
            let b = 128;
            let blocks = [(3 << 5) | 7, 0x03, 0x0C, 0x00, 0x10, 0x00, 0x00, 33, (2 << 5) | 1, 0x80 | 16];
            edid[b + 4..b + 4 + blocks.len()].copy_from_slice(&blocks);
            edid[b + 2] = (4 + blocks.len()) as u8;
            checksum(&mut edid, b);
        }
        edid
    }

    #[test]
    fn capabilities_list_every_announced_mode_with_limits() {
        let caps = capabilities(&with_detailed(false, 18_125, 2560, 160, 1080, 31)).unwrap();
        assert_eq!((caps.version, caps.length, caps.has_cea_extension), ((1, 4), 128, false));
        let native = caps.preferred().unwrap();
        assert_eq!((native.width, native.height, native.pixel_clock_khz), (2560, 1080, Some(181_250)));
        assert_eq!(native.source, ModeSource::Detailed);
        assert!((native.refresh_hz - 59.98).abs() < 0.01, "{}", native.refresh_hz);
        let has = |w, h, hz: f64, src| {
            caps.modes
                .iter()
                .any(|m| m.width == w && m.height == h && (m.refresh_hz - hz).abs() < 0.1 && m.source == src)
        };
        assert!(has(640, 480, 60.0, ModeSource::Established));
        assert!(has(1920, 1080, 60.0, ModeSource::Standard));
        assert_eq!(caps.modes.len(), 3, "unused standard slots are skipped: {:?}", caps.modes);
        let r = caps.range.unwrap();
        assert_eq!(
            (r.min_vertical_hz, r.max_vertical_hz, r.max_horizontal_khz, r.max_pixel_clock_mhz),
            (50, 75, 90, Some(190))
        );
        assert!(caps.digital && caps.interface.is_none(), "no interface type declared");
    }

    #[test]
    fn cea_extension_adds_vics_and_the_hdmi_tmds_limit() {
        let caps = capabilities(&with_detailed(true, 18_125, 2560, 160, 1080, 31)).unwrap();
        assert!(caps.has_cea_extension && caps.has_hdmi_vsdb && caps.announces_audio);
        assert_eq!(caps.hdmi_max_tmds_mhz, Some(165));
        let vic = caps.modes.iter().find(|m| m.source == ModeSource::CeaVic).unwrap();
        assert_eq!((vic.width, vic.height, vic.pixel_clock_khz), (1920, 1080, Some(148_500)));
        assert!(!vic.preferred, "the first detailed timing stays the preferred mode");
        assert_eq!(caps.preferred().unwrap().width, 2560);
    }

    #[test]
    fn edid_1_4_declares_the_interface_and_bit_depth() {
        let mut edid = with_detailed(false, 18_125, 2560, 160, 1080, 31);
        edid[19] = 4;
        edid[20] = 0x80 | (2 << 4) | 5; // digital, 8 bits per colour, DisplayPort
        checksum(&mut edid, 0);
        let caps = capabilities(&edid).unwrap();
        assert_eq!((caps.bits_per_color, caps.interface), (Some(8), Some("DisplayPort")));
        assert!(capabilities(&[0u8; 100]).is_none());
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
