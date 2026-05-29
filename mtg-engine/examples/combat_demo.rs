// TODO(mtg-211): Remove once wildcard patterns are audited
#![allow(clippy::wildcard_enum_match_arm)]
//! Combat Demonstration Example
//!
//! Demonstrates the complete combat system using the game loop and controllers.
//! Shows how combat integrates with the full game engine:
//! - Custom controllers for Alice and Bob
//! - Declaring attackers through the game loop
//! - Declaring blockers through the game loop
//! - Automatic combat damage assignment
//! - Creature death from combat damage
//!
//! Uses classic cards from Limited/Alpha/Beta/4th Edition

use mtg_engine::core::{Card, CardId, CardType, EntityId, ManaCost, Player, PlayerId, SpellAbility};
use mtg_engine::game::controller::ChoiceResult;
use mtg_engine::game::controller::PlayerController;
use mtg_engine::game::{GameLoop, GameState, GameStateView, Step};
use smallvec::SmallVec;

/// Alice's controller - attacks with all creatures
struct AliceController {
    player_id: PlayerId,
    creatures_to_attack: Vec<CardId>,
}

impl AliceController {
    fn new(player_id: PlayerId) -> Self {
        AliceController {
            player_id,
            creatures_to_attack: Vec::new(),
        }
    }

    fn set_creatures(&mut self, creatures: Vec<CardId>) {
        self.creatures_to_attack = creatures;
    }
}

