use crate::sdk_runtime::{RuntimeCommand, RuntimeSnapshot, SdkRuntime};
use crate::sound::SoundEngine;

const MAX_FLASH_EVENTS: usize = 12;

pub struct DiagnosticApp {
    runtime: SdkRuntime,
    snapshot: RuntimeSnapshot,
    flash_events: FlashEventLog,
    sound: SoundEngine,
    sound_enabled: bool,
    pub(crate) discovery_url_input: String,
    pub(crate) cluster_name_input: String,
    pub(crate) display_name_input: String,
}

#[derive(Debug, Default)]
struct FlashEventLog {
    events: Vec<String>,
    last_on: bool,
}

impl DiagnosticApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let runtime = SdkRuntime::spawn();
        let snapshot = runtime.snapshot();
        let sound = SoundEngine::new();
        let sound_enabled = sound.is_available();
        Self {
            discovery_url_input: snapshot.discovery_url.clone(),
            cluster_name_input: snapshot.cluster_name.clone(),
            display_name_input: snapshot.display_name.clone(),
            runtime,
            snapshot,
            flash_events: FlashEventLog::default(),
            sound,
            sound_enabled,
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

    pub fn record_flash_state(&mut self, on: bool, now_ns: u128) -> bool {
        let started = self.flash_events.record(on, now_ns);
        if started && self.sound_enabled {
            self.sound.beep();
        }
        started
    }

    pub fn flash_events(&self) -> &[String] {
        self.flash_events.events()
    }

    pub fn sound_enabled(&self) -> bool {
        self.sound_enabled
    }

    pub fn set_sound_enabled(&mut self, enabled: bool) {
        self.sound_enabled = enabled;
    }

    pub fn sound_available(&self) -> bool {
        self.sound.is_available()
    }

    pub fn sound_unavailable_reason(&self) -> Option<&str> {
        self.sound.unavailable_reason()
    }
}

impl eframe::App for DiagnosticApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.refresh_snapshot();
        crate::ui::render(ctx, self);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

impl FlashEventLog {
    fn record(&mut self, on: bool, now_ns: u128) -> bool {
        let started = on && !self.last_on;
        if started {
            self.events
                .push(format!("flash UTC {}", format_utc_time_of_day(now_ns)));
            if self.events.len() > MAX_FLASH_EVENTS {
                self.events.remove(0);
            }
        }
        self.last_on = on;
        started
    }

    fn events(&self) -> &[String] {
        &self.events
    }
}

fn format_utc_time_of_day(now_ns: u128) -> String {
    let total_ms = now_ns / 1_000_000;
    let ms = total_ms % 1_000;
    let total_seconds = total_ms / 1_000;
    let seconds_in_day = total_seconds % 86_400;
    let hours = seconds_in_day / 3_600;
    let minutes = (seconds_in_day % 3_600) / 60;
    let seconds = seconds_in_day % 60;

    format!("{hours:02}:{minutes:02}:{seconds:02}.{ms:03}")
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

    #[test]
    fn flash_event_logs_on_rising_edge() {
        let mut state = FlashEventLog::default();

        assert!(!state.record(false, 11_999_900_000));
        assert!(state.record(true, 12_000_000_000));

        assert_eq!(state.events(), &["flash UTC 00:00:12.000"]);
    }

    #[test]
    fn flash_event_does_not_repeat_while_flash_stays_on() {
        let mut state = FlashEventLog::default();

        assert!(state.record(true, 12_000_000_000));
        assert!(!state.record(true, 12_000_050_000));

        assert_eq!(state.events().len(), 1);
    }

    #[test]
    fn flash_event_log_is_bounded() {
        let mut state = FlashEventLog::default();

        for tick in 1..=20 {
            state.record(false, tick * 3_000_000_000 - 1);
            state.record(true, tick * 3_000_000_000);
        }

        assert_eq!(state.events().len(), MAX_FLASH_EVENTS);
        assert_eq!(state.events().first().unwrap(), "flash UTC 00:00:27.000");
    }
}
