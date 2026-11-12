//! Device page actions: restart / enable / disable / uninstall, "scan for hardware changes", and the driver
//! chooser (compatible drivers on this PC, install one, roll back). All go through the broker with its plan.

use crate::ops;
use crate::ui::{AppWindow, DriverChoice};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::rc::Rc;

fn reload_after(ui: &AppWindow) {
    crate::reload(ui.as_weak());
}

#[derive(serde::Deserialize)]
struct Candidates {
    drivers: Vec<bitbybit_drivers::DriverCandidate>,
}

pub fn wire(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_device_action(move |id, action| {
        let params = serde_json::json!({ "instanceId": id.as_str(), "action": action.as_str() });
        ops::run(weak.clone(), "devices.action", params, |ui, _| reload_after(ui));
    });
    let weak = ui.as_weak();
    ui.on_rescan(move || ops::run(weak.clone(), "devices.rescan", serde_json::json!({}), |ui, _| reload_after(ui)));
    let weak = ui.as_weak();
    ui.on_device_drivers(move |id, name| {
        let params = serde_json::json!({ "instanceId": id.as_str() });
        let (id, name) = (id.to_string(), name.to_string());
        ops::run(weak.clone(), "drivers.compatible", params, move |ui, value| {
            ui.set_op_state(0); // a read: nothing was changed
            let list: Vec<DriverChoice> = serde_json::from_value::<Candidates>(value)
                .map(|c| c.drivers)
                .unwrap_or_default()
                .into_iter()
                .map(|c| DriverChoice {
                    inf: c.inf_name.clone().into(),
                    title: c.description.into(),
                    detail: [c.provider, c.version, Some(c.inf_name)]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ")
                        .into(),
                    current: c.is_current,
                })
                .collect();
            ui.set_driver_choices(ModelRc::from(Rc::new(VecModel::from(list))));
            ui.set_driver_device_name(name.into());
            ui.set_driver_device_id(id.into());
        });
    });
    let weak = ui.as_weak();
    ui.on_install_driver(move |id, inf| {
        let params = serde_json::json!({ "instanceId": id.as_str(), "infName": inf.as_str() });
        ops::run(weak.clone(), "drivers.install", params, |ui, _| {
            ui.set_driver_device_id("".into());
            reload_after(ui);
        });
    });
    let weak = ui.as_weak();
    ui.on_rollback_driver(move |id| {
        ops::run(weak.clone(), "drivers.rollback", serde_json::json!({ "instanceId": id.as_str() }), |ui, _| {
            ui.set_driver_device_id("".into());
            reload_after(ui);
        });
    });
    let weak = ui.as_weak();
    ui.on_close_drivers(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_driver_device_id("".into());
        }
    });
}