impl PlayerController for AliceController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        _available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        ChoiceResult::Ok(None) // Alice doesn't take actions in this demo (combat-only)
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        _valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        ChoiceResult::Ok(SmallVec::new()) // Alice doesn't cast spells in this demo
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        _cost: &ManaCost,
        _available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(SmallVec::new()) // Alice doesn't tap for mana in this demo
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Attack with all our creatures that are in the list
        let mut attackers = SmallVec::new();
        for &creature_id in available_creatures {
            if self.creatures_to_attack.contains(&creature_id) {
                println!(
                    "  Alice declares {} as attacker",
                    view.get_card_name(creature_id).unwrap_or_else(|| "Unknown".to_string())
                );
                attackers.push(creature_id);
            }
        }
        if !attackers.is_empty() {
            println!("  Alice finishes declaring attackers");
        }
        ChoiceResult::Ok(attackers)
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        _available_blockers: &[CardId],
        _attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        ChoiceResult::Ok(SmallVec::new()) // Alice doesn't block in this demo
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Keep blockers in the order they were provided
        ChoiceResult::Ok(blockers.iter().copied().collect())
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Alice discards the first N cards in hand
        ChoiceResult::Ok(hand.iter().take(count).copied().collect())
    }

    fn choose_from_library(
        &mut self,
        _view: &GameStateView,
        valid_cards: &[&mtg_engine::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        // Alice picks the first valid card if available
        ChoiceResult::Ok(if valid_cards.is_empty() { None } else { Some(0) })
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Alice sacrifices the first N permanents
        ChoiceResult::Ok(valid_permanents.iter().take(count).copied().collect())
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Alice untaps everything
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
        // Default to first N modes
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> mtg_engine::game::ControllerType {
        mtg_engine::game::ControllerType::Zero
    }
}

/// Bob's controller - blocks with specific assignments
struct BobController {
    player_id: PlayerId,
    blocking_assignments: Vec<(CardId, CardId)>, // (blocker, attacker)
}

impl BobController {
    fn new(player_id: PlayerId) -> Self {
        BobController {
            player_id,
            blocking_assignments: Vec::new(),
        }
    }

    fn set_blocks(&mut self, blocks: Vec<(CardId, CardId)>) {
        self.blocking_assignments = blocks;
    }
}

impl PlayerController for BobController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        _available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        ChoiceResult::Ok(None) // Bob doesn't take actions in this demo (combat-only)
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        _valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        ChoiceResult::Ok(SmallVec::new()) // Bob doesn't cast spells in this demo
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        _cost: &ManaCost,
        _available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(SmallVec::new()) // Bob doesn't tap for mana in this demo
    }

    fn choose_attackers(
        &mut self,
        _view: &GameStateView,
        _available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(SmallVec::new()) // Bob doesn't attack in this demo
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        _attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Block according to our plan
        let mut blocks = SmallVec::new();
        for (blocker_id, attacker_id) in &self.blocking_assignments {
            // Check if blocker is available
            if available_blockers.contains(blocker_id) {
                println!(
                    "  Bob: {} blocks {}",
                    view.get_card_name(*blocker_id).unwrap_or_else(|| "Unknown".to_string()),
                    view.get_card_name(*attacker_id)
                        .unwrap_or_else(|| "Unknown".to_string())
                );
                blocks.push((*blocker_id, *attacker_id));
            }
        }
        if !blocks.is_empty() {
            println!("  Bob finishes declaring blockers");
        }
        ChoiceResult::Ok(blocks)
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Keep blockers in the order they were provided
        ChoiceResult::Ok(blockers.iter().copied().collect())
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Bob discards the first N cards in hand
        ChoiceResult::Ok(hand.iter().take(count).copied().collect())
    }

    fn choose_from_library(
        &mut self,
        _view: &GameStateView,
        valid_cards: &[&mtg_engine::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        // Bob picks the first valid card if available
        ChoiceResult::Ok(if valid_cards.is_empty() { None } else { Some(0) })
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Bob sacrifices the first N permanents
        ChoiceResult::Ok(valid_permanents.iter().take(count).copied().collect())
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Bob untaps everything
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
        // Default to first N modes
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> mtg_engine::game::ControllerType {
        mtg_engine::game::ControllerType::Zero
    }
}

fn main() {
    println!("=== MTG Forge - Combat System Demo ===\n");
    println!("Demonstrating:");
    println!("  - Game loop integration with combat");
    println!("  - Custom controllers for combat decisions");
    println!("  - Declaring attackers and blockers");
    println!("  - Combat damage and creature death");
    println!("  - Using classic 4ED cards\n");

    // Create a two-player game
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);

    let alice = game.players[0].id;
    let bob = game.players[1].id;

    println!("=== Game Setup ===");
    println!("Alice: 20 life (will be attacking)");
    println!("Bob: 20 life (will be defending)\n");

    // Create creatures for Alice (the attacker)
    println!("=== Creating Creatures ===");

    // Alice gets a Grizzly Bears (2/2) and a Gray Ogre (2/2)
    let bears_id = create_creature(&mut game, alice, "Grizzly Bears", 2, 2, "Alice's 2/2 creature [4ED]");
    let ogre_id = create_creature(&mut game, alice, "Gray Ogre", 2, 2, "Alice's 2/2 creature [4ED]");

    // Bob gets a Wall of Stone (0/8) and a Hill Giant (3/3)
    let wall_id = create_creature(&mut game, bob, "Wall of Stone", 0, 8, "Bob's 0/8 defender [4ED]");
    let giant_id = create_creature(&mut game, bob, "Hill Giant", 3, 3, "Bob's 3/3 creature [4ED]");

    println!();
    print_battlefield(&game, alice, bob);

    // Set up controllers with combat plans
    let mut alice_controller = AliceController::new(alice);
    alice_controller.set_creatures(vec![bears_id, ogre_id]);

    let mut bob_controller = BobController::new(bob);
    bob_controller.set_blocks(vec![
        (wall_id, bears_id), // Wall blocks Bears
        (giant_id, ogre_id), // Giant blocks Ogre
    ]);

    println!("\n=== Combat Plan ===");
    println!("Alice will attack with:");
    println!("  - Grizzly Bears (2/2)");
    println!("  - Gray Ogre (2/2)");
    println!("\nBob will block:");
    println!("  - Wall of Stone (0/8) blocks Grizzly Bears");
    println!("  - Hill Giant (3/3) blocks Gray Ogre");

    // Run the game loop through combat
    println!("\n=== Starting Combat Phase ===");
    println!("(Game loop will coordinate the combat steps)\n");

    // Advance to combat phase
    // We need to skip to Alice's turn, main phase 1, then enter combat
    game.turn.current_step = Step::Main1;
    game.turn.active_player = alice;

    let mut game_loop = GameLoop::new(&mut game);

    // Execute one step at a time through combat
    let steps_to_run = vec![
        Step::BeginCombat,
        Step::DeclareAttackers,
        Step::DeclareBlockers,
        Step::CombatDamage,
        Step::EndCombat,
    ];

    for expected_step in steps_to_run {
        // Advance to next step
        game_loop.game.turn.current_step = expected_step;

        println!("--- {} ---", step_name(expected_step));

        // Execute the step
        match game_loop.execute_step(&mut alice_controller, &mut bob_controller) {
            Ok(_) => {}
            Err(e) => {
                println!("Error executing step: {e:?}");
                return;
            }
        }

        // Show what happened based on the step
        match expected_step {
            Step::BeginCombat => {
                println!("  Players receive priority before attackers are declared");
            }
            Step::DeclareAttackers => {
                println!();
                let attackers = game_loop.game.combat.get_attackers();
                if attackers.is_empty() {
                    println!("  No attackers declared");
                } else {
                    println!("  Attacking: {} creature(s)", attackers.len());
                    for attacker in &attackers {
                        if let Ok(card) = game_loop.game.cards.get(*attacker) {
                            println!(
                                "    - {} ({}/{})",
                                card.name,
                                card.current_power(),
                                card.current_toughness()
                            );
                        }
                    }
                }
            }
            Step::DeclareBlockers => {
                println!();
                let blockers = game_loop.game.combat.get_blockers_list();
                if blockers.is_empty() {
                    println!("  No blockers declared");
                } else {
                    println!("  Blocking: {} creature(s)", blockers.len());
                    for blocker_id in &blockers {
                        if let Ok(blocker) = game_loop.game.cards.get(*blocker_id) {
                            println!(
                                "    - {} ({}/{}) is blocking",
                                blocker.name,
                                blocker.current_power(),
                                blocker.current_toughness()
                            );
                        }
                    }
                }
            }
            Step::CombatDamage => {
                println!("  Combat damage dealt:");
                println!("    Grizzly Bears (2/2) vs Wall of Stone (0/8):");
                println!("      - Bears deals 2 damage to Wall");
                println!("      - Wall deals 0 damage to Bears");
                println!("      - Result: Both survive");
                println!();
                println!("    Gray Ogre (2/2) vs Hill Giant (3/3):");
                println!("      - Ogre deals 2 damage to Giant");
                println!("      - Giant deals 3 damage to Ogre");
                println!("      - Result: Ogre dies (3 >= 2 toughness) 💀");
            }
            Step::EndCombat => {
                println!("  Combat phase ends, creatures removed from combat");
            }
            _ => {}
        }

        println!();
    }

    println!("=== After Combat ===");
    print_battlefield(game_loop.game, alice, bob);

    // Check results
    let alice_life = game_loop.game.players[alice.as_u32() as usize].life;
    let bob_life = game_loop.game.players[bob.as_u32() as usize].life;

    println!("\n=== Final State ===");
    println!("Alice: {alice_life} life (no damage - all attackers blocked)");
    println!("Bob: {bob_life} life (no damage - all attackers blocked)");

    // Verify expected outcome
    println!("\n=== Verification ===");
    let mut success = true;

    // Gray Ogre should be dead
    if let Some(zones) = game_loop.game.get_player_zones(alice) {
        if zones.graveyard.contains(ogre_id) {
            println!("✓ Gray Ogre died from combat damage");
        } else {
            println!("✗ Gray Ogre should be in graveyard");
            success = false;
        }
    }

    // Grizzly Bears should still be alive
    if game_loop.game.battlefield.contains(bears_id) {
        println!("✓ Grizzly Bears survived");
    } else {
        println!("✗ Grizzly Bears should be alive");
        success = false;
    }

    // Wall should still be alive
    if game_loop.game.battlefield.contains(wall_id) {
        println!("✓ Wall of Stone survived");
    } else {
        println!("✗ Wall of Stone should be alive");
        success = false;
    }

    // Hill Giant should still be alive
    if game_loop.game.battlefield.contains(giant_id) {
        println!("✓ Hill Giant survived");
    } else {
        println!("✗ Hill Giant should be alive");
        success = false;
    }

    // No player damage
    if alice_life == 20 && bob_life == 20 {
        println!("✓ Life totals unchanged (all blocked)");
    } else {
        println!("✗ Life totals should be 20 each");
        success = false;
    }

    println!("\n=== Combat Demo Complete ===");
    if success {
        println!("✅ All combat mechanics working correctly!");
        println!("\nKey architecture demonstrated:");
        println!("  ✓ Game loop coordinates combat steps");
        println!("  ✓ Controllers make combat decisions");
        println!("  ✓ Attackers declared via controller");
        println!("  ✓ Blockers declared via controller");
        println!("  ✓ Engine handles damage automatically");
        println!("  ✓ State-based actions (creature death)");
    } else {
        println!("❌ Some verifications failed");
        std::process::exit(1);
    }
}

