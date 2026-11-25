//! What the monitor announces (EDID) against what the graphics driver offers, per mode (DSP-03), and why a native
//! mode may be missing. Rules: no mode is invented (every option comes from the EDID or the driver); whether an
//! announced-only mode can be tried is decided ONLY by the driver's own validation (`driver_accepts`, i.e.
//! CDS_TEST); pixel-clock estimates and link limits only explain, they never block or allow anything. When the
//! data cannot tell the cause, it is a Hypothesis with the alternatives named.

use crate::edid::{EdidCapabilities, EdidMode, ModeSource};
use crate::mode::{DisplayMode, best_first};
use bitbybit_core::{LocalizedText, text};
use bitbybit_evidence::EvidenceKind;

/// How a pixel clock was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockBasis {
    /// The EDID detailed timing of this very mode (exact).
    Detailed,
    /// The CEA-861 VIC definition (exact).
    CeaVic,
    /// The VESA DMT table entry for this resolution and refresh (assumed: the monitor may use another timing).
    Dmt,
    /// Computed with the CVT reduced-blanking (v1) formula (assumed).
    CvtRb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelClock {
    pub khz: u32,
    pub basis: ClockBasis,
}

impl PixelClock {
    pub fn exact(&self) -> bool {
        matches!(self.basis, ClockBasis::Detailed | ClockBasis::CeaVic)
    }

    pub fn mhz_text(&self) -> String {
        format!("{:.2}", f64::from(self.khz) / 1000.0).trim_end_matches('0').trim_end_matches('.').replace('.', ",")
    }
}

/// VESA DMT entries used for modes known only by resolution and refresh: (w, h, Hz, kHz).
const DMT: [(u32, u32, u32, u32); 14] = [
    (640, 480, 60, 25_175),
    (800, 600, 60, 40_000),
    (1024, 768, 60, 65_000),
    (1280, 720, 60, 74_250),
    (1280, 1024, 60, 108_000),
    (1280, 1024, 75, 135_000),
    (1366, 768, 60, 85_500),
    (1600, 900, 60, 108_000),
    (1680, 1050, 60, 146_250),
    (1920, 1080, 60, 148_500),
    (1920, 1200, 60, 154_000),
    (2560, 1440, 60, 241_500),
    (2560, 1600, 60, 268_500),
    (3840, 2160, 60, 533_250),
];

/// CVT 1.1 reduced blanking (v1): 160-pixel horizontal blank, ≥460 µs vertical blank, 0.25 MHz clock steps.
pub fn cvt_rb_khz(width: u32, height: u32, refresh_hz: f64) -> u32 {
    let vsync = match (width * 3 == height * 4, width * 9 == height * 16, width * 10 == height * 16) {
        (true, _, _) => 4,
        (_, true, _) => 5,
        (_, _, true) => 6,
        _ if width * 4 == height * 5 || width * 9 == height * 15 => 7,
        _ => 10,
    };
    let h_period_us = (1_000_000.0 / refresh_hz - 460.0) / f64::from(height);
    let vbi = ((460.0 / h_period_us).floor() as u32 + 1).max(3 + vsync + 6);
    let mhz = refresh_hz * f64::from(height + vbi) * f64::from(width + 160) / 1_000_000.0;
    ((mhz / 0.25).floor() * 250.0) as u32
}

fn same(edid: &EdidMode, mode: DisplayMode) -> bool {
    edid.width == mode.width
        && edid.height == mode.height
        && (edid.refresh_hz * 1000.0 - f64::from(mode.refresh_mhz)).abs() <= 1000.0
}

