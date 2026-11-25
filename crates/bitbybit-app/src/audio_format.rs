//! Áudio page: an endpoint's default format (AUD-07). The formats offered are those the device accepts in
//! exclusive mode (read-only test, as the Sound panel builds its list). A change is confirmed (current vs requested,
//! labelled as using the undocumented IPolicyConfig), journaled in the user journal, applied, read back, and
//! revertible in "Alterações do bitBybit" with drift detection. Windows refusing it for this user offers the UAC
//! restart.

use crate::ui::AppWindow;
use bitbybit_audio::format::{AudioFormat, AudioFormats, choices};
use bitbybit_audio::profile::{AudioEndpoint, EndpointStatus};
use bitbybit_core::{ErrorCode, LocalizedText, RiskLevel, text};
use bitbybit_localization::TextCatalog;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

/// One active endpoint: id, name, the format in use and what can be picked (the current one included).
pub struct Entry {
    id: String,
    name: String,
    current: AudioFormat,
    choices: Vec<AudioFormat>,
}

thread_local! {
    static ENTRIES: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

pub fn port() -> Option<&'static dyn AudioFormats> {
    #[cfg(windows)]
    return Some(&bitbybit_platform_windows::audio::CoreAudio);
    #[cfg(not(windows))]
    None
}

/// Read-only, off the UI thread: active endpoints whose format Windows reports.
pub fn read(endpoints: Option<&[AudioEndpoint]>) -> Vec<Entry> {
    let (Some(port), Some(endpoints)) = (port(), endpoints) else { return Vec::new() };
    endpoints
        .iter()
        .filter(|e| e.state == EndpointStatus::Active)
        .filter_map(|e| {
            let current = port.device_format(&e.id).map_err(|x| log::info!("No format for {}: {x}", e.id)).ok()?;
            let supported = port.supported(&e.id, &current).unwrap_or_default();
            Some(Entry { id: e.id.clone(), name: e.display_name(), current, choices: choices(&current, &supported) })
        })
        .collect()
}

pub fn store(entries: Vec<Entry>) {
    ENTRIES.with(|e| *e.borrow_mut() = entries);
}

pub fn label(f: &AudioFormat, catalog: &TextCatalog) -> String {
    catalog.resolve(&text!(
        "app.audio.format.label",
        "{0} bit, {1} Hz",
        f.valid_bits.to_string(),
        f.sample_rate.to_string()
    ))
}

/// (labels, index of the format in use) for one endpoint tile; empty when nothing can be changed.
pub fn picker(id: &str, catalog: &TextCatalog) -> (ModelRc<SharedString>, i32) {
    ENTRIES.with(|e| {
        let e = e.borrow();
        let Some(entry) = e.iter().find(|x| x.id == id) else {
            return (ModelRc::from(Rc::new(VecModel::from(Vec::<SharedString>::new()))), -1);
        };
        let labels: Vec<SharedString> = entry.choices.iter().map(|f| label(f, catalog).into()).collect();
        let index = entry.choices.iter().position(|f| *f == entry.current).map_or(-1, |i| i as i32);
        (ModelRc::from(Rc::new(VecModel::from(labels))), index)
    })
}

fn plan(name: &str, from: &AudioFormat, to: &AudioFormat, catalog: &TextCatalog) -> bitbybit_ipc::plan::OperationPlan {
    let mut plan = bitbybit_ipc::plan::OperationPlan::new(
        text!(
            "app.audio.format.summary",
            "Change the format of '{0}' from {1} to {2}",
            name.to_owned(),
            label(from, catalog),
            label(to, catalog)
        ),
        vec![
            text!(
                "app.audio.format.stepCheck",
                "Check that the device still accepts {0} (a read-only test)",
                label(to, catalog)
            ),
            text!(
                "app.audio.format.stepSet",
                "Record the current format in the change journal, set the new one and read it back"
            ),
        ],
        RiskLevel::Medium,
    );
    plan.may_interrupt_audio = true;
    plan.reversible = true;
    plan.warnings.push(text!(
        "app.audio.format.undocumented",
        "This uses an undocumented Windows interface (IPolicyConfig), the one the Sound panel uses. It works on current Windows 10/11 but may stop working after a Windows update."
    ));
    plan
}

