use crate::app_state::DiagnosticApp;
use crate::flash::{
    FLASH_ON, FLASH_PERIOD, FlashMode, apply_simulated_utc_offset_ns, flash_is_on, flash_is_on_i64,
    next_period_boundary_ns, utc_now_ns,
};
use crate::sdk_runtime::{Role, RuntimeCommand};
use crate::tick_report::TickReport;

pub fn render(ctx: &egui::Context, app: &mut DiagnosticApp) {
    egui::SidePanel::left("sidebar")
        .resizable(false)
        .default_width(300.0)
        .show(ctx, |ui| render_sidebar(ui, app));

    egui::CentralPanel::default().show(ctx, |ui| {
        render_status_strip(ui, app);
        ui.add_space(12.0);
        render_flash_panel(ui, app);
        ui.add_space(12.0);
        render_sync_quality_panel(ui, app);
        ui.add_space(12.0);
        render_bottom_diagnostics(ui, app);
    });
}

fn render_sidebar(ui: &mut egui::Ui, app: &mut DiagnosticApp) {
    let snapshot = app.snapshot().clone();
    ui.heading("Auki Diagnostics");
    ui.separator();

    ui.label("Peer self");
    ui.monospace(
        snapshot
            .local_peer_suffix
            .as_deref()
            .unwrap_or("not started"),
    );

    ui.horizontal(|ui| {
        ui.label("name:");
        if ui
            .text_edit_singleline(&mut app.display_name_input)
            .changed()
        {
            app.send(RuntimeCommand::SetDisplayName(
                app.display_name_input.clone(),
            ));
        }
    });
    ui.label(format!("role: {}", role_label(snapshot.role)));

    ui.separator();
    ui.label("Cluster");
    if ui
        .text_edit_singleline(&mut app.cluster_name_input)
        .changed()
    {
        app.send(RuntimeCommand::SetClusterName(
            app.cluster_name_input.clone(),
        ));
    }
    ui.label(format!("peers: {}", snapshot.peer_count));
    ui.label(format!(
        "manager: {}",
        snapshot.manager_suffix.as_deref().unwrap_or("none")
    ));

    if ui
        .add_enabled(!snapshot.join_in_flight, egui::Button::new("Join / Create"))
        .clicked()
    {
        app.send(RuntimeCommand::JoinOrCreate);
    }
    if ui
        .add_enabled(
            snapshot.role != Role::Unclustered,
            egui::Button::new("Leave Cluster"),
        )
        .clicked()
    {
        app.send(RuntimeCommand::LeaveCluster);
    }

    ui.separator();
    ui.label("Discovery URL");
    if ui
        .text_edit_singleline(&mut app.discovery_url_input)
        .changed()
    {
        app.send(RuntimeCommand::SetDiscoveryUrl(
            app.discovery_url_input.clone(),
        ));
    }
}

fn render_status_strip(ui: &mut egui::Ui, app: &DiagnosticApp) {
    let snapshot = app.snapshot();
    ui.horizontal(|ui| {
        status_box(ui, "Networking", networking_label(snapshot.role));
        status_box(ui, "Heartbeat", heartbeat_label(snapshot.role));
        status_box(
            ui,
            "Session -> domain",
            domain_status_label(&snapshot.domain_status),
        );
    });
}

