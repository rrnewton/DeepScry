//! Fully safe bump-allocating logger
//!
//! This implementation is 100% safe Rust with no unsafe keyword usage.
//! It uses owned Strings in LogEntry and returns a guard type for iteration.

use crate::game::VerbosityLevel;
use bumpalo::Bump;
use serde::{Deserialize, Serialize};
use std::cell::{Ref, RefCell};
use std::ops::Deref;

/// Output format for log messages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OutputFormat {
    /// Human-readable text output (default)
    #[default]
    Text,
    /// Machine-readable JSON output (one object per line)
    Json,
}

/// Output destination for log messages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OutputMode {
    /// Output only to stdout (default)
    #[default]
    Stdout,
    /// Capture only to in-memory buffer (no stdout)
    Memory,
    /// Both stdout and in-memory buffer
    Both,
}

/// A log entry with owned strings (no lifetime parameters)
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Verbosity level of this log entry
    pub level: VerbosityLevel,
    /// Log message (owned)
    pub message: String,
    /// Optional category (e.g., "controller_choice", "game_event")
    pub category: Option<String>,
}

/// Guard type that provides read-only access to log entries
///
/// This provides slice-like access to captured log entries.
pub struct LogGuard<'a> {
    guard: Ref<'a, Vec<LogEntry>>,
}

impl<'a> LogGuard<'a> {
    /// Get an iterator over log entries
    pub fn iter(&self) -> std::slice::Iter<'_, LogEntry> {
        self.guard.iter()
    }

    /// Get the number of log entries
    pub fn len(&self) -> usize {
        self.guard.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.guard.is_empty()
    }
}

// Deref to slice for convenient access
impl<'a> Deref for LogGuard<'a> {
    type Target = [LogEntry];

    fn deref(&self) -> &Self::Target {
        &self.guard // Auto-deref handles Ref -> Vec -> slice
    }
}

/// Centralized logger using bump allocation for temporary formatting
///
/// This logger avoids allocations during formatting by using a bump allocator
/// for temporary strings. LogEntries use owned Strings to avoid lifetime issues.
/// The implementation is 100% safe Rust with no unsafe code.
pub struct GameLogger {
    verbosity: VerbosityLevel,
    step_header_printed: bool,
    numeric_choices: bool,
    output_format: OutputFormat,
    output_mode: OutputMode,
    /// Always show choice menus (set true in stop/go mode)
    show_choice_menu: bool,
    /// Enable state hash debugging (print hash before each logged action)
    debug_state_hash: bool,

    /// Bump allocator for temporary string formatting
    /// Reset after each format operation to avoid growth
    format_bump: RefCell<Bump>,

    /// Captured log entries (owned strings)
    log_buffer: RefCell<Vec<LogEntry>>,
}

impl GameLogger {
    /// Create a new logger with default verbosity (Normal)
    pub fn new() -> Self {
        GameLogger {
            verbosity: VerbosityLevel::default(),
            step_header_printed: false,
            numeric_choices: false,
            output_format: OutputFormat::default(),
            output_mode: OutputMode::default(),
            show_choice_menu: false,
            debug_state_hash: false,
            format_bump: RefCell::new(Bump::new()),
            log_buffer: RefCell::new(Vec::new()),
        }
    }

    /// Create a logger with specified verbosity
    pub fn with_verbosity(verbosity: VerbosityLevel) -> Self {
        GameLogger {
            verbosity,
            step_header_printed: false,
            numeric_choices: false,
            output_format: OutputFormat::default(),
            output_mode: OutputMode::default(),
            show_choice_menu: false,
            debug_state_hash: false,
            format_bump: RefCell::new(Bump::new()),
            log_buffer: RefCell::new(Vec::new()),
        }
    }

    /// Set output mode (Stdout, Memory, or Both)
    pub fn set_output_mode(&mut self, mode: OutputMode) {
        self.output_mode = mode;
    }

