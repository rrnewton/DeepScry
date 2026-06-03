//! Fully safe bump-allocating logger
//!
//! This implementation is 100% safe Rust with no unsafe keyword usage.
//! It uses owned Strings in LogEntry and returns a guard type for iteration.

use crate::core::PlayerId;
use crate::game::VerbosityLevel;
use bumpalo::Bump;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, Ref, RefCell};
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

/// Marks a log entry as containing hidden information visible only to a
/// specific player (e.g., the contents of a card a player just drew).
///
/// UIs viewing the game from another player's perspective MUST substitute
/// `public_message` for the entry's full `message` to prevent information
/// leaks. See `GameLogger::gamelog_private` and the per-card draw log in
/// `GameState::draw_card_inner` for the canonical example.
///
/// Closes bug-draw-reveals-opponent-hand.
#[derive(Debug, Clone)]
pub struct PrivateLogInfo {
    /// The player to whom the full `message` is visible.
    pub owner: PlayerId,
    /// The masked replacement other players (and spectators) should see —
    /// e.g. `"P2 draws a card"` instead of `"P2 draws Disenchant (88)"`.
    pub public_message: String,
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
    /// If `Some`, this entry contains hidden information visible only to
    /// `owner`. UIs rendering from a different perspective should display
    /// `public_message` instead. See [`PrivateLogInfo`].
    pub private_to: Option<PrivateLogInfo>,
}

impl LogEntry {
    /// Return the message text appropriate for the given perspective player.
    ///
    /// Returns `&self.message` for entries that aren't perspective-restricted
    /// or where `perspective` matches the owner; otherwise returns the masked
    /// `public_message` so opponent-private info is not leaked.
    pub fn message_for(&self, perspective: PlayerId) -> &str {
        match &self.private_to {
            Some(info) if info.owner != perspective => &info.public_message,
            _ => &self.message,
        }
    }
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
    /// Enable gamelog tagging (prepend [GAMELOG TurnN STEP] to official game actions)
    tag_gamelogs: bool,
    /// Current turn number for gamelog tagging
    gamelog_turn: RefCell<u32>,
    /// Current step abbreviation for gamelog tagging (UK, UP, DR, M1, BC, DA, DB, CD, EC, M2, ET, CL)
    gamelog_step: RefCell<&'static str>,

    /// Bump allocator for temporary string formatting
    /// Reset after each format operation to avoid growth
    format_bump: RefCell<Bump>,

    /// Captured log entries (owned strings)
    log_buffer: RefCell<Vec<LogEntry>>,

    /// Monotonic counter bumped whenever the log buffer is truncated or
    /// cleared (i.e. a rewind/undo discards entries). Renderers cache wrapped
    /// log lines keyed by entry count; a truncate that replay later re-grows
    /// to the same (or greater) count is invisible to a count-only staleness
    /// check, so caches additionally compare this epoch and force a full
    /// rebuild on mismatch. See mtg-432 / mtg-570 (rewind/replay left stale
    /// or duplicated turn-banner lines in the wrapped TUI log cache).
    log_epoch: Cell<u64>,

    /// Count of controller choices made in the game
    choice_count: RefCell<usize>,

    /// Enable ANSI colored output for CLI mode (default: true)
    /// Set to false via --no-color-logs CLI flag or NO_COLOR env var
    color_enabled: bool,

