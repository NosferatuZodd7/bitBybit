//! Debug builds only: `BITBYBIT_UI_DEMO=1` fills the pages with FAKE sample rows (and `=dialog` also opens a
//! high-risk confirmation; `=adapter` shows the sharing panel in adapter mode) so the layout can be checked where no Windows broker exists. Never in release builds.

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

pub fn fill(ui: &AppWindow, mode: &str) {
    let dialog = mode == "dialog";
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
        can_force: true,
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
    let choice = |luid: u64, label: &str, vpn: bool| crate::view::ShareChoice { luid, label: label.into(), vpn };
    crate::sharing_page::show(
        ui,
        crate::view::ShareChoices {
            sources: vec![choice(1, "Ethernet (demo)", false), choice(3, "WireGuard (demo)", true)],
            targets: vec![choice(1, "Ethernet (demo)", false), choice(2, "Ethernet 2 (não ligada) (demo)", false)],
            wifi: true,
        },
        if mode == "share" { uuid::Uuid::from_u128(1).into() } else { None },
        None,
    );
    if mode == "share" {
        let node = |id: &str, kind, title: &str, detail: &str, parent, col, row| crate::view::TopoNode {
            id: id.into(),
            kind,
            title: title.into(),
            detail: detail.into(),
            parent,
            col,
            row,
        };
        crate::sharing_page::apply_topology(
            ui,
            &[
                node("e", 0, "Ethernet (demo)", "Ethernet (física)", -1, 0, 0),
                node("v", 1, "WireGuard (demo)", "VPN · proteção contra fugas", 0, 1, 0),
                node("s", 2, "Casa-bitBybit", "Hotspot Wi-Fi · ponto de acesso bitBybit · 192.168.50.0/24", 1, 2, 0),
                node("a", 3, "telemovel-ana", "192.168.50.10 · 3A:1F:…", 2, 3, 0),
                node("b", 3, "portatil-escola", "192.168.50.11 · 9C:B6:…", 2, 3, 1),
                node("c", 3, "Dispositivo", "192.168.50.12 · F0:18:…", 2, 3, 2),
            ],
        );
    }
    if mode == "update" {
        ui.set_update_version("0.7.0-dev.36 (demo)".into());
        ui.set_update_notes("Hotspot Wi-Fi com nome e palavra-passe editáveis.\nBotões já não esticam.".into());
        ui.set_app_update_text("Há uma versão nova: 0.7.0-dev.36 (demo).".into());
        ui.set_app_update_available(true);
        ui.set_app_update_state(2);
        ui.set_update_dialog_visible(true);
    }
    if mode == "force" {
        let f = crate::view::ForceOutput {
            target_id: "hdmi".into(),
            target_name: "LG TV (Intel Display Audio) (demo)".into(),
            others: vec![("spk".into(), "Altifalantes (Realtek) (demo)".into())],
            unplugged: false,
        };
        let v = crate::view::plan_view(&crate::view::force_plan(&f), bitbybit_localization::TextCatalog::default_catalog());
        ui.set_dialog_summary(v.summary.into());
        ui.set_dialog_steps(strings(&v.steps.iter().map(String::as_str).collect::<Vec<_>>()));
        ui.set_dialog_warnings(strings(&v.warnings.iter().map(String::as_str).collect::<Vec<_>>()));
        ui.set_dialog_effects(strings(&v.effects.iter().map(String::as_str).collect::<Vec<_>>()));
        ui.set_dialog_risk(v.risk.into());
        ui.set_dialog_level(v.level);
        ui.set_dialog_visible(true);
    }
    if mode == "newcard" {
        ui.set_new_card_text("Nova placa de rede detetada: «Wi-Fi 2 (Linksys AE1200)» (demo).".into());
        ui.set_share_suggestion(
            "A Internet entra por «Ethernet (demo)». Pode partilhá-la por Wi-Fi com a placa «Wi-Fi 2 (Linksys AE1200)» (hotspot)."
                .into(),
        );
    }
    if mode == "adapter" {
        ui.set_share_mode(1);
    }
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