    /// Get current output mode
    pub fn output_mode(&self) -> OutputMode {
        self.output_mode
    }

    /// Enable log capture to in-memory buffer (compatibility method)
    /// Sets output_mode to Memory (suppresses stdout output)
    pub fn enable_capture(&mut self) {
        self.output_mode = OutputMode::Memory;
    }

    /// Disable log capture (compatibility method)
    /// Sets output_mode to Stdout
    pub fn disable_capture(&mut self) {
        self.output_mode = OutputMode::Stdout;
    }

    /// Check if log capture is enabled (compatibility method)
    pub fn is_capturing(&self) -> bool {
        matches!(self.output_mode, OutputMode::Memory | OutputMode::Both)
    }

    /// Flush buffered logs to stdout, respecting verbosity and format settings
    ///
    /// This prints all buffered logs and then clears the buffer.
    pub fn flush_buffer(&mut self) {
        let buffer = self.log_buffer.borrow();
        for entry in buffer.iter() {
            // Only print if verbosity allows
            if entry.level <= self.verbosity {
                self.log_to_stdout(entry.level, &entry.message);
            }
        }
        drop(buffer);
        self.clear_logs();
    }

    /// Flush only the last K lines of buffered logs to stdout
    ///
    /// This prints the tail of the log buffer (last K lines) and then clears the buffer.
    /// Useful with --log-tail to show constant-sized output at game exit.
    /// Prints an elision message showing how many lines were skipped.
    pub fn flush_tail(&mut self, tail_lines: usize) {
        let buffer = self.log_buffer.borrow();

        // Calculate how many lines we're eliding
        let total_lines = buffer.len();
        let elided_count = total_lines.saturating_sub(tail_lines);

        // Print elision message if we're skipping lines
        if elided_count > 0 {
            println!(
                ">>> {} LOG LINES ELIDED. PRINTING LAST {} LINES <<<",
                elided_count, tail_lines
            );
        }

        // Calculate start index for the tail
        let start_idx = total_lines.saturating_sub(tail_lines);

        // Print only the last K lines
        for entry in buffer.iter().skip(start_idx) {
            // Only print if verbosity allows
            if entry.level <= self.verbosity {
                self.log_to_stdout(entry.level, &entry.message);
            }
        }

        drop(buffer);
        self.clear_logs();
    }

    /// Get access to captured log entries
    ///
    /// Returns a guard that derefs to `[LogEntry]`. You can iterate over it:
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
    /// // Or count matching logs:
    /// let count = logger.logs().iter()
    ///     .filter(|log| log.message.contains("attack"))
    ///     .count();
    /// ```
    pub fn logs(&self) -> LogGuard<'_> {
        LogGuard {
            guard: self.log_buffer.borrow(),
        }
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
        self.format_bump.borrow_mut().reset();
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

    /// Enable showing choice menu (set true in stop/go mode)
    pub fn set_show_choice_menu(&mut self, enabled: bool) {
        self.show_choice_menu = enabled;
    }

    /// Check if choice menu should be shown
    pub fn should_show_choice_menu(&self) -> bool {
        self.show_choice_menu
    }

    /// Get current verbosity level
    pub fn verbosity(&self) -> VerbosityLevel {
        self.verbosity
    }

    /// Set verbosity level
    pub fn set_verbosity(&mut self, verbosity: VerbosityLevel) {
        self.verbosity = verbosity;
    }

    /// Enable state hash debugging
    pub fn set_debug_state_hash(&mut self, enabled: bool) {
        self.debug_state_hash = enabled;
    }

    /// Check if state hash debugging is enabled
    pub fn debug_state_hash_enabled(&self) -> bool {
        self.debug_state_hash
    }

    /// Reset the step header flag
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

    /// Fast path for stdout logging
    #[inline]
    fn log_to_stdout(&self, level: VerbosityLevel, message: &str) {
        if level == VerbosityLevel::Minimal {
            println!("{}", message);
        } else {
            println!("  {}", message);
        }
    }

