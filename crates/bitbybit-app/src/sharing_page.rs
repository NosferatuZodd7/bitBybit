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
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static CHOICES: RefCell<ShareChoices> = RefCell::new(ShareChoices::default());
}

fn labels(list: &[ShareChoice]) -> ModelRc<SharedString> {
    ModelRc::from(Rc::new(VecModel::from(
        list.iter().map(|c| SharedString::from(c.label.as_str())).collect::<Vec<_>>(),
    )))
}

pub fn show(ui: &AppWindow, choices: ShareChoices, session: Option<uuid::Uuid>) {
    ui.set_share_labels(labels(&choices.sources));
    ui.set_share_target_labels(labels(&choices.targets));
    ui.set_share_has_wifi(choices.wifi);
    if !choices.wifi {
        ui.set_share_mode(1);
    }
    CHOICES.with(|c| *c.borrow_mut() = choices);
    ui.set_share_session(session.map(|s| s.to_string()).unwrap_or_default().into());
    upstream_changed(ui);
}

fn upstream_changed(ui: &AppWindow) {
    let (vpn, source) = CHOICES.with(|c| {
        let c = c.borrow();
        let up = c.sources.get(ui.get_share_upstream() as usize);
        (up.is_some_and(|x| x.vpn), up.map(|x| x.luid))
    });
    // Offer a different card than the source by default.
    let target = CHOICES.with(|c| c.borrow().targets.iter().position(|t| Some(t.luid) != source));
    if CHOICES.with(|c| c.borrow().targets.get(ui.get_share_downstream() as usize).map(|t| t.luid)) == source
        || ui.get_share_downstream() < 0
    {
        ui.set_share_downstream(target.unwrap_or(0) as i32);
    }
    ui.set_share_upstream_vpn(vpn);
    if !vpn {
        ui.set_share_leak(false);
    }
}

pub fn wire(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_share_upstream_changed(move || {
        if let Some(ui) = weak.upgrade() {
            upstream_changed(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_start_sharing(move || {
        let Some(ui) = weak.upgrade() else { return };
        let (up, down) = CHOICES.with(|c| {
            let c = c.borrow();
            let get = |list: &[ShareChoice], i: i32| list.get(i as usize).map(|x| x.luid);
            (get(&c.sources, ui.get_share_upstream()), get(&c.targets, ui.get_share_downstream()))
        });
        let Some(up) = up else { return };
        let params = if ui.get_share_mode() == 0 {
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
                "leakProtection": ui.get_share_leak(),
            })
        } else {
            let Some(down) = down else { return };
            let lan = ui.get_share_lan().trim().to_owned();
            serde_json::json!({
                "upstreamLuid": up,
                "downstreamLuid": down,
                "lan": if lan.is_empty() { None } else { Some(lan) },
                "leakProtection": ui.get_share_leak(),
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
