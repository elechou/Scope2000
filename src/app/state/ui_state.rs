pub(crate) struct UiState {
    pub source_filter: String,
    pub stop_warning_action: Option<&'static str>,
    pub show_project_switch_warning: bool,
    pub show_system_panel: bool,
    pub show_console_panel: bool,
    pub show_selection_panel: bool,
    pub show_connection_settings: bool,
    pub show_device_info_window: bool,
    pub show_abz_zeroing: bool,
    pub show_current_sensor_calibration: bool,
    pub show_about_window: bool,
    pub varmap_split: f32,
    pub varmap_continuous_refresh: bool,
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
            show_project_switch_warning: false,
            show_system_panel: true,
            show_console_panel: false,
            show_selection_panel: true,
            show_connection_settings: false,
            show_device_info_window: false,
            show_abz_zeroing: false,
            show_current_sensor_calibration: false,
            show_about_window: false,
            varmap_split: VARMAP_SPLIT_DEFAULT,
            varmap_continuous_refresh: false,
            data_panel_width: None,
            selection_panel_width: None,
            console_height: None,
            apply_panel_sizes: false,
        }
    }
}
