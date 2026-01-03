//! Interactive TUI controller for human players
//!
//! Reads player choices from stdin and displays game state using GameStateView

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::command_parsing::{card_matches, parse_spell_ability_choice};
use crate::game::controller::{sort_spell_abilities, ChoiceResult, GameStateView, PlayerController};
use smallvec::SmallVec;
use std::io::{self, Write};

/// A controller that prompts a human player for decisions via stdin
pub struct InteractiveController {
    player_id: PlayerId,
    numeric_choices: bool,
    /// Command buffer for semicolon-separated inputs
    command_buffer: Vec<String>,
}

impl InteractiveController {
    /// Create a new interactive controller for the given player
    pub fn new(player_id: PlayerId) -> Self {
        InteractiveController {
            player_id,
            numeric_choices: false,
            command_buffer: Vec::new(),
        }
    }

    /// Create a new interactive controller with numeric choices mode
    pub fn with_numeric_choices(player_id: PlayerId, numeric_choices: bool) -> Self {
        InteractiveController {
            player_id,
            numeric_choices,
            command_buffer: Vec::new(),
        }
    }

    /// Get the next buffered command, or read new input from stdin
    ///
    /// If the input contains semicolons, splits and buffers the remaining commands
    fn get_next_command(&mut self) -> Option<String> {
        // Check if we have buffered commands
        if !self.command_buffer.is_empty() {
            return Some(self.command_buffer.remove(0));
        }
        None
    }

    /// Read and buffer commands from stdin, splitting on semicolons
    fn read_and_buffer_commands(&mut self) -> Result<(), std::io::Error> {
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        // Split on semicolons and buffer all commands
        for cmd in input.split(';') {
            let trimmed = cmd.trim();
            if !trimmed.is_empty() {
                self.command_buffer.push(trimmed.to_string());
            }
        }
        Ok(())
    }

    /// Helper: prompt user for a choice and validate input
    ///
    /// Optionally accepts a GameStateView to enable special informational commands:
    /// - '?' shows help
    /// - 'v' views battlefield
    /// - 'g' views graveyard
    fn get_user_choice(&self, prompt: &str, num_options: usize, allow_pass: bool) -> Option<usize> {
        self.get_user_choice_with_view(prompt, num_options, allow_pass, None)
    }

