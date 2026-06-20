#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_icon;
mod console;
mod source;
mod theme;
mod ui;
mod variable;
mod wave;

use eframe::egui;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let icon = app_icon::prepare_icon(
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/scope2000.png"))
            .expect("load Scope2000 icon"),
    );
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_icon(icon),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Scope2000",
        options,
        Box::new(|context| Ok(Box::new(app::ScopeApp::new(context)))),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(())
}
