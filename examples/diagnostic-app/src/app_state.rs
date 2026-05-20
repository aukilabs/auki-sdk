use crate::sdk_runtime::{RuntimeCommand, RuntimeSnapshot, SdkRuntime};

pub struct DiagnosticApp {
    runtime: SdkRuntime,
    snapshot: RuntimeSnapshot,
    pub(crate) discovery_url_input: String,
    pub(crate) cluster_name_input: String,
    pub(crate) display_name_input: String,
}

impl DiagnosticApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let runtime = SdkRuntime::spawn();
        let snapshot = runtime.snapshot();
        Self {
            discovery_url_input: snapshot.discovery_url.clone(),
            cluster_name_input: snapshot.cluster_name.clone(),
            display_name_input: snapshot.display_name.clone(),
            runtime,
            snapshot,
        }
    }

    pub fn send(&self, command: RuntimeCommand) {
        self.runtime.send(command);
    }

    pub fn refresh_snapshot(&mut self) {
        self.snapshot = self.runtime.snapshot();
    }

    pub fn snapshot(&self) -> &RuntimeSnapshot {
        &self.snapshot
    }
}

impl eframe::App for DiagnosticApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.refresh_snapshot();
        crate::ui::render(ctx, self);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flash::FlashMode;
    use crate::sdk_runtime::{PeerRow, Role};

    #[test]
    fn domain_mode_is_disabled_when_unavailable() {
        let mut state = RuntimeSnapshot::default_for_tests();
        state.flash_mode = FlashMode::Utc;
        state.domain_mode_available = false;

        state.apply_command(RuntimeCommand::SetFlashMode(FlashMode::Domain));

        assert_eq!(state.flash_mode, FlashMode::Utc);
        assert_eq!(state.events.last().unwrap(), "domain flash unavailable");
    }

    #[test]
    fn domain_mode_can_be_selected_when_available() {
        let mut state = RuntimeSnapshot::default_for_tests();
        state.domain_mode_available = true;

        state.apply_command(RuntimeCommand::SetFlashMode(FlashMode::Domain));

        assert_eq!(state.flash_mode, FlashMode::Domain);
    }

    #[test]
    fn manager_row_uses_role_from_snapshot() {
        let row = PeerRow {
            suffix: "...Q2La9F".into(),
            role: Role::Manager,
        };

        assert_eq!(row.role, Role::Manager);
        assert_eq!(row.suffix, "...Q2La9F");
    }
}