    /// Helper: prompt user for a choice with optional game state view for info commands
    fn get_user_choice_with_view(
        &self,
        prompt: &str,
        num_options: usize,
        allow_pass: bool,
        view: Option<&GameStateView>,
    ) -> Option<usize> {
        loop {
            print!("{} ", prompt);
            io::stdout().flush().unwrap();

            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                eprintln!("Error reading input");
                continue;
            }

            let trimmed = input.trim();

            // Check for special informational commands (if view is provided)
            if let Some(game_view) = view {
                match trimmed {
                    "?" => {
                        self.display_help();
                        continue; // Re-prompt
                    }
                    "b" => {
                        self.display_battlefield_view(game_view);
                        continue; // Re-prompt
                    }
                    "g" => {
                        self.display_graveyard_view(game_view);
                        continue; // Re-prompt
                    }
                    "v" => {
                        self.display_card_view(game_view);
                        continue; // Re-prompt
                    }
                    _ => {} // Not a special command, continue with normal parsing
                }
            }

            // In non-numeric mode, empty input just re-prompts
            if trimmed.is_empty() {
                if !allow_pass {
                    // In numeric mode, empty = option 0
                    return Some(0);
                }
                // In pass mode, empty just re-prompts
                continue;
            }

            // Check for pass in non-numeric mode (allow_pass: true)
            if allow_pass && (trimmed == "p" || trimmed == "pass") {
                return None;
            }

            // Try to parse as number
            match trimmed.parse::<usize>() {
                Ok(choice) if choice < num_options => return Some(choice),
                _ => {
                    eprintln!(
                        "Invalid choice. Enter 0-{}{}.",
                        num_options - 1,
                        if allow_pass { " or 'p' to pass" } else { "" }
                    );
                }
            }
        }
    }

    /// Display help menu for interactive commands
    fn display_help(&self) {
        println!("\n=== Help ===");
        println!("Available commands:");
        println!("  ?  - Show this help menu");
        println!("  b  - View battlefield");
        println!("  g  - View graveyard");
        println!("  v  - View card details");
        println!("\nGame actions:");
        if self.numeric_choices {
            println!("  Enter a number to choose an action");
            println!("  0  - Pass priority / Skip / Done");
            println!("  Press Enter alone to select option 0");
        } else {
            println!("  Enter a number to choose an action");
            println!("  p  - Pass priority");
            println!("\nRich text commands:");
            println!("  play <card>     - Play a land (e.g., 'play swamp')");
            println!("  cast <card>     - Cast a spell (e.g., 'cast bolt')");
            println!("  activate <card> - Activate an ability");
            println!("\nCard names are case-insensitive and support prefix matching");
            println!("(e.g., 'cast black' matches 'Black Knight')");
        }
        println!();
    }

    /// Display battlefield view
    fn display_battlefield_view(&self, view: &GameStateView) {
        println!("\n=== Battlefield ===");
        let battlefield = view.battlefield();
        if battlefield.is_empty() {
            println!("  (empty)");
        } else {
            for &card_id in battlefield {
                let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
                let tapped = if view.is_tapped(card_id) { " (tapped)" } else { "" };

                // Try to get more info about the card
                if let Some(card) = view.get_card(card_id) {
                    let controller_name = view.get_player_name_by_id(card.controller);
                    let pt = if card.is_creature() {
                        format!(" {}/{}", card.current_power(), card.current_toughness())
                    } else {
                        String::new()
                    };
                    println!("  {} - {}{}{}", controller_name, name, pt, tapped);
                } else {
                    println!("  {}{}", name, tapped);
                }
            }
        }
        println!();
    }

    /// Display graveyard view
    fn display_graveyard_view(&self, view: &GameStateView) {
        println!("\n=== Graveyard ===");

        // Show player's own graveyard
        let player_name = view.player_name();
        println!("{}'s graveyard:", player_name);
        let graveyard = view.graveyard();
        if graveyard.is_empty() {
            println!("  (empty)");
        } else {
            for &card_id in graveyard {
                let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
                println!("  {}", name);
            }
        }

        // Show opponent graveyards
        for opponent_id in view.opponents() {
            println!("\nOpponent graveyard:");
            let opp_graveyard = view.player_graveyard(opponent_id);
            if opp_graveyard.is_empty() {
                println!("  (empty)");
            } else {
                for &card_id in opp_graveyard {
                    let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
                    println!("  {}", name);
                }
            }
        }

        println!();
    }

    /// Display card details view with selectable card menu
    fn display_card_view(&self, view: &GameStateView) {
        println!("\n=== Card Details ===");

        // Collect all distinct cards from hand and battlefield
        let mut card_names_to_ids: std::collections::HashMap<String, Vec<CardId>> = std::collections::HashMap::new();

        // Add cards from hand
        for &card_id in view.hand() {
            if let Some(name) = view.card_name(card_id) {
                card_names_to_ids.entry(name.clone()).or_default().push(card_id);
            }
        }

        // Add cards from battlefield (all players)
        for &card_id in view.battlefield() {
            if let Some(name) = view.card_name(card_id) {
                card_names_to_ids.entry(name.clone()).or_default().push(card_id);
            }
        }

        // Sort card names alphabetically
        let mut card_names: Vec<_> = card_names_to_ids.keys().collect();
        card_names.sort();

        if card_names.is_empty() {
            println!("  No cards visible.");
            println!();
            return;
        }

        // Display menu of cards
        println!("Select a card to view details:");
        for (idx, name) in card_names.iter().enumerate() {
            let count = card_names_to_ids[*name].len();
            let count_str = if count > 1 {
                format!(" (x{})", count)
            } else {
                String::new()
            };
            println!("  [{}] {}{}", idx, name, count_str);
        }

        // Get user selection
        print!("Enter card number (or press Enter to cancel): ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return;
        }

        let trimmed = input.trim();
        if trimmed.is_empty() {
            return; // Canceled
        }

        // Parse selection
        if let Ok(choice) = trimmed.parse::<usize>() {
            if choice < card_names.len() {
                let card_name = card_names[choice];
                if let Some(card_ids) = card_names_to_ids.get(card_name) {
                    // Get the first card with this name to display details
                    if let Some(&card_id) = card_ids.first() {
                        Self::print_card_details(view, card_id);
                    }
                }
            } else {
                eprintln!("Invalid selection: {}", choice);
            }
        } else {
            eprintln!("Invalid input: {}", trimmed);
        }

        println!();
    }

    /// Print detailed information about a card
    ///
    /// This method shares the same formatting logic as the Fancy TUI's Card Details panel
    fn print_card_details(view: &GameStateView, card_id: CardId) {
        if let Some(card) = view.get_card(card_id) {
            println!("\n────────────────────────────────────────");
            println!("{}", card.name);
            println!("────────────────────────────────────────");

            // Card type line
            let types_str = card
                .types
                .iter()
                .map(|t| format!("{:?}", t))
                .collect::<Vec<_>>()
                .join(" ");
            println!("Type: {}", types_str);

            // Mana cost
            println!("Cost: {}", card.mana_cost);

            // Power/Toughness for creatures
            if card.is_creature() {
                println!("P/T: {}/{}", card.current_power(), card.current_toughness());
            }

            // Card text
            if !card.text.is_empty() {
                println!();
                for line in card.text.split('\n') {
                    println!("{}", line);
                }
            }

            println!("────────────────────────────────────────");
        } else {
            println!("  Card not found: {:?}", card_id);
        }
    }

    /// Helper: display a list of cards with indices
    fn display_cards(&self, view: &GameStateView, cards: &[CardId], _prefix: &str) {
        for (idx, &card_id) in cards.iter().enumerate() {
            let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
            let tapped = if view.is_tapped(card_id) { " (tapped)" } else { "" };
            println!("  [{}] {}{}", idx, name, tapped);
        }
    }
}

