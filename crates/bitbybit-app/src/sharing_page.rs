//! Internet sharing on the network page. Hotspot mode: the Windows Mobile Hotspot shares the chosen source
//! (cable, Wi-Fi or VPN) under an editable name and password. Adapter mode: share on another network card,
//! connected or not, with optional subnet and (VPN sources only) leak protection. Start and stop go through the
//! broker with the plan.

use crate::ops;
use crate::ui::AppWindow;
use crate::view::{ShareChoice, ShareChoices};
use bitbybit_core::text;
use bitbybit_localization::TextCatalog;
use bitbybit_network::session::{HotspotSettings, Passphrase};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static CHOICES: RefCell<ShareChoices> = RefCell::new(ShareChoices::default());
    static OVERVIEW: RefCell<Option<bitbybit_network::overview::NetworkOverview>> = const { RefCell::new(None) };
    /// Where the user dragged each box (by node id), kept across refreshes.
    static OFFSETS: RefCell<std::collections::HashMap<String, (f32, f32)>> = RefCell::new(Default::default());
    static NODE_IDS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TIMER: slint::Timer = slint::Timer::default();
    static WATCH: slint::Timer = slint::Timer::default();
    /// Physical cards seen so far (`None` until the first read, so start-up raises no alert).
    static CARDS: RefCell<Option<std::collections::HashSet<u64>>> = const { RefCell::new(None) };
    static SUGGESTION: RefCell<Option<crate::view::ShareSuggestion>> = const { RefCell::new(None) };
    /// The form was filled from a suggestion once (later reloads keep the user's choices).
    static SUGGESTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// "Internet a partilhar": 0 Automático (where the Internet comes in now), 1 no Internet sharing, then the sources.
const AUTO: i32 = 0;
const NO_INTERNET: i32 = 1;
const FIRST_SOURCE: i32 = 2;

/// The chosen source: `None` for "no Internet sharing", else (luid, is VPN).
fn source_of(ui: &AppWindow) -> Option<(u64, bool)> {
    let i = ui.get_share_upstream();
    CHOICES.with(|c| {
        let c = c.borrow();
        let pick = |luid: u64| c.sources.iter().find(|s| s.luid == luid).map(|s| (s.luid, s.vpn));
        match i {
            NO_INTERNET => None,
            AUTO => OVERVIEW
                .with(|o| o.borrow().as_ref().and_then(|o| o.ipv4_default.as_ref().map(|d| d.interface_luid)))
                .and_then(pick),
            _ => c.sources.get((i - FIRST_SOURCE) as usize).map(|s| (s.luid, s.vpn)),
        }
    })
}

/// Fills the form from the suggestion (never starts anything).
fn apply_suggestion(ui: &AppWindow) {
    let Some(sug) = SUGGESTION.with(|s| s.borrow().clone()) else { return };
    // The suggestion's source is where the Internet comes in now, i.e. "Automático".
    let _ = sug.source;
    ui.set_share_upstream(AUTO);
    ui.set_share_mode(if sug.hotspot { 0 } else { 1 });
    if let Some(t) = sug.target {
        ui.set_share_downstream(t as i32);
    }
    upstream_changed(ui);
}

/// Notices network cards being plugged in or removed (every 8 s) and reloads; a new card raises the banner.
fn watch_cards(weak: slint::Weak<AppWindow>) {
    std::thread::spawn(move || {
        let Ok(o) = crate::broker::network() else { return };
        let cards = crate::view::physical_cards(&o);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let now: std::collections::HashSet<u64> = cards.iter().map(|c| c.0).collect();
            let before = CARDS.with(|k| k.borrow_mut().replace(now.clone()));
            let Some(before) = before else { return };
            if before == now {
                return;
            }
            if let Some((_, alias)) = cards.iter().find(|c| !before.contains(&c.0)) {
                let text = TextCatalog::default_catalog().resolve(&text!(
                    "app.share.newCard",
                    "New network card found: '{0}'.",
                    alias.clone()
                ));
                ui.set_new_card_text(text.into());
                SUGGESTED.with(|s| s.set(false));
            }
            crate::reload(ui.as_weak());
        });
    });
}

/// Diagram grid (logical pixels).
const COL_W: f32 = 212.0;
const NODE_W: f32 = 180.0;
const ROW_H: f32 = 84.0;
const NODE_H: f32 = 64.0;

