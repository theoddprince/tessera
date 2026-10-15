#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bridge;
mod state;
mod theme;
mod ui;
mod view;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Tessera")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([820.0, 520.0])
            .with_icon(theme::app_icon()),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native("Tessera", options, Box::new(|cc| Ok(Box::new(app::SwiftApp::new(cc)))))
}
