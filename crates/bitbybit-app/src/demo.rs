//! Debug builds only: `BITBYBIT_UI_DEMO=1` fills the pages with FAKE sample rows (and `=dialog` also opens a
//! high-risk confirmation; `=adapter` shows the sharing panel in adapter mode, `=drivers` opens the driver chooser, `=toast` shows a failed operation, `=netdiag` a finished ping, `=displaykeep` the "keep these display settings?" countdown) so the layout can be checked where no Windows broker exists. Never in release builds.

use crate::ui::{
    AppWindow, AudioRow, ChangeRow, ChartData, DeviceRow, DisplayRow, DriverChoice, FindingRow, InterfaceRow, Kpi,
    StoreItem,
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
    // The chooser is a modal dialog: only in its own demo mode.
    if mode == "drivers" {
        ui.set_driver_device_id("gpu".into());
    }
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
        formats: strings(&["16 bits, 44100 Hz", "16 bits, 48000 Hz", "24 bits, 48000 Hz", "24 bits, 96000 Hz"]),
        format_index: 1,
    }]));
    ui.set_display_topologies(strings(&["Duplicar", "Expandir", "Apenas o ecrã do PC", "Apenas o segundo ecrã"]));
    ui.set_display_topology(1);
    ui.set_display_can_topology(true);
    if mode == "displaykeep" {
        ui.set_display_keep_detail("Alterar 'DELL U2723QE (demo)' de 3840 × 2160 · 60 Hz para 2560 × 1440 · 60 Hz".into());
        ui.set_display_keep_seconds(12);
        ui.set_display_keep_visible(true);
    }
    ui.set_displays(model(vec![DisplayRow {
        id: r"\\.\DISPLAY1".into(),
        modes: strings(&["3840 × 2160 · 60 Hz", "3840 × 2160 · 30 Hz", "2560 × 1440 · 60 Hz", "1920 × 1080 · 60 Hz"]),
        mode_index: 0,
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
    ui.set_broker_state(1);
    let kpi = |title: &str, value: &str, note: &str, page: i32| Kpi {
        title: title.into(),
        value: value.into(),
        note: note.into(),
        page,
    };
    ui.set_kpis(model(vec![
        kpi("Interfaces de rede ativas", "3", "de 5, 1 através de VPN (demo)", 0),
        kpi("Dispositivos ligados", "212", "1 com problema (demo)", 1),
        kpi("Alterações ativas do bitBybit", "4", "Todas podem ser revertidas (demo)", 3),
    ]));
    let series = |name: &str, labels: &[&str], values: &[u32], summary: &str| {
        let max = crate::view::chart_max(values);
        let (line, area) = crate::view::chart_paths(values, max, 1000.0, 300.0);
        ChartData {
            name: name.into(),
            line: line.into(),
            area: area.into(),
            x_labels: strings(labels),
            y_labels: model((0..5).rev().map(|q| SharedString::from((max * q / 4).to_string())).collect()),
            summary: summary.into(),
            empty: false,
        }
    };
    ui.set_charts(model(vec![
        series(
            "Últimos 12 meses",
            &["dez", "jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov"],
            &[2, 5, 3, 8, 6, 4, 9, 5, 7, 3, 6, 12],
            "70 alterações nos últimos 12 meses (demo)",
        ),
        series(
            "Últimos 30 dias",
            &[
                "19", "", "", "", "", "24", "", "", "", "", "29", "", "", "", "", "4", "", "", "", "", "9", "", "", "",
                "", "14", "", "", "", "17",
            ],
            &[0, 1, 0, 2, 0, 0, 1, 3, 0, 0, 0, 1, 0, 0, 2, 0, 1, 0, 0, 0, 4, 0, 0, 1, 0, 0, 2, 0, 1, 1],
            "21 alterações nos últimos 30 dias (demo)",
        ),
    ]));
    ui.set_changes(model(vec![
        ChangeRow {
            id: "c".into(),
            when: "Hoje 10:41".into(),
            kind: "Troca de controlador".into(),
            description: "Realtek High Definition Audio: 6.0.9600.1 → 6.0.9381.1 (demo)".into(),
            status: "Ativa".into(),
            scope: "Máquina".into(),
            can_revert: true,
        },
        ChangeRow {
            id: "d".into(),
            when: "Ontem 18:02".into(),
            kind: "Partilha de Internet".into(),
            description: "Hotspot bitBybit-demo a partilhar Ethernet (demo)".into(),
            status: "Revertida".into(),
            scope: "Rede".into(),
            can_revert: false,
        },
        ChangeRow {
            id: "e".into(),
            when: "12 nov 09:15".into(),
            kind: "Regra de firewall".into(),
            description: "Grupo bitBybit: bloquear acesso local de um cliente (demo)".into(),
            status: "Ativa".into(),
            scope: "Rede".into(),
            can_revert: true,
        },
        ChangeRow {
            id: "f".into(),
            when: "3 nov 21:40".into(),
            kind: "Dispositivo predefinido".into(),
            description: "Saída de áudio: Colunas Realtek (demo)".into(),
            status: "Revertida".into(),
            scope: "Utilizador".into(),
            can_revert: false,
        },
    ]));
    let choice =
        |luid: u64, label: &str, vpn: bool| crate::view::ShareChoice { luid, label: label.into(), vpn, wifi: false };
    crate::sharing_page::show(
        ui,
        crate::view::ShareChoices {
            sources: vec![choice(1, "Ethernet (demo)", false), choice(3, "WireGuard (demo)", true)],
            targets: vec![choice(1, "Ethernet (demo)", false), choice(2, "Ethernet 2 (não ligada) (demo)", false)],
            wifi: true,
        },
        if mode.starts_with("share") { uuid::Uuid::from_u128(1).into() } else { None },
        None,
    );
    if mode.starts_with("share") {
        crate::sharing_page::apply_cards(
            ui,
            &[crate::view::ClientCard {
                mac: "A4:5E:60:12:7B:C3".into(),
                title: "PlayStation 5 (demo)".into(),
                since: "Ligado há 48 min".into(),
                ip: "192.168.50.24".into(),
                allow_internet: true,
                allow_local: true,
                gaming: 1,
                fixed: String::new(),
            }],
        );
        // `=sharepath`: the live share on its settings tab (the signal path).
        ui.set_share_tab(if mode == "sharepath" { 0 } else { 1 });
        ui.set_share_name("Casa-bitBybit".into());
        ui.set_share_password("segredo-123".into());
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
        let v =
            crate::view::plan_view(&crate::view::force_plan(&f), bitbybit_localization::TextCatalog::default_catalog());
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
    if mode == "toast" {
        ui.set_op_state(3);
        ui.set_op_message("A operação não foi concluída: o Windows recusou a alteração (demo).".into());
        ui.set_op_elevate(true);
    }
    if mode == "adapter" {
        ui.set_share_mode(1);
    }
    // Network diagnostics (netdiag.slint): a finished ping.
    if mode == "netdiag" {
        use slint::ComponentHandle;
        let g = ui.global::<crate::ui::NetDiag>();
        let row = |n: &str, time: &str, status: &str, tone| crate::ui::NetDiagRow {
            item: n.into(),
            address: if tone == 4 { "192.168.1.1".into() } else { "".into() },
            time: time.into(),
            detail: if tone == 4 { "64".into() } else { "".into() },
            status: status.into(),
            tone,
        };
        g.set_target("192.168.1.1".into());
        g.set_kind(0);
        g.set_rows(model(vec![
            row("1", "3 ms", "Respondeu", 4),
            row("2", "2 ms", "Respondeu", 4),
            row("3", "", "Sem resposta", 1),
            row("4", "4 ms", "Respondeu", 4),
        ]));
        g.set_summary(
            "192.168.1.1: 4 enviados, 3 recebidos, 25 % perdidos · mín. 2 ms / méd. 3 ms / máx. 4 ms (demo)".into(),
        );
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
