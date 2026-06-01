mod anki;
mod app;
mod error;
mod models;
mod providers;
mod storage;

use app::English2AnkiApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "English2Anki",
        options,
        Box::new(|cc| Box::new(English2AnkiApp::new(cc))),
    )
}
