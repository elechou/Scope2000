pub mod panel;

use std::collections::VecDeque;

pub const MAX_INFO_ENTRIES: usize = 500;

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

/// A status-bar message promoted from the console log (Notice/Warn/Error).
pub struct StatusMessage {
    pub level: LogLevel,
    pub text: String,
    pub timestamp: std::time::Instant,
}

/// Central log buffer: console entries + promoted status-bar message.
pub struct LogBuffer {
    pub logs: VecDeque<LogEntry>,
    /// Minimum severity shown in the console panel.
    pub log_min_level: LogLevel,
    /// Latest Notice/Warn/Error message, displayed in the status bar.
    pub status_message: Option<StatusMessage>,
    info_entries: usize,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self {
            logs: VecDeque::new(),
            log_min_level: LogLevel::Info,
            status_message: None,
            info_entries: 0,
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
        if level == LogLevel::Info {
            self.info_entries += 1;
        }
        self.logs.push_back(LogEntry {
            time,
            level,
            message,
        });
        self.trim_info_entries();
    }

    pub fn clear(&mut self) {
        self.logs.clear();
        self.info_entries = 0;
    }

    pub fn visible_entry_count(&self, min_level: LogLevel) -> usize {
        self.visible_entries(min_level).count()
    }

    pub fn visible_entries(&self, min_level: LogLevel) -> impl Iterator<Item = &LogEntry> {
        self.logs
            .iter()
            .filter(move |entry| entry.level >= min_level)
    }

    fn trim_info_entries(&mut self) {
        while self.info_entries > MAX_INFO_ENTRIES {
            let Some(index) = self
                .logs
                .iter()
                .position(|entry| entry.level == LogLevel::Info)
            else {
                self.info_entries = 0;
                return;
            };
            self.logs.remove(index);
            self.info_entries -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_entries_are_trimmed_to_the_latest_500() {
        let mut log = LogBuffer::default();

        for index in 0..600 {
            log.push(LogLevel::Info, format!("entry {index}"));
        }

        assert_eq!(log.logs.len(), MAX_INFO_ENTRIES);
        assert_eq!(
            log.logs.front().map(|entry| entry.message.as_str()),
            Some("entry 100")
        );
        assert_eq!(
            log.logs.back().map(|entry| entry.message.as_str()),
            Some("entry 599")
        );
    }

    #[test]
    fn warn_and_error_entries_are_not_trimmed() {
        let mut log = LogBuffer::default();

        for index in 0..600 {
            log.push(LogLevel::Warn, format!("warn {index}"));
            log.push(LogLevel::Error, format!("error {index}"));
        }

        assert_eq!(log.logs.len(), 1200);
        assert_eq!(
            log.logs.front().map(|entry| entry.message.as_str()),
            Some("warn 0")
        );
        assert_eq!(
            log.logs.back().map(|entry| entry.message.as_str()),
            Some("error 599")
        );
    }

    #[test]
    fn info_trim_preserves_warning_entries() {
        let mut log = LogBuffer::default();

        log.push(LogLevel::Warn, "first warning".to_owned());
        for index in 0..600 {
            log.push(LogLevel::Info, format!("entry {index}"));
        }

        assert_eq!(log.logs.len(), MAX_INFO_ENTRIES + 1);
        assert_eq!(
            log.logs.front().map(|entry| entry.message.as_str()),
            Some("first warning")
        );
        assert_eq!(
            log.visible_entry_count(LogLevel::Info),
            MAX_INFO_ENTRIES + 1
        );
        assert_eq!(log.visible_entry_count(LogLevel::Warn), 1);
    }
}
