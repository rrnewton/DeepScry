//! Rich input controller that parses textual commands
//!
//! This controller accepts rich text commands like "play swamp" or "cast black_knight"
//! and converts them to numeric choices based on available options.
//!
//! ## Command Syntax
//!
//! - **Verbs**: Play, Cast, Equip, Activate, Attack, Block, Discard, Pass (case-insensitive)
//! - **Card names**: Case-insensitive, spaces/underscores equivalent, prefix matching allowed
//! - **Quotes**: Optional for card names at end of command
//! - **Examples**:
//!   - `play swamp` - Play a land
//!   - `cast "Black Knight"` - Cast a spell
//!   - `equip accorder` - Activate Equipment's Equip ability
//!   - `activate forest` - Activate mana ability (first ability if multiple)
//!   - `activate forest[2]` - Activate second ability (1-indexed)
//!   - `attack serra` - Attack with Serra Angel
//!
//! ## Wildcard Separator
//!
//! Use `*` to skip choices until the next command matches:
//! - `play mountain;*;cast fireball` - Play mountain, then pass priority until "cast fireball" is available
//! - `equip accorder;*;attack grizzly` - Equip, then pass until attack phase
//!
//! This allows flexible scripts that don't need to specify every single priority pass.
//!
//! ## Error Handling
//!
//! - **Normal mode**: Commands MUST match an available action or be an explicit pass ("pass", "p", "0").
//!   If a command doesn't match, the controller returns an error with available actions.
//! - **Wildcard mode**: Non-matching commands cause the controller to pass priority and wait for a match.
//!   No error is raised - the controller keeps waiting until the command becomes available.
//!
//! ## Blocking Syntax
//!
//! Comma-separated clauses: `BlackKnight blocks WhiteKnight, SerraAngel blocks RoyalAssassin`

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::command_parsing::{card_matches, is_explicit_pass, parse_pass_until, parse_spell_ability_choice};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use smallvec::SmallVec;

/// Controller that parses rich text commands
pub struct RichInputController {
    player_id: PlayerId,
    /// Script of text commands (consumed from front)
    commands: Vec<String>,
    /// Current index in the command queue
    current_index: usize,
    /// Whether we're in wildcard mode (waiting for a specific command to match)
    wildcard_mode: bool,
    /// Target selector requested by an inline `cast <card> targeting <selector>`
    /// clause, stashed when the cast is chosen and consumed by the next
    /// `choose_targets` call.
    ///
    /// This makes targeted plays robust to whether the engine actually *asks*
    /// for a target: a single-legal-target spell (e.g. Lightning Bolt vs. the
    /// only creature) is auto-targeted by the engine without a `choose_targets`
    /// callback (CR 601.2c forced choice), so a standalone `target` command on
    /// the next line would strand and error. The inline clause is consumed when
    /// `choose_targets` IS called and is otherwise a harmless no-op.
    ///
    /// Holds the lowercased selector string (a card name or a `pN` player
    /// sentinel), matched against valid targets the same way a standalone
    /// `target` command is. Information-independent: only matches against the
    /// public list of valid targets the engine already offered.
    pending_target: Option<String>,
}

impl RichInputController {
    /// Create a new rich input controller
    ///
    /// # Arguments
    /// * `player_id` - The player ID this controller manages
    /// * `commands` - Vector of text commands to execute
    pub fn new(player_id: PlayerId, commands: Vec<String>) -> Self {
        RichInputController {
            player_id,
            commands,
            current_index: 0,
            wildcard_mode: false,
            pending_target: None,
        }
    }

    /// Split an inline `cast <card> targeting <selector>` command into its
    /// spell-command part and an optional target selector.
    ///
    /// The split is on the case-insensitive ` targeting ` keyword. The returned
    /// spell command (e.g. `"cast Lightning Bolt"`) is what the normal verb
    /// matcher consumes; the selector (e.g. `"grizzly bears"`, lowercased) is
    /// stashed for the next `choose_targets`. When no ` targeting ` keyword is
    /// present the whole command is returned with `None`.
    fn split_targeting_clause(command: &str) -> (String, Option<String>) {
        // Case-insensitive search for the keyword without allocating a second
        // lowercased copy for slicing: find on a lowercased scan, slice the
        // ORIGINAL so card-name casing is preserved for logging.
        const KW: &str = " targeting ";
        let lower = command.to_lowercase();
        if let Some(pos) = lower.find(KW) {
            let spell_cmd = command[..pos].trim().to_string();
            let selector = command[pos + KW.len()..].trim().to_lowercase();
            let selector = if selector.is_empty() { None } else { Some(selector) };
            (spell_cmd, selector)
        } else {
            (command.to_string(), None)
        }
    }