fn render_flash_panel(ui: &mut egui::Ui, app: &mut DiagnosticApp) {
    let snapshot = app.snapshot().clone();
    ui.horizontal(|ui| {
        ui.label("Flash timing");
        let utc_selected = snapshot.flash_mode == FlashMode::Utc;
        if ui.selectable_label(utc_selected, "UTC").clicked() {
            app.send(RuntimeCommand::SetFlashMode(FlashMode::Utc));
        }

        let domain_selected = snapshot.flash_mode == FlashMode::Domain;
        if ui
            .add_enabled_ui(snapshot.domain_mode_available, |ui| {
                ui.selectable_label(domain_selected, "Domain")
            })
            .inner
            .clicked()
        {
            app.send(RuntimeCommand::SetFlashMode(FlashMode::Domain));
        }
    });
    ui.horizontal(|ui| {
        let mut sound_enabled = app.sound_enabled();
        if ui
            .add_enabled(
                app.sound_available(),
                egui::Checkbox::new(&mut sound_enabled, "Sound"),
            )
            .changed()
        {
            app.set_sound_enabled(sound_enabled);
        }
        if let Some(reason) = app.sound_unavailable_reason() {
            ui.label(format!("sound unavailable: {reason}"));
        }
    });
    ui.horizontal(|ui| {
        ui.label("Simulated UTC offset");
        let mut offset_ms = app.simulated_utc_offset_ms();
        if ui
            .add(
                egui::DragValue::new(&mut offset_ms)
                    .suffix(" ms")
                    .range(-5_000..=5_000),
            )
            .changed()
        {
            app.set_simulated_utc_offset_ms(offset_ms);
        }
        if ui.small_button("-250ms").clicked() {
            app.set_simulated_utc_offset_ms(-250);
        }
        if ui.small_button("0").clicked() {
            app.set_simulated_utc_offset_ms(0);
        }
        if ui.small_button("+250ms").clicked() {
            app.set_simulated_utc_offset_ms(250);
        }
    });
    ui.add_space(4.0);
    ui.label(format!("mode: {}", flash_mode_label(snapshot.flash_mode)));
    if snapshot.flash_mode == FlashMode::Domain {
        ui.label("baseline: domain clock estimate");
    } else {
        ui.label("baseline: local UTC");
    }
    ui.label(format!("period: {:.3}s", FLASH_PERIOD.as_secs_f32()));
    if !snapshot.domain_mode_available {
        ui.label(format!("Domain unavailable: {}", snapshot.domain_status));
    } else {
        if let Some(offset) = snapshot.domain_offset_ns {
            ui.label(format!("domain offset: {offset}ns"));
        }
        if let Some(uncertainty) = snapshot.domain_uncertainty_ns {
            ui.label(format!("uncertainty: {uncertainty}ns"));
        }
    }

    let now_ns = utc_now_ns();
    let biased_utc_ns = apply_simulated_utc_offset_ns(now_ns, app.simulated_utc_offset_ms());
    let next_tick_ns = next_period_boundary_ns(biased_utc_ns, FLASH_PERIOD.as_nanos());
    let next_tick_ms = next_tick_ns.saturating_sub(biased_utc_ns) as f64 / 1_000_000.0;
    ui.label(format!("next UTC tick: {next_tick_ms:.0}ms"));
    if app.simulated_utc_offset_ms() != 0 {
        ui.label(format!(
            "simulated offset: {:+}ms",
            app.simulated_utc_offset_ms()
        ));
    }
    if let Some(domain_now_ns) = snapshot
        .domain_now_ns
        .and_then(|ns| u128::try_from(ns).ok())
    {
        let next_domain_tick_ns = next_period_boundary_ns(domain_now_ns, FLASH_PERIOD.as_nanos());
        let next_domain_tick_ms =
            next_domain_tick_ns.saturating_sub(domain_now_ns) as f64 / 1_000_000.0;
        ui.label(format!("next domain tick: {next_domain_tick_ms:.0}ms"));
    }

    let on = match snapshot.flash_mode {
        FlashMode::Utc => flash_is_on(biased_utc_ns, FLASH_PERIOD.as_nanos(), FLASH_ON.as_nanos()),
        FlashMode::Domain => snapshot
            .domain_now_ns
            .map(|now| flash_is_on_i64(now, FLASH_PERIOD.as_nanos(), FLASH_ON.as_nanos()))
            .unwrap_or(false),
    };
    if app.record_flash_state(on, now_ns) {
        publish_tick_report(app, &snapshot, now_ns, biased_utc_ns);
    }

    let desired = egui::vec2(ui.available_width(), 380.0);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let bg = if on {
        egui::Color32::from_rgb(250, 204, 21)
    } else {
        egui::Color32::from_rgb(15, 23, 42)
    };
    painter.rect_filled(rect, 8.0, bg);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        if on { "FLASH" } else { "waiting" },
        egui::FontId::proportional(56.0),
        if on {
            egui::Color32::from_rgb(17, 24, 39)
        } else {
            egui::Color32::from_rgb(229, 231, 235)
        },
    );
}