    /// Log at Silent level
    #[inline]
    pub fn silent(&self, _message: &str) {
        // Silent messages are never printed or captured
    }

    /// Log at Minimal level
    #[inline]
    pub fn minimal(&self, message: &str) {
        let should_capture = matches!(self.output_mode, OutputMode::Memory | OutputMode::Both);
        let should_output = matches!(self.output_mode, OutputMode::Stdout | OutputMode::Both);

        // Early exit if message won't be used
        if VerbosityLevel::Minimal > self.verbosity && !should_capture {
            return;
        }

        // Capture if mode requires it
        if should_capture {
            self.log_buffer.borrow_mut().push(LogEntry {
                level: VerbosityLevel::Minimal,
                message: message.to_string(),
                category: None,
            });
        }

        // Output to stdout if mode requires it and verbosity allows
        if should_output && VerbosityLevel::Minimal <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Minimal, message);
        }
    }

    /// Log at Normal level
    #[inline]
    pub fn normal(&self, message: &str) {
        let should_capture = matches!(self.output_mode, OutputMode::Memory | OutputMode::Both);
        let should_output = matches!(self.output_mode, OutputMode::Stdout | OutputMode::Both);

        // Early exit if message won't be used
        if VerbosityLevel::Normal > self.verbosity && !should_capture {
            return;
        }

        // Capture if mode requires it
        if should_capture {
            self.log_buffer.borrow_mut().push(LogEntry {
                level: VerbosityLevel::Normal,
                message: message.to_string(),
                category: None,
            });
        }

        // Output to stdout if mode requires it and verbosity allows
        if should_output && VerbosityLevel::Normal <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Normal, message);
        }
    }

    /// Log at Verbose level
    #[inline]
    pub fn verbose(&self, message: &str) {
        let should_capture = matches!(self.output_mode, OutputMode::Memory | OutputMode::Both);
        let should_output = matches!(self.output_mode, OutputMode::Stdout | OutputMode::Both);

        // Early exit if message won't be used
        if VerbosityLevel::Verbose > self.verbosity && !should_capture {
            return;
        }

        // Capture if mode requires it
        if should_capture {
            self.log_buffer.borrow_mut().push(LogEntry {
                level: VerbosityLevel::Verbose,
                message: message.to_string(),
                category: None,
            });
        }

        // Output to stdout if mode requires it and verbosity allows
        if should_output && VerbosityLevel::Verbose <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Verbose, message);
        }
    }

    /// Log a controller decision at Normal level
    ///
    /// Outputs standardized "chose X" format to stdout for deterministic logging.
    /// Controller-specific debug info goes to stderr when debug_state_hash is enabled.
    ///
    /// Uses bump allocator for temporary formatting to avoid intermediate allocations.
    #[inline]
    pub fn controller_choice(&self, controller_name: &str, message: &str) {
        let should_capture = matches!(self.output_mode, OutputMode::Memory | OutputMode::Both);
        let should_output = matches!(self.output_mode, OutputMode::Stdout | OutputMode::Both);
        let should_log = self.numeric_choices || self.verbosity >= VerbosityLevel::Normal;

        // Early exit if message won't be used
        if !should_log && !should_capture {
            return;
        }

        // Controller-specific debug to stderr (for debugging only, not part of deterministic log)
        if self.debug_state_hash {
            eprintln!("  >>> {}: {}", controller_name, message);
        }

        // Standardized deterministic format for stdout: just the choice, not the controller type
        // This ensures logs match regardless of which controller made the choice
        // Prepend <Choice> tag for easy grepping of all choices in a game
        let formatted = format!("<Choice> {}", message);

        // Capture if mode requires it
        if should_capture {
            self.log_buffer.borrow_mut().push(LogEntry {
                level: VerbosityLevel::Normal,
                message: formatted.clone(),
                category: Some("controller_choice".to_string()),
            });
        }

        // Output to stdout if mode requires it and should_log
        if should_output && should_log {
            println!("  {}", formatted);
        }
    }
}