impl PlayerController for InteractiveController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        if available.is_empty() {
            return ChoiceResult::Ok(None);
        }

        // Sort abilities in canonical order: PlayLand, CastSpell, ActivateAbility
        let sorted = sort_spell_abilities(available);

        // Get player name from view
        let player_name = view.player_name();
        println!(
            "\n  ==> Priority {}: life {}, {:?}",
            player_name,
            view.life(),
            view.current_step()
        );

        if self.numeric_choices {
            // Numeric mode: 0 = Pass, 1-N = actions
            println!("\nAvailable actions:");
            println!("  [0] Pass");
            for (idx, ability) in sorted.iter().enumerate() {
                let desc = crate::game::controller::format_spell_ability_choice(view, ability);
                println!("  [{}] {}", idx + 1, desc);
            }

            let choice_opt = self.get_user_choice_with_view(
                &format!("Enter choice (0-{}, or ? for help):", sorted.len()),
                sorted.len() + 1,
                false,
                Some(view),
            );

            let choice = match choice_opt {
                Some(c) => c,
                None => {
                    // User cancelled or input error - pass priority
                    println!("Passed priority.");
                    view.logger()
                        .controller_choice("TUI", &format!("{} chose to pass priority", player_name));
                    return ChoiceResult::Ok(None);
                }
            };

            if choice == 0 {
                println!("Passed priority.");
                view.logger()
                    .controller_choice("TUI", &format!("{} chose to pass priority", player_name));
                return ChoiceResult::Ok(None); // Pass
            }

            // Acknowledge the chosen action
            let ability = &sorted[choice - 1];
            let desc = crate::game::controller::format_spell_ability_choice(view, ability);
            println!("{}", desc);
            view.logger()
                .controller_choice("TUI", &format!("{} chose {}", player_name, desc));

            ChoiceResult::Ok(Some(sorted[choice - 1].clone()))
        } else {
            // Non-numeric mode: Index 0 = Pass, 1+ = actions, OR rich text commands
            // Use shared format_choice_menu for consistency (it sorts internally)
            print!("{}", crate::game::controller::format_choice_menu(view, &sorted));

            // Read user input and try rich command parsing first
            loop {
                print!("Choose action (0-{}, or ? for help): ", sorted.len());
                io::stdout().flush().unwrap();

                let mut input = String::new();
                if io::stdin().read_line(&mut input).is_err() {
                    eprintln!("Error reading input");
                    continue;
                }

                let trimmed = input.trim();

                // Check for special informational commands
                match trimmed {
                    "?" => {
                        self.display_help();
                        continue; // Re-prompt
                    }
                    "b" => {
                        self.display_battlefield_view(view);
                        continue; // Re-prompt
                    }
                    "g" => {
                        self.display_graveyard_view(view);
                        continue; // Re-prompt
                    }
                    "v" => {
                        self.display_card_view(view);
                        continue; // Re-prompt
                    }
                    _ => {} // Not a special command, continue with parsing
                }

                // Try rich command parsing first (searches by card name, order doesn't matter)
                let rich_result = parse_spell_ability_choice(trimmed, view, &sorted);

                // Check if it was a valid command (pass or ability selection)
                if trimmed == "p"
                    || trimmed == "pass"
                    || trimmed.starts_with("play ")
                    || trimmed.starts_with("cast ")
                    || trimmed.starts_with("activate ")
                {
                    // This is a rich command attempt
                    if let Some(ability) = rich_result {
                        // Found matching ability
                        let desc = crate::game::controller::format_spell_ability_choice(view, &ability);
                        println!("  {} chose {}", player_name, desc);
                        view.logger()
                            .controller_choice("TUI", &format!("{} chose {}", player_name, desc));
                        return ChoiceResult::Ok(Some(ability));
                    } else if trimmed == "p" || trimmed == "pass" {
                        // Explicit pass command
                        println!("  {} passed priority.", player_name);
                        view.logger()
                            .controller_choice("TUI", &format!("{} chose to pass priority", player_name));
                        return ChoiceResult::Ok(None);
                    } else {
                        // Rich command but no match found
                        eprintln!("No matching action found for '{}'. Try again.", trimmed);
                        continue;
                    }
                }

                // Try numeric parsing
                // INVARIANT: Index 0 = Pass, Index 1+ = actions (shifted by 1)
                match trimmed.parse::<usize>() {
                    Ok(0) => {
                        // Index 0 = pass
                        println!("  {} passed priority.", player_name);
                        view.logger()
                            .controller_choice("TUI", &format!("{} chose to pass priority", player_name));
                        return ChoiceResult::Ok(None);
                    }
                    Ok(choice) if choice <= sorted.len() => {
                        // Index 1 to N maps to sorted[0] to sorted[N-1]
                        let action_index = choice - 1;

                        // Acknowledge the chosen action
                        let desc = crate::game::controller::format_spell_ability_choice(view, &sorted[action_index]);
                        println!("  {} chose {}", player_name, desc);
                        view.logger()
                            .controller_choice("TUI", &format!("{} chose {}", player_name, desc));
                        return ChoiceResult::Ok(Some(sorted[action_index].clone()));
                    }
                    _ => {
                        eprintln!(
                            "Invalid choice. Enter 0 to pass, 1-{} for actions, or 'play X', 'cast Y' commands.",
                            sorted.len()
                        );
                    }
                }
            }
        }
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if valid_targets.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        let spell_name = view.card_name(spell).unwrap_or_default();
        println!("\n--- Targeting for: {} ---", spell_name);

        let mut targets = SmallVec::new();

        if self.numeric_choices {
            // Numeric mode: 0 = No target, 1-N = targets
            println!("Valid targets:");
            println!("  [0] No target");
            for (idx, &card_id) in valid_targets.iter().enumerate() {
                let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
                let tapped = if view.is_tapped(card_id) { " (tapped)" } else { "" };
                println!("  [{}] {}{}", idx + 1, name, tapped);
            }

            if let Some(choice) = self.get_user_choice_with_view(
                &format!("Enter choice (0-{}, or ? for help):", valid_targets.len()),
                valid_targets.len() + 1,
                false,
                Some(view),
            ) {
                if choice > 0 {
                    targets.push(valid_targets[choice - 1]);
                }
            }
        } else {
            // Original mode: indices match array, 'p' for no targets
            println!("Valid targets:");
            self.display_cards(view, valid_targets, "  ");

            if let Some(choice) = self.get_user_choice_with_view(
                &format!(
                    "Choose target (0-{}, 'p' for no targets, or ? for help):",
                    valid_targets.len() - 1
                ),
                valid_targets.len(),
                true,
                Some(view),
            ) {
                targets.push(valid_targets[choice]);
            }
        }

        // Log the choice
        if targets.is_empty() {
            view.logger().controller_choice("TUI", "Chose no target");
        } else {
            let target_names: Vec<String> = targets
                .iter()
                .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id)))
                .collect();
            view.logger()
                .controller_choice("TUI", &format!("chose target {}", target_names.join(", ")));
        }

        ChoiceResult::Ok(targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if available_sources.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        println!("\n--- Paying Mana Cost: {} ---", cost);
        println!("Available mana sources:");
        self.display_cards(view, available_sources, "  ");

        let mut sources = SmallVec::new();
        let needed = cost.cmc() as usize;

        if needed == 0 {
            return ChoiceResult::Ok(sources);
        }

        println!("Select {} sources to tap:", needed);
        for i in 0..needed {
            if let Some(choice) = self.get_user_choice(
                &format!(
                    "Choose source ({}/{}), 0-{}:",
                    i + 1,
                    needed,
                    available_sources.len() - 1
                ),
                available_sources.len(),
                false,
            ) {
                sources.push(available_sources[choice]);
            }
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

        // Note: Attacker selection prompt is now printed by game loop before this method is called
        let mut attackers = SmallVec::new();

        if self.numeric_choices {
            // Numeric mode: 0 = Done, 1-N = creatures
            loop {
                println!("Available creatures:");
                println!("  [0] Done selecting attackers");
                for (idx, &card_id) in available_creatures.iter().enumerate() {
                    let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
                    let tapped = if view.is_tapped(card_id) { " (tapped)" } else { "" };
                    let selected = if attackers.contains(&card_id) {
                        " [SELECTED]"
                    } else {
                        ""
                    };
                    println!("  [{}] {}{}{}", idx + 1, name, tapped, selected);
                }

                if let Some(choice) = self.get_user_choice(
                    &format!("Enter choice (0-{}):", available_creatures.len()),
                    available_creatures.len() + 1,
                    false,
                ) {
                    if choice == 0 {
                        break; // Done
                    }
                    let card_id = available_creatures[choice - 1];
                    if !attackers.contains(&card_id) {
                        attackers.push(card_id);
                    }
                } else {
                    break;
                }
            }
        } else {
            // Rich input mode: support "attack X" commands and buffering
            println!("Available creatures:");
            self.display_cards(view, available_creatures, "  ");

            loop {
                // Check if we have a buffered command
                let command = if let Some(cmd) = self.get_next_command() {
                    cmd
                } else {
                    // No buffered commands, read new input
                    println!("\nSelect attackers ('attack X', numeric indices, 'done', or press Enter):");
                    if self.read_and_buffer_commands().is_err() {
                        break;
                    }

                    // Get the first buffered command
                    if let Some(cmd) = self.get_next_command() {
                        cmd
                    } else {
                        // Empty input = done
                        break;
                    }
                };

                let trimmed = command.trim().to_lowercase();

                // Check for "done" or empty
                if trimmed.is_empty() || trimmed == "done" {
                    break;
                }

                // Try parsing as "attack X" command
                if let Some(card_pattern) = trimmed.strip_prefix("attack ") {
                    let mut found = false;
                    for &creature_id in available_creatures {
                        if let Some(card_name) = view.card_name(creature_id) {
                            if card_matches(&card_name, card_pattern) && !attackers.contains(&creature_id) {
                                attackers.push(creature_id);
                                println!("  Attacking with {}", card_name);
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        eprintln!("No matching creature found for '{}'", card_pattern);
                    }
                    continue;
                }

                // Try parsing as numeric index
                if let Ok(idx) = trimmed.parse::<usize>() {
                    if idx < available_creatures.len() {
                        let card_id = available_creatures[idx];
                        if !attackers.contains(&card_id) {
                            attackers.push(card_id);
                            let name = view.card_name(card_id).unwrap_or_default();
                            println!("  Attacking with {}", name);
                        }
                    } else {
                        eprintln!("Invalid index: {}", idx);
                    }
                    continue;
                }

                eprintln!("Invalid command: '{}'. Use 'attack <name>', index, or 'done'.", command);
            }
        }

        // Log the choice
        if attackers.is_empty() {
            view.logger().controller_choice(
                "TUI",
                &format!(
                    "chose not to attack with {} available creatures",
                    available_creatures.len()
                ),
            );
        } else {
            view.logger().controller_choice(
                "TUI",
                &format!(
                    "chose {} attackers from {} available creatures",
                    attackers.len(),
                    available_creatures.len()
                ),
            );
        }

        ChoiceResult::Ok(attackers)
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        if attackers.is_empty() || available_blockers.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Note: Blocker selection prompt is now printed by game loop before this method is called
        let mut blocks = SmallVec::new();

        if self.numeric_choices {
            // Numeric mode: 0 = Skip/Done, 1-N = attackers
            println!("\nFor each blocker, choose which attacker it blocks");
            for (blocker_idx, &blocker_id) in available_blockers.iter().enumerate() {
                let blocker_name = view.card_name(blocker_id).unwrap_or_default();

                println!("\nBlocker: [{}] {}", blocker_idx, blocker_name);
                println!("Block which attacker?");
                println!("  [0] Skip this blocker / Done");
                for (idx, &attacker_id) in attackers.iter().enumerate() {
                    let name = view
                        .card_name(attacker_id)
                        .unwrap_or_else(|| format!("Card {attacker_id:?}"));
                    println!("  [{}] {}", idx + 1, name);
                }

                if let Some(choice) = self.get_user_choice(
                    &format!("Enter choice (0-{}):", attackers.len()),
                    attackers.len() + 1,
                    false,
                ) {
                    if choice == 0 {
                        break; // Done assigning blockers
                    }
                    blocks.push((blocker_id, attackers[choice - 1]));
                } else {
                    break;
                }
            }
        } else {
            // Rich input mode: support "X blocks Y" commands and buffering
            println!("\nSelect blockers ('X blocks Y', numeric syntax, 'done', or press Enter):");

            loop {
                // Check if we have a buffered command
                let command = if let Some(cmd) = self.get_next_command() {
                    cmd
                } else {
                    // No buffered commands, read new input
                    println!("\nEnter blocker assignments:");
                    if self.read_and_buffer_commands().is_err() {
                        break;
                    }

                    // Get the first buffered command
                    if let Some(cmd) = self.get_next_command() {
                        cmd
                    } else {
                        // Empty input = done
                        break;
                    }
                };

                let trimmed = command.trim().to_lowercase();

                // Check for "done" or empty
                if trimmed.is_empty() || trimmed == "done" {
                    break;
                }

                // Try parsing as "X blocks Y" command
                if trimmed.contains(" blocks ") {
                    if let Some(blocks_pos) = trimmed.find(" blocks ") {
                        let blocker_pattern = &trimmed[..blocks_pos];
                        let attacker_pattern = &trimmed[blocks_pos + 8..];

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
                            let blocker_name = view.card_name(blocker).unwrap_or_default();
                            let attacker_name = view.card_name(attacker).unwrap_or_default();
                            println!("  {} blocks {}", blocker_name, attacker_name);
                        } else {
                            eprintln!("Could not find matching blocker or attacker for '{}'", command);
                        }
                    }
                    continue;
                }

                // Try parsing as numeric syntax (blocker_idx attacker_idx)
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() == 2 {
                    if let (Ok(blocker_idx), Ok(attacker_idx)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>())
                    {
                        if blocker_idx < available_blockers.len() && attacker_idx < attackers.len() {
                            let blocker_id = available_blockers[blocker_idx];
                            let attacker_id = attackers[attacker_idx];
                            blocks.push((blocker_id, attacker_id));
                            let blocker_name = view.card_name(blocker_id).unwrap_or_default();
                            let attacker_name = view.card_name(attacker_id).unwrap_or_default();
                            println!("  {} blocks {}", blocker_name, attacker_name);
                        } else {
                            eprintln!("Invalid indices: {} {}", blocker_idx, attacker_idx);
                        }
                        continue;
                    }
                }

                eprintln!(
                    "Invalid command: '{}'. Use 'X blocks Y', 'blocker_idx attacker_idx', or 'done'.",
                    command
                );
            }
        }

        // Log the choice
        if blocks.is_empty() {
            view.logger().controller_choice(
                "TUI",
                &format!(
                    "chose not to block (no favorable blocks among {} blockers vs {} attackers)",
                    available_blockers.len(),
                    attackers.len()
                ),
            );
        } else {
            view.logger().controller_choice(
                "TUI",
                &format!("chose {} blockers for {} attackers", blocks.len(), attackers.len()),
            );
        }

        ChoiceResult::Ok(blocks)
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if blockers.len() <= 1 {
            return ChoiceResult::Ok(blockers.iter().copied().collect());
        }

        println!("\n--- Damage Assignment Order ---");

        let attacker_name = view.card_name(attacker).unwrap_or_default();
        println!("Attacker: {}", attacker_name);

        println!("\nBlockers (choose damage assignment order):");
        self.display_cards(view, blockers, "  ");

        let mut ordered: SmallVec<[CardId; 4]> = SmallVec::new();

        if self.numeric_choices {
            // Numeric mode: loop and ask one at a time
            for i in 0..blockers.len() {
                // Show remaining blockers
                let remaining: Vec<_> = blockers
                    .iter()
                    .enumerate()
                    .filter(|(_, &b)| !ordered.contains(&b))
                    .collect();

                if remaining.is_empty() {
                    break;
                }

                println!(
                    "\nChoose blocker {} of {} (remaining: {}):",
                    i + 1,
                    blockers.len(),
                    remaining.len()
                );
                for (idx, _) in &remaining {
                    let name = view.card_name(blockers[*idx]).unwrap_or_default();
                    println!("  [{}] {}", idx, name);
                }

                if let Some(choice) = self.get_user_choice(
                    &format!("Choose blocker (0-{}):", blockers.len() - 1),
                    blockers.len(),
                    false,
                ) {
                    let card_id = blockers[choice];
                    if !ordered.contains(&card_id) {
                        ordered.push(card_id);
                    }
                }
            }
        } else {
            // Original mode: space-separated input
            println!("\nEnter blocker indices in order of damage assignment");
            println!("(separated by space):");

            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                return ChoiceResult::Ok(blockers.iter().copied().collect());
            }

            for index_str in input.split_whitespace() {
                if let Ok(idx) = index_str.parse::<usize>() {
                    if idx < blockers.len() {
                        ordered.push(blockers[idx]);
                    }
                }
            }
        }

        // If user didn't specify all blockers, add remaining in original order
        for &blocker in blockers {
            if !ordered.contains(&blocker) {
                ordered.push(blocker);
            }
        }

        ChoiceResult::Ok(ordered)
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Note: Discard selection prompt is now printed by game loop before this method is called
        let mut discards = SmallVec::new();

        if self.numeric_choices {
            // Numeric mode: loop and ask one at a time
            for i in 0..count {
                if let Some(choice) = self.get_user_choice(
                    &format!("Choose card to discard ({}/{}, 0-{}):", i + 1, count, hand.len() - 1),
                    hand.len(),
                    false,
                ) {
                    let card_id = hand[choice];
                    if !discards.contains(&card_id) {
                        discards.push(card_id);
                    } else {
                        eprintln!("Card already selected for discard, choose another.");
                        // Don't increment i, retry this selection
                    }
                }
            }
        } else {
            // Original mode: space-separated input
            println!("\nSelect cards to discard (enter indices separated by space):");

            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                // Auto-discard first N cards if input fails
                return ChoiceResult::Ok(hand.iter().take(count).copied().collect());
            }

            for index_str in input.split_whitespace() {
                if let Ok(idx) = index_str.parse::<usize>() {
                    if idx < hand.len() && discards.len() < count {
                        discards.push(hand[idx]);
                    }
                }
            }
        }

        // If not enough cards selected, auto-select from beginning
        if discards.len() < count {
            for &card in hand {
                if discards.len() < count && !discards.contains(&card) {
                    discards.push(card);
                }
            }
        }

        ChoiceResult::Ok(discards)
    }

    fn choose_from_library(&mut self, view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        // Interactive: Show library and let user choose
        if valid_cards.is_empty() {
            println!("\nLibrary search: No valid cards found.");
            return ChoiceResult::Ok(None);
        }

        println!("\n=== Library Search ===");
        println!("Choose a card from your library (or enter 'n' to fail to find):");
        for (i, &card_id) in valid_cards.iter().enumerate() {
            if let Some(card_name) = view.get_card_name(card_id) {
                let card = view.get_card(card_id);
                if let Some(c) = card {
                    // Show card details
                    let mana_str = c.mana_cost.to_string();
                    let type_str = c.types.iter().map(|t| format!("{:?}", t)).collect::<Vec<_>>().join(" ");

                    if c.is_creature() {
                        let power_str = c.base_power().map(|p| p.to_string()).unwrap_or("*".to_string());
                        let toughness_str = c.base_toughness().map(|t| t.to_string()).unwrap_or("*".to_string());
                        println!("  [{}] {} {} - {}/{}", i, mana_str, card_name, power_str, toughness_str);
                        println!("       Type: {}", type_str);
                    } else {
                        println!("  [{}] {} {}", i, mana_str, card_name);
                        println!("       Type: {}", type_str);
                    }
                } else {
                    println!("  [{}] {}", i, card_name);
                }
            }
        }

        println!("\nEnter choice (0-{}, or 'n' to fail to find):", valid_cards.len() - 1);

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            // On input error, auto-select first card
            return ChoiceResult::Ok(Some(valid_cards[0]));
        }

        let trimmed = input.trim();

        // Check for fail to find
        if trimmed.eq_ignore_ascii_case("n") || trimmed.eq_ignore_ascii_case("no") {
            println!("Failed to find.");
            return ChoiceResult::Ok(None);
        }

        // Parse numeric choice
        if let Ok(idx) = trimmed.parse::<usize>() {
            if idx < valid_cards.len() {
                let chosen = valid_cards[idx];
                if let Some(card_name) = view.get_card_name(chosen) {
                    println!("Selected: {}", card_name);
                }
                return ChoiceResult::Ok(Some(chosen));
            }
        }

        // Invalid input - auto-select first card
        println!("Invalid choice, auto-selecting first card.");
        ChoiceResult::Ok(Some(valid_cards[0]))
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        use std::io;

        if valid_permanents.is_empty() || count == 0 {
            return ChoiceResult::Ok(SmallVec::new());
        }

        println!("\n=== Sacrifice {} ===", card_type_description);
        println!("You must sacrifice {} {}:", count, card_type_description);

        let mut sacrifices: SmallVec<[CardId; 8]> = SmallVec::new();

        while sacrifices.len() < count && sacrifices.len() < valid_permanents.len() {
            println!(
                "\nChoose a {} to sacrifice ({} remaining):",
                card_type_description,
                count - sacrifices.len()
            );

            // Show available permanents
            let available: Vec<_> = valid_permanents
                .iter()
                .filter(|&card_id| !sacrifices.contains(card_id))
                .collect();

            for (i, &&card_id) in available.iter().enumerate() {
                if let Some(card_name) = view.get_card_name(card_id) {
                    println!("  [{}] {}", i, card_name);
                } else {
                    println!("  [{}] {:?}", i, card_id);
                }
            }

            print!("Enter choice (0-{}): ", available.len().saturating_sub(1));
            let _ = io::Write::flush(&mut io::stdout());

            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                // On input error, auto-select first available
                if let Some(&&card_id) = available.first() {
                    sacrifices.push(card_id);
                }
                continue;
            }

            let trimmed = input.trim();
            if let Ok(idx) = trimmed.parse::<usize>() {
                if idx < available.len() {
                    sacrifices.push(*available[idx]);
                    continue;
                }
            }

            println!("Invalid choice, please try again.");
        }

        ChoiceResult::Ok(sacrifices)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Interactive selection for which permanents to keep tapped
        if may_not_untap_permanents.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        println!("\n=== Untap Step ===");
        println!("The following permanents have 'You may choose not to untap':");
        for (idx, &card_id) in may_not_untap_permanents.iter().enumerate() {
            let name = view.get_card_name(card_id).unwrap_or_else(|| "Unknown".to_string());
            println!("  [{}] {}", idx, name);
        }

        let mut stay_tapped = SmallVec::new();
        println!("\nEnter indices of permanents to KEEP TAPPED (space-separated), or press Enter to untap all:");

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let input = input.trim();
            if !input.is_empty() {
                for part in input.split_whitespace() {
                    if let Ok(idx) = part.parse::<usize>() {
                        if idx < may_not_untap_permanents.len() {
                            stay_tapped.push(may_not_untap_permanents[idx]);
                        }
                    }
                }
            }
        }

        ChoiceResult::Ok(stay_tapped)
    }

    fn choose_modes(
        &mut self,
        view: &GameStateView,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // Interactive mode selection
        if mode_descriptions.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        let spell_name = view
            .get_card_name(spell_id)
            .unwrap_or_else(|| "Unknown Spell".to_string());
        println!(
            "\n=== Choose {} Mode{} for {} ===",
            mode_count,
            if mode_count > 1 { "s" } else { "" },
            spell_name
        );
        println!("Minimum modes required: {}", min_modes);

        for (idx, desc) in mode_descriptions.iter().enumerate() {
            println!("  [{}] {}", idx, desc);
        }

        let mut chosen = SmallVec::new();
        println!("\nEnter mode indices (space-separated):");

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let input = input.trim();
            for part in input.split_whitespace() {
                if let Ok(idx) = part.parse::<usize>() {
                    if idx < mode_descriptions.len() && chosen.len() < mode_count {
                        chosen.push(idx);
                    }
                }
            }
        }

        // If no modes chosen and min_modes > 0, default to first modes
        while chosen.len() < min_modes {
            for i in 0..mode_descriptions.len() {
                if !chosen.contains(&i) {
                    chosen.push(i);
                    break;
                }
            }
        }

        ChoiceResult::Ok(chosen)
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Optional: log when player passes
    }

    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        let player_name = view.player_name();
        println!("\n=== Game Over ===");
        println!("{} {}", player_name, if won { "WON!" } else { "LOST!" });
        println!("Final life total: {}", view.life());
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        crate::game::snapshot::ControllerType::Tui
    }
}