/// Shows the diagram rows: grid position plus the user's drag offset.
pub fn apply_topology(ui: &AppWindow, nodes: &[crate::view::TopoNode]) {
    let rows: Vec<crate::ui::TopoNode> = OFFSETS.with(|o| {
        let o = o.borrow();
        nodes
            .iter()
            .map(|n| {
                let (dx, dy) = o.get(&n.id).copied().unwrap_or_default();
                crate::ui::TopoNode {
                    id: n.id.clone().into(),
                    kind: n.kind,
                    title: n.title.clone().into(),
                    detail: n.detail.clone().into(),
                    parent: n.parent,
                    x: (16.0 + n.col as f32 * COL_W + dx).max(0.0),
                    y: (16.0 + n.row as f32 * ROW_H + dy).max(0.0),
                }
            })
            .collect()
    });
    let height = rows.iter().map(|r| r.y).fold(0.0, f32::max) + NODE_H + 32.0;
    let width = rows.iter().map(|r| r.x).fold(0.0, f32::max) + NODE_W + 32.0;
    ui.set_topo_height(height);
    ui.set_topo_width(width);
    ui.set_share_clients(nodes.iter().filter(|n| n.kind == 3).count() as i32);
    // The header names what is shared: the VPN if any, else the carrier.
    let shared = nodes.iter().rfind(|n| n.kind <= 1).map(|n| n.title.clone()).unwrap_or_default();
    ui.set_share_sharing(shared.into());
    NODE_IDS.with(|ids| *ids.borrow_mut() = nodes.iter().map(|n| n.id.clone()).collect());
    ui.set_topo_nodes(ModelRc::from(Rc::new(VecModel::from(rows))));
}

/// Reads the active share and its devices off the UI thread and redraws the diagram.
fn refresh_status(weak: slint::Weak<AppWindow>) {
    std::thread::spawn(move || {
        let status = crate::broker::share_status();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Ok(status) = status else { return };
            let nodes = OVERVIEW.with(|o| {
                o.borrow().as_ref().map(|o| crate::view::topology(o, &status, TextCatalog::default_catalog()))
            });
            // No network read yet: nothing to draw the chain from.
            if let Some(nodes) = nodes {
                apply_topology(&ui, &nodes);
            }
        });
    });
}

fn labels(list: &[ShareChoice]) -> ModelRc<SharedString> {
    ModelRc::from(Rc::new(VecModel::from(
        list.iter().map(|c| SharedString::from(c.label.as_str())).collect::<Vec<_>>(),
    )))
}

pub fn show(
    ui: &AppWindow,
    choices: ShareChoices,
    session: Option<uuid::Uuid>,
    overview: Option<bitbybit_network::overview::NetworkOverview>,
) {
    if overview.is_some() {
        OVERVIEW.with(|o| *o.borrow_mut() = overview);
    }
    // While a share is active, the diagram follows the devices joining and leaving (every 5 s).
    let weak = ui.as_weak();
    TIMER.with(|t| {
        if session.is_some() {
            refresh_status(weak.clone());
            t.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(5), move || {
                refresh_status(weak.clone())
            });
        } else {
            t.stop();
            apply_topology(ui, &[]);
        }
    });
    let suggestion = OVERVIEW.with(|o| {
        o.borrow().as_ref().and_then(|o| crate::view::share_suggestion(o, &choices, TextCatalog::default_catalog()))
    });
    ui.set_share_suggestion(suggestion.as_ref().map(|s| s.text.clone()).unwrap_or_default().into());
    SUGGESTION.with(|s| *s.borrow_mut() = suggestion);
    let catalog = TextCatalog::default_catalog();
    let mut source_labels = vec![
        SharedString::from(catalog.resolve(&text!("app.share.auto", "Automatic (where the Internet comes in now)"))),
        SharedString::from(catalog.resolve(&text!("app.share.noInternet", "No Internet sharing (local network only)"))),
    ];
    source_labels.extend(choices.sources.iter().map(|c| SharedString::from(c.label.as_str())));
    ui.set_share_labels(ModelRc::from(Rc::new(VecModel::from(source_labels))));
    ui.set_share_target_labels(labels(&choices.targets));
    ui.set_share_has_wifi(choices.wifi);
    if !choices.wifi {
        ui.set_share_mode(1);
    }
    CHOICES.with(|c| *c.borrow_mut() = choices);
    ui.set_share_session(session.map(|s| s.to_string()).unwrap_or_default().into());
    if !SUGGESTED.with(|s| s.replace(true)) {
        apply_suggestion(ui);
    }
    upstream_changed(ui);
}

fn upstream_changed(ui: &AppWindow) {
    let source = source_of(ui);
    let vpn = source.is_some_and(|s| s.1);
    let luid = source.map(|s| s.0);
    // Offer a different card than the source by default.
    let target = CHOICES.with(|c| c.borrow().targets.iter().position(|t| Some(t.luid) != luid));
    if CHOICES.with(|c| c.borrow().targets.get(ui.get_share_downstream() as usize).map(|t| t.luid)) == luid
        || ui.get_share_downstream() < 0
    {
        ui.set_share_downstream(target.unwrap_or(0) as i32);
    }
    // "Sem partilha de Internet" unticks the Internet access; any source ticks it again.
    ui.set_share_allow_internet(ui.get_share_upstream() != NO_INTERNET);
    ui.set_share_upstream_vpn(vpn);
    if !vpn {
        ui.set_share_leak(false);
    }
}

