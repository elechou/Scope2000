use std::sync::mpsc;
use std::time::Duration;

use eframe::egui;
use semver::Version;
use serde::Deserialize;

use crate::app::ScopeApp;
use crate::app::state::{ReleaseUpdate, UpdateCheckStatus};

const LATEST_RELEASE_API_URL: &str =
    "https://api.github.com/repos/elechou/Scope2000/releases/latest";
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    html_url: String,
}

impl ScopeApp {
    pub(in crate::app) fn begin_update_check_if_needed(&mut self, ctx: &egui::Context) {
        if self.update_check.requested || self.update_check_rx.is_some() {
            return;
        }
        self.update_check.requested = true;
        self.update_check.status = UpdateCheckStatus::Checking;

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = fetch_latest_release()
                .map(|release| {
                    release
                        .map(|release| {
                            update_status_for_release(env!("CARGO_PKG_VERSION"), release)
                        })
                        .unwrap_or(UpdateCheckStatus::UpToDate)
                })
                .unwrap_or(UpdateCheckStatus::Failed);
            let _ = tx.send(result);
        });
        self.update_check_rx = Some(rx);
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    pub(in crate::app) fn poll_update_check(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.update_check_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(status) => {
                self.update_check.status = status;
                self.update_check_rx = None;
                ctx.request_repaint();
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.update_check.status = UpdateCheckStatus::Failed;
                self.update_check_rx = None;
                ctx.request_repaint();
            }
        }
    }
}

fn fetch_latest_release() -> Result<Option<GitHubRelease>, reqwest::Error> {
    let response = reqwest::blocking::Client::builder()
        .timeout(UPDATE_CHECK_TIMEOUT)
        .user_agent(format!("Scope2000/{}", env!("CARGO_PKG_VERSION")))
        .build()?
        .get(LATEST_RELEASE_API_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }

    response.error_for_status()?.json().map(Some)
}

fn update_status_for_release(current_version: &str, release: GitHubRelease) -> UpdateCheckStatus {
    if is_newer_version(current_version, &release.tag_name) {
        let title = release
            .name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| release.tag_name.clone());
        UpdateCheckStatus::UpdateAvailable(ReleaseUpdate {
            version: release.tag_name,
            title,
            url: release.html_url,
        })
    } else {
        UpdateCheckStatus::UpToDate
    }
}

fn is_newer_version(current_version: &str, release_tag: &str) -> bool {
    let Ok(current) = Version::parse(normalize_version(current_version)) else {
        return false;
    };
    let Ok(latest) = Version::parse(normalize_version(release_tag)) else {
        return false;
    };
    latest > current
}

fn normalize_version(version: &str) -> &str {
    version
        .trim()
        .strip_prefix(['v', 'V'])
        .unwrap_or_else(|| version.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_accepts_plain_and_v_prefixed_tags() {
        assert!(is_newer_version("0.1.0", "v0.2.0"));
        assert!(is_newer_version("0.1.0", "0.1.1"));
    }

    #[test]
    fn version_compare_rejects_equal_older_and_non_semver_tags() {
        assert!(!is_newer_version("0.1.0", "v0.1.0"));
        assert!(!is_newer_version("0.1.0", "v0.0.9"));
        assert!(!is_newer_version("0.1.0", "latest"));
    }

    #[test]
    fn update_status_uses_release_url_when_newer() {
        let status = update_status_for_release(
            "0.1.0",
            GitHubRelease {
                tag_name: "v0.2.0".to_owned(),
                name: Some("Scope2000 0.2".to_owned()),
                html_url: "https://github.com/elechou/Scope2000/releases/tag/v0.2.0".to_owned(),
            },
        );

        match status {
            UpdateCheckStatus::UpdateAvailable(update) => {
                assert_eq!(update.version, "v0.2.0");
                assert_eq!(update.title, "Scope2000 0.2");
                assert!(update.url.ends_with("/v0.2.0"));
            }
            other => panic!("expected update, got {other:?}"),
        }
    }
}
