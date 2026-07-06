#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReleaseUpdate {
    pub version: String,
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum UpdateCheckStatus {
    #[default]
    Idle,
    Checking,
    UpToDate,
    UpdateAvailable(ReleaseUpdate),
    Failed,
}

#[derive(Debug, Default)]
pub(crate) struct UpdateCheckState {
    pub status: UpdateCheckStatus,
    pub requested: bool,
}