    /// Does the valid-target `tid` match the (already-lowercased) `selector`?
    ///
    /// Shared by the standalone `target <selector>` command and the inline
    /// `cast <card> targeting <selector>` clause so both behave identically.
    /// Player targets match `pN` (0- or 1-based) or the player's name; card
    /// targets use the same prefix / space- / case-insensitive `card_matches`
    /// matcher as `cast <card>` (anti-overfitting — survives card renames that
    /// keep a shared prefix and avoids brittle exact-string coupling).
    fn target_matches_selector(view: &GameStateView, tid: CardId, selector: &str) -> bool {
        let name = view.card_name(tid).unwrap_or_default();
        if let Some(pid) = crate::core::player_target_from_sentinel(tid) {
            let pid_idx = pid.as_u32();
            selector == format!("p{}", pid_idx + 1)
                || selector == format!("p{}", pid_idx)
                || selector == name.to_lowercase()
        } else {
            card_matches(&name, selector)
        }
    }

    /// Peek at the current command without consuming it
    fn peek_command(&self) -> Option<&str> {
        if self.current_index < self.commands.len() {
            Some(&self.commands[self.current_index])
        } else {
            None
        }
    }

    /// Check if the current command (at current_index) is a wildcard
    fn current_is_wildcard(&self) -> bool {
        if self.current_index < self.commands.len() {
            self.commands[self.current_index].trim() == "*"
        } else {
            false
        }
    }

    /// If the current command is a `PASS_UNTIL` whose condition is already met,
    /// consume it and return `false` (caller should proceed normally).
    /// If it is a `PASS_UNTIL` whose condition is NOT yet met, return `true`
    /// (caller should pass priority without consuming the command).
    /// If the current command is not a `PASS_UNTIL` at all, return `false`.
    ///
    /// On a malformed `PASS_UNTIL` directive, logs the error, consumes the
    /// command to avoid looping forever, and returns `false`.
    fn handle_pass_until(&mut self, view: &GameStateView) -> bool {
        let Some(cmd_str) = self.peek_command() else {
            return false;
        };
        let Some(parse_result) = parse_pass_until(cmd_str) else {
            return false; // not a PASS_UNTIL command
        };
        match parse_result {
            Err(e) => {
                // Malformed PASS_UNTIL: consume and log so script can continue
                log::error!("PASS_UNTIL parse error: {e}");
                self.current_index += 1;
                false
            }
            Ok(cond) => {
                let turn = view.turn_number();
                let step = view.current_step();
                if cond.is_satisfied(turn, step) {
                    // Condition met: consume the directive, resume normal script
                    view.logger().controller_choice(
                        "RICHINPUT",
                        &format!("PASS_UNTIL satisfied at turn={turn} step={step:?}: resuming script"),
                    );
                    self.current_index += 1;
                    false // do NOT pass priority; let the normal command run
                } else {
                    // Condition not yet met: pass priority without consuming
                    view.logger().controller_choice(
                        "RICHINPUT",
                        &format!(
                            "PASS_UNTIL waiting (turn={turn} step={step:?}); \
                             target turn={:?} step={:?}",
                            cond.turn, cond.step
                        ),
                    );
                    true // pass priority
                }
            }
        }
    }

    /// Get the next command from the script and advance the index
    /// Automatically skips wildcard separators and enters wildcard mode
    fn next_command(&mut self) -> Option<String> {
        if self.current_index < self.commands.len() {
            let cmd = self.commands[self.current_index].clone();
            self.current_index += 1;

            // Check if this was a wildcard separator
            if cmd.trim() == "*" {
                // Enter wildcard mode and get the next actual command
                self.wildcard_mode = true;
                return self.next_command();
            }

            // Check if current command (after advancing) is a wildcard - if so, enter wildcard mode
            if self.current_is_wildcard() {
                // Consume the wildcard
                self.current_index += 1;
                self.wildcard_mode = true;
            } else if !self.wildcard_mode {
                // Only reset to false if we weren't already in wildcard mode
                // (prevents recursive next_command calls from resetting the flag)
                self.wildcard_mode = false;
            }

            Some(cmd)
        } else {
            None
        }
    }
}

