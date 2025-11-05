//! Centralized game logging
//!
//! This module provides a centralized logger for game events, controller decisions,
//! and other gameplay information. The logger internally tracks verbosity level
//! and can be accessed by different parts of the game (controllers, game loop, etc.)
//!
//! The logger supports multiple output modes:
//! - Text output (human-readable, to stdout)
//! - JSON output (machine-readable, one JSON object per line)
//! - In-memory capture (for programmatic access in tests)

use crate::game::VerbosityLevel;
use serde::{Deserialize, Serialize};
use std::cell::{Ref, RefCell};

/// Output format for log messages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OutputFormat {
    /// Human-readable text output (default)
    #[default]
    Text,
    /// Machine-readable JSON output (one object per line)
    Json,
}

/// A structured log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Verbosity level of this log entry
    pub level: VerbosityLevel,
    /// Log message
    pub message: String,
    /// Optional category (e.g., "controller_choice", "game_event")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Optional metadata (e.g., controller name, card name)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Centralized logger for game events
///
/// This logger is stored in GameState and can be accessed via GameStateView.
/// It internally tracks the verbosity level and provides methods for logging
/// at different verbosity levels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameLogger {
    verbosity: VerbosityLevel,
    /// Track if we've printed the step header (for lazy printing)
    #[serde(skip)]
    step_header_printed: bool,
    /// Enable numeric-only choice format (for Java Forge comparison)
    numeric_choices: bool,
    /// Output format (Text or JSON)
    output_format: OutputFormat,
    /// In-memory log buffer (when capture is enabled)
    /// Using RefCell for interior mutability to avoid requiring &mut self
    #[serde(skip)]
    log_buffer: RefCell<Vec<LogEntry>>,
    /// Whether to capture logs in memory
    capture_logs: bool,
}

impl GameLogger {
    /// Create a new logger with default verbosity (Normal)
    pub fn new() -> Self {
        GameLogger {
            verbosity: VerbosityLevel::default(),
            step_header_printed: false,
            numeric_choices: false,
            output_format: OutputFormat::default(),
            log_buffer: RefCell::new(Vec::new()),
            capture_logs: false,
        }
    }

    /// Create a logger with specified verbosity
    pub fn with_verbosity(verbosity: VerbosityLevel) -> Self {
        GameLogger {
            verbosity,
            step_header_printed: false,
            numeric_choices: false,
            output_format: OutputFormat::default(),
            log_buffer: RefCell::new(Vec::new()),
            capture_logs: false,
        }
    }

    /// Enable log capture to in-memory buffer
    pub fn enable_capture(&mut self) {
        self.capture_logs = true;
    }

    /// Disable log capture
    pub fn disable_capture(&mut self) {
        self.capture_logs = false;
    }

    /// Check if log capture is enabled
    pub fn is_capturing(&self) -> bool {
        self.capture_logs
    }

