use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;

use eframe::egui;

use crate::app::ScopeApp;
use crate::console::LogLevel;
use crate::theme;
use crate::wave::csv::{CsvSnapshot, OverwritePending};

impl ScopeApp {
    pub(in crate::app) fn snapshot_wave_data(&self) -> Option<CsvSnapshot> {
        let mut names: Vec<String> = self.plot_data.series.keys().cloned().collect();
        names.sort();

        let mut times = None;
        let mut values = Vec::new();
        let mut channel_names = Vec::new();
        for name in names {
            let Some(series) = self.plot_data.series.get(&name) else {
                continue;
            };
            if series.times.is_empty() {
                continue;
            }
            if times.is_none() {
                times = Some(series.times.iter().copied().collect());
            }
            values.push(series.values.iter().copied().collect());
            channel_names.push(name);
        }

        let times = times?;
        (!channel_names.is_empty()).then_some(CsvSnapshot {
            channel_names,
            times,
            values,
        })
    }

    pub(in crate::app) fn poll_csv_save(&mut self) {
        let result = self.csv.save_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            self.csv.save_rx = None;
            match result {
                Ok(path) => self
                    .log
                    .push(LogLevel::Notice, format!("CSV saved: {}", path.display())),
                Err(error) => self
                    .log
                    .push(LogLevel::Error, format!("CSV save failed: {error}")),
            }
        }

        let result = self
            .csv
            .screenshot_save_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            self.csv.screenshot_save_rx = None;
            match result {
                Ok(path) => self.log.push(
                    LogLevel::Notice,
                    format!("Screenshot saved: {}", path.display()),
                ),
                Err(error) => self
                    .log
                    .push(LogLevel::Error, format!("Screenshot save failed: {error}")),
            }
        }
    }

    pub(in crate::app) fn poll_csv_screenshot(&mut self, ctx: &egui::Context) {
        let image = ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| {
                    if let egui::Event::Screenshot { image, .. } = event {
                        Some(Arc::clone(image))
                    } else {
                        None
                    }
                })
                .next_back()
        });

        if let Some(image) = image
            && let Some(path) = self.csv.pending_screenshot_path.take()
        {
            self.csv.screenshot_save_rx = Some(spawn_screenshot_write(path, image));
        }
    }

    pub(in crate::app) fn save_csv_with_dialog(&mut self, ctx: &egui::Context) {
        let Some(snapshot) = self.snapshot_wave_data() else {
            self.log
                .push(LogLevel::Warn, "No wave data to save".to_owned());
            return;
        };

        let dialog_dir = resolve_snapshot_dir(&self.csv.snapshot_dir);
        let dialog = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("scope2000.csv")
            .set_directory(dialog_dir);

        if let Some(path) = dialog.save_file() {
            let screenshot_path = screenshot_path_for_csv(&path, self.csv.save_with_screenshot);
            self.begin_csv_export(ctx, path, snapshot, screenshot_path);
        }
    }

    pub(in crate::app) fn quick_snapshot(&mut self, ctx: &egui::Context) {
        let Some(snapshot) = self.snapshot_wave_data() else {
            self.log
                .push(LogLevel::Warn, "No wave data to save".to_owned());
            return;
        };

        let lookup = |name: &str| self.inspector.value_by_name(name);
        let filename = match evaluate_template(&self.csv.filename_template, &lookup) {
            Ok(filename) => filename,
            Err(error) => {
                self.log
                    .push(LogLevel::Error, format!("Template error: {error}"));
                return;
            }
        };
        let csv_path = resolve_snapshot_dir(&self.csv.snapshot_dir).join(format!("{filename}.csv"));
        let screenshot_path = screenshot_path_for_csv(&csv_path, self.csv.save_with_screenshot);
        if csv_path.exists() || screenshot_path.as_ref().is_some_and(|path| path.exists()) {
            self.csv.overwrite_pending = Some(OverwritePending {
                csv_path,
                screenshot_path,
                snapshot,
            });
        } else {
            self.begin_csv_export(ctx, csv_path, snapshot, screenshot_path);
        }
    }

    fn begin_csv_export(
        &mut self,
        ctx: &egui::Context,
        csv_path: PathBuf,
        snapshot: CsvSnapshot,
        screenshot_path: Option<PathBuf>,
    ) {
        self.csv.save_rx = Some(spawn_csv_write(csv_path, snapshot));
        if let Some(path) = screenshot_path {
            self.csv.pending_screenshot_path = Some(path);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            ctx.request_repaint();
        }
    }

    pub(in crate::app) fn show_overwrite_modal(&mut self, ui: &egui::Ui) {
        if self.csv.overwrite_pending.is_none() {
            return;
        }

        let mut action = None;
        egui::Modal::new("csv_overwrite_modal".into()).show(ui.ctx(), |ui| {
            ui.set_width(400.0);
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                theme::modal_title(ui, "File already exists");
                ui.add_space(4.0);
                if let Some(ref pending) = self.csv.overwrite_pending {
                    if pending.csv_path.exists() {
                        ui.label(pending.csv_path.display().to_string());
                    }
                    if let Some(path) = pending
                        .screenshot_path
                        .as_ref()
                        .filter(|path| path.exists())
                    {
                        ui.label(path.display().to_string());
                    }
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let spacing = ui.spacing().item_spacing.x;
                    let btn_total = 60.0 * 2.0 + spacing;
                    ui.add_space(((ui.available_width() - btn_total) / 2.0).max(0.0));
                    if theme::modal_button(ui, "Overwrite", theme::RED) {
                        action = Some(true);
                    }
                    if theme::modal_button(ui, "Cancel", theme::WIDGET_BG) {
                        action = Some(false);
                    }
                });
                ui.add_space(4.0);
            });
        });

        match action {
            Some(true) => {
                let pending = self
                    .csv
                    .overwrite_pending
                    .take()
                    .expect("pending overwrite");
                self.begin_csv_export(
                    ui.ctx(),
                    pending.csv_path,
                    pending.snapshot,
                    pending.screenshot_path,
                );
            }
            Some(false) => {
                self.csv.overwrite_pending = None;
            }
            None => {}
        }
    }
}