/// Create a creature and add it to the battlefield
fn create_creature(
    game: &mut GameState,
    owner: PlayerId,
    name: &str,
    power: i8,
    toughness: i8,
    description: &str,
) -> CardId {
    let card_id = game.next_card_id();
    let mut card = Card::new(card_id, name.to_string(), owner);
    card.add_type(CardType::Creature);
    card.set_base_power(Some(power));
    card.set_base_toughness(Some(toughness));
    card.controller = owner;
    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);

    println!("  {description}: {name} ({power}/{toughness})");

    card_id
}

/// Print the current battlefield state
fn print_battlefield(game: &GameState, alice: EntityId<Player>, bob: EntityId<Player>) {
    println!("Battlefield:");

    // Alice's creatures
    print!("  Alice: ");
    let alice_creatures: Vec<_> = game
        .battlefield
        .cards
        .iter()
        .filter_map(|&id| {
            game.cards.get(id).ok().and_then(|card| {
                if card.controller == alice && card.is_creature() {
                    Some((
                        card.name.as_str(),
                        card.current_power(),
                        card.current_toughness(),
                        card.tapped,
                    ))
                } else {
                    None
                }
            })
        })
        .collect();

    if alice_creatures.is_empty() {
        println!("(no creatures)");
    } else {
        for (i, (name, pow, tou, tapped)) in alice_creatures.iter().enumerate() {
            if i > 0 {
                print!(", ");
            }
            let tap_str = if *tapped { " (tapped)" } else { "" };
            print!("{name} ({pow}/{tou}){tap_str}");
        }
        println!();
    }

    // Bob's creatures
    print!("  Bob: ");
    let bob_creatures: Vec<_> = game
        .battlefield
        .cards
        .iter()
        .filter_map(|&id| {
            game.cards.get(id).ok().and_then(|card| {
                if card.controller == bob && card.is_creature() {
                    Some((
                        card.name.as_str(),
                        card.current_power(),
                        card.current_toughness(),
                        card.tapped,
                    ))
                } else {
                    None
                }
            })
        })
        .collect();

    if bob_creatures.is_empty() {
        println!("(no creatures)");
    } else {
        for (i, (name, pow, tou, tapped)) in bob_creatures.iter().enumerate() {
            if i > 0 {
                print!(", ");
            }
            let tap_str = if *tapped { " (tapped)" } else { "" };
            print!("{name} ({pow}/{tou}){tap_str}");
        }
        println!();
    }
}

fn step_name(step: Step) -> &'static str {
    match step {
        Step::Untap => "Untap Step",
        Step::Upkeep => "Upkeep Step",
        Step::Draw => "Draw Step",
        Step::Main1 => "Main Phase 1",
        Step::BeginCombat => "Beginning of Combat Step",
        Step::DeclareAttackers => "Declare Attackers Step",
        Step::DeclareBlockers => "Declare Blockers Step",
        Step::CombatDamage => "Combat Damage Step",
        Step::EndCombat => "End of Combat Step",
        Step::Main2 => "Main Phase 2",
        Step::End => "End Step",
        Step::Cleanup => "Cleanup Step",
    }
}