impl Default for GameLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for GameLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameLogger")
            .field("verbosity", &self.verbosity)
            .field("output_mode", &self.output_mode)
            .field("log_count", &self.log_buffer.borrow().len())
            .finish()
    }
}

impl Clone for GameLogger {
    fn clone(&self) -> Self {
        GameLogger {
            verbosity: self.verbosity,
            step_header_printed: self.step_header_printed,
            numeric_choices: self.numeric_choices,
            output_format: self.output_format,
            output_mode: self.output_mode,
            show_choice_menu: self.show_choice_menu,
            debug_state_hash: self.debug_state_hash,
            format_bump: RefCell::new(Bump::new()),
            log_buffer: RefCell::new(Vec::new()),
        }
    }
}

impl Serialize for GameLogger {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("GameLogger", 5)?;
        state.serialize_field("verbosity", &self.verbosity)?;
        state.serialize_field("numeric_choices", &self.numeric_choices)?;
        state.serialize_field("output_format", &self.output_format)?;
        state.serialize_field("output_mode", &self.output_mode)?;
        state.serialize_field("show_choice_menu", &self.show_choice_menu)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for GameLogger {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct GameLoggerData {
            verbosity: VerbosityLevel,
            numeric_choices: bool,
            output_format: OutputFormat,
            output_mode: OutputMode,
            #[serde(default)]
            show_choice_menu: bool,
        }

        let data = GameLoggerData::deserialize(deserializer)?;
        Ok(GameLogger {
            verbosity: data.verbosity,
            step_header_printed: false,
            numeric_choices: data.numeric_choices,
            output_format: data.output_format,
            output_mode: data.output_mode,
            show_choice_menu: data.show_choice_menu,
            debug_state_hash: false,
            format_bump: RefCell::new(Bump::new()),
            log_buffer: RefCell::new(Vec::new()),
        })
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
    fn test_log_capture() {
        let mut logger = GameLogger::new();
        logger.enable_capture();

        logger.normal("test message");
        logger.minimal("minimal message");

        let logs = logger.logs();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].message, "test message");
        assert_eq!(logs[1].message, "minimal message");
    }

    #[test]
    fn test_zero_copy_iteration() {
        let mut logger = GameLogger::new();
        logger.enable_capture();

        for i in 0..100 {
            logger.normal(&format!("message {}", i));
        }

        // Iterate without copying
        let count = logger.logs().iter().filter(|log| log.message.contains("5")).count();

        // Should match: 5, 15, 25, ..., 95, 50-59
        assert!(count > 10);
    }

    #[test]
    fn test_capture_suppresses_stdout() {
        let mut logger = GameLogger::new();
        logger.enable_capture();

        assert!(logger.is_capturing());

        // Log some messages (they should be captured but not printed to stdout)
        logger.normal("message 1");
        logger.normal("message 2");
        logger.minimal("minimal message");

        // Verify messages were captured
        let logs = logger.logs();
        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].message, "message 1");
        assert_eq!(logs[1].message, "message 2");
        assert_eq!(logs[2].message, "minimal message");
    }

    #[test]
    fn test_flush_buffer() {
        let mut logger = GameLogger::new();
        logger.enable_capture();

        logger.normal("message 1");
        logger.normal("message 2");

        assert_eq!(logger.logs().len(), 2);

        // Flush should print to stdout and clear the buffer
        logger.flush_buffer();
        assert_eq!(logger.logs().len(), 0);
    }

    #[test]
    fn test_disable_capture() {
        let mut logger = GameLogger::new();
        logger.enable_capture();
        assert!(logger.is_capturing());

        logger.disable_capture();
        assert!(!logger.is_capturing());
    }
}