/// The pixel clock of `mode` from its own timing when known, else the DMT entry, else CVT-RB (both assumed).
pub fn pixel_clock(mode: DisplayMode, edid: Option<&EdidCapabilities>) -> PixelClock {
    let exact = edid.and_then(|c| c.modes.iter().find(|m| m.pixel_clock_khz.is_some() && same(m, mode)));
    if let Some(m) = exact {
        let basis = if m.source == ModeSource::CeaVic { ClockBasis::CeaVic } else { ClockBasis::Detailed };
        return PixelClock { khz: m.pixel_clock_khz.unwrap_or_default(), basis };
    }
    let hz = (f64::from(mode.refresh_mhz) / 1000.0).round() as u32;
    match DMT.iter().find(|d| (d.0, d.1, d.2) == (mode.width, mode.height, hz)) {
        Some(d) => PixelClock { khz: d.3, basis: ClockBasis::Dmt },
        None => PixelClock {
            khz: cvt_rb_khz(mode.width, mode.height, f64::from(mode.refresh_mhz) / 1000.0),
            basis: ClockBasis::CvtRb,
        },
    }
}

/// The EDID's modes as the driver would name them (whole hertz, as DEVMODE carries them), deduplicated.
pub fn announced(caps: &EdidCapabilities) -> Vec<DisplayMode> {
    best_first(
        caps.modes
            .iter()
            .map(|m| DisplayMode { width: m.width, height: m.height, refresh_mhz: m.refresh_hz.round() as u32 * 1000 })
            .collect(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// Offered by the driver and announced by the EDID.
    Offered,
    /// Offered by the driver, not in the EDID (kept, labelled).
    OfferedNotAnnounced,
    /// Announced by the EDID, not offered, and the driver accepted it in a validation call: can be tried.
    AnnouncedTestable,
    /// Announced by the EDID, not offered, refused by the driver's validation: never applied.
    AnnouncedRefused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeOption {
    pub mode: DisplayMode,
    pub availability: Availability,
    pub native: bool,
    pub recommended: bool,
    pub current: bool,
    pub clock: PixelClock,
}

impl ModeOption {
    /// Whether the user may choose it (offered, or validated by the driver).
    pub fn selectable(&self) -> bool {
        self.availability != Availability::AnnouncedRefused
    }
}

/// Every option, largest and fastest first. `driver_accepts` is the driver's own validation (CDS_TEST) and is
/// asked only for announced modes the driver does not list.
pub fn compare(
    current: DisplayMode,
    offered: &[DisplayMode],
    edid: Option<&EdidCapabilities>,
    driver_accepts: impl Fn(DisplayMode) -> bool,
) -> Vec<ModeOption> {
    let announced = edid.map(announced).unwrap_or_default();
    let near = |a: &DisplayMode, b: &DisplayMode| {
        a.width == b.width && a.height == b.height && a.refresh_mhz.abs_diff(b.refresh_mhz) <= 1000
    };
    let native = edid.and_then(EdidCapabilities::preferred).map(|p| (p.width, p.height));
    let option = |mode: DisplayMode, availability| ModeOption {
        mode,
        availability,
        native: native == Some((mode.width, mode.height)),
        recommended: false,
        current: mode == current,
        clock: pixel_clock(mode, edid),
    };
    let mut out: Vec<ModeOption> = best_first(offered.to_vec())
        .into_iter()
        .map(|m| {
            let listed = edid.is_none() || announced.iter().any(|a| near(a, &m));
            option(m, if listed { Availability::Offered } else { Availability::OfferedNotAnnounced })
        })
        .collect();
    for a in announced.iter().filter(|a| !offered.iter().any(|o| near(o, a))) {
        let availability =
            if driver_accepts(*a) { Availability::AnnouncedTestable } else { Availability::AnnouncedRefused };
        out.push(option(*a, availability));
    }
    if let Some(best) = out
        .iter_mut()
        .filter(|o| o.native && o.availability == Availability::Offered)
        .max_by_key(|o| o.mode.refresh_mhz)
    {
        best.recommended = true;
    }
    out.sort_by_key(|o| {
        std::cmp::Reverse((u64::from(o.mode.width) * u64::from(o.mode.height), o.mode.width, o.mode.refresh_mhz))
    });
    out
}

/// One typed statement of a limitation diagnosis.
#[derive(Debug, Clone, PartialEq)]
pub struct Evidence {
    pub kind: EvidenceKind,
    pub text: LocalizedText,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Limitation {
    pub evidence: Vec<Evidence>,
    /// What would have to change (advice only; nothing is changed).
    pub guidance: Vec<LocalizedText>,
}

fn label(m: DisplayMode) -> String {
    format!("{} × {} · {} Hz", m.width, m.height, (f64::from(m.refresh_mhz) / 1000.0).to_string().replace('.', ","))
}

/// A link that the CCD output technology allows, with its typical TMDS limit in kHz (`None`: not known).
struct Link {
    can_carry: LocalizedText,
    cannot_carry: LocalizedText,
    max_khz: Option<u32>,
}

fn links(technology: Option<i32>, caps: Option<&EdidCapabilities>, mode: &str, clock: &str) -> Vec<Link> {
    let (m, c) = (mode.to_owned(), clock.to_owned());
    let link = |max_khz: u32, can: LocalizedText, cannot: LocalizedText| Link {
        can_carry: can,
        cannot_carry: cannot,
        max_khz: Some(max_khz),
    };
    match technology {
        Some(4) => vec![
            link(
                165_000,
                text!("display.limit.link.dviSingle.ok", "Single-link DVI (about 165 MHz) could carry {0}.", m.clone()),
                text!(
                    "display.limit.link.dviSingle",
                    "If the connection is single-link DVI (about 165 MHz), it cannot carry {0} ({1} MHz).",
                    m.clone(),
                    c.clone()
                ),
            ),
            link(
                330_000,
                text!("display.limit.link.dviDual.ok", "Dual-link DVI (2 × 165 MHz) could carry {0}.", m.clone()),
                text!(
                    "display.limit.link.dviDual",
                    "If the connection is dual-link DVI (2 × 165 MHz), it cannot carry {0} ({1} MHz).",
                    m.clone(),
                    c.clone()
                ),
            ),
            link(
                165_000,
                text!(
                    "display.limit.link.dpType1.ok",
                    "A passive DisplayPort-to-DVI/HDMI adapter of DP++ type 1 (about 165 MHz) could carry {0}.",
                    m.clone()
                ),
                text!(
                    "display.limit.link.dpType1",
                    "If a passive DisplayPort-to-DVI/HDMI adapter of DP++ type 1 is in the chain (about 165 MHz), it cannot carry {0} ({1} MHz).",
                    m.clone(),
                    c.clone()
                ),
            ),
            link(
                300_000,
                text!(
                    "display.limit.link.dpType2.ok",
                    "A DP++ type 2 adapter (about 300 MHz) could carry {0}.",
                    m.clone()
                ),
                text!(
                    "display.limit.link.dpType2",
                    "If a DP++ type 2 adapter is in the chain (about 300 MHz), it cannot carry {0} ({1} MHz).",
                    m.clone(),
                    c.clone()
                ),
            ),
        ],
        Some(5) => match caps.and_then(|x| x.hdmi_max_tmds_mhz) {
            Some(max) => vec![link(
                u32::from(max) * 1000,
                text!(
                    "display.limit.link.hdmiDeclared.ok",
                    "The monitor declares HDMI up to {0} MHz, enough for {1}.",
                    max.to_string(),
                    m.clone()
                ),
                text!(
                    "display.limit.link.hdmiDeclared",
                    "The monitor declares HDMI up to {0} MHz; {1} needs {2} MHz.",
                    max.to_string(),
                    m.clone(),
                    c.clone()
                ),
            )],
            None => vec![
                link(
                    165_000,
                    text!("display.limit.link.hdmiOld.ok", "HDMI 1.0–1.2 (165 MHz) could carry {0}.", m.clone()),
                    text!(
                        "display.limit.link.hdmiOld",
                        "If the link runs as HDMI 1.0–1.2 or as DVI over HDMI (165 MHz), it cannot carry {0} ({1} MHz).",
                        m.clone(),
                        c.clone()
                    ),
                ),
                link(
                    340_000,
                    text!("display.limit.link.hdmi13.ok", "HDMI 1.3 or newer (340 MHz) could carry {0}.", m.clone()),
                    text!(
                        "display.limit.link.hdmi13",
                        "Even HDMI 1.3 or newer (340 MHz) cannot carry {0} ({1} MHz).",
                        m.clone(),
                        c.clone()
                    ),
                ),
            ],
        },
        _ => Vec::new(),
    }
}

fn technology_text(technology: Option<i32>) -> LocalizedText {
    match technology {
        Some(4) => text!(
            "display.limit.tech.dvi",
            "Windows reports the connection as DVI. It does not say whether it is single- or dual-link, or whether an adapter is in the chain."
        ),
        Some(5) => text!("display.limit.tech.hdmi", "Windows reports the connection as HDMI."),
        Some(10 | 11 | 18) => text!(
            "display.limit.tech.dp",
            "Windows reports the connection as DisplayPort. Its lane count and rate are not reported, so its bandwidth limit is not known."
        ),
        Some(0) => text!(
            "display.limit.tech.vga",
            "Windows reports an analogue VGA connection: the limit depends on the card's converter and the cable."
        ),
        Some(t) => text!(
            "display.limit.tech.other",
            "Windows reports the connection type as {0}.",
            crate::ccd::output_technology_name(t)
        ),
        None => text!("display.limit.tech.unknown", "Windows did not report the connection type."),
    }
}

/// Explains, from the data only, why the monitor's native/fastest mode is not offered; `None` when it is.
pub fn diagnose(
    options: &[ModeOption],
    caps: Option<&EdidCapabilities>,
    technology: Option<i32>,
    raw: &[DisplayMode],
) -> Option<Limitation> {
    use EvidenceKind::{Fact, Hypothesis, Inference, Observation};
    let caps = caps?;
    let preferred = caps.preferred()?;
    let is_announced = |o: &&ModeOption| o.availability != Availability::OfferedNotAnnounced;
    let target = options.iter().filter(|o| o.native).filter(is_announced).max_by_key(|o| o.mode.refresh_mhz)?;
    let driver = |o: &&ModeOption| matches!(o.availability, Availability::Offered | Availability::OfferedNotAnnounced);
    let best_native = options.iter().filter(|o| o.native).filter(driver).map(|o| o.mode.refresh_mhz).max();
    if best_native.is_some_and(|hz| hz + 1000 > target.mode.refresh_mhz) {
        return None;
    }
    let (mode, clock) = (label(target.mode), target.clock.mhz_text());
    let mut l = Limitation::default();
    let mut add = |kind, text| l.evidence.push(Evidence { kind, text });
    add(
        Fact,
        text!(
            "display.limit.native",
            "The monitor's EDID announces {0} × {1} as its native mode, up to {2}.",
            preferred.width.to_string(),
            preferred.height.to_string(),
            mode.clone()
        ),
    );
    match target.clock.basis {
        ClockBasis::Detailed | ClockBasis::CeaVic => add(
            Fact,
            text!("display.limit.clock.exact", "Its timing in the EDID needs a pixel clock of {0} MHz.", clock.clone()),
        ),
        ClockBasis::Dmt => add(
            Inference,
            text!(
                "display.limit.clock.dmt",
                "Assuming the VESA DMT timing, it needs a pixel clock of about {0} MHz (estimate).",
                clock.clone()
            ),
        ),
        ClockBasis::CvtRb => add(
            Inference,
            text!(
                "display.limit.clock.cvtRb",
                "Assuming CVT reduced-blanking timing, it needs a pixel clock of about {0} MHz (estimate).",
                clock.clone()
            ),
        ),
    }
    let offered = options
        .iter()
        .filter(driver)
        .max_by_key(|o| (u64::from(o.mode.width) * u64::from(o.mode.height), o.mode.refresh_mhz));
    match (best_native, offered) {
        (Some(hz), _) => add(
            Fact,
            text!(
                "display.limit.offeredNative",
                "The graphics driver offers {0} × {1} only up to {2} Hz.",
                target.mode.width.to_string(),
                target.mode.height.to_string(),
                (hz / 1000).to_string()
            ),
        ),
        (None, Some(o)) => add(
            Fact,
            text!(
                "display.limit.offeredMax",
                "The graphics driver does not offer {0} × {1}; the largest mode it offers is {2}.",
                target.mode.width.to_string(),
                target.mode.height.to_string(),
                label(o.mode)
            ),
        ),
        (None, None) => {}
    }
    if raw.iter().any(|r| r.width == target.mode.width && r.height == target.mode.height) {
        add(
            Observation,
            text!(
                "display.limit.raw",
                "The driver knows {0} × {1} in its unfiltered list but does not offer it for this monitor and connection.",
                target.mode.width.to_string(),
                target.mode.height.to_string()
            ),
        );
    }
    add(Fact, technology_text(technology));
    let mut explained = false;
    if caps.length <= 128 && !caps.has_cea_extension {
        add(
            Observation,
            text!(
                "display.limit.edidShort",
                "Windows read only the 128-byte base EDID, with no CEA-861 extension: the monitor is treated as a DVI device (no audio, no HDMI modes)."
            ),
        );
    }
    let candidates = links(technology, Some(caps), &mode, &clock);
    let certain = candidates.len() == 1 && target.clock.exact();
    for link in &candidates {
        let Some(max) = link.max_khz else { continue };
        let fails = target.clock.khz > max;
        explained |= fails;
        let kind = if certain { Inference } else { Hypothesis };
        add(kind, if fails { link.cannot_carry.clone() } else { link.can_carry.clone() });
    }
    if let Some(r) = caps.range {
        if let Some(max) = r.max_pixel_clock_mhz.filter(|m| u32::from(*m) * 1000 < target.clock.khz) {
            explained = true;
            add(
                Inference,
                text!(
                    "display.limit.rangeClock",
                    "The monitor's range limits declare a pixel clock of at most {0} MHz, below the {1} MHz this mode needs.",
                    max.to_string(),
                    clock.clone()
                ),
            );
        }
        if u32::from(r.max_vertical_hz) * 1000 + 1000 < target.mode.refresh_mhz {
            explained = true;
            add(
                Inference,
                text!(
                    "display.limit.rangeRefresh",
                    "The monitor's range limits declare at most {0} Hz, below {1}.",
                    r.max_vertical_hz.to_string(),
                    mode.clone()
                ),
            );
        }
    }
    if !explained {
        add(
            Hypothesis,
            text!(
                "display.limit.driver",
                "Nothing in the collected data explains it: probably a limit of the graphics driver or card for this output (for example its maximum pixel clock)."
            ),
        );
    }
    if matches!(technology, Some(4 | 5)) {
        {
            l.guidance.push(text!(
                "display.limit.fix.dp",
                "If the card and the monitor both have DisplayPort, use a DisplayPort–DisplayPort cable (or a dual-link DVI cable and port)."
            ));
            l.guidance.push(text!(
                "display.limit.fix.adapter",
                "If an adapter is in the chain, use an active adapter rated for this resolution."
            ));
        }
    }
    if caps.length <= 128 && !caps.has_cea_extension {
        l.guidance.push(text!(
            "display.limit.fix.edid",
            "Connect without adapters, switches or KVMs so Windows can read the full EDID, or try another input on the monitor."
        ));
    }
    l.guidance.push(text!(
        "display.limit.fix.driver",
        "Install the newest graphics driver from the card maker and check the card's maximum resolution for this output."
    ));
    Some(l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edid::{capabilities, tests::with_detailed};
    use bitbybit_localization::TextCatalog;

    const M1080: DisplayMode = DisplayMode { width: 1920, height: 1080, refresh_mhz: 60_000 };
    const UW: DisplayMode = DisplayMode { width: 2560, height: 1080, refresh_mhz: 60_000 };

    /// The reported case (fixture, not a real dump): 128-byte EDID, native 2560×1080 CVT-RB at 181.25 MHz.
    fn ultrawide() -> EdidCapabilities {
        capabilities(&with_detailed(false, 18_125, 2560, 160, 1080, 31)).unwrap()
    }

    fn texts(l: &Limitation, kind: EvidenceKind) -> Vec<String> {
        let catalog = TextCatalog::default_catalog();
        l.evidence.iter().filter(|e| e.kind == kind).map(|e| catalog.resolve(&e.text)).collect()
    }

    #[test]
    fn pixel_clocks_exact_from_the_timing_or_computed_with_a_named_formula() {
        assert_eq!(cvt_rb_khz(2560, 1080, 60.0), 181_250);
        assert_eq!(cvt_rb_khz(1920, 1080, 60.0), 138_500);
        let caps = ultrawide();
        assert_eq!(pixel_clock(UW, Some(&caps)), PixelClock { khz: 181_250, basis: ClockBasis::Detailed });
        assert_eq!(pixel_clock(M1080, None), PixelClock { khz: 148_500, basis: ClockBasis::Dmt });
        let odd = DisplayMode { width: 2560, height: 1080, refresh_mhz: 75_000 };
        assert_eq!(pixel_clock(odd, None).basis, ClockBasis::CvtRb);
        assert!(!pixel_clock(odd, None).exact() && pixel_clock(UW, Some(&caps)).exact());
        assert_eq!(pixel_clock(UW, Some(&caps)).mhz_text(), "181,25");
    }

    #[test]
    fn options_are_compared_and_the_native_mode_is_recommended() {
        let caps = ultrawide();
        let offered = [UW, M1080, DisplayMode { width: 1024, height: 768, refresh_mhz: 60_000 }];
        let options = compare(M1080, &offered, Some(&caps), |_| false);
        let uw = options.iter().find(|o| o.mode == UW).unwrap();
        assert!(uw.native && uw.recommended && uw.availability == Availability::Offered);
        assert!(options.iter().find(|o| o.mode == M1080).unwrap().current);
        assert_eq!(options.iter().filter(|o| o.recommended).count(), 1);
        // Not in the fixture's EDID: offered by the driver only, kept and labelled.
        assert_eq!(
            options.iter().find(|o| o.mode.width == 1024).unwrap().availability,
            Availability::OfferedNotAnnounced
        );
        assert!(diagnose(&options, Some(&caps), Some(4), &[]).is_none(), "native offered: nothing to explain");
    }

    #[test]
    fn testability_depends_only_on_the_driver_validation_never_on_the_estimate() {
        let caps = ultrawide();
        // The estimate says single-link DVI cannot carry 181.25 MHz, but the driver validates it: testable.
        let accepted = compare(M1080, &[M1080], Some(&caps), |m| m == UW);
        let uw = accepted.iter().find(|o| o.mode == UW).unwrap();
        assert_eq!(uw.availability, Availability::AnnouncedTestable);
        assert!(uw.selectable() && !uw.recommended, "only offered modes are recommended");
        // A mode the estimate says fits but the driver refuses is never selectable.
        let small = capabilities(&with_detailed(false, 7_425, 1280, 370, 720, 30)).unwrap();
        let refused = compare(M1080, &[M1080], Some(&small), |_| false);
        let m720 = refused.iter().find(|o| o.mode.width == 1280).unwrap();
        assert!(m720.clock.khz < 165_000);
        assert_eq!(m720.availability, Availability::AnnouncedRefused);
        assert!(!m720.selectable());
    }

    #[test]
    fn the_dvi_case_names_single_and_dual_link_as_hypotheses() {
        let caps = ultrawide();
        let options = compare(M1080, &[M1080], Some(&caps), |_| false);
        let l = diagnose(&options, Some(&caps), Some(4), &[UW]).unwrap();
        let facts = texts(&l, EvidenceKind::Fact);
        assert!(facts.iter().any(|t| t.contains("2560 × 1080") && t.contains("nativo")), "{facts:?}");
        assert!(facts.iter().any(|t| t.contains("181,25")), "the DTD clock is a fact: {facts:?}");
        assert!(facts.iter().any(|t| t.contains("DVI")));
        let hyp = texts(&l, EvidenceKind::Hypothesis);
        assert!(hyp.iter().any(|t| t.contains("ligação simples") && t.contains("181,25")), "{hyp:?}");
        assert!(hyp.iter().any(|t| t.contains("ligação dupla")), "{hyp:?}");
        assert!(texts(&l, EvidenceKind::Inference).is_empty(), "the link is not certain: no inference");
        assert!(texts(&l, EvidenceKind::Observation).iter().any(|t| t.contains("128")));
        assert!(!hyp.iter().any(|t| t.contains("controlador gráfico ou da placa")), "explained by the link");
        assert!(l.guidance.len() >= 3);
    }

    #[test]
    fn hdmi_with_a_declared_tmds_limit_is_an_inference() {
        let caps = capabilities(&with_detailed(true, 18_125, 2560, 160, 1080, 31)).unwrap();
        assert_eq!(caps.hdmi_max_tmds_mhz, Some(165));
        let options = compare(M1080, &[M1080], Some(&caps), |_| false);
        let l = diagnose(&options, Some(&caps), Some(5), &[]).unwrap();
        let inf = texts(&l, EvidenceKind::Inference);
        assert!(inf.iter().any(|t| t.contains("165") && t.contains("181,25")), "{inf:?}");
    }

    #[test]
    fn displayport_with_nothing_explaining_it_is_a_driver_hypothesis() {
        let caps = ultrawide();
        let options = compare(M1080, &[M1080], Some(&caps), |_| false);
        let l = diagnose(&options, Some(&caps), Some(10), &[]).unwrap();
        let hyp = texts(&l, EvidenceKind::Hypothesis);
        assert_eq!(hyp.len(), 1, "{hyp:?}");
        assert!(hyp[0].contains("controlador gráfico ou da placa"), "{hyp:?}");
    }

    #[test]
    fn every_limitation_key_is_in_the_catalog() {
        let catalog = TextCatalog::default_catalog();
        let caps = ultrawide();
        let options = compare(M1080, &[M1080], Some(&caps), |_| false);
        for tech in [Some(0), Some(4), Some(5), Some(10), Some(15), None] {
            let l = diagnose(&options, Some(&caps), tech, &[UW]).unwrap();
            for t in l.evidence.iter().map(|e| &e.text).chain(&l.guidance) {
                assert!(catalog.template(&t.key).is_some(), "{}", t.key);
            }
        }
        for t in links(Some(4), None, "x", "1").into_iter().chain(links(Some(5), None, "x", "1")) {
            assert!(catalog.template(&t.can_carry.key).is_some(), "{}", t.can_carry.key);
            assert!(catalog.template(&t.cannot_carry.key).is_some(), "{}", t.cannot_carry.key);
        }
        let hdmi = capabilities(&with_detailed(true, 18_125, 2560, 160, 1080, 31)).unwrap();
        for t in links(Some(5), Some(&hdmi), "x", "1") {
            assert!(catalog.template(&t.can_carry.key).is_some() && catalog.template(&t.cannot_carry.key).is_some());
        }
        for k in [
            "display.limit.clock.dmt",
            "display.limit.clock.cvtRb",
            "display.limit.rangeClock",
            "display.limit.rangeRefresh",
            "display.limit.offeredNative",
        ] {
            assert!(catalog.template(k).is_some(), "{k}");
        }
    }
}
