use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use super::{AppConfig, WorkspaceState};
use crate::source::DeviceInfo;

pub(crate) const UNTITLED_PROJECT: &str = "untitled";
const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalProject {
    pub name: String,
    pub project_file: PathBuf,
    pub build_time_utc: Option<u32>,
    pub build_hash: Option<u32>,
}

impl LocalProject {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let project_file = path.as_ref().canonicalize().with_context(|| {
            format!(
                "cannot resolve CCS project file {}",
                path.as_ref().display()
            )
        })?;
        let text = std::fs::read_to_string(&project_file)
            .with_context(|| format!("cannot read {}", project_file.display()))?;
        let xml = roxmltree::Document::parse(&text)
            .with_context(|| format!("invalid XML in {}", project_file.display()))?;
        let name = xml
            .descendants()
            .find(|node| node.has_tag_name("projectDescription"))
            .and_then(|root| {
                root.children()
                    .find(|node| node.is_element() && node.has_tag_name("name"))
            })
            .and_then(|node| node.text())
            .unwrap_or_default();
        let name = normalize_project_name(name)?;
        Ok(Self {
            name,
            project_file,
            build_time_utc: None,
            build_hash: None,
        })
    }

    pub fn build_time_display_text(&self) -> String {
        display_build_time(self.build_time_utc)
    }
}

#[derive(Deserialize)]
struct BakerReport {
    #[serde(default)]
    build_hash: Option<u32>,
    project_info: BakerProjectInfo,
}

#[derive(Deserialize)]
struct BakerProjectInfo {
    project_name: String,
    build_time_utc: u32,
}

/// Reads a single baker report, returning its build identity when it still
/// belongs to `project_name`. Cheap enough to call on every refresh tick.
fn read_build_report(report_path: &Path, project_name: &str) -> Option<(u32, Option<u32>)> {
    let text = std::fs::read_to_string(report_path).ok()?;
    let report = serde_json::from_str::<BakerReport>(&text).ok()?;
    (report.project_info.project_name == project_name && report.project_info.build_time_utc != 0)
        .then_some((report.project_info.build_time_utc, report.build_hash))
}

fn discover_local_build(
    project_dir: &Path,
    project_name: &str,
) -> Option<(PathBuf, u32, Option<u32>)> {
    walkdir::WalkDir::new(project_dir)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_file() && entry.file_name() == "v2k_user_desc_report.json"
        })
        .filter_map(|entry| {
            let report = read_build_report(entry.path(), project_name)?;
            Some((entry.into_path(), report))
        })
        .max_by_key(|(_, (build_time_utc, _))| *build_time_utc)
        .map(|(path, (build_time_utc, build_hash))| (path, build_time_utc, build_hash))
}

/// Loads the fast CCS identity first, then discovers baker metadata. Call this
/// only from a worker thread: the metadata search walks build directories.
pub(crate) fn load_local_project_with_metadata(
    path: impl AsRef<Path>,
) -> anyhow::Result<LocalProject> {
    let mut project = LocalProject::load(path)?;
    if let Some((_, build_time_utc, build_hash)) = discover_local_build(
        project
            .project_file
            .parent()
            .unwrap_or(&project.project_file),
        &project.name,
    ) {
        project.build_time_utc = Some(build_time_utc);
        project.build_hash = build_hash;
    }
    Ok(project)
}

/// A refreshed project plus the report file the build identity came from, so
/// the next refresh can re-read just that file instead of walking the tree.
#[derive(Debug, Clone)]
pub(crate) struct LocalBuildScan {
    pub project: LocalProject,
    pub report_path: Option<PathBuf>,
}

/// Periodic build refresh for the bound project. When the previously used
/// report is still readable it is re-read directly (a rebuild rewrites it in
/// place); otherwise the tree is walked to find the newest matching report.
pub(crate) fn refresh_local_build(
    project_file: PathBuf,
    expected_name: String,
    cached_report: Option<PathBuf>,
) -> anyhow::Result<LocalBuildScan> {
    if let Some(report_path) = cached_report
        && let Some((build_time_utc, build_hash)) = read_build_report(&report_path, &expected_name)
    {
        return Ok(LocalBuildScan {
            project: LocalProject {
                name: expected_name,
                project_file,
                build_time_utc: Some(build_time_utc),
                build_hash,
            },
            report_path: Some(report_path),
        });
    }

    let mut project = LocalProject::load(&project_file)?;
    let report_path = discover_local_build(
        project
            .project_file
            .parent()
            .unwrap_or(&project.project_file),
        &project.name,
    )
    .map(|(report_path, build_time_utc, build_hash)| {
        project.build_time_utc = Some(build_time_utc);
        project.build_hash = build_hash;
        report_path
    });
    Ok(LocalBuildScan {
        project,
        report_path,
    })
}