fn resolve_snapshot_dir(configured_dir: &str) -> PathBuf {
    let configured_dir = configured_dir.trim();
    if !configured_dir.is_empty() {
        return PathBuf::from(configured_dir);
    }

    dirs::download_dir()
        .or_else(dirs::home_dir)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn screenshot_path_for_csv(csv_path: &Path, enabled: bool) -> Option<PathBuf> {
    enabled.then(|| csv_path.with_extension("png"))
}

fn write_screenshot_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    let width = u32::try_from(image.size[0])
        .map_err(|_| format!("Screenshot width is too large: {}", image.size[0]))?;
    let height = u32::try_from(image.size[1])
        .map_err(|_| format!("Screenshot height is too large: {}", image.size[1]))?;
    let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
    for pixel in &image.pixels {
        rgba.extend_from_slice(&pixel.to_srgba_unmultiplied());
    }

    image::save_buffer_with_format(
        path,
        &rgba,
        width,
        height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|error| error.to_string())
}

fn spawn_screenshot_write(
    path: PathBuf,
    image: Arc<egui::ColorImage>,
) -> mpsc::Receiver<Result<PathBuf, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = write_screenshot_png(&path, &image).map(|()| path);
        let _ = tx.send(result);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_dir_keeps_non_empty_config() {
        let path = if cfg!(windows) {
            r"C:\scope2000\captures"
        } else {
            "/tmp/scope2000/captures"
        };

        assert_eq!(resolve_snapshot_dir(path), PathBuf::from(path));
        assert_eq!(
            resolve_snapshot_dir(&format!("  {path}  ")),
            PathBuf::from(path)
        );
    }

    #[test]
    fn empty_snapshot_dir_uses_platform_default() {
        let expected = dirs::download_dir()
            .or_else(dirs::home_dir)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        assert_eq!(resolve_snapshot_dir(""), expected);
        assert_eq!(resolve_snapshot_dir(" \t"), expected);
    }

    #[test]
    fn screenshot_path_uses_csv_stem_when_enabled() {
        let csv_path = PathBuf::from("scope_20260708_120000.csv");

        assert_eq!(
            screenshot_path_for_csv(&csv_path, true),
            Some(PathBuf::from("scope_20260708_120000.png"))
        );
        assert_eq!(screenshot_path_for_csv(&csv_path, false), None);
    }
}