fn publish_tick_report(
    app: &DiagnosticApp,
    snapshot: &crate::sdk_runtime::RuntimeSnapshot,
    now_ns: u128,
    biased_utc_ns: u128,
) {
    let Some(peer_id) = snapshot.local_peer_id.clone() else {
        return;
    };
    let Some(peer_suffix) = snapshot.local_peer_suffix.clone() else {
        return;
    };

    let tick_source_ns = match snapshot.flash_mode {
        FlashMode::Utc => i64::try_from(biased_utc_ns).ok(),
        FlashMode::Domain => snapshot.domain_now_ns,
    };
    let Some(tick_source_ns) = tick_source_ns else {
        return;
    };

    app.publish_tick_report(TickReport {
        peer_id,
        peer_suffix,
        tick_id: tick_source_ns / FLASH_PERIOD.as_nanos() as i64,
        mode: snapshot.flash_mode,
        utc_observed_ns: i64::try_from(now_ns).unwrap_or(i64::MAX),
        biased_utc_observed_ns: i64::try_from(biased_utc_ns).unwrap_or(i64::MAX),
        domain_observed_ns: snapshot.domain_now_ns,
        simulated_utc_offset_ms: app.simulated_utc_offset_ms(),
    });
}

fn render_sync_quality_panel(ui: &mut egui::Ui, app: &DiagnosticApp) {
    let stats = &app.snapshot().peer_tick_stats;
    ui.heading("Sync Quality");
    if stats.is_empty() {
        ui.label("Waiting for matching peer tick reports");
        return;
    }

    egui::Grid::new("sync_quality_grid")
        .striped(true)
        .show(ui, |ui| {
            ui.label("Peer");
            ui.label("UTC latest / p95");
            ui.label("Domain latest / p95");
            ui.label("Improvement");
            ui.label("Samples");
            ui.end_row();

            for row in stats {
                ui.monospace(&row.peer_suffix);
                ui.label(format_latest_p95(
                    row.utc_latest_delta_ms,
                    row.utc_p95_delta_ms,
                ));
                ui.label(format_latest_p95(
                    row.domain_latest_delta_ms,
                    row.domain_p95_delta_ms,
                ));
                ui.label(
                    row.improvement_ratio
                        .map(|ratio| format!("{ratio:.1}x"))
                        .unwrap_or_else(|| "-".into()),
                );
                ui.label(row.samples.to_string());
                ui.end_row();
            }
        });
}

fn render_bottom_diagnostics(ui: &mut egui::Ui, app: &DiagnosticApp) {
    let snapshot = app.snapshot();
    ui.columns(2, |columns| {
        columns[0].heading("Peers");
        for peer in &snapshot.peers {
            columns[0].monospace(format!("{} {}", peer.suffix, role_label(peer.role)));
        }

        columns[1].heading("Events");
        for event in snapshot.events.iter().rev().take(8) {
            columns[1].monospace(event);
        }
        columns[1].add_space(8.0);
        columns[1].heading("Flash Events");
        for event in app.flash_events().iter().rev() {
            columns[1].monospace(event);
        }
    });
}

fn status_box(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(180.0);
        ui.label(label);
        ui.heading(value);
    });
}

fn format_latest_p95(latest: Option<f64>, p95: Option<f64>) -> String {
    match (latest, p95) {
        (Some(latest), Some(p95)) => format!("{latest:.1} / {p95:.1}ms"),
        (Some(latest), None) => format!("{latest:.1}ms"),
        _ => "-".into(),
    }
}

fn role_label(role: Role) -> &'static str {
    match role {
        Role::Unclustered => "Unclustered",
        Role::Manager => "Manager",
        Role::Member => "Member",
    }
}

fn networking_label(role: Role) -> &'static str {
    match role {
        Role::Unclustered => "Not clustered",
        Role::Manager | Role::Member => "Clustered",
    }
}

fn heartbeat_label(role: Role) -> &'static str {
    match role {
        Role::Unclustered => "inactive",
        Role::Manager | Role::Member => "active",
    }
}

fn domain_status_label(status: &str) -> &str {
    match status {
        "synced" => "synced",
        "not clustered" => "not clustered",
        _ => "waiting",
    }
}

fn flash_mode_label(mode: FlashMode) -> &'static str {
    match mode {
        FlashMode::Utc => "UTC",
        FlashMode::Domain => "Domain",
    }
}
