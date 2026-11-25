//! Headless UI tests (matrix UI-18): every page is rendered with the FAKE demo rows (`demo.rs`) at several window
//! sizes by Slint's software renderer into a pixel buffer, with no display server. Checks: no panic, the buffer
//! is not blank, the page title and a key panel of the page are present, nothing laid out spills into the right
//! margin or past the window, the sidebar entries fit, and a scrolling page leaves the scroll bar its own gutter.
//! Elements are found by accessible label through `i-slint-backend-testing` (same version as slint).

use crate::ui::{AppWindow, Theme, Tr};
use bitbybit_core::text;
use bitbybit_localization::TextCatalog;
use i_slint_backend_testing::ElementHandle;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, Rgb8Pixel};
use std::rc::Rc;

/// Every page (the order of the sidebar) with the catalog key of a panel title it must show.
const PAGES: [(i32, &str); 10] = [
    (8, "app.chart.title"),
    (6, "app.case.panel"),
    (0, "app.network.interfaces"),
    (1, "app.devices.panel"),
    (7, "app.page.updates.title"),
    (2, "app.audio.panel"),
    (5, "app.displays.panel"),
    (3, "app.changes.panel"),
    (9, "app.settings.updates"),
    (4, "app.settings.credits"),
];
/// The minimum, the initial and two common screen sizes.
const SIZES: [(u32, u32); 5] = [(1100, 660), (1280, 720), (1366, 768), (1600, 900), (1920, 1080)];
/// Pages wrapped as a whole in the `Scroller` component.
const SCROLLED_PAGES: [i32; 4] = [0, 7, 9, 4];

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
    // Animations and the spinner stay at time zero: renders are deterministic.
    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::ZERO
    }
}

struct Harness {
    window: Rc<MinimalSoftwareWindow>,
    ui: AppWindow,
    catalog: &'static TextCatalog,
}

impl Harness {
    /// One per test thread (Slint's platform is per thread).
    fn new() -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform(window.clone())))
            .unwrap_or_else(|e| panic!("platform: {e}"));
        let ui = AppWindow::new().unwrap_or_else(|e| panic!("window: {e}"));
        let catalog = TextCatalog::default_catalog();
        ui.global::<Tr>().on_t(move |key| catalog.get(&key, &[]).into());
        crate::demo::fill(&ui, "1");
        ui.show().unwrap_or_else(|e| panic!("show: {e}"));
        Self { window, ui, catalog }
    }

    fn t(&self, key: &str) -> String {
        self.catalog.resolve(&text!(key, ""))
    }

    /// Lays out `page` at `w`×`h` and renders it; returns the pixels (row-major).
    fn render(&self, page: i32, w: u32, h: u32) -> Vec<Rgb8Pixel> {
        self.ui.set_page(page);
        self.window.set_size(slint::PhysicalSize::new(w, h));
        slint::platform::update_timers_and_animations();
        self.window.request_redraw();
        let mut pixels = vec![Rgb8Pixel::default(); (w * h) as usize];
        assert!(self.window.draw_if_needed(|r| {
            r.render(&mut pixels, w as usize);
        }));
        pixels
    }

    fn elements(&self) -> i_slint_backend_testing::ElementQuery {
        use i_slint_backend_testing::ElementRoot;
        self.ui.root_element().query_descendants()
    }

    fn labelled(&self) -> Vec<(String, ElementHandle)> {
        self.elements()
            .match_predicate(|e| e.accessible_label().is_some_and(|l| !l.is_empty()))
            .find_all()
            .into_iter()
            .map(|e| (e.accessible_label().unwrap_or_default().to_string(), e))
            .collect()
    }
}

fn rgb(c: slint::Color) -> Rgb8Pixel {
    Rgb8Pixel { r: c.red(), g: c.green(), b: c.blue() }
}