    /// Get a reference to captured log entries (no copy)
    ///
    /// Returns a guard that dereferences to a slice of log entries. This provides
    /// read-only access to the log buffer without cloning. Clients can iterate,
    /// filter, and inspect logs without allocation.
    ///
    /// # Example
    /// ```ignore
    /// let logs = logger.logs();
    /// for log in logs.iter() {
    ///     if log.message.contains("attack") {
    ///         println!("{}", log.message);
    ///     }
    /// }
    ///
    /// // Or count matching logs without copying:
    /// let attack_count = logger.logs().iter()
    ///     .filter(|log| log.message.contains("attack"))
    ///     .count();
    /// ```
    pub fn logs(&self) -> Ref<'_, [LogEntry]> {
        Ref::map(self.log_buffer.borrow(), |v| v.as_slice())
    }

    /// Get captured log entries (clones the buffer)
    ///
    /// Deprecated: Use `logs()` instead to avoid unnecessary copying.
    /// This method is kept for backward compatibility.
    pub fn get_logs(&self) -> Vec<LogEntry> {
        self.log_buffer.borrow().clone()
    }

    /// Clear the log buffer
    pub fn clear_logs(&mut self) {
        self.log_buffer.borrow_mut().clear();
    }

    /// Set output format (Text or JSON)
    pub fn set_output_format(&mut self, format: OutputFormat) {
        self.output_format = format;
    }

    /// Get current output format
    pub fn output_format(&self) -> OutputFormat {
        self.output_format
    }

    /// Enable numeric-only choice logging
    pub fn set_numeric_choices(&mut self, enabled: bool) {
        self.numeric_choices = enabled;
    }

    /// Check if numeric choices mode is enabled
    pub fn numeric_choices_enabled(&self) -> bool {
        self.numeric_choices
    }

    /// Get current verbosity level
    pub fn verbosity(&self) -> VerbosityLevel {
        self.verbosity
    }

    /// Set verbosity level
    pub fn set_verbosity(&mut self, verbosity: VerbosityLevel) {
        self.verbosity = verbosity;
    }

    /// Reset the step header flag (called when starting a new step)
    pub fn reset_step_header(&mut self) {
        self.step_header_printed = false;
    }

    /// Mark that step header has been printed
    pub fn mark_step_header_printed(&mut self) {
        self.step_header_printed = true;
    }

    /// Check if step header has been printed
    pub fn step_header_printed(&self) -> bool {
        self.step_header_printed
    }

    /// Internal method to log an entry
    fn log_entry(&self, entry: LogEntry) {
        // Capture to buffer if enabled
        if self.capture_logs {
            self.log_buffer.borrow_mut().push(entry.clone());
        }

        // Output based on format and verbosity
        if entry.level <= self.verbosity {
            match self.output_format {
                OutputFormat::Text => {
                    // Text output with indentation
                    let indent = if entry.level == VerbosityLevel::Minimal {
                        ""
                    } else {
                        "  "
                    };
                    println!("{}{}", indent, entry.message);
                }
                OutputFormat::Json => {
                    // JSON output (one object per line)
                    if let Ok(json) = serde_json::to_string(&entry) {
                        println!("{}", json);
                    }
                }
            }
        }
    }

    /// Log at Silent level (always suppressed - this is just for API consistency)
    #[inline]
    pub fn silent(&self, _message: &str) {
        // Silent messages are never printed or captured
    }

    /// Log at Minimal level (game outcomes, major events)
    #[inline]
    pub fn minimal(&self, message: &str) {
        // Early exit if message won't be used
        if VerbosityLevel::Minimal > self.verbosity && !self.capture_logs {
            return;
        }

        // Fast path: only stdout logging, use temp buffer
        if !self.capture_logs && VerbosityLevel::Minimal <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Minimal, message);
            return;
        }

        // Slow path: need to capture or complex logic
        let entry = LogEntry {
            level: VerbosityLevel::Minimal,
            message: message.to_string(),
            category: None,
            metadata: None,
        };
        self.log_entry(entry);
    }

    /// Log at Normal level (turns, steps, key actions)
    #[inline]
    pub fn normal(&self, message: &str) {
        // Early exit if message won't be used
        if VerbosityLevel::Normal > self.verbosity && !self.capture_logs {
            return;
        }

        // Fast path: only stdout logging, use temp buffer
        if !self.capture_logs && VerbosityLevel::Normal <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Normal, message);
            return;
        }

        // Slow path: need to capture or complex logic
        let entry = LogEntry {
            level: VerbosityLevel::Normal,
            message: message.to_string(),
            category: None,
            metadata: None,
        };
        self.log_entry(entry);
    }

    /// Log at Verbose level (all actions and state changes)
    #[inline]
    pub fn verbose(&self, message: &str) {
        // Early exit if message won't be used
        if VerbosityLevel::Verbose > self.verbosity && !self.capture_logs {
            return;
        }

        // Fast path: only stdout logging, use temp buffer
        if !self.capture_logs && VerbosityLevel::Verbose <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Verbose, message);
            return;
        }

        // Slow path: need to capture or complex logic
        let entry = LogEntry {
            level: VerbosityLevel::Verbose,
            message: message.to_string(),
            category: None,
            metadata: None,
        };
        self.log_entry(entry);
    }

    /// Fast path for stdout logging without allocation
    #[inline]
    fn log_to_stdout(&self, level: VerbosityLevel, message: &str) {
        // Write indent and message directly to stdout, relying on stdout buffering
        if level == VerbosityLevel::Minimal {
            println!("{}", message);
        } else {
            println!("  {}", message);
        }
    }

    /// Log a controller decision at Normal level
    ///
    /// This is a convenience method for logging AI/controller choices.
    /// If numeric_choices mode is enabled, choices are always logged regardless of verbosity.
    #[inline]
    pub fn controller_choice(&self, controller_name: &str, message: &str) {
        // Controller choices are always logged if numeric_choices is enabled
        let should_log = self.numeric_choices || self.verbosity >= VerbosityLevel::Normal;

        if should_log || self.capture_logs {
            let mut metadata = serde_json::Map::new();
            metadata.insert(
                "controller".to_string(),
                serde_json::Value::String(controller_name.to_string()),
            );

            let entry = LogEntry {
                level: VerbosityLevel::Normal,
                message: format!(">>> {controller_name}: {message}"),
                category: Some("controller_choice".to_string()),
                metadata: Some(serde_json::Value::Object(metadata)),
            };

            // Capture if enabled
            if self.capture_logs {
                self.log_buffer.borrow_mut().push(entry.clone());
            }

            // Output if should_log
            if should_log && entry.level <= self.verbosity {
                match self.output_format {
                    OutputFormat::Text => {
                        println!("  >>> {controller_name}: {message}");
                    }
                    OutputFormat::Json => {
                        if let Ok(json) = serde_json::to_string(&entry) {
                            println!("{}", json);
                        }
                    }
                }
            }
        }
    }
}

impl Default for GameLogger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logger_creation() {
        let logger = GameLogger::new();
        assert_eq!(logger.verbosity(), VerbosityLevel::Normal);
    }

    #[test]
    fn test_logger_with_verbosity() {
        let logger = GameLogger::with_verbosity(VerbosityLevel::Silent);
        assert_eq!(logger.verbosity(), VerbosityLevel::Silent);
    }

    #[test]
    fn test_set_verbosity() {
        let mut logger = GameLogger::new();
        logger.set_verbosity(VerbosityLevel::Verbose);
        assert_eq!(logger.verbosity(), VerbosityLevel::Verbose);
    }

    #[test]
    fn test_step_header_tracking() {
        let mut logger = GameLogger::new();
        assert!(!logger.step_header_printed());

        logger.mark_step_header_printed();
        assert!(logger.step_header_printed());

        logger.reset_step_header();
        assert!(!logger.step_header_printed());
    }
}