    /// Temporarily suppress all output (used during undo/rewind operations)
    /// When true, logging methods will not output to stdout or capture to buffer
    suppressed: bool,
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
            tag_gamelogs: false,
            gamelog_turn: RefCell::new(1),
            gamelog_step: RefCell::new("UK"),
            format_bump: RefCell::new(Bump::new()),
            log_buffer: RefCell::new(Vec::new()),
            log_epoch: Cell::new(0),
            choice_count: RefCell::new(0),
            color_enabled: true, // Colors enabled by default
            suppressed: false,
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
            tag_gamelogs: false,
            gamelog_turn: RefCell::new(1),
            gamelog_step: RefCell::new("UK"),
            format_bump: RefCell::new(Bump::new()),
            log_buffer: RefCell::new(Vec::new()),
            log_epoch: Cell::new(0),
            choice_count: RefCell::new(0),
            color_enabled: true, // Colors enabled by default
            suppressed: false,
        }
    }

    /// Suppress all logging output temporarily
    ///
    /// When suppressed, logging methods will not output to stdout or capture to buffer.
    /// Used during undo/rewind operations to prevent "action reversed" messages from appearing.
    pub fn set_suppressed(&mut self, suppressed: bool) {
        self.suppressed = suppressed;
    }

    /// Check if logging is currently suppressed
    pub fn is_suppressed(&self) -> bool {
        self.suppressed
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

    /// Check if controller_choice logging is active
    ///
    /// Returns true if calls to `controller_choice()` will actually produce output.
    /// Use this before expensive string formatting to avoid allocation overhead
    /// when logging is disabled.
    ///
    /// # Example
    /// ```ignore
    /// if logger.is_choice_logging_active() {
    ///     logger.controller_choice("RANDOM", &format!("expensive: {}", data));
    /// }
    /// ```
    #[inline]
    pub fn is_choice_logging_active(&self) -> bool {
        let should_capture = matches!(self.output_mode, OutputMode::Memory | OutputMode::Both);
        let should_log = self.numeric_choices || self.verbosity >= VerbosityLevel::Normal;
        should_log || should_capture || self.debug_state_hash
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
        self.bump_epoch();
    }

    /// Monotonic epoch that changes whenever the buffer is truncated or
    /// cleared. Renderers compare this against the epoch their cache was
    /// built with to detect a rewind/undo discard even when replay re-grows
    /// the buffer back to an identical length. See `log_epoch` field docs.
    #[inline]
    pub fn log_epoch(&self) -> u64 {
        self.log_epoch.get()
    }

    #[inline]
    fn bump_epoch(&self) {
        self.log_epoch.set(self.log_epoch.get().wrapping_add(1));
    }

    /// Truncate the log buffer to a specific size
    ///
    /// Removes all entries beyond the specified size.
    /// If size >= current length, does nothing.
    /// This is used to synchronize log removal with undo operations.
    pub fn truncate_to(&mut self, size: usize) {
        let mut buffer = self.log_buffer.borrow_mut();
        if size < buffer.len() {
            buffer.truncate(size);
            drop(buffer);
            self.bump_epoch();
        }
    }

    /// Get the current number of log entries
    pub fn log_count(&self) -> usize {
        self.log_buffer.borrow().len()
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

    /// Enable gamelog tagging (prepend [GAMELOG TurnN STEP] to official actions)
    pub fn set_tag_gamelogs(&mut self, enabled: bool) {
        self.tag_gamelogs = enabled;
    }

    /// Check if gamelog tagging is enabled
    pub fn tag_gamelogs_enabled(&self) -> bool {
        self.tag_gamelogs
    }

    /// Enable or disable ANSI colored output for CLI mode
    ///
    /// When enabled, log messages to stdout will include ANSI color codes
    /// for improved readability. Colors are applied based on message content
    /// (e.g., turn headers, damage, mana tapping).
    ///
    /// This only affects stdout output; TUI and web modes have their own
    /// color rendering via ratatui/RatZilla.
    pub fn set_color_enabled(&mut self, enabled: bool) {
        self.color_enabled = enabled;
    }

    /// Check if ANSI colored output is enabled
    pub fn color_enabled(&self) -> bool {
        self.color_enabled
    }

    /// Update the current turn number for gamelog tagging
    pub fn set_gamelog_turn(&self, turn: u32) {
        *self.gamelog_turn.borrow_mut() = turn;
    }

    /// Update the current step for gamelog tagging
    /// Step abbreviations: UK (Untap), UP (Upkeep), DR (Draw), M1 (Main1),
    /// BC (Begin Combat), DA (Declare Attackers), DB (Declare Blockers),
    /// CD (Combat Damage), EC (End Combat), M2 (Main2), ET (End), CL (Cleanup)
    pub fn set_gamelog_step(&self, step: &'static str) {
        *self.gamelog_step.borrow_mut() = step;
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

    /// Colorize a log message based on its content patterns
    ///
    /// Returns the message with ANSI escape codes when color_enabled is true.
    /// When colors are disabled or crossterm is unavailable, returns the message unchanged.
    #[cfg(feature = "native-tui")]
    fn colorize_message<'a>(&self, message: &'a str) -> std::borrow::Cow<'a, str> {
        use crossterm::style::Stylize;

        if !self.color_enabled {
            return std::borrow::Cow::Borrowed(message);
        }

        // Turn headers: ">>> Turn N" - Yellow, bold, underlined
        if message.contains(">>> Turn") || message.contains("<<<< ") {
            return std::borrow::Cow::Owned(message.yellow().bold().underlined().to_string());
        }

        // Step headers: "--- ... ---" - Cyan
        if message.starts_with("--- ") && message.ends_with(" ---") {
            return std::borrow::Cow::Owned(message.cyan().to_string());
        }

        // Combat events - Magenta
        if message.contains("attacks") || message.contains("blocks") {
            return std::borrow::Cow::Owned(message.magenta().to_string());
        }

        // Damage/life events - Red (bold for emphasis)
        if (message.contains("damage") && message.contains("life:"))
            || message.contains("takes") && message.contains("damage")
        {
            return std::borrow::Cow::Owned(message.red().bold().to_string());
        }

        // Life gain - Green
        if message.contains("gains") && message.contains("life") {
            return std::borrow::Cow::Owned(message.green().to_string());
        }

        // Resolution - Green
        if message.contains("resolves") {
            return std::borrow::Cow::Owned(message.green().to_string());
        }

        // Mana tapping - Dark gray (dim)
        if message.contains("Tap ") && message.contains("for {") {
            return std::borrow::Cow::Owned(message.dark_grey().to_string());
        }

        // Mana production - Dark gray
        if message.contains("taps") && message.contains("for {") {
            return std::borrow::Cow::Owned(message.dark_grey().to_string());
        }

        // Target selection - Dark gray (auxiliary info)
        if message.starts_with("  → targeting") {
            return std::borrow::Cow::Owned(message.dark_grey().to_string());
        }

        // Choice markers - Cyan, dim
        if message.starts_with("<Choice>") {
            return std::borrow::Cow::Owned(message.cyan().dim().to_string());
        }

        // Player names coloring for "Player1" and "Player2"
        // This is a simple approach - color the whole line based on which player is mentioned first
        if message.starts_with("Player1") || message.contains(" Player1 ") {
            // Blue tint for Player1
            return std::borrow::Cow::Owned(message.blue().to_string());
        }
        if message.starts_with("Player2") || message.contains(" Player2 ") {
            // Red tint for Player2
            return std::borrow::Cow::Owned(message.dark_red().to_string());
        }

        // Default: return as-is
        std::borrow::Cow::Borrowed(message)
    }

    /// Fallback colorize_message when crossterm is not available
    #[cfg(not(feature = "native-tui"))]
    fn colorize_message<'a>(&self, message: &'a str) -> std::borrow::Cow<'a, str> {
        std::borrow::Cow::Borrowed(message)
    }

    /// Fast path for stdout logging
    #[inline]
    fn log_to_stdout(&self, level: VerbosityLevel, message: &str) {
        let colored = self.colorize_message(message);
        if level == VerbosityLevel::Minimal {
            println!("{}", colored);
        } else {
            println!("  {}", colored);
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
        if self.suppressed {
            return;
        }
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
                private_to: None,
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
        if self.suppressed {
            return;
        }
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
                private_to: None,
            });
        }

        // Output to stdout if mode requires it and verbosity allows
        if should_output && VerbosityLevel::Normal <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Normal, message);
        }
    }

    /// Log at Normal level, marking the captured entry as containing hidden
    /// information visible only to `owner`.
    ///
    /// This is the Normal-verbosity counterpart to [`Self::gamelog_private`]
    /// (which is for tagged official game-action logs). Use it for
    /// `normal()`-level lines whose text reveals a single player's hidden
    /// state/decision — e.g. the card a player scried to the top/bottom of
    /// their own library. UIs rendering from another player's perspective
    /// substitute `public_message` for `message` via [`LogEntry::message_for`].
    ///
    /// stdout still receives the full `message` (the CLI/server log is the
    /// canonical full-information replay log); masking happens only at the
    /// structured-view boundary used by `web/native_game.html` /
    /// `web/tui_game.html`. The entry is left UNtagged (no `[GAMELOG ...]`
    /// prefix) so it does not alter tagged-gamelog network comparisons. See
    /// mtg-412.
    #[inline]
    pub fn normal_private(&self, message: &str, owner: PlayerId, public_message: &str) {
        if self.suppressed {
            return;
        }
        let should_capture = matches!(self.output_mode, OutputMode::Memory | OutputMode::Both);
        let should_output = matches!(self.output_mode, OutputMode::Stdout | OutputMode::Both);

        // Early exit if message won't be used
        if VerbosityLevel::Normal > self.verbosity && !should_capture {
            return;
        }

        if should_capture {
            self.log_buffer.borrow_mut().push(LogEntry {
                level: VerbosityLevel::Normal,
                message: message.to_string(),
                category: None,
                private_to: Some(PrivateLogInfo {
                    owner,
                    public_message: public_message.to_string(),
                }),
            });
        }

        if should_output && VerbosityLevel::Normal <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Normal, message);
        }
    }

    /// Log a turn separator line (e.g., ">>> Turn N - Player X (Player Y) <<<<")
    ///
    /// This is used for TUI navigation markers. The separator is:
    /// - Always captured to buffer (for TUI display and navigation)
    /// - Never output to stdout (since the turn header is printed directly in CLI mode)
    ///
    /// This prevents duplicate turn info in basic CLI mode where both
    /// the separator and the turn header would otherwise appear.
    #[inline]
    pub fn turn_separator(&self, message: &str) {
        if self.suppressed {
            return;
        }
        let should_capture = matches!(self.output_mode, OutputMode::Memory | OutputMode::Both);

        // Early exit if message won't be used
        if VerbosityLevel::Normal > self.verbosity && !should_capture {
            return;
        }

        // Always capture to buffer for TUI navigation (never output to stdout)
        self.log_buffer.borrow_mut().push(LogEntry {
            level: VerbosityLevel::Normal,
            message: message.to_string(),
            category: None,
            private_to: None,
        });
    }

    /// Log at Verbose level
    #[inline]
    pub fn verbose(&self, message: &str) {
        if self.suppressed {
            return;
        }
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
                private_to: None,
            });
        }

        // Output to stdout if mode requires it and verbosity allows
        if should_output && VerbosityLevel::Verbose <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Verbose, message);
        }
    }

    /// Log an official game action at Normal level
    ///
    /// When tag_gamelogs is enabled, prepends `[GAMELOG TurnN STEP]` prefix.
    /// This allows comparing game logs between local and network modes.
    ///
    /// Use this for official game actions like:
    /// - Card plays (lands, spells)
    /// - Combat (attacks, blocks, damage)
    /// - Life changes
    /// - Card draws
    /// - Turn/step transitions
    ///
    /// Do NOT use this for:
    /// - Battlefield display printouts
    /// - Choice selection menus
    /// - Debug output
    #[inline]
    pub fn gamelog(&self, message: &str) {
        if self.suppressed {
            return;
        }
        let should_capture = matches!(self.output_mode, OutputMode::Memory | OutputMode::Both);
        let should_output = matches!(self.output_mode, OutputMode::Stdout | OutputMode::Both);

        // Early exit if message won't be used
        if VerbosityLevel::Normal > self.verbosity && !should_capture {
            return;
        }

        // Format with tag prefix if enabled
        let formatted = if self.tag_gamelogs {
            let turn = *self.gamelog_turn.borrow();
            let step = *self.gamelog_step.borrow();
            format!("[GAMELOG Turn{} {}] {}", turn, step, message)
        } else {
            message.to_string()
        };

        // Capture if mode requires it
        if should_capture {
            self.log_buffer.borrow_mut().push(LogEntry {
                level: VerbosityLevel::Normal,
                message: formatted.clone(),
                category: Some("gamelog".to_string()),
                private_to: None,
            });
        }

        // Output to stdout if mode requires it and verbosity allows
        if should_output && VerbosityLevel::Normal <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Normal, &formatted);
        }
    }

    /// Log a perspective-restricted official game action at Normal level.
    ///
    /// Identical to [`Self::gamelog`] except that the captured log entry is
    /// marked as containing hidden information visible only to `owner`. UIs
    /// rendering from another player's perspective MUST substitute
    /// `public_message` for the full `message` (see
    /// [`LogEntry::message_for`]).
    ///
    /// **stdout is unaffected** — the full message is still printed for the
    /// CLI/server log (which is the canonical, full-information replay log).
    /// Filtering happens only at the structured-view boundary used by
    /// `web/native_game.html` and `web/tui_game.html`.
    ///
    /// Closes bug-draw-reveals-opponent-hand.
    #[inline]
    pub fn gamelog_private(&self, message: &str, owner: PlayerId, public_message: &str) {
        if self.suppressed {
            return;
        }
        let should_capture = matches!(self.output_mode, OutputMode::Memory | OutputMode::Both);
        let should_output = matches!(self.output_mode, OutputMode::Stdout | OutputMode::Both);

        // Early exit if message won't be used
        if VerbosityLevel::Normal > self.verbosity && !should_capture {
            return;
        }

        // Format with tag prefix if enabled (matches `gamelog` so the tagged
        // entry stays comparable across local/network modes).
        let formatted = if self.tag_gamelogs {
            let turn = *self.gamelog_turn.borrow();
            let step = *self.gamelog_step.borrow();
            format!("[GAMELOG Turn{} {}] {}", turn, step, message)
        } else {
            message.to_string()
        };
        let formatted_public = if self.tag_gamelogs {
            let turn = *self.gamelog_turn.borrow();
            let step = *self.gamelog_step.borrow();
            format!("[GAMELOG Turn{} {}] {}", turn, step, public_message)
        } else {
            public_message.to_string()
        };

        if should_capture {
            self.log_buffer.borrow_mut().push(LogEntry {
                level: VerbosityLevel::Normal,
                message: formatted.clone(),
                category: Some("gamelog".to_string()),
                private_to: Some(PrivateLogInfo {
                    owner,
                    public_message: formatted_public,
                }),
            });
        }

        if should_output && VerbosityLevel::Normal <= self.verbosity {
            self.log_to_stdout(VerbosityLevel::Normal, &formatted);
        }
    }

    /// Log a controller decision at Normal level
    ///
    /// Outputs standardized "chose X" format to stdout for deterministic logging.
    /// Controller-specific debug info goes to stderr when debug_state_hash is enabled.
    ///
    /// Uses bump allocator for temporary formatting to avoid intermediate allocations.
    /// Increments the global choice counter for display in TUI status.
    #[inline]
    pub fn controller_choice(&self, controller_name: &str, message: &str) {
        // Increment choice counter (always increment, regardless of logging or suppression)
        *self.choice_count.borrow_mut() += 1;

        if self.suppressed {
            return;
        }
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
                private_to: None,
            });
        }

        // Output to stdout if mode requires it and should_log
        if should_output && should_log {
            println!("  {}", formatted);
        }
    }

    /// Get the current count of controller choices made
    ///
    /// Returns the total number of times controller_choice() has been called.
    /// Used by the fancy TUI to display choice count status.
    pub fn choice_count(&self) -> usize {
        *self.choice_count.borrow()
    }

    /// Decrement the choice counter
    ///
    /// Used when undoing a choice to keep the counter accurate.
    /// Does nothing if the counter is already at 0.
    pub fn decrement_choice_count(&self) {
        let current = *self.choice_count.borrow();
        if current > 0 {
            *self.choice_count.borrow_mut() = current - 1;
        }
    }

    /// Set the choice counter to a specific value
    ///
    /// Used when restoring to a specific choice point during undo.
    /// This directly sets the counter instead of incrementing/decrementing.
    pub fn set_choice_count(&self, count: usize) {
        *self.choice_count.borrow_mut() = count;
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
            tag_gamelogs: self.tag_gamelogs,
            gamelog_turn: RefCell::new(*self.gamelog_turn.borrow()),
            gamelog_step: RefCell::new(*self.gamelog_step.borrow()),
            format_bump: RefCell::new(Bump::new()),
            log_buffer: RefCell::new(Vec::new()),
            // The clone starts with an empty buffer; bump past the source's
            // epoch so any wrap cache built against the original is treated
            // as stale and rebuilt rather than matching by coincidence.
            log_epoch: Cell::new(self.log_epoch.get().wrapping_add(1)),
            choice_count: RefCell::new(0),
            color_enabled: self.color_enabled,
            suppressed: false, // Never clone the suppressed state
        }
    }
}

