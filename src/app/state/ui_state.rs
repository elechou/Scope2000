pub(crate) struct UiState {
    pub source_filter: String,
    pub stop_warning_action: Option<&'static str>,
    pub show_device_panel: bool,
    pub show_console_panel: bool,
    pub show_selection_panel: bool,
    pub show_about_window: bool,
    pub varmap_split: f32,
    pub data_panel_width: Option<f32>,
    pub selection_panel_width: Option<f32>,
    pub console_height: Option<f32>,
    pub apply_panel_sizes: bool,
}

pub(crate) const VARMAP_SPLIT_DEFAULT: f32 = 0.25;

impl Default for UiState {
    fn default() -> Self {
        Self {
            source_filter: String::new(),
            stop_warning_action: None,
            show_device_panel: true,
            show_console_panel: false,
            show_selection_panel: true,
            show_about_window: false,
            varmap_split: VARMAP_SPLIT_DEFAULT,
            data_panel_width: None,
            selection_panel_width: None,
            console_height: None,
            apply_panel_sizes: false,
        }
    }
}
