pub mod panel;

/// Severity level for console log entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Notice,
    Warn,
    Error,
}

impl LogLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Debug => "DBG",
            Self::Info => "INF",
            Self::Notice => "NTC",
            Self::Warn => "WRN",
            Self::Error => "ERR",
        }
    }
}

/// A single console log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Wall-clock timestamp formatted as "YYYY-MM-DD, HH:MM:SS".
    pub time: String,
    pub level: LogLevel,
    pub message: String,
}

/// A status-bar message promoted from the console log (Warn/Error level).
pub struct StatusMessage {
    pub level: LogLevel,
    pub text: String,
    pub timestamp: std::time::Instant,
}

/// Central log buffer: console entries + promoted status-bar message.
pub struct LogBuffer {
    pub logs: Vec<LogEntry>,
    /// Minimum severity shown in the console panel.
    pub log_min_level: LogLevel,
    /// Latest Warn/Error message, displayed in the status bar.
    pub status_message: Option<StatusMessage>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self {
            logs: Vec::new(),
            log_min_level: LogLevel::Info,
            status_message: None,
        }
    }
}

impl LogBuffer {
    pub fn push(&mut self, level: LogLevel, message: String) {
        let time = chrono::Local::now()
            .format("%Y-%m-%d, %H:%M:%S")
            .to_string();
        // Promote Notice/Warn/Error to the status bar
        if level >= LogLevel::Notice {
            self.status_message = Some(StatusMessage {
                level,
                text: message.clone(),
                timestamp: std::time::Instant::now(),
            });
        }
        self.logs.push(LogEntry {
            time,
            level,
            message,
        });
        if self.logs.len() > 500 {
            self.logs.drain(..self.logs.len() - 500);
        }
    }
}
