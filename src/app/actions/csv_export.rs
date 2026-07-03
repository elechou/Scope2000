use std::path::PathBuf;
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
    }

    pub(in crate::app) fn save_csv_with_dialog(&mut self) {
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
            self.csv.save_rx = Some(spawn_csv_write(path, snapshot));
        }
    }

    pub(in crate::app) fn quick_snapshot(&mut self) {
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
        let path = resolve_snapshot_dir(&self.csv.snapshot_dir).join(format!("{filename}.csv"));
        if path.exists() {
            self.csv.overwrite_pending = Some(OverwritePending { path, snapshot });
        } else {
            self.csv.save_rx = Some(spawn_csv_write(path, snapshot));
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
                    ui.label(pending.path.display().to_string());
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
                self.csv.save_rx = Some(spawn_csv_write(pending.path, pending.snapshot));
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
