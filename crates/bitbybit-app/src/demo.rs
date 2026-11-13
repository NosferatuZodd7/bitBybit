//! Debug builds only: `BITBYBIT_UI_DEMO=1` fills the pages with FAKE sample rows (and `=dialog` also opens a
//! high-risk confirmation) so the layout can be checked where no Windows broker exists. Never in release builds.

use crate::ui::{
    AppWindow, AudioRow, ChangeRow, DeviceRow, DisplayRow, DriverChoice, FindingRow, InterfaceRow, StoreItem,
};
use slint::{ModelRc, SharedString, VecModel};
use std::rc::Rc;

fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

fn strings(items: &[&str]) -> ModelRc<SharedString> {
    model(items.iter().map(|s| SharedString::from(*s)).collect())
}

pub fn fill(ui: &AppWindow, dialog: bool) {
    let device = |name: &str, class: &str, status: &str, problem: bool| DeviceRow {
        id: name.into(),
        name: name.into(),
        class: class.into(),
        status: status.into(),
        driver: "Realtek 6.0.9600.1 · 2025-03-02 · oem42.inf (demo)".into(),
        problem,
        can_restart: true,
        can_enable: problem,
        can_disable: !problem,
        can_uninstall: true,
        can_change_driver: true,
    };
    ui.set_devices(model(vec![
        device("Realtek High Definition Audio (demo)", "MEDIA", "A funcionar", false),
        device("Monitor DisplayPort (demo)", "Monitor", "Código 43: o Windows parou o dispositivo", true),
        device("Intel Wi-Fi 6E AX211 (demo)", "Net", "A funcionar", false),
    ]));
    ui.set_driver_device_id("gpu".into());
    ui.set_driver_device_name("Intel(R) HD Graphics (demo)".into());
    ui.set_driver_choices(model(vec![
        DriverChoice {
            inf: "oem14.inf".into(),
            title: "Intel(R) HD Graphics".into(),
            detail: "Intel Corporation · 10.18.10.4252 · oem14.inf".into(),
            current: true,
        },
        DriverChoice {
            inf: "display.inf".into(),
            title: "Placa Gráfica Básica da Microsoft".into(),
            detail: "Microsoft · 10.0.19041.3636 · display.inf".into(),
            current: false,
        },
    ]));
    ui.set_store_items(model(vec![
        StoreItem {
            inf: "oem0.inf".into(),
            title: "Microsoft · Printer (demo)".into(),
            detail: "oem0.inf · 10.0.19041.3636 · 2006-06-21".into(),
            used_by: String::new().into(),
        },
        StoreItem {
            inf: "oem2.inf".into(),
            title: "Linksys, LLC · Net (demo)".into(),
            detail: "oem2.inf · 6.32.145.11 · 2014-12-31".into(),
            used_by: "Em uso por: Linksys AE1200".into(),
        },
    ]));
    ui.set_devices_summary("3 de 212 dispositivos · dados de demonstração".into());
    ui.set_interfaces(model(vec![
        InterfaceRow {
            name: "Wi-Fi (demo)".into(),
            kind: "Wi-Fi · Intel AX211".into(),
            status: "Ligada".into(),
            addresses: "192.168.1.23/24 · fe80::1c2b:3d4e".into(),
            vpn: false,
            up: true,
        },
        InterfaceRow {
            name: "WireGuard (demo)".into(),
            kind: "Túnel".into(),
            status: "Ligada".into(),
            addresses: "10.8.0.2/32".into(),
            vpn: true,
            up: true,
        },
    ]));
    ui.set_audio(model(vec![AudioRow {
        id: "a".into(),
        name: "Altifalantes (Realtek) (demo)".into(),
        flow: "Reprodução".into(),
        state: "Ativo".into(),
        defaults: "Predefinido".into(),
        can_make_default: false,
        volume: 64,
        muted: false,
        can_prefer: true,
    }]));
    ui.set_displays(model(vec![DisplayRow {
        name: "DELL U2723QE (demo)".into(),
        connection: "DisplayPort · placa NVIDIA".into(),
        mode: "3840×2160 · 60 Hz".into(),
        edid: "EDID válido · 2022".into(),
        warning: String::new().into(),
    }]));
    ui.set_findings(model(vec![FindingRow {
        level: 2,
        severity: "Problema".into(),
        title: "O monitor DisplayPort desliga-se após o arranque (demo)".into(),
        detail: "O controlador de áudio Realtek foi reinstalado às 09:12 e o monitor perdeu o sinal logo a seguir."
            .into(),
        evidence: "Facto: evento 20001 no registo do sistema. Hipótese: atualização automática.".into(),
        actions: "Recomendado: repor o controlador anterior.".into(),
        buttons: strings(&["Repor o controlador anterior"]),
    }]));
    ui.set_changes(model(vec![ChangeRow {
        id: "c".into(),
        when: "Hoje 10:41".into(),
        kind: "Troca de controlador".into(),
        description: "Realtek High Definition Audio: 6.0.9600.1 → 6.0.9381.1 (demo)".into(),
        status: "Ativa".into(),
        scope: "Máquina".into(),
        can_revert: true,
    }]));
    if dialog {
        ui.set_dialog_summary("Desinstalar o dispositivo Monitor DisplayPort (demo)".into());
        ui.set_dialog_risk("Alto".into());
        ui.set_dialog_level(2);
        ui.set_dialog_steps(strings(&["Guardar o estado atual no diário", "Desinstalar o dispositivo", "Verificar"]));
        ui.set_dialog_effects(strings(&["O ecrã pode ficar sem imagem até voltar a procurar hardware."]));
        ui.set_dialog_warnings(strings(&["Esta ação não pode ser revertida automaticamente."]));
        ui.set_dialog_visible(true);
    }
}
