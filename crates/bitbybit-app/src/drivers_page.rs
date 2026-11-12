//! "Controladores" page: Windows Update driver updates, cleanup (unused packages, absent devices), the Driver
//! Store and bitBybit's backups. Reads happen here (unelevated where Windows allows it); every change goes
//! through the broker with its plan and the user's explicit selection (nothing is pre-selected).

use crate::ui::{AppWindow, CleanupItem, StoreItem, UpdateItem};
use crate::{ops, view};
use bitbybit_core::text;
use bitbybit_localization::TextCatalog;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

thread_local! {
    static CLEANUP: RefCell<BTreeSet<(bool, String)>> = const { RefCell::new(BTreeSet::new()) };
    static UPDATES: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}

/// Store packages, backups and cleanup candidates (off the UI thread).
pub fn refresh(weak: slint::Weak<AppWindow>) {
    std::thread::spawn(move || {
        let catalog = TextCatalog::default_catalog();
        let devices = crate::broker::devices().unwrap_or_default();
        let packages = crate::platform_reads::store_packages();
        let backups = crate::platform_reads::backups();
        let mut snapshot =
            bitbybit_domain::snapshot::Snapshot::empty(bitbybit_core::version(), time::OffsetDateTime::now_utc());
        snapshot.devices = devices.clone();
        snapshot.driver_store_packages = packages.clone();
        let cleanup = view::cleanup_rows(&bitbybit_devices::cleanup::analyze(&snapshot), catalog);
        let store = view::store_rows(&packages, &devices, catalog);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            CLEANUP.with(|s| s.borrow_mut().clear());
            ui.set_cleanup_items(model(
                cleanup
                    .into_iter()
                    .map(|c| CleanupItem {
                        id: c.id.into(),
                        device: c.device,
                        title: c.title.into(),
                        detail: c.detail.into(),
                        impact: c.impact.into(),
                        risk: c.risk.into(),
                        recommended: c.recommended,
                    })
                    .collect(),
            ));
            ui.set_store_items(model(
                store
                    .into_iter()
                    .map(|r| StoreItem {
                        inf: r.inf.into(),
                        title: r.title.into(),
                        detail: r.detail.into(),
                        used_by: r.used_by.into(),
                    })
                    .collect(),
            ));
            let summary = if backups.is_empty() {
                catalog.resolve(&text!("app.page.backups.empty", "No backups yet."))
            } else {
                backups.join("\n")
            };
            ui.set_backups_text(summary.into());
        });
    });
}

#[derive(serde::Deserialize)]
struct Updates {
    updates: Vec<bitbybit_drivers::DriverUpdateCandidate>,
}

pub fn wire(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_refresh_drivers(move || refresh(weak.clone()));
    ui.on_toggle_cleanup(|id, device, on| {
        CLEANUP.with(|s| {
            let key = (device, id.to_string());
            if on { s.borrow_mut().insert(key) } else { s.borrow_mut().remove(&key) }
        });
    });
    ui.on_toggle_update(|id, on| {
        UPDATES.with(|s| if on { s.borrow_mut().insert(id.to_string()) } else { s.borrow_mut().remove(id.as_str()) });
    });
    let weak = ui.as_weak();
    ui.on_apply_cleanup(move || {
        let selected = CLEANUP.with(|s| s.borrow().clone());
        if selected.is_empty() {
            return;
        }
        let packages: Vec<&String> = selected.iter().filter(|(d, _)| !d).map(|(_, id)| id).collect();
        let devices: Vec<&String> = selected.iter().filter(|(d, _)| *d).map(|(_, id)| id).collect();
        let params = serde_json::json!({ "packages": packages, "absentDevices": devices });
        ops::run(weak.clone(), "cleanup.apply", params, |ui, _| refresh(ui.as_weak()));
    });
    let weak = ui.as_weak();
    ui.on_remove_package(move |inf| {
        ops::run(weak.clone(), "drivers.deletePackage", serde_json::json!({ "infName": inf.as_str() }), |ui, _| {
            refresh(ui.as_weak())
        });
    });
    let weak = ui.as_weak();
    ui.on_search_updates(move || {
        ops::run(weak.clone(), "drivers.windowsUpdateSearch", serde_json::json!({}), |ui, value| {
            ui.set_op_state(0);
            UPDATES.with(|s| s.borrow_mut().clear());
            let list = serde_json::from_value::<Updates>(value).map(|u| u.updates).unwrap_or_default();
            ui.set_updates_searched(true);
            ui.set_update_items(model(
                list.into_iter()
                    .filter_map(|u| {
                        Some(UpdateItem {
                            id: u.update_id?.into(),
                            title: u.title.into(),
                            detail: [u.driver_class, u.provider, u.hardware_id]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" · ")
                                .into(),
                        })
                    })
                    .collect(),
            ));
        });
    });
    let weak = ui.as_weak();
    ui.on_install_updates(move || {
        let ids: Vec<String> = UPDATES.with(|s| s.borrow().iter().cloned().collect());
        if ids.is_empty() {
            return;
        }
        ops::run(weak.clone(), "drivers.windowsUpdateInstall", serde_json::json!({ "updateIds": ids }), |ui, _| {
            ui.set_update_items(model(vec![]));
            refresh(ui.as_weak());
        });
    });
}
