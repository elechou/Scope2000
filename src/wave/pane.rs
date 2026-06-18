use eframe::egui;
use serde::{Deserialize, Serialize};

/// The type of view a pane displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaneKind {
    TimeSeries,
    Dataframe,
}

impl PaneKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::TimeSeries => "Time Series",
            Self::Dataframe => "Dataframe",
        }
    }

    /// SVG icon URI (for egui::Image contexts).
    pub fn icon_uri(&self) -> &'static str {
        match self {
            Self::TimeSeries => crate::theme::ICON_TIMESERIES,
            Self::Dataframe => crate::theme::ICON_DATAFRAME,
        }
    }
}

/// Legend position corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LegendCorner {
    LeftTop,
    #[default]
    RightTop,
    LeftBottom,
    RightBottom,
}

impl LegendCorner {
    pub const ALL: &[Self] = &[
        Self::LeftTop,
        Self::RightTop,
        Self::LeftBottom,
        Self::RightBottom,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::LeftTop => "Left Top",
            Self::RightTop => "Right Top",
            Self::LeftBottom => "Left Bottom",
            Self::RightBottom => "Right Bottom",
        }
    }
}

/// Axis range: auto-fit or manually specified bounds.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum AxisRange {
    #[default]
    Auto,
    Manual {
        min: f64,
        max: f64,
    },
}

/// Time axis coordinate mode for TimeSeries panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimeAxisMode {
    #[default]
    System,
    TriggerRelative,
}

impl TimeAxisMode {
    pub const ALL: &[Self] = &[Self::System, Self::TriggerRelative];

    pub fn label(&self) -> &'static str {
        match self {
            Self::System => "System time",
            Self::TriggerRelative => "Trigger = 0",
        }
    }
}

/// Serde adapter for egui::Color32 — serializes as "#RRGGBB".
mod color32_serde {
    use eframe::egui;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(color: &egui::Color32, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let [r, g, b, _] = color.to_array();
        serializer.serialize_str(&format!("#{r:02X}{g:02X}{b:02X}"))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<egui::Color32, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.trim_start_matches('#');
        if s.len() < 6 {
            return Err(serde::de::Error::custom("color string too short"));
        }
        let r = u8::from_str_radix(&s[0..2], 16).map_err(serde::de::Error::custom)?;
        let g = u8::from_str_radix(&s[2..4], 16).map_err(serde::de::Error::custom)?;
        let b = u8::from_str_radix(&s[4..6], 16).map_err(serde::de::Error::custom)?;
        Ok(egui::Color32::from_rgb(r, g, b))
    }
}

/// Configuration for a single series in a time-series plot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesConfig {
    pub var_name: String,
    #[serde(with = "color32_serde")]
    pub color: egui::Color32,
    pub visible: bool,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default = "default_line_width")]
    pub width: f32,
}

fn default_line_width() -> f32 {
    1.5
}

impl SeriesConfig {
    pub fn new(var_name: String, color: egui::Color32) -> Self {
        Self {
            var_name,
            color,
            visible: true,
            display_name: None,
            width: 1.5,
        }
    }

    /// Returns the display name, falling back to var_name.
    pub fn label(&self) -> &str {
        self.display_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.var_name)
    }
}

/// View-level properties for a TimeSeries pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewProperties {
    #[serde(with = "color32_serde")]
    pub background: egui::Color32,
    pub show_grid: bool,
    pub legend_visible: bool,
    pub legend_corner: LegendCorner,
    pub time_axis_mode: TimeAxisMode,
    pub time_axis_range: AxisRange,
    pub scalar_axis_range: AxisRange,
    /// Last known auto-fit bounds from the plot (updated every frame, not persisted).
    #[serde(skip)]
    pub last_bounds_x: (f64, f64),
    #[serde(skip)]
    pub last_bounds_y: (f64, f64),
    /// One-shot flag: push our `AxisRange` state into egui_plot's PlotMemory on
    /// the next frame. Set by Selection-panel edits, and defaults to `true` so
    /// freshly created or deserialized panes apply their saved Manual bounds
    /// on the first frame. Otherwise the plot's own interaction (drag/zoom/
    /// double-click) drives state, which we mirror back into the AxisRange.
    #[serde(skip)]
    pub axis_apply_pending: bool,
}

impl Default for ViewProperties {
    fn default() -> Self {
        Self {
            background: egui::Color32::from_rgb(0, 0, 0),
            show_grid: true,
            legend_visible: true,
            legend_corner: LegendCorner::default(),
            time_axis_mode: TimeAxisMode::default(),
            time_axis_range: AxisRange::Auto,
            scalar_axis_range: AxisRange::Auto,
            last_bounds_x: (0.0, 1.0),
            last_bounds_y: (0.0, 1.0),
            axis_apply_pending: true,
        }
    }
}

/// A view pane that lives inside egui_tiles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewPane {
    pub name: String,
    pub kind: PaneKind,
    /// Series configurations (used by TimeSeries panes).
    pub series: Vec<SeriesConfig>,
    #[serde(default)]
    pub properties: ViewProperties,
}

impl ViewPane {
    pub fn new(name: impl Into<String>, kind: PaneKind) -> Self {
        Self {
            name: name.into(),
            kind,
            series: Vec::new(),
            properties: ViewProperties::default(),
        }
    }

    pub fn add_series(&mut self, var_name: String, color: egui::Color32) {
        self.series.push(SeriesConfig::new(var_name, color));
    }
}

/// Default color palette for plot series.
const COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(255, 107, 107), // red
    egui::Color32::from_rgb(78, 205, 196),  // teal
    egui::Color32::from_rgb(255, 230, 109), // yellow
    egui::Color32::from_rgb(107, 185, 240), // blue
    egui::Color32::from_rgb(200, 150, 255), // purple
    egui::Color32::from_rgb(255, 180, 107), // orange
    egui::Color32::from_rgb(150, 255, 150), // green
    egui::Color32::from_rgb(255, 150, 200), // pink
];

pub fn default_color(index: usize) -> egui::Color32 {
    COLORS[index % COLORS.len()]
}