pub(crate) fn display_build_time(build_time_utc: Option<u32>) -> String {
    build_time_utc
        .filter(|time| *time != 0)
        .and_then(|time| chrono::DateTime::from_timestamp(i64::from(time), 0))
        .map(|time| time.with_timezone(&chrono::Local))
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "Not available".to_owned())
}

pub(crate) fn normalize_project_name(name: &str) -> anyhow::Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(UNTITLED_PROJECT.to_owned());
    }
    if name.len() > 32
        || !name
            .as_bytes()
            .iter()
            .all(|value| (0x20..=0x7e).contains(value))
    {
        bail!("CCS project name must be 1-32 printable ASCII characters");
    }
    Ok(name.to_owned())
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectCandidate {
    pub relative_path: PathBuf,
    pub project: Result<LocalProject, String>,
}

pub(crate) fn scan_project_directory(root: &Path) -> Vec<ProjectCandidate> {
    let mut candidates = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() || entry.file_name() != ".project" {
            continue;
        }
        let path = entry.path().to_path_buf();
        let relative_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        let project = load_local_project_with_metadata(&path).map_err(|error| error.to_string());
        candidates.push(ProjectCandidate {
            relative_path,
            project,
        });
    }
    candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    candidates
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ProjectBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_file: Option<PathBuf>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_time_utc: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_hash: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ProjectRegistry {
    pub format_version: u32,
    pub projects: BTreeMap<String, ProjectBinding>,
    pub recent_projects: Vec<String>,
}

impl Default for ProjectRegistry {
    fn default() -> Self {
        Self::default_with_version()
    }
}

impl ProjectRegistry {
    pub fn load(protected_name: Option<&str>) -> Self {
        let Some(path) = Self::path() else {
            return Self::default_with_version();
        };
        let mut registry = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .filter(|registry: &Self| registry.format_version == FORMAT_VERSION)
            .unwrap_or_else(Self::default_with_version);
        let original_names: std::collections::BTreeSet<_> =
            registry.projects.keys().cloned().collect();
        registry.projects.retain(|name, _| {
            normalize_project_name(name)
                .is_ok_and(|normalized| normalized == *name && normalized != UNTITLED_PROJECT)
        });
        let mut seen = std::collections::BTreeSet::new();
        registry
            .recent_projects
            .retain(|name| registry.projects.contains_key(name) && seen.insert(name.clone()));
        for name in registry.projects.keys() {
            if !registry.recent_projects.contains(name) {
                registry.recent_projects.push(name.clone());
            }
        }
        if let Some(name) = protected_name.filter(|name| registry.projects.contains_key(*name)) {
            registry.recent_projects.retain(|recent| recent != name);
            registry.recent_projects.insert(0, name.to_owned());
        }
        registry.recent_projects.truncate(100);
        let retained: std::collections::BTreeSet<_> =
            registry.recent_projects.iter().cloned().collect();
        registry.projects.retain(|name, _| retained.contains(name));
        for evicted in original_names.difference(&retained) {
            let _ = WorkspaceStore::delete(evicted);
        }
        registry
    }