impl PlayerController for RichInputController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // If there are no available abilities, just pass - no point trying to match commands
        // This can happen in network mode when server asks for priority with 0 abilities
        if available.is_empty() {
            if let Some(command_str) = self.peek_command() {
                let command = command_str.to_string();
                if is_explicit_pass(&command) {
                    self.next_command();
                } else if self.wildcard_mode {
                    // Do nothing
                } else {
                    self.next_command();
                }
            }
            return ChoiceResult::Ok(None);
        }

        // Check for PASS_UNTIL before any other processing.
        // If it returns true we should pass priority (condition not yet met).
        if self.handle_pass_until(view) {
            return ChoiceResult::Ok(None);
        }

        // Peek at the next command without consuming it
        if let Some(command_str) = self.peek_command() {
            let raw_command = command_str.to_string();

            // Check if this is a wildcard separator - if so, consume it and enter wildcard mode
            if raw_command.trim() == "*" {
                // Consume the wildcard and enter wildcard mode
                self.current_index += 1;
                self.wildcard_mode = true;
                // Now recursively call to process the next actual command
                return self.choose_spell_ability_to_play(view, available);
            }

            // Split off an optional inline `... targeting <selector>` clause.
            // `command` is the spell/ability part used for matching; the target
            // selector (if any) is stashed below ONLY when the action matches,
            // so it is consumed by the next `choose_targets` call.
            let (command, inline_target) = Self::split_targeting_clause(&raw_command);

            // Try to parse it
            let mut result = None;
            let mut consumed_extra = false;

            if let Some(card_pattern) = command.strip_prefix("activate ") {
                let next_idx = self.current_index + 1;
                let next_cmd_is_num = if next_idx < self.commands.len() {
                    self.commands[next_idx].trim().parse::<usize>().ok()
                } else {
                    None
                };

                if let Some(choice_idx) = next_cmd_is_num {
                    // Find all matching ActivateAbility
                    let mut matches = Vec::new();
                    for ability in available {
                        if let SpellAbility::ActivateAbility { card_id, .. } = ability {
                            if let Some(card_name) = view.card_name(*card_id) {
                                if card_matches(&card_name, card_pattern) {
                                    matches.push(ability);
                                }
                            }
                        }
                    }
                    if choice_idx < matches.len() {
                        result = Some(matches[choice_idx].clone());
                        consumed_extra = true;
                    }
                }
            }

            if result.is_none() {
                result = parse_spell_ability_choice(&command, view, available);
            }

            // Check if this is an explicit pass command
            let explicit_pass = is_explicit_pass(&command);

            // If the action matched and carried an inline `targeting <selector>`
            // clause, stash the selector so the upcoming `choose_targets` (if the
            // engine asks for a target at all) selects the named target. Only
            // stash on a real match — a non-matching command must not leave a
            // stale target queued.
            if result.is_some() {
                self.pending_target = inline_target;
            }

            // Check if this is a combat command (attack/block) - these should be deferred
            // to choose_attackers/choose_blockers, so we pass priority here
            let cmd_lower = command.trim().to_lowercase();
            let is_combat_command = cmd_lower.starts_with("attack ") || cmd_lower.starts_with("block ");

            // In wildcard mode, only advance if we found a match or explicit pass
            if self.wildcard_mode {
                if result.is_some() || explicit_pass {
                    // Found a match or explicit pass! Exit wildcard mode first, then consume
                    self.wildcard_mode = false;
                    self.next_command();
                    if consumed_extra {
                        self.next_command();
                    }
                    ChoiceResult::Ok(result)
                } else {
                    // No match - pass priority and stay in wildcard mode
                    ChoiceResult::Ok(None)
                }
            } else {
                // Normal mode - command MUST match or error
                if result.is_some() || explicit_pass {
                    // Valid command or explicit pass - consume and execute
                    self.next_command();
                    if consumed_extra {
                        self.next_command();
                    }
                    ChoiceResult::Ok(result)
                } else if is_combat_command {
                    // Combat command (attack/block) during priority - don't consume, pass priority
                    // The command will be handled later by choose_attackers/choose_blockers
                    ChoiceResult::Ok(None)
                } else {
                    // Command didn't match any available action - ERROR
                    self.next_command(); // Consume the bad command to avoid infinite loop
                    ChoiceResult::Error(format!(
                        "Command '{}' did not match any available action. Available actions: {:?}",
                        command,
                        available
                            .iter()
                            .map(|a| crate::game::controller::format_spell_ability_choice(view, a))
                            .collect::<Vec<_>>()
                    ))
                }
            }
        } else {
            // No more commands - pass priority
            ChoiceResult::Ok(None)
        }
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
        min_targets: usize,
        max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if valid_targets.is_empty() {
            self.pending_target = None;
            return ChoiceResult::Ok(SmallVec::new());
        }

        // First, honour a stashed inline `targeting <selector>` clause from a
        // `cast <card> targeting <selector>` command. Matched the same way as a
        // standalone `target` command, but against the selector captured at
        // cast time. Consumed whether or not it matches (a one-shot hint).
        if let Some(selector) = self.pending_target.take() {
            let matched: SmallVec<[CardId; 4]> = valid_targets
                .iter()
                .filter(|&&tid| Self::target_matches_selector(_view, tid, &selector))
                .copied()
                .collect();
            if !matched.is_empty() {
                return ChoiceResult::Ok(matched);
            }
            // Selector did not match any valid target: fall through to the
            // standalone-command path / deterministic default below.
        }

        // Try to match next command if present
        if let Some(cmd) = self.peek_command() {
            let cmd_clean = cmd.trim().to_lowercase();
            let matched_targets: SmallVec<[CardId; 4]> = valid_targets
                .iter()
                .filter(|&&tid| Self::target_matches_selector(_view, tid, &cmd_clean))
                .copied()
                .collect();
            if !matched_targets.is_empty() {
                self.next_command();
                return ChoiceResult::Ok(matched_targets);
            }
        }

        // Best-effort: rich text target syntax for variable-count spells is a
        // deferred follow-up (mtg-tyvcn). For now take the first `min_targets`
        // (at least 1, capped at max) valid targets deterministically.
        let lo = min_targets.max(1);
        let count = lo.min(max_targets.max(1)).min(valid_targets.len());
        ChoiceResult::Ok(valid_targets.iter().take(count).copied().collect())
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Simple greedy approach: take first N sources
        let mut sources = SmallVec::new();
        let needed = cost.cmc() as usize;

        for &source_id in available_sources.iter().take(needed) {
            sources.push(source_id);
        }

        ChoiceResult::Ok(sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if available_creatures.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // PASS_UNTIL: if waiting for a later turn/phase, don't declare attackers
        if self.handle_pass_until(view) {
            return ChoiceResult::Ok(SmallVec::new());
        }

        if let Some(command) = self.next_command() {
            let cmd = command.trim().to_lowercase();

            // Handle numeric choice (legacy format)
            if let Ok(num) = cmd.parse::<usize>() {
                let num_attackers = num.min(available_creatures.len());
                return ChoiceResult::Ok(available_creatures.iter().take(num_attackers).copied().collect());
            }

            // Parse "attack X" commands
            let mut attackers = SmallVec::new();
            for clause in command.split(';') {
                let clause = clause.trim().to_lowercase();
                if let Some(card_pattern) = clause.strip_prefix("attack ") {
                    for &creature_id in available_creatures {
                        if let Some(card_name) = view.card_name(creature_id) {
                            if card_matches(&card_name, card_pattern) && !attackers.contains(&creature_id) {
                                attackers.push(creature_id);
                            }
                        }
                    }
                } else if clause == "done" {
                    break;
                }
            }

            ChoiceResult::Ok(attackers)
        } else {
            // No more commands - don't attack
            ChoiceResult::Ok(SmallVec::new())
        }
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        if available_blockers.is_empty() || attackers.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // PASS_UNTIL: if waiting for a later turn/phase, don't declare blockers
        if self.handle_pass_until(view) {
            return ChoiceResult::Ok(SmallVec::new());
        }

        if let Some(command) = self.next_command() {
            let cmd = command.trim().to_lowercase();

            // Handle numeric choice (legacy format)
            if let Ok(num) = cmd.parse::<usize>() {
                let num_blockers = num.min(available_blockers.len());
                let mut blocks = SmallVec::new();
                for &blocker_id in available_blockers.iter().take(num_blockers) {
                    blocks.push((blocker_id, attackers[0]));
                }
                return ChoiceResult::Ok(blocks);
            }

            // Parse "X blocks Y" commands
            let mut blocks = SmallVec::new();
            for clause in command.split(';') {
                let clause = clause.trim().to_lowercase();
                if clause.contains(" blocks ") {
                    if let Some(blocks_pos) = clause.find(" blocks ") {
                        let blocker_pattern = &clause[..blocks_pos];
                        let attacker_pattern = &clause[blocks_pos + 8..];

                        // Find matching blocker
                        let mut blocker_id = None;
                        for &creature_id in available_blockers {
                            if let Some(card_name) = view.card_name(creature_id) {
                                if card_matches(&card_name, blocker_pattern) {
                                    blocker_id = Some(creature_id);
                                    break;
                                }
                            }
                        }

                        // Find matching attacker
                        let mut attacker_id = None;
                        for &creature_id in attackers {
                            if let Some(card_name) = view.card_name(creature_id) {
                                if card_matches(&card_name, attacker_pattern) {
                                    attacker_id = Some(creature_id);
                                    break;
                                }
                            }
                        }

                        if let (Some(blocker), Some(attacker)) = (blocker_id, attacker_id) {
                            blocks.push((blocker, attacker));
                        }
                    }
                } else if clause == "done" {
                    break;
                }
            }

            ChoiceResult::Ok(blocks)
        } else {
            // No more commands - don't block
            ChoiceResult::Ok(SmallVec::new())
        }
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Keep original order (no reordering via rich input yet)
        ChoiceResult::Ok(blockers.iter().copied().collect())
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Simple: discard first N cards
        // TODO(mtg-144): Implement rich syntax for discard selection
        ChoiceResult::Ok(hand.iter().take(count).copied().collect())
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&crate::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        // RichInputController: Auto-select first valid card
        // TODO(mtg-144): Implement rich syntax for library search selection
        if valid_cards.is_empty() {
            view.logger()
                .controller_choice("RICHINPUT", "Library search: fail to find (no valid cards)");
            return ChoiceResult::Ok(None);
        }

        let card_def = valid_cards[0];
        view.logger().controller_choice(
            "RICHINPUT",
            &format!("Library search: found {} (auto-selected first)", card_def.name),
        );

        ChoiceResult::Ok(Some(0))
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // RichInputController: Auto-select first N permanents
        // TODO(mtg-144): Implement rich syntax for sacrifice selection
        let num_to_sacrifice = count.min(valid_permanents.len());
        let to_sacrifice: SmallVec<[CardId; 8]> = valid_permanents.iter().take(num_to_sacrifice).copied().collect();

        if !to_sacrifice.is_empty() {
            view.logger().controller_choice(
                "RICHINPUT",
                &format!(
                    "Sacrifice {}: auto-selected first {} permanents",
                    card_type_description, num_to_sacrifice
                ),
            );
        }

        ChoiceResult::Ok(to_sacrifice)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Rich input controller always untaps everything (returns empty list = untap all)
        // TODO(mtg-144): Could add command syntax for controlling untap decisions
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        _view: &GameStateView,
        _spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // Rich input controller: use next command as mode index, or default to first N modes
        // TODO(mtg-144): Could add command syntax like "mode 0 1" for selecting specific modes
        if let Some(command_str) = self.peek_command() {
            let command = command_str.to_string();
            self.current_index += 1;

            // Try to parse as mode index
            if let Ok(idx) = command.trim().parse::<usize>() {
                if idx < mode_descriptions.len() {
                    return ChoiceResult::Ok(smallvec::smallvec![idx]);
                }
            }
        }

        // Default: choose first N modes
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // No action needed
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // No action needed
    }

    /// Choose from string options (used by network client)
    ///
    /// This implements the same command parsing logic as choose_spell_ability_to_play,
    /// but works with the simplified string-based options used by the network protocol.
    fn choose_from_options(&mut self, options: &[String]) -> usize {
        // Check if we have commands left
        if let Some(command_str) = self.peek_command() {
            let command = command_str.to_string();

            // Check if this is a wildcard separator
            if command.trim() == "*" {
                self.current_index += 1;
                self.wildcard_mode = true;
                return self.choose_from_options(options);
            }

            // Try to parse numeric choice first
            if let Ok(idx) = command.trim().parse::<usize>() {
                self.next_command();
                self.wildcard_mode = false;
                if idx < options.len() {
                    return idx;
                } else {
                    // Invalid index, default to 0
                    return 0;
                }
            }

            // Try to match command text against options
            // Check for pass commands (match "pass priority" option)
            let cmd_lower = command.trim().to_lowercase();
            if cmd_lower == "pass" || cmd_lower == "p" || cmd_lower == "0" {
                self.next_command();
                self.wildcard_mode = false;
                // Find "pass priority" option or return 0
                for (i, opt) in options.iter().enumerate() {
                    if opt.to_lowercase().contains("pass") {
                        return i;
                    }
                }
                return 0;
            }

            // Try to match by option text (e.g., "play island" matching "Play land: Island")
            for (i, opt) in options.iter().enumerate() {
                let opt_lower = opt.to_lowercase();
                // Check if command appears to match this option
                // Handle "play X" -> "Play land: X"
                if let Some(card_name) = cmd_lower.strip_prefix("play ") {
                    if opt_lower.contains("play land") && opt_lower.contains(card_name.trim()) {
                        self.next_command();
                        self.wildcard_mode = false;
                        return i;
                    }
                }
                // Handle "cast X" -> "Cast spell: X" or just "Cast: X"
                if let Some(card_name) = cmd_lower.strip_prefix("cast ") {
                    if (opt_lower.contains("cast spell") || opt_lower.contains("cast:"))
                        && opt_lower.contains(card_name.trim())
                    {
                        self.next_command();
                        self.wildcard_mode = false;
                        return i;
                    }
                }
            }

            // No match found
            if self.wildcard_mode {
                // In wildcard mode, pass priority and keep waiting
                return 0;
            } else {
                // Normal mode - consume command and default to 0
                log::warn!(
                    "Command '{}' did not match any option, defaulting to 0. Options: {:?}",
                    command,
                    options
                );
                self.next_command();
                return 0;
            }
        }

        // No more commands - default to 0 (usually "pass priority")
        0
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        crate::game::snapshot::ControllerType::Tui
    }

    fn wants_context(&self) -> bool {
        true
    }

    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        // RichInputController state is not needed for snapshot restoration
        // (choices are replayed from the choices files, not from controller state).
        // Return None to avoid deserialization errors with ControllerState enum.
        None
    }

    fn has_more_choices(&self) -> bool {
        self.current_index < self.commands.len()
    }
}

