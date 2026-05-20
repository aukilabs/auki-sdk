mod app_state;
mod flash;
mod sdk_runtime;
mod ui;

use app_state::DiagnosticApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Auki Diagnostics")
            .with_inner_size([1120.0, 760.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Auki Diagnostics",
        options,
        Box::new(|cc| Ok(Box::new(DiagnosticApp::new(cc)))),
    )
}