    fn default_with_version() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            projects: BTreeMap::new(),
            recent_projects: Vec::new(),
        }
    }

    pub fn path() -> Option<PathBuf> {
        AppConfig::config_dir().map(|dir| dir.join("projects.toml"))
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        super::workspace_state::write_toml_atomic(&path, self)
    }

    pub fn valid_local(&self, name: &str, verified_only: bool) -> Option<LocalProject> {
        let binding = self.projects.get(name)?;
        if verified_only && !binding.verified {
            return None;
        }
        LocalProject::load(binding.project_file.as_ref()?)
            .ok()
            .filter(|project| project.name == name)
            .map(|mut project| {
                project.build_time_utc = binding.build_time_utc;
                project.build_hash = binding.build_hash;
                project
            })
    }

    pub fn touch(&mut self, name: &str) -> Vec<String> {
        if name == UNTITLED_PROJECT {
            return Vec::new();
        }
        self.projects.entry(name.to_owned()).or_default();
        self.recent_projects.retain(|recent| recent != name);
        self.recent_projects.insert(0, name.to_owned());

        let mut evicted = Vec::new();
        while self.recent_projects.len() > 100 {
            if let Some(name) = self.recent_projects.pop() {
                self.projects.remove(&name);
                evicted.push(name);
            }
        }
        evicted
    }

    pub fn remove(&mut self, name: &str) {
        self.projects.remove(name);
        self.recent_projects.retain(|recent| recent != name);
    }

    pub fn unbind(&mut self, name: &str) {
        if let Some(binding) = self.projects.get_mut(name) {
            binding.project_file = None;
            binding.verified = false;
            binding.build_time_utc = None;
            binding.build_hash = None;
        }
    }

    pub fn recent_names(&self) -> impl Iterator<Item = &str> {
        self.recent_projects
            .iter()
            .filter(|name| self.projects.contains_key(name.as_str()))
            .map(String::as_str)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceFile {
    format_version: u32,
    project_name: String,
    workspace: WorkspaceState,
}

pub(crate) struct WorkspaceStore;

impl WorkspaceStore {
    fn project_dir(name: &str) -> Option<PathBuf> {
        let key: String = name
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        AppConfig::config_dir().map(|dir| dir.join("projects").join(key))
    }

    pub fn workspace_path(name: &str) -> Option<PathBuf> {
        Self::project_dir(name).map(|dir| dir.join("workspace.toml"))
    }

    pub fn load(name: &str) -> WorkspaceState {
        if name == UNTITLED_PROJECT {
            return WorkspaceState::default();
        }
        Self::workspace_path(name)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str::<WorkspaceFile>(&text).ok())
            .filter(|file| file.format_version == FORMAT_VERSION && file.project_name == name)
            .map(|file| file.workspace)
            .unwrap_or_default()
    }

    pub fn save(name: &str, workspace: &WorkspaceState) -> anyhow::Result<()> {
        if name == UNTITLED_PROJECT {
            return Ok(());
        }
        let Some(path) = Self::workspace_path(name) else {
            return Ok(());
        };
        super::workspace_state::write_toml_atomic(
            &path,
            &WorkspaceFile {
                format_version: FORMAT_VERSION,
                project_name: name.to_owned(),
                workspace: workspace.clone(),
            },
        )
    }

    pub fn save_legacy(workspace: &WorkspaceState) -> anyhow::Result<()> {
        let Some(path) = AppConfig::config_dir().map(|dir| dir.join("legacy-workspace.toml"))
        else {
            return Ok(());
        };
        super::workspace_state::write_toml_atomic(&path, workspace)
    }

    pub fn delete(name: &str) -> anyhow::Result<()> {
        let Some(path) = Self::project_dir(name) else {
            return Ok(());
        };
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectStatus {
    NoProject,
    LocalUnverified,
    CachedDisconnected,
    FirmwareOnly,
    Matched,
    Conflict,
    UntitledDemo,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MutationPolicy {
    pub system_start: bool,
    pub wave_start: bool,
    pub calibration_write: bool,
    pub edit_variable_refs: bool,
}

impl MutationPolicy {
    pub fn for_status(status: ProjectStatus) -> Self {
        match status {
            ProjectStatus::Matched | ProjectStatus::FirmwareOnly => Self {
                system_start: true,
                wave_start: true,
                calibration_write: true,
                edit_variable_refs: true,
            },
            ProjectStatus::UntitledDemo => Self {
                system_start: true,
                wave_start: true,
                calibration_write: false,
                edit_variable_refs: true,
            },
            ProjectStatus::Conflict => Self {
                system_start: false,
                wave_start: false,
                calibration_write: false,
                edit_variable_refs: false,
            },
            ProjectStatus::NoProject
            | ProjectStatus::LocalUnverified
            | ProjectStatus::CachedDisconnected => Self {
                system_start: false,
                wave_start: false,
                calibration_write: false,
                edit_variable_refs: true,
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UnresolvedRefs {
    pub pinned: Vec<String>,
    pub watch: Vec<String>,
    pub wave: Vec<String>,
    pub trigger: Vec<String>,
}

impl UnresolvedRefs {
    pub fn count(&self) -> usize {
        self.pinned.len() + self.watch.len() + self.wave.len() + self.trigger.len()
    }
}

pub(crate) struct ProjectContext {
    pub registry: ProjectRegistry,
    pub active_name: Option<String>,
    pub local: Option<LocalProject>,
    pub unresolved: UnresolvedRefs,
    pub show_missing: bool,
    pub show_migration: bool,
    pub show_project_manager: bool,
    pub project_search: String,
}

impl ProjectContext {
    pub fn load(config: &AppConfig) -> Self {
        let active_name = config
            .last_project_name
            .clone()
            .and_then(|name| normalize_project_name(&name).ok())
            .filter(|name| name != UNTITLED_PROJECT);
        let mut registry = ProjectRegistry::load(active_name.as_deref());
        if let Some(name) = active_name.as_deref() {
            for evicted in registry.touch(name) {
                let _ = WorkspaceStore::delete(&evicted);
            }
        }
        // Persist normalization and the restored active project immediately so
        // older registries acquire a stable MRU order before any UI action.
        let _ = registry.save();
        let local = active_name
            .as_deref()
            .and_then(|name| registry.valid_local(name, false));
        Self {
            registry,
            active_name,
            local,
            unresolved: UnresolvedRefs::default(),
            show_missing: false,
            show_migration: false,
            show_project_manager: false,
            project_search: String::new(),
        }
    }

    pub fn device_name<'a>(&self, info: Option<&'a DeviceInfo>) -> Option<&'a str> {
        info.map(|info| info.project_name.as_str())
    }

    pub fn status(&self, info: Option<&DeviceInfo>) -> ProjectStatus {
        let device = self.device_name(info);
        match (device, self.local.as_ref(), self.active_name.as_deref()) {
            (Some(UNTITLED_PROJECT), None, _) => ProjectStatus::UntitledDemo,
            (Some(device), Some(local), _) if device == local.name => {
                if device == UNTITLED_PROJECT {
                    ProjectStatus::UntitledDemo
                } else {
                    ProjectStatus::Matched
                }
            }
            (Some(_), Some(_), _) => ProjectStatus::Conflict,
            (Some(_), None, _) => ProjectStatus::FirmwareOnly,
            (None, Some(_), _) => ProjectStatus::LocalUnverified,
            (None, None, Some(_)) => ProjectStatus::CachedDisconnected,
            (None, None, None) => ProjectStatus::NoProject,
        }
    }

    pub fn policy(&self, info: Option<&DeviceInfo>) -> MutationPolicy {
        MutationPolicy::for_status(self.status(info))
    }

    pub fn title(&self, info: Option<&DeviceInfo>) -> String {
        match self.status(info) {
            ProjectStatus::NoProject => "No Project".to_owned(),
            ProjectStatus::LocalUnverified => format!(
                "{} · Unverified",
                self.local.as_ref().map_or("No Project", |p| &p.name)
            ),
            ProjectStatus::CachedDisconnected => format!(
                "{} · Disconnected",
                self.active_name.as_deref().unwrap_or("No Project")
            ),
            ProjectStatus::UntitledDemo => "untitled · Non-persistent Demo".to_owned(),
            ProjectStatus::Conflict | ProjectStatus::Matched => self
                .local
                .as_ref()
                .map(|project| project.name.clone())
                .unwrap_or_else(|| "No Project".to_owned()),
            ProjectStatus::FirmwareOnly => {
                self.device_name(info).unwrap_or("No Project").to_owned()
            }
        }
    }

    pub fn can_reconcile(&self, info: Option<&DeviceInfo>) -> bool {
        matches!(
            self.status(info),
            ProjectStatus::Matched | ProjectStatus::FirmwareOnly | ProjectStatus::UntitledDemo
        )
    }

    pub fn build_mismatch(&self, info: Option<&DeviceInfo>) -> bool {
        if self.status(info) != ProjectStatus::Matched {
            return false;
        }
        let (Some(local), Some(device)) = (self.local.as_ref(), info) else {
            return false;
        };
        // The contract hash catches descriptor-layout drift, but it is unchanged
        // by a plain recompile. A zero hash means the firmware did not report one.
        let hash_differs = local
            .build_hash
            .is_some_and(|local_hash| device.build_hash != 0 && device.build_hash != local_hash);
        // The build time is the field the panel shows, so a recompile that keeps
        // the same contract must still alarm — otherwise the two displayed
        // "Built Time" values disagree with nothing flagged.
        let time_differs = local.build_time_utc.is_some_and(|local_time| {
            local_time != 0 && device.build_time_utc != 0 && local_time != device.build_time_utc
        });
        hash_differs || time_differs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "scope2000-project-test-{}-{id}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_project(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(".project"),
            format!(
                "<?xml version=\"1.0\"?><projectDescription><name>{name}</name></projectDescription>"
            ),
        )
        .unwrap();
    }

    fn write_build_report(dir: &Path, project_name: &str, build_time_utc: u32, build_hash: u32) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("v2k_user_desc_report.json"),
            format!(
                r#"{{"build_hash":{build_hash},"project_info":{{"project_name":"{project_name}","build_time_utc":{build_time_utc}}}}}"#
            ),
        )
        .unwrap();
    }

    fn device(name: &str) -> DeviceInfo {
        DeviceInfo {
            protocol_version: 6,
            contract_version: 13,
            build_hash: 1,
            descriptor_count: 0,
            firmware_name: "viewer2000".to_owned(),
            tick_hz: 20_000,
            capabilities: 0,
            project_name: name.to_owned(),
            build_time_utc: 0,
        }
    }

    fn context(local: Option<&str>, active: Option<&str>) -> ProjectContext {
        ProjectContext {
            registry: ProjectRegistry::default_with_version(),
            active_name: active.map(str::to_owned),
            local: local.map(|name| LocalProject {
                name: name.to_owned(),
                project_file: PathBuf::from("/.project"),
                build_time_utc: None,
                build_hash: None,
            }),
            unresolved: UnresolvedRefs::default(),
            show_missing: false,
            show_migration: false,
            show_project_manager: false,
            project_search: String::new(),
        }
    }

    #[test]
    fn project_state_matrix_is_explicit() {
        assert_eq!(context(None, None).status(None), ProjectStatus::NoProject);
        assert_eq!(
            context(Some("B"), Some("B")).status(None),
            ProjectStatus::LocalUnverified
        );
        assert_eq!(
            context(None, Some("A")).status(None),
            ProjectStatus::CachedDisconnected
        );
        assert_eq!(
            context(None, Some("A")).status(Some(&device("A"))),
            ProjectStatus::FirmwareOnly
        );
        assert_eq!(
            context(Some("A"), Some("A")).status(Some(&device("A"))),
            ProjectStatus::Matched
        );
        assert_eq!(
            context(Some("B"), Some("B")).status(Some(&device("A"))),
            ProjectStatus::Conflict
        );
        assert_eq!(
            context(None, None).status(Some(&device("untitled"))),
            ProjectStatus::UntitledDemo
        );
    }

    #[test]
    fn conflict_policy_is_read_only_safe() {
        let policy = MutationPolicy::for_status(ProjectStatus::Conflict);
        assert!(!policy.system_start);
        assert!(!policy.wave_start);
        assert!(!policy.calibration_write);
        assert!(!policy.edit_variable_refs);
    }

    #[test]
    fn same_name_build_difference_is_visible_without_becoming_a_conflict() {
        let mut context = context(Some("A"), Some("A"));
        context.local.as_mut().unwrap().build_hash = Some(2);
        let device = device("A");

        assert_eq!(context.status(Some(&device)), ProjectStatus::Matched);
        assert!(context.build_mismatch(Some(&device)));
        assert!(context.policy(Some(&device)).calibration_write);
    }

    #[test]
    fn unreported_firmware_hash_does_not_alarm() {
        let mut context = context(Some("A"), Some("A"));
        context.local.as_mut().unwrap().build_hash = Some(2);
        let mut device = device("A");
        device.build_hash = 0;

        assert_eq!(context.status(Some(&device)), ProjectStatus::Matched);
        assert!(!context.build_mismatch(Some(&device)));
    }

    #[test]
    fn recompile_with_same_contract_still_alarms_on_build_time() {
        let mut context = context(Some("A"), Some("A"));
        let local = context.local.as_mut().unwrap();
        local.build_hash = Some(1);
        local.build_time_utc = Some(200);
        let mut device = device("A");
        device.build_hash = 1; // identical descriptor contract
        device.build_time_utc = 100; // firmware predates the local rebuild

        assert!(context.build_mismatch(Some(&device)));
    }

    #[test]
    fn matching_hash_and_build_time_does_not_alarm() {
        let mut context = context(Some("A"), Some("A"));
        let local = context.local.as_mut().unwrap();
        local.build_hash = Some(1);
        local.build_time_utc = Some(100);
        let mut device = device("A");
        device.build_hash = 1;
        device.build_time_utc = 100;

        assert!(!context.build_mismatch(Some(&device)));
    }

    #[test]
    fn untitled_demo_allows_demo_start_but_not_calibration() {
        let policy = MutationPolicy::for_status(ProjectStatus::UntitledDemo);
        assert!(policy.system_start);
        assert!(policy.wave_start);
        assert!(!policy.calibration_write);
        assert!(policy.edit_variable_refs);
    }

    #[test]
    fn project_name_validation_matches_baker_rules() {
        assert_eq!(normalize_project_name(" demo ").unwrap(), "demo");
        assert_eq!(normalize_project_name(" ").unwrap(), UNTITLED_PROJECT);
        assert!(normalize_project_name("电机").is_err());
        assert!(normalize_project_name(&"x".repeat(33)).is_err());
    }

    #[test]
    fn recursive_scan_lists_valid_and_invalid_candidates_in_path_order() {
        let root = TempDir::new();
        write_project(&root.0.join("z-project"), "zeta");
        write_project(&root.0.join("nested/a-project"), "alpha");
        std::fs::create_dir_all(root.0.join("broken")).unwrap();
        std::fs::write(root.0.join("broken/.project"), "<not-closed>").unwrap();

        let candidates = scan_project_directory(&root.0);

        assert_eq!(candidates.len(), 3);
        assert_eq!(
            candidates[0].relative_path,
            PathBuf::from("broken/.project")
        );
        assert!(candidates[0].project.is_err());
        assert_eq!(
            candidates[1]
                .project
                .as_ref()
                .map(|project| project.name.as_str()),
            Ok("alpha")
        );
        assert_eq!(
            candidates[2]
                .project
                .as_ref()
                .map(|project| project.name.as_str()),
            Ok("zeta")
        );
    }

    #[test]
    fn scan_does_not_follow_directory_symlinks() {
        let root = TempDir::new();
        write_project(&root.0.join("real"), "real");
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.0.join("real"), root.0.join("linked")).unwrap();

        let candidates = scan_project_directory(&root.0);

        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0]
                .project
                .as_ref()
                .map(|project| project.name.as_str()),
            Ok("real")
        );
    }

    #[test]
    fn duplicate_names_remain_separate_user_choices() {
        let root = TempDir::new();
        write_project(&root.0.join("one"), "same");
        write_project(&root.0.join("two"), "same");

        let candidates = scan_project_directory(&root.0);

        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| {
            candidate
                .project
                .as_ref()
                .is_ok_and(|project| project.name == "same")
        }));
    }

    #[test]
    fn registry_only_auto_opens_verified_matching_bindings() {
        let root = TempDir::new();
        write_project(&root.0, "demo");
        let project_file = root.0.join(".project").canonicalize().unwrap();
        let mut registry = ProjectRegistry::default();
        registry.projects.insert(
            "demo".to_owned(),
            ProjectBinding {
                project_file: Some(project_file),
                verified: false,
                build_time_utc: None,
                build_hash: None,
            },
        );

        assert!(registry.valid_local("demo", true).is_none());
        assert!(registry.valid_local("demo", false).is_some());

        registry.projects.get_mut("demo").unwrap().verified = true;
        assert!(registry.valid_local("demo", true).is_some());

        write_project(&root.0, "renamed");
        assert!(registry.valid_local("demo", true).is_none());
    }

    #[test]
    fn local_project_uses_latest_matching_baker_build_time() {
        let root = TempDir::new();
        write_project(&root.0, "demo");
        write_build_report(&root.0.join("RAM"), "demo", 100, 10);
        write_build_report(&root.0.join("FLASH"), "demo", 200, 20);
        write_build_report(&root.0.join("other"), "different", 300, 30);

        let fast_project = LocalProject::load(root.0.join(".project")).unwrap();
        assert_eq!(fast_project.build_time_utc, None);

        let project = load_local_project_with_metadata(root.0.join(".project")).unwrap();

        assert_eq!(project.build_time_utc, Some(200));
        assert_eq!(project.build_hash, Some(20));
    }

    #[test]
    fn refresh_local_build_walks_and_reports_the_chosen_report_path() {
        let root = TempDir::new();
        write_project(&root.0, "demo");
        write_build_report(&root.0.join("FLASH"), "demo", 200, 20);
        let project_file = root.0.join(".project").canonicalize().unwrap();

        let scan = refresh_local_build(project_file, "demo".to_owned(), None).unwrap();

        assert_eq!(scan.project.build_time_utc, Some(200));
        assert_eq!(scan.project.build_hash, Some(20));
        assert!(
            scan.report_path
                .as_ref()
                .unwrap()
                .ends_with("FLASH/v2k_user_desc_report.json")
        );
    }

    #[test]
    fn refresh_local_build_fast_path_rereads_cached_report() {
        let root = TempDir::new();
        write_project(&root.0, "demo");
        let report_dir = root.0.join("FLASH");
        write_build_report(&report_dir, "demo", 200, 20);
        let report_path = report_dir.join("v2k_user_desc_report.json");
        let project_file = root.0.join(".project").canonicalize().unwrap();

        // A rebuild rewrites the same report in place; the cached path must pick
        // the new build up directly.
        write_build_report(&report_dir, "demo", 300, 30);
        let scan =
            refresh_local_build(project_file, "demo".to_owned(), Some(report_path.clone())).unwrap();

        assert_eq!(scan.project.build_time_utc, Some(300));
        assert_eq!(scan.project.build_hash, Some(30));
        assert_eq!(scan.report_path, Some(report_path));
    }

    #[test]
    fn refresh_local_build_rejects_cached_report_from_another_project() {
        let root = TempDir::new();
        write_project(&root.0, "demo");
        write_build_report(&root.0.join("FLASH"), "demo", 200, 20);
        let project_file = root.0.join(".project").canonicalize().unwrap();

        let other = TempDir::new();
        write_build_report(&other.0, "other", 999, 99);
        let stale = other.0.join("v2k_user_desc_report.json");

        let scan = refresh_local_build(project_file, "demo".to_owned(), Some(stale)).unwrap();

        // The stale report belongs to "other", so the walk finds demo's instead.
        assert_eq!(scan.project.build_time_utc, Some(200));
        assert_eq!(scan.project.build_hash, Some(20));
    }

    #[test]
    fn recent_project_cache_is_mru_and_capped_at_one_hundred() {
        let mut registry = ProjectRegistry::default();
        let mut evicted = Vec::new();
        for index in 0..105 {
            evicted.extend(registry.touch(&format!("project-{index}")));
        }

        assert_eq!(registry.recent_projects.len(), 100);
        assert_eq!(registry.recent_projects.first().unwrap(), "project-104");
        assert_eq!(registry.recent_projects.last().unwrap(), "project-5");
        assert_eq!(
            evicted,
            (0..5).map(|i| format!("project-{i}")).collect::<Vec<_>>()
        );
        assert!(!registry.projects.contains_key("project-0"));

        registry.touch("project-50");
        assert_eq!(registry.recent_projects.first().unwrap(), "project-50");
        assert_eq!(registry.recent_projects.len(), 100);
    }

    #[test]
    fn registry_reads_previous_required_path_binding_shape() {
        let text = r#"
format_version = 1

[projects.demo]
project_file = "/tmp/demo/.project"
verified = true
"#;

        let registry: ProjectRegistry = toml::from_str(text).unwrap();

        assert_eq!(
            registry.projects["demo"].project_file.as_deref(),
            Some(Path::new("/tmp/demo/.project"))
        );
        assert!(registry.projects["demo"].verified);
    }

    #[test]
    fn registry_serializes_recent_projects_at_the_root() {
        let mut registry = ProjectRegistry::default();
        registry.touch("demo");

        let text = toml::to_string_pretty(&registry).unwrap();

        assert!(text.contains("recent_projects = [\"demo\"]"), "{text}");
    }

    #[test]
    fn workspace_path_uses_project_name_bytes_not_build_hash() {
        let path = WorkspaceStore::workspace_path("phase4-demo").unwrap();
        assert!(path.ends_with("7068617365342d64656d6f/workspace.toml"));
    }
}