#[test]
fn every_page_renders_at_every_size_without_layout_problems() {
    let hx = Harness::new();
    let theme = hx.ui.global::<Theme>();
    let (sidebar, pad, header) = (theme.get_sidebar_width(), theme.get_pad(), theme.get_header_height());
    let workspace = rgb(theme.get_workspace());
    let mut problems = Vec::new();
    for (w, h) in SIZES {
        let (wf, hf) = (w as f32, h as f32);
        for (page, key) in PAGES {
            let at = format!("page {page} at {w}x{h}");
            let pixels = hx.render(page, w, h);

            // Not blank: many colours, and not almost only bare background.
            let mut colours: Vec<(u8, u8, u8)> = pixels.iter().map(|p| (p.r, p.g, p.b)).collect();
            colours.sort_unstable();
            colours.dedup();
            let bare = pixels.iter().filter(|p| **p == workspace).count();
            if colours.len() < 50 || bare * 10 > pixels.len() * 9 {
                problems.push(format!("{at}: looks blank ({} colours, {bare} bare pixels)", colours.len()));
            }

            let labelled = hx.labelled();
            let shown = |label: &str, inside: &dyn Fn(f32, f32) -> bool| {
                labelled.iter().any(|(l, e)| {
                    let (p, s) = (e.absolute_position(), e.size());
                    l == label && s.width > 0.0 && s.height > 0.0 && inside(p.x, p.y)
                })
            };
            // The page title in the header, and the page's key panel in the workspace.
            let title = hx.ui.get_page_title().to_string();
            if !shown(&title, &|x, y| x >= sidebar && y < pad + header) {
                problems.push(format!("{at}: no page title \"{title}\" in the header"));
            }
            let panel = hx.t(key);
            if !shown(&panel, &|x, y| x >= sidebar && y >= pad + header && y < hf) {
                problems.push(format!("{at}: no \"{panel}\" ({key}) on the page"));
            }

            let mut bottom = 0.0f32;
            for (label, e) in &labelled {
                let (p, s) = (e.absolute_position(), e.size());
                if s.width < 0.0 || s.height < 0.0 {
                    problems.push(format!("{at}: \"{label}\" has a negative size {s:?}"));
                }
                if p.x >= sidebar {
                    // Nothing spills into the workspace's right margin (or past the window).
                    if p.x + s.width > wf - pad + 0.5 {
                        problems.push(format!("{at}: \"{label}\" ends at x={} past the right margin", p.x + s.width));
                    }
                    if p.y >= pad + header {
                        bottom = bottom.max(p.y + s.height);
                    }
                } else if e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Tab)
                    && p.y + s.height > hf + 0.5
                {
                    problems.push(format!("{at}: sidebar entry \"{label}\" is cut at the bottom"));
                }
            }

            // A scrolled page longer than the window: the scroll bar has its own gutter, the panels stop before it
            // (a column 10 px left of the scroller's right edge, between the panels and the thumb, is bare).
            if SCROLLED_PAGES.contains(&page) && bottom > hf - pad {
                let x = (wf - pad - 10.0) as u32;
                let top = (pad + header + theme.get_gap()) as u32;
                let covered = (top..h - pad as u32).filter(|y| pixels[(y * w + x) as usize] != workspace).count();
                if covered > 0 {
                    problems.push(format!("{at}: {covered} px of content under the scroll bar (x={x})"));
                }
            }

            // Every panel holds its content: nothing labelled sticks out of it (below it only when the panel
            // scrolls its own list, where rows out of view are expected).
            for panel in hx.elements().match_type_name("Panel").find_all() {
                let (pp, ps) = (panel.absolute_position(), panel.size());
                if ps.width <= 0.0 || ps.height <= 0.0 {
                    continue;
                }
                if pp.x + ps.width > wf - pad + 0.5 {
                    problems.push(format!("{at}: a panel ends at x={} past the right margin", pp.x + ps.width));
                }
                let scrolls = panel.query_descendants().match_inherits("ScrollView").find_first().is_some();
                for e in panel.query_descendants().match_predicate(|e| e.accessible_label().is_some()).find_all() {
                    let (p, s) = (e.absolute_position(), e.size());
                    let label = e.accessible_label().unwrap_or_default();
                    if s.height <= 0.0 || label.is_empty() {
                        continue;
                    }
                    let outside_x = p.x < pp.x - 0.5 || p.x + s.width > pp.x + ps.width + 0.5;
                    let below = !scrolls && p.y + s.height > pp.y + ps.height + 0.5;
                    if outside_x || below {
                        problems
                            .push(format!("{at}: \"{label}\" ({p:?} {s:?}) sticks out of its panel ({pp:?} {ps:?})"));
                    }
                }
            }
        }
    }
    assert!(problems.is_empty(), "{} layout problems:\n{}", problems.len(), problems.join("\n"));
}

#[test]
fn the_operation_queue_panel_lists_operations_with_their_state() {
    use bitbybit_core::RiskLevel;
    use bitbybit_domain::operations::{OperationQueue, OperationState};
    let hx = Harness::new();
    let mut q = OperationQueue::default();
    let now = time::OffsetDateTime::UNIX_EPOCH;
    let id = q.push("drivers.install", crate::ops::operation_title("drivers.install"), None, RiskLevel::Low, now);
    q.update(id, now, |r| r.state = OperationState::Running);
    let rows = crate::ops::queue_rows(q.records(), hx.catalog);
    hx.ui.set_queue_rows(slint::ModelRc::from(Rc::new(slint::VecModel::from(rows))));
    hx.ui.set_queue_pending(1);
    hx.ui.invoke_show_queue();
    hx.render(8, 1100, 660);
    let name = hx.t("app.queue.op.driversInstall");
    let row = hx.labelled().into_iter().find(|(l, _)| l.starts_with(&format!("{name}, ")));
    let (label, e) = row.expect("the queue row is shown");
    assert!(label.contains(&hx.t("app.queue.state.running")), "{label}");
    let (p, s) = (e.absolute_position(), e.size());
    assert!(s.height > 0.0 && p.x + s.width <= 1100.0 && p.y + s.height <= 660.0, "{p:?} {s:?}");
    // The header button names the count for screen readers.
    let button = format!("{} (1)", hx.t("app.queue.title"));
    assert!(hx.labelled().iter().any(|(l, _)| *l == button), "no \"{button}\" button");
}

#[test]
fn the_palette_checked_for_contrast_is_the_one_the_ui_uses() {
    let hx = Harness::new();
    let theme = hx.ui.global::<Theme>();
    let hex = |c: slint::Color| (u32::from(c.red()) << 16) | (u32::from(c.green()) << 8) | u32::from(c.blue());
    let p = crate::theme::palette::ALL;
    let live = [
        theme.get_panel(),
        theme.get_workspace(),
        theme.get_sidebar(),
        theme.get_app_bg(),
        theme.get_text(),
        theme.get_text_secondary(),
        theme.get_text_muted(),
        theme.get_green(),
        theme.get_cyan(),
        theme.get_amber(),
        theme.get_red(),
    ];
    for ((name, value), colour) in p.iter().zip(live) {
        assert_eq!(*value, hex(colour), "Theme.{name} differs from the palette the contrast test checks");
    }
}