impl Serialize for GameLogger {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("GameLogger", 6)?;
        state.serialize_field("verbosity", &self.verbosity)?;
        state.serialize_field("numeric_choices", &self.numeric_choices)?;
        state.serialize_field("output_format", &self.output_format)?;
        state.serialize_field("output_mode", &self.output_mode)?;
        state.serialize_field("show_choice_menu", &self.show_choice_menu)?;
        state.serialize_field("color_enabled", &self.color_enabled)?;
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
            #[serde(default = "default_color_enabled")]
            color_enabled: bool,
        }

        fn default_color_enabled() -> bool {
            true
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
            tag_gamelogs: false,
            gamelog_turn: RefCell::new(1),
            gamelog_step: RefCell::new("UK"),
            format_bump: RefCell::new(Bump::new()),
            log_buffer: RefCell::new(Vec::new()),
            log_epoch: Cell::new(0),
            choice_count: RefCell::new(0),
            color_enabled: data.color_enabled,
            suppressed: false,
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

    /// mtg-432/mtg-570: a rewind/undo discards entries via `truncate_to` (or
    /// `clear_logs`). Renderers cache wrapped log lines and need to know the
    /// buffer was discarded even when replay re-grows it to the same length,
    /// so the logger exposes a monotonic epoch that bumps on every such event.
    #[test]
    fn test_log_epoch_bumps_on_truncate_and_clear() {
        let mut logger = GameLogger::new();
        logger.enable_capture();

        let e0 = logger.log_epoch();
        logger.normal("a");
        logger.normal("b");
        logger.normal("c");
        // Plain appends do NOT change the epoch.
        assert_eq!(logger.log_epoch(), e0, "appends must not bump epoch");

        // A no-op truncate (size >= len) must not bump the epoch.
        logger.truncate_to(3);
        assert_eq!(logger.log_epoch(), e0, "no-op truncate must not bump epoch");

        // A real truncate bumps the epoch.
        logger.truncate_to(1);
        let e1 = logger.log_epoch();
        assert_ne!(e1, e0, "real truncate must bump epoch");
        assert_eq!(logger.logs().len(), 1);

        // clear_logs also bumps the epoch.
        logger.clear_logs();
        assert_ne!(logger.log_epoch(), e1, "clear_logs must bump epoch");
    }

    /// mtg-412: `normal_private` must mask hidden information for the
    /// non-owner perspective (e.g. the card a player scried to the top of
    /// their own library), while the owner and the canonical stdout log keep
    /// the full text. Mirrors the per-card-draw `gamelog_private` contract for
    /// Normal-level (untagged) log lines.
    #[test]
    fn test_normal_private_masks_card_name_for_opponent() {
        use crate::core::PlayerId;

        let mut logger = GameLogger::new();
        logger.enable_capture();

        let p1 = PlayerId::new(0);
        let p2 = PlayerId::new(1);

        logger.normal_private(
            "P1 scries 1, keeps Lightning Bolt on top",
            p1,
            "P1 scries 1, keeps the card on top",
        );

        let logs = logger.logs();
        assert_eq!(logs.len(), 1);
        let entry = &logs[0];

        // The scrying player sees the full line including the card identity.
        assert_eq!(entry.message_for(p1), "P1 scries 1, keeps Lightning Bolt on top");
        assert!(entry.message_for(p1).contains("Lightning Bolt"));

        // The opponent sees the masked public form — NO card name leaked.
        assert_eq!(entry.message_for(p2), "P1 scries 1, keeps the card on top");
        assert!(
            !entry.message_for(p2).contains("Lightning Bolt"),
            "leak: opponent must not see the scried card name"
        );

        // Untagged (Normal-level) — must not carry a gamelog category so it
        // does not alter tagged-gamelog network comparisons.
        assert_eq!(entry.category, None);
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

    #[test]
    fn test_color_enabled_default() {
        let logger = GameLogger::new();
        // Colors are enabled by default
        assert!(logger.color_enabled());
    }

    #[test]
    fn test_color_enabled_setter() {
        let mut logger = GameLogger::new();
        assert!(logger.color_enabled());

        logger.set_color_enabled(false);
        assert!(!logger.color_enabled());

        logger.set_color_enabled(true);
        assert!(logger.color_enabled());
    }

    #[cfg(feature = "native-tui")]
    #[test]
    fn test_colorize_message_turn_headers() {
        let logger = GameLogger::new();

        // Turn header should get colorized
        let result = logger.colorize_message(">>> Turn 1 - Player1 20 (Player2 20) <<<<");
        assert!(result.contains("\x1b[")); // Contains ANSI escape codes

        // When disabled, should return unchanged
        let mut logger2 = GameLogger::new();
        logger2.set_color_enabled(false);
        let result2 = logger2.colorize_message(">>> Turn 1 - Player1 20 (Player2 20) <<<<");
        assert!(!result2.contains("\x1b[")); // No ANSI escape codes
    }

    #[cfg(feature = "native-tui")]
    #[test]
    fn test_colorize_message_patterns() {
        let logger = GameLogger::new();

        // Step headers - cyan
        let result = logger.colorize_message("--- Main Phase 1 ---");
        assert!(result.contains("\x1b["));

        // Mana tapping - dark gray
        let result = logger.colorize_message("Tap Mountain for {R}");
        assert!(result.contains("\x1b["));

        // Combat - magenta
        let result = logger.colorize_message("Grizzly Bears attacks");
        assert!(result.contains("\x1b["));

        // Default text - no color change when there's no pattern match
        let result = logger.colorize_message("Some random message");
        assert_eq!(result, "Some random message");
    }
}
