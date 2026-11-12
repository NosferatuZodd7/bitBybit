//! Internet sharing on the network page: choose the Internet source and the shared interface, optional subnet,
//! Mobile Hotspot and (VPN sources only) leak protection; start and stop go through the broker with the plan.

use crate::ops;
use crate::ui::AppWindow;
use crate::view::ShareChoice;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static CHOICES: RefCell<Vec<ShareChoice>> = const { RefCell::new(Vec::new()) };
}

pub fn show(ui: &AppWindow, choices: Vec<ShareChoice>, session: Option<uuid::Uuid>) {
    let labels: Vec<SharedString> = choices.iter().map(|c| SharedString::from(c.label.as_str())).collect();
    ui.set_share_labels(ModelRc::from(Rc::new(VecModel::from(labels))));
    CHOICES.with(|c| *c.borrow_mut() = choices);
    ui.set_share_session(session.map(|s| s.to_string()).unwrap_or_default().into());
    upstream_changed(ui);
}

fn upstream_changed(ui: &AppWindow) {
    let vpn = CHOICES.with(|c| c.borrow().get(ui.get_share_upstream() as usize).is_some_and(|x| x.vpn));
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
        let luid = |i: i32| CHOICES.with(|c| c.borrow().get(i as usize).map(|x| x.luid));
        let (Some(up), Some(down)) = (luid(ui.get_share_upstream()), luid(ui.get_share_downstream())) else { return };
        let lan = ui.get_share_lan().trim().to_owned();
        let params = serde_json::json!({
            "upstreamLuid": up,
            "downstreamLuid": down,
            "lan": if lan.is_empty() { None } else { Some(lan) },
            "useHotspot": ui.get_share_hotspot(),
            "leakProtection": ui.get_share_leak(),
        });
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