/// Wi-Fi repeater: the source is the Wi-Fi the PC is connected to.
fn mode_changed(ui: &AppWindow) {
    if ui.get_share_mode() == 2
        && let Some(i) = CHOICES.with(|c| c.borrow().sources.iter().position(|s| s.wifi))
    {
        ui.set_share_upstream(FIRST_SOURCE + i as i32);
    }
    upstream_changed(ui);
}

pub fn wire(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_share_mode_changed(move || {
        if let Some(ui) = weak.upgrade() {
            mode_changed(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_use_suggestion(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_new_card_text("".into());
            ui.set_page(0);
            apply_suggestion(&ui);
        }
    });
    let weak = ui.as_weak();
    WATCH.with(|t| {
        t.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(8), move || watch_cards(weak.clone()))
    });
    let weak = ui.as_weak();
    ui.on_topo_move(move |i, dx, dy| {
        let Some(ui) = weak.upgrade() else { return };
        let model = ui.get_topo_nodes();
        let Some(mut row) = model.row_data(i as usize) else { return };
        row.x = (row.x + dx).max(0.0);
        row.y = (row.y + dy).max(0.0);
        // The canvas grows with the boxes.
        ui.set_topo_height(ui.get_topo_height().max(row.y + NODE_H + 32.0));
        ui.set_topo_width(ui.get_topo_width().max(row.x + NODE_W + 32.0));
        model.set_row_data(i as usize, row);
        if let Some(id) = NODE_IDS.with(|ids| ids.borrow().get(i as usize).cloned()) {
            OFFSETS.with(|o| {
                let mut o = o.borrow_mut();
                let e = o.entry(id).or_default();
                e.0 += dx;
                e.1 += dy;
            });
        }
    });
    let weak = ui.as_weak();
    ui.on_share_upstream_changed(move || {
        if let Some(ui) = weak.upgrade() {
            upstream_changed(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_start_sharing(move || {
        let Some(ui) = weak.upgrade() else { return };
        let source = source_of(&ui);
        let allow_internet = ui.get_share_allow_internet() && source.is_some();
        if ui.get_share_upstream() == AUTO && source.is_none() {
            let message = TextCatalog::default_catalog()
                .resolve(&text!("app.share.noSource", "This PC has no Internet connection to share right now."));
            ui.set_op_state(3);
            ui.set_op_message(message.into());
            return;
        }
        let up = source.map_or(0, |s| s.0);
        let down = CHOICES.with(|c| c.borrow().targets.get(ui.get_share_downstream() as usize).map(|t| t.luid));
        let params = if ui.get_share_mode() != 1 {
            let hotspot = HotspotSettings {
                ssid: ui.get_share_name().trim().to_owned(),
                passphrase: Passphrase(ui.get_share_password().to_string()),
            };
            if hotspot.validate().is_err() {
                let message = TextCatalog::default_catalog().resolve(&text!(
                    "app.share.badHotspot",
                    "The network name needs 1 to 32 characters and the password 8 to 63 (letters, digits, symbols)."
                ));
                ui.set_op_state(3);
                ui.set_op_message(message.into());
                return;
            }
            serde_json::json!({
                "upstreamLuid": up,
                "hotspotName": hotspot.ssid,
                "hotspotPassphrase": hotspot.passphrase.0,
                "leakProtection": ui.get_share_leak() && allow_internet,
                "allowInternet": allow_internet,
            })
        } else {
            let Some(down) = down else { return };
            let lan = ui.get_share_lan().trim().to_owned();
            serde_json::json!({
                "upstreamLuid": up,
                "downstreamLuid": down,
                "lan": if lan.is_empty() { None } else { Some(lan) },
                "leakProtection": ui.get_share_leak() && allow_internet,
                "allowInternet": allow_internet,
            })
        };
        ops::run(weak.clone(), "sharing.start", params, |ui, _| crate::reload(ui.as_weak()));
    });
    let weak = ui.as_weak();
    ui.on_stop_sharing(move || {
        let Some(ui) = weak.upgrade() else { return };
        let session = ui.get_share_session().to_string();
        ops::run(weak.clone(), "sharing.stop", serde_json::json!({ "sessionId": session }), |ui, _| {
            crate::reload(ui.as_weak())
        });
    });
}