pub fn wire(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_set_audio_format(move |id, index| {
        let picked = ENTRIES.with(|e| {
            let e = e.borrow();
            let entry = e.iter().find(|x| x.id == id.as_str())?;
            let to = *entry.choices.get(usize::try_from(index).ok()?)?;
            (to != entry.current).then(|| (entry.id.clone(), entry.name.clone(), entry.current, to))
        });
        let Some((id, name, from, to)) = picked else { return };
        let weak = weak.clone();
        std::thread::spawn(move || {
            let catalog = TextCatalog::default_catalog();
            let Some(port) = port() else { return };
            let plan = plan(&name, &from, &to, catalog);
            use bitbybit_domain::operations::OperationState;
            let queued = crate::ops::queue_push(&weak, "audio.deviceFormat", plan.summary.clone(), plan.risk);
            let queue = |state, error: Option<String>| crate::ops::queue_set(&weak, queued, state, error, None);
            queue(OperationState::AwaitingConfirmation, None);
            if !crate::ops::ask(&weak, &plan) {
                return queue(OperationState::Cancelled, None);
            }
            queue(OperationState::Running, None);
            let journal = bitbybit_journal::JsonLinesJournal::new(
                bitbybit_core::paths::AppPaths::from_environment().user_journal_file(),
            );
            let now = time::OffsetDateTime::now_utc();
            let result = bitbybit_audio::format::set_format(port, &journal, &id, &name, &to, now);
            match &result {
                Ok(true) => queue(OperationState::Succeeded, None),
                Ok(false) => queue(OperationState::Failed, Some("Windows did not report the new format".into())),
                Err(e) => queue(OperationState::Failed, Some(e.message.clone())),
            }
            let failed = |why: String| text!("app.audio.changeFailed", "The audio device could not be changed: {0}", why);
            let (message, elevate): (LocalizedText, bool) = match result {
                Ok(true) => (text!("app.audio.format.done", "Format changed, verified and recorded in the change journal."), false),
                Ok(false) => (failed("Windows did not report the new format".into()), false),
                Err(e) if e.code == ErrorCode::AccessDenied => (
                    text!(
                        "app.audio.format.elevation",
                        "Windows refused to change the format for this user. Restart bitBybit as administrator and try again."
                    ),
                    true,
                ),
                Err(e) => (failed(e.message), false),
            };
            let w = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = w.upgrade() else { return };
                let message = catalog.resolve(&message);
                if elevate {
                    ui.set_op_elevate(true);
                    crate::ops::status(&ui, 3, &message);
                }
                ui.set_audio_message(message.into());
            });
            crate::reload(weak);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_shows_both_formats_and_the_undocumented_warning() {
        let catalog = TextCatalog::default_catalog();
        let from = AudioFormat {
            channels: 2,
            sample_rate: 48_000,
            container_bits: 16,
            valid_bits: 16,
            float: false,
            channel_mask: 3,
        };
        let to = AudioFormat { sample_rate: 96_000, container_bits: 32, valid_bits: 24, ..from };
        let p = plan("Altifalantes", &from, &to, catalog);
        let summary = catalog.resolve(&p.summary);
        assert!(summary.contains("16 bits, 48000 Hz") && summary.contains("24 bits, 96000 Hz"), "{summary}");
        assert!(catalog.resolve(&p.warnings[0]).contains("IPolicyConfig"));
        for key in [
            "app.audio.format.stepCheck",
            "app.audio.format.stepSet",
            "app.audio.format.done",
            "app.audio.format.elevation",
        ] {
            assert!(catalog.template(key).is_some(), "{key}");
        }
    }
}