// Implement serialization for snapshots
impl serde::Serialize for RichInputController {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("RichInputController", 5)?;
        state.serialize_field("player_id", &self.player_id)?;
        state.serialize_field("commands", &self.commands)?;
        state.serialize_field("current_index", &self.current_index)?;
        state.serialize_field("wildcard_mode", &self.wildcard_mode)?;
        state.serialize_field("pending_target", &self.pending_target)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for RichInputController {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct RichInputControllerData {
            player_id: PlayerId,
            commands: Vec<String>,
            current_index: usize,
            #[serde(default)]
            wildcard_mode: bool,
            #[serde(default)]
            pending_target: Option<String>,
        }

        let data = RichInputControllerData::deserialize(deserializer)?;
        Ok(RichInputController {
            player_id: data.player_id,
            commands: data.commands,
            current_index: data.current_index,
            wildcard_mode: data.wildcard_mode,
            pending_target: data.pending_target,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;
    use crate::game::command_parsing::normalize;
    use crate::game::GameState;

    #[test]
    fn test_normalize() {
        // Tests now use shared normalize function from command_parsing module
        assert_eq!(normalize("Black Knight"), "blackknight");
        assert_eq!(normalize("Serra_Angel"), "serraangel");
        assert_eq!(normalize("Royal  Assassin"), "royalassassin");
    }

    #[test]
    fn test_split_targeting_clause() {
        // No clause: whole command, no target.
        let (cmd, tgt) = RichInputController::split_targeting_clause("cast Lightning Bolt");
        assert_eq!(cmd, "cast Lightning Bolt");
        assert_eq!(tgt, None);

        // Inline clause: spell part + lowercased selector.
        let (cmd, tgt) = RichInputController::split_targeting_clause("cast Lightning Bolt targeting Grizzly Bears");
        assert_eq!(cmd, "cast Lightning Bolt");
        assert_eq!(tgt.as_deref(), Some("grizzly bears"));

        // Case-insensitive keyword; player sentinel selector preserved lowercase.
        let (cmd, tgt) = RichInputController::split_targeting_clause("cast Shock TARGETING p2");
        assert_eq!(cmd, "cast Shock");
        assert_eq!(tgt.as_deref(), Some("p2"));

        // Empty selector after the keyword yields no target (defensive).
        let (cmd, tgt) = RichInputController::split_targeting_clause("cast Bolt targeting ");
        assert_eq!(cmd, "cast Bolt");
        assert_eq!(tgt, None);
    }

    #[test]
    fn test_card_matches() {
        // Tests now use shared card_matches function from command_parsing module
        assert!(card_matches("Black Knight", "black"));
        assert!(card_matches("Black Knight", "blackkn"));
        assert!(card_matches("Serra Angel", "serra"));
        assert!(!card_matches("Black Knight", "white"));
    }

    #[test]
    fn test_numeric_choice() {
        let player_id = EntityId::new(1);
        // Choice "1" selects first ability (available[0]), matching menu format where [0] = Pass
        let mut controller = RichInputController::new(player_id, vec!["1".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let abilities = vec![SpellAbility::PlayLand {
            card_id: EntityId::new(10),
        }];

        let choice = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(choice.unwrap().is_some());
    }

    #[test]
    fn test_numeric_choice_pass() {
        let player_id = EntityId::new(1);
        // Choice "0" means pass priority
        let mut controller = RichInputController::new(player_id, vec!["0".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let abilities = vec![SpellAbility::PlayLand {
            card_id: EntityId::new(10),
        }];

        let choice = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(choice.unwrap().is_none()); // Should pass (return None)
    }

    #[test]
    fn test_pass_command() {
        let player_id = EntityId::new(1);
        let mut controller = RichInputController::new(player_id, vec!["pass".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let choice = controller.choose_spell_ability_to_play(&view, &[]);
        assert!(choice.unwrap().is_none());
    }

    #[test]
    fn test_equip_command() {
        let player_id = EntityId::new(1);
        // Without actual card names in the test view, "equip accorder" won't match and should error
        let mut controller = RichInputController::new(player_id, vec!["equip accorder".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let equipment_id = EntityId::new(10);
        let abilities = vec![SpellAbility::ActivateAbility {
            card_id: equipment_id,
            ability_index: 0,
        }];

        let choice = controller.choose_spell_ability_to_play(&view, &abilities);
        // Without actual card data, this should error
        assert!(matches!(choice, ChoiceResult::Error(_)));
    }

    #[test]
    fn test_activate_command() {
        let player_id = EntityId::new(1);
        // Without actual card names, "activate forest" won't match and should error
        let mut controller = RichInputController::new(player_id, vec!["activate forest".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(20);
        let abilities = vec![SpellAbility::ActivateAbility {
            card_id: land_id,
            ability_index: 0,
        }];

        let choice = controller.choose_spell_ability_to_play(&view, &abilities);
        // Without actual card data, this should error
        assert!(matches!(choice, ChoiceResult::Error(_)));
    }

    #[test]
    fn test_activate_command_with_index() {
        let player_id = EntityId::new(1);
        // Without actual card names, indexed activation should also error
        let mut controller = RichInputController::new(player_id, vec!["activate forest[2]".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(20);
        let abilities = vec![
            SpellAbility::ActivateAbility {
                card_id: land_id,
                ability_index: 0,
            },
            SpellAbility::ActivateAbility {
                card_id: land_id,
                ability_index: 1,
            },
        ];

        let choice = controller.choose_spell_ability_to_play(&view, &abilities);
        // Without actual card data, this should error
        assert!(matches!(choice, ChoiceResult::Error(_)));
    }

    #[test]
    fn test_command_error_on_no_match() {
        let player_id = EntityId::new(1);
        // Command "cast fireball" should error when fireball is not available
        let mut controller = RichInputController::new(player_id, vec!["cast fireball".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        // Should return an error because "cast fireball" doesn't match "play land"
        assert!(matches!(result, ChoiceResult::Error(_)));
        if let ChoiceResult::Error(msg) = result {
            assert!(msg.contains("cast fireball"));
            assert!(msg.contains("did not match"));
        }
    }

    #[test]
    fn test_wildcard_mode_no_error_on_no_match() {
        let player_id = EntityId::new(1);
        // In wildcard mode, "cast fireball" should pass priority (not error) when not available
        let mut controller = RichInputController::new(
            player_id,
            vec!["pass".to_string(), "*".to_string(), "cast fireball".to_string()],
        );
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // First choice: "pass" - should work
        let result1 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(matches!(result1, ChoiceResult::Ok(_)));
        if let ChoiceResult::Ok(choice) = result1 {
            assert!(choice.is_none());
        }

        // Now in wildcard mode, waiting for "cast fireball"
        // Should pass priority without error
        let result2 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(matches!(result2, ChoiceResult::Ok(_)));
        if let ChoiceResult::Ok(choice) = result2 {
            assert!(choice.is_none()); // Passes priority, waiting for fireball
        }
    }

    #[test]
    fn test_wildcard_at_beginning() {
        let player_id = EntityId::new(1);
        // Wildcard at the start means immediately enter wildcard mode
        let mut controller = RichInputController::new(player_id, vec!["*".to_string(), "cast fireball".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // First choice: wildcard mode active, "cast fireball" doesn't match
        // Should pass priority without error
        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(matches!(result, ChoiceResult::Ok(_)));
        if let ChoiceResult::Ok(choice) = result {
            assert!(choice.is_none()); // Passes priority, waiting for fireball
        }

        // Controller should still be in wildcard mode
        assert!(controller.wildcard_mode);
    }

    #[test]
    fn test_first_command_must_match_strictly() {
        let player_id = EntityId::new(1);
        // First command is NOT a wildcard, so it must match strictly
        let mut controller = RichInputController::new(player_id, vec!["cast fireball".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // Should ERROR because "cast fireball" doesn't match and we're not in wildcard mode
        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(matches!(result, ChoiceResult::Error(_)));
        if let ChoiceResult::Error(msg) = result {
            assert!(msg.contains("cast fireball"));
            assert!(msg.contains("did not match"));
        }
    }

    #[test]
    fn test_wildcard_at_beginning_then_match() {
        let player_id = EntityId::new(1);
        // Start with wildcard, then command matches available action
        let mut controller = RichInputController::new(
            player_id,
            vec!["*".to_string(), "0".to_string()], // 0 = pass
        );
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // First choice: wildcard mode, "0" (pass) always matches
        // Should execute the pass and exit wildcard mode
        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(matches!(result, ChoiceResult::Ok(_)));
        if let ChoiceResult::Ok(choice) = result {
            assert!(choice.is_none()); // Pass priority
        }

        // Should have exited wildcard mode after match
        assert!(!controller.wildcard_mode);

        // Should have consumed both wildcard and the "0" command
        assert_eq!(controller.current_index, 2);
    }

    // -----------------------------------------------------------------------
    // PASS_UNTIL integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_pass_until_not_yet_satisfied_passes_priority() {
        let player_id = EntityId::new(1);
        // The game starts at turn 1 (new_two_player defaults), but we wait for turn 3
        let mut controller = RichInputController::new(
            player_id,
            vec!["PASS_UNTIL turn=3,phase=MAIN2".to_string(), "0".to_string()],
        );
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        // Default game starts at turn 1, Untap phase, so PASS_UNTIL turn=3,MAIN2 is NOT met
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // Should pass priority (condition not satisfied yet)
        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(
            matches!(result, ChoiceResult::Ok(None)),
            "PASS_UNTIL should pass priority when condition not met"
        );
        // Command should NOT be consumed (still at index 0)
        assert_eq!(controller.current_index, 0);
    }

    #[test]
    fn test_pass_until_satisfied_consumes_and_proceeds() {
        use crate::game::Step;

        let player_id = EntityId::new(1);
        // Wait for turn 1, Main1 — which is what new_two_player's FIRST actual
        // priority window would be. We set the game state directly to that step.
        let mut controller = RichInputController::new(
            player_id,
            vec!["PASS_UNTIL turn=1,phase=MAIN1".to_string(), "pass".to_string()],
        );
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        // Force the game to turn 1 Main1
        game.turn.turn_number = 1;
        game.turn.current_step = Step::Main1;
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // PASS_UNTIL condition IS satisfied (turn=1, step=Main1)
        // So the directive should be consumed and the next command "pass" should run
        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(
            matches!(result, ChoiceResult::Ok(None)),
            "After PASS_UNTIL consumed, 'pass' should pass priority: {result:?}"
        );
        // Both the PASS_UNTIL directive AND "pass" should have been consumed
        assert_eq!(controller.current_index, 2, "both commands should be consumed");
    }

    #[test]
    fn test_pass_until_malformed_consumes_and_continues() {
        let player_id = EntityId::new(1);
        // A malformed PASS_UNTIL should be consumed (to avoid looping) and not crash
        let mut controller = RichInputController::new(
            player_id,
            vec!["PASS_UNTIL phase=BOGUS".to_string(), "pass".to_string()],
        );
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // Should not panic; the malformed directive is consumed and "pass" runs
        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        // Either passes priority or runs "pass" — either way, ChoiceResult::Ok(None)
        assert!(matches!(result, ChoiceResult::Ok(None | Some(_))));
    }
}