pub(in crate::app) fn evaluate_template(
    template: &str,
    var_lookup: &dyn Fn(&str) -> Option<f64>,
) -> Result<String, String> {
    let now = chrono::Local::now();
    let mut result = String::with_capacity(template.len());
    let chars: Vec<char> = template.chars().collect();
    let mut index = 0;

    while index < chars.len() {
        if index + 1 < chars.len() && chars[index] == '{' && chars[index + 1] == '$' {
            let start = index;
            let mut end = None;
            for (cursor, character) in chars.iter().enumerate().skip(index + 2) {
                if *character == '}' {
                    end = Some(cursor);
                    break;
                }
            }
            let Some(end) = end else {
                result.push(chars[index]);
                index += 1;
                continue;
            };
            let token: String = chars[(start + 2)..end].iter().collect();
            result.push_str(&resolve_token(&token, &now, var_lookup)?);
            index = end + 1;
        } else {
            result.push(chars[index]);
            index += 1;
        }
    }

    Ok(result)
}

fn resolve_token(
    token: &str,
    now: &chrono::DateTime<chrono::Local>,
    var_lookup: &dyn Fn(&str) -> Option<f64>,
) -> Result<String, String> {
    match token {
        "Date" => Ok(now.format("%Y%m%d").to_string()),
        "Time" => Ok(now.format("%H%M%S").to_string()),
        "DateTime" => Ok(now.format("%Y%m%d_%H%M%S").to_string()),
        _ => {
            if let Some((name, fmt)) = token.split_once(':') {
                let value = var_lookup(name).ok_or_else(|| format!("Unknown variable: {name}"))?;
                let precision = parse_precision(fmt)?;
                Ok(format!("{value:.precision$}"))
            } else {
                let value =
                    var_lookup(token).ok_or_else(|| format!("Unknown variable: {token}"))?;
                Ok(format!("{value:.6}"))
            }
        }
    }
}

fn parse_precision(fmt: &str) -> Result<usize, String> {
    let fmt = fmt.strip_prefix('.').unwrap_or(fmt);
    let fmt = fmt.strip_suffix('f').unwrap_or(fmt);
    fmt.parse::<usize>()
        .map_err(|_| format!("Invalid format spec: {fmt}"))
}

fn write_csv_file(path: &PathBuf, snapshot: &CsvSnapshot) -> Result<(), String> {
    let mut writer = csv::Writer::from_path(path).map_err(|error| error.to_string())?;
    let mut header = Vec::with_capacity(1 + snapshot.channel_names.len());
    header.push("trigger_time_s");
    for name in &snapshot.channel_names {
        header.push(name.as_str());
    }
    writer
        .write_record(&header)
        .map_err(|error| error.to_string())?;

    for sample in 0..snapshot.times.len() {
        let mut row = Vec::with_capacity(1 + snapshot.values.len());
        row.push(snapshot.times[sample].to_string());
        for channel in &snapshot.values {
            row.push(channel.get(sample).copied().unwrap_or(f64::NAN).to_string());
        }
        writer
            .write_record(&row)
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    Ok(())
}

fn spawn_csv_write(
    path: PathBuf,
    snapshot: CsvSnapshot,
) -> mpsc::Receiver<Result<PathBuf, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = write_csv_file(&path, &snapshot).map(|()| path);
        let _ = tx.send(result);
    });
    rx
}
