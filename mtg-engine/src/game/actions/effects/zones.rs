//! Zone-movement effect-family handlers extracted from the `execute_effect`
//! dispatcher (see `game/actions/mod.rs`).
//!
//! This module will eventually hold the whole zone-change family
//! (Destroy/Exile/Return/Sacrifice/Search/ChangeZoneAll/Balance/Dig). It starts
//! with [`Effect::Dig`] — the "look at top N, keep some, put the rest
//! elsewhere" effect — because that extraction is the structural prerequisite
//! for the mtg-908 network-desync fix.
//!
//! ## mtg-677/mtg-908 fix (READ BEFORE editing `execute_dig`)
//!
//! The `execute_dig` self-dig path previously used an inline AI heuristic
//! ([`GameState::dig_card_score`]) that peeked at actual (potentially hidden)
//! library card identities to rank which cards to keep. On a network game the
//! server scores the real top-N while each client shadow scores its own
//! (different) hidden view of the top-N, so they pick different cards →
//! fatal state-hash desync (mtg-908: user's 2025 04-vs-02 game died at
//! turn 13 this way).
//!
//! The fix (mtg-677/mtg-908, DONE HERE) follows the same broadcast pattern
//! as `LibraryReordered` (the scry/surveil fix from mtg-420):
//!
//! - **Server**: runs the heuristic, records the kept-list in
//!   `sub_action_scratch.pending_dig_decisions`, which `NetworkController`
//!   drains into a `ChoiceRequest`. The coordinator broadcasts it to BOTH
//!   clients as `ServerMessage::DigDecision` before the choice.
//! - **Shadow**: `apply_state_sync_at` / `apply_state_sync_up_to_frontier`
//!   delivers the kept-list via `StateSyncEntry::DigDecision` into
//!   `sub_action_scratch.pending_dig_authoritative_decision`. `execute_dig`
//!   checks that field FIRST and uses it if present, bypassing the heuristic.
//! - **Non-network / local AI** games: neither field is set; the heuristic
//!   runs as before (hidden info is acceptable when there is no network shadow).

use crate::core::{Card, CardId, DigFilter, PlayerId};
use crate::game::GameState;
use crate::zones::Zone;
use crate::Result;

/// Check whether a card matches a `RevealValid$` filter string.
///
/// Filter syntax: `"Card.Artifact+YouCtrl"`, `"Card.Creature"`, etc.
/// We extract the first dot-delimited segment after `Card.` (if present) and
/// check it against the card's types.  `+YouCtrl` and similar qualifiers are
/// always true here (we only reveal cards from the controlling player's own hand).
///
/// Returns `true` for the catch-all filter `"Card"` (no type restriction).
fn card_matches_reveal_filter(card: &Card, filter: &str) -> bool {
    // Normalise: drop "Card." prefix, take just the type segment before "+".
    let type_part = filter
        .strip_prefix("Card.")
        .unwrap_or(filter)
        .split('+')
        .next()
        .unwrap_or(filter);

    if type_part.is_empty() || type_part == "Card" {
        return true; // No type restriction.
    }

    // Match against card type names (case-insensitive).
    card.types
        .iter()
        .any(|t| format!("{t:?}").eq_ignore_ascii_case(type_part))
}

impl GameState {
    /// [`Effect::Dig`]: look at the top `dig_count` cards of a library and move
    /// some to `destination`, the rest to `rest_destination`.
    ///
    /// Two patterns:
    /// 1. `target_self` (Impulse, Stock Up, Wrenn and Seven): look at the top N
    ///    of YOUR library, select up to `change_count` cards matching
    ///    `change_valid` to `destination`, put the rest at `rest_destination`.
    /// 2. `!target_self` (Fire Lord Ozai, Xander's Pact): exile the top N from
    ///    each opponent's library.
    ///
    /// Network-safety: on a shadow game (mtg-677/mtg-908) the server's kept-list
    /// arrives via `sub_action_scratch.pending_dig_authoritative_decision` (set by
    /// `apply_state_sync_at` before the shadow reaches this call) and is used
    /// instead of the hidden-info `dig_card_score` heuristic. On a server or local
    /// AI game the heuristic runs and the result is recorded in
    /// `sub_action_scratch.pending_dig_decisions` for broadcast. See module doc.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::actions) fn execute_dig(
        &mut self,
        dig_count: u8,
        change_count: u8,
        change_all: bool,
        destination: Zone,
        rest_destination: Zone,
        may_play: bool,
        may_play_without_mana_cost: bool,
        target_self: bool,
        optional: bool,
        rest_random: bool,
        reveal: bool,
        change_valid: &[DigFilter],
    ) -> Result<()> {
        let digger = self.turn.active_player;
        let mut moved_cards: Vec<CardId> = Vec::with_capacity(dig_count as usize);

        if target_self {
            // Self-dig: look at top N cards of YOUR library
            let library = self
                .player_zones
                .iter()
                .find(|(id, _)| *id == digger)
                .map(|(_, zones)| &zones.library);

            if let Some(library) = library {
                let take_count = dig_count as usize;
                // Library top is at the end of the Vec, so use .rev()
                let card_ids: smallvec::SmallVec<[CardId; 8]> =
                    library.cards.iter().rev().take(take_count).copied().collect();

                let digger_name = self.get_player(digger)?.name.to_string();

                if !card_ids.is_empty() {
                    let verb = if reveal { "reveals" } else { "looks at" };
                    self.logger.gamelog(&format!(
                        "{} {} the top {} card{} of their library",
                        digger_name,
                        verb,
                        card_ids.len(),
                        if card_ids.len() == 1 { "" } else { "s" }
                    ));
                }

                // Separate cards into valid (matchable) and invalid (rest)
                // If change_valid is empty, all cards are valid
                let has_filter = !change_valid.is_empty();
                let mut valid_ids: smallvec::SmallVec<[CardId; 8]> = smallvec::SmallVec::new();
                let mut invalid_ids: smallvec::SmallVec<[CardId; 8]> = smallvec::SmallVec::new();

                for &card_id in &card_ids {
                    if has_filter {
                        let matches = self
                            .cards
                            .try_get(card_id)
                            .is_some_and(|card| change_valid.iter().any(|f| f.matches(card)));
                        if matches {
                            valid_ids.push(card_id);
                        } else {
                            invalid_ids.push(card_id);
                        }
                    } else {
                        valid_ids.push(card_id);
                    }
                }

                // Determine how many cards to select
                let max_select = if change_all {
                    valid_ids.len()
                } else {
                    (change_count as usize).min(valid_ids.len())
                };

                // --- MTG-677/MTG-908: Authoritative Dig decision ---
                //
                // On a SHADOW game, the server has already pushed the
                // authoritative kept-list via `StateSyncEntry::DigDecision`
                // which `apply_state_sync_at` stored in
                // `sub_action_scratch.pending_dig_authoritative_decision`.
                // We consume it here instead of running the hidden-info
                // heuristic, eliminating the information leak that caused
                // state-hash desync (the heuristic peeked at real hidden
                // card identities that the shadow does not know).
                //
                // On the SERVER, we run the heuristic as before and then
                // record the decision in `pending_dig_decisions` so
                // `NetworkController` can broadcast it to clients.
                let selected: smallvec::SmallVec<[CardId; 8]> = if let Some(authoritative) =
                    self.sub_action_scratch.pending_dig_authoritative_decision.take()
                {
                    // Shadow path: use the server's decision verbatim.
                    // Only keep IDs that are actually in `valid_ids` (the
                    // shadow may have dummy IDs for unidentified cards, but
                    // the server's CardIds are globally unique and stable).
                    log::debug!(
                        target: "dig",
                        "execute_dig (shadow): using authoritative kept list: {:?}",
                        authoritative
                    );
                    authoritative.into_iter().filter(|id| valid_ids.contains(id)).collect()
                } else {
                    // Server path (or non-network local AI game): run the
                    // heuristic, then record for broadcast.
                    if valid_ids.len() > 1 && max_select < valid_ids.len() {
                        valid_ids.sort_by(|&a, &b| {
                            let score_a = self.dig_card_score(a);
                            let score_b = self.dig_card_score(b);
                            score_b.cmp(&score_a) // Descending: best first
                        });
                    }
                    let select_count = if optional && max_select > 0 {
                        let best_score = valid_ids.first().map(|&id| self.dig_card_score(id)).unwrap_or(0);
                        if best_score < 30 {
                            0
                        } else {
                            max_select
                        }
                    } else {
                        max_select
                    };
                    let kept: smallvec::SmallVec<[CardId; 8]> = valid_ids.iter().take(select_count).copied().collect();
                    // Record for network broadcast (no-op in non-network games).
                    if !self.is_shadow_game {
                        let ac = self.undo_log.len() as u64;
                        self.sub_action_scratch
                            .pending_dig_decisions
                            .borrow_mut()
                            .push((digger, kept.clone(), ac));
                        log::debug!(
                            target: "dig",
                            "execute_dig (server): recorded kept list ({} cards) at ac={}",
                            kept.len(),
                            ac
                        );
                    }
                    kept
                };
                // Rest = valid cards NOT in `selected` (order-independent: valid_ids
                // may have been sorted by the heuristic, or `selected` came from the
                // authoritative server list; either way, "rest" is "every valid card
                // that was NOT kept").
                let rest_from_valid: smallvec::SmallVec<[CardId; 8]> =
                    valid_ids.iter().copied().filter(|id| !selected.contains(id)).collect();

                for &card_id in &selected {
                    let card_name = self
                        .cards
                        .get(card_id)
                        .map(|c| c.name.to_string())
                        .unwrap_or_else(|_| format!("card#{}", card_id.as_u32()));

                    self.move_card(card_id, Zone::Library, destination, digger)?;

                    // mtg-212: the DISPLAYED name depends on async reveal
                    // timing on a network shadow (the dug card's public
                    // `RevealCard` may not have arrived on the shadow's
                    // first forward pass — `card_name` falls back to
                    // `card#<id>` — but is present on a rewind replay).
                    // Supply the rewind/replay verifier a reveal-timing-
                    // INDEPENDENT id form so the presentation asymmetry is
                    // not flagged as a fatal desync (the card is in the
                    // destination zone either way — the turn-start hash
                    // proves the STATE). Same mechanism as the
                    // discard-into-graveyard line (mtg-677).
                    let action = if reveal { "reveals and puts" } else { "puts" };
                    let stable = format!(
                        "{} {} card#{} into {:?}",
                        digger_name,
                        action,
                        card_id.as_u32(),
                        destination
                    );
                    self.logger.gamelog_reveal_stable(
                        &format!("{} {} {} into {:?}", digger_name, action, card_name, destination),
                        &stable,
                    );
                    moved_cards.push(card_id);
                }

                // Handle rest: non-selected valid cards + invalid cards
                let mut rest_cards: smallvec::SmallVec<[CardId; 8]> = smallvec::SmallVec::new();
                rest_cards.extend(rest_from_valid.iter().copied());
                rest_cards.extend(invalid_ids.iter().copied());

                if !rest_cards.is_empty() {
                    // Shuffle rest if RestRandomOrder$ True
                    if rest_random {
                        // Use a simple deterministic shuffle based on game state
                        // (card IDs provide enough entropy for reasonable shuffling)
                        let len = rest_cards.len();
                        for i in (1..len).rev() {
                            let j = (rest_cards[i].as_u32() as usize + i) % (i + 1);
                            rest_cards.swap(i, j);
                        }
                    }

                    // Move rest to rest_destination
                    if rest_destination == Zone::Library {
                        // Capture pre-reorder library order so a rewind
                        // can restore it (mtg-ba6uq #2): the raw
                        // remove/add_to_bottom below is not otherwise
                        // undo-logged.
                        self.log_library_reorder(digger, false);
                        // Put on bottom of library: remove from current position,
                        // then insert at index 0 (bottom)
                        if let Some(zones) = self.get_player_zones_mut(digger) {
                            for &card_id in &rest_cards {
                                zones.library.remove(card_id);
                                zones.library.add_to_bottom(card_id);
                            }
                        }
                        let rest_count = rest_cards.len();
                        self.logger.gamelog(&format!(
                            "{} puts {} card{} on the bottom of their library",
                            digger_name,
                            rest_count,
                            if rest_count == 1 { "" } else { "s" }
                        ));
                    } else {
                        for &card_id in &rest_cards {
                            let card_name = self
                                .cards
                                .get(card_id)
                                .map(|c| c.name.to_string())
                                .unwrap_or_else(|_| format!("card#{}", card_id.as_u32()));

                            self.move_card(card_id, Zone::Library, rest_destination, digger)?;

                            let dest_name = match rest_destination {
                                Zone::Graveyard => "their graveyard",
                                Zone::Exile => "exile",
                                Zone::Hand => "their hand",
                                Zone::Library | Zone::Battlefield | Zone::Stack | Zone::Command => "another zone",
                            };
                            // mtg-212: reveal-timing-independent verifier
                            // key (see the selected-cards branch above).
                            let stable = format!("{} puts card#{} into {}", digger_name, card_id.as_u32(), dest_name);
                            self.logger.gamelog_reveal_stable(
                                &format!("{} puts {} into {}", digger_name, card_name, dest_name),
                                &stable,
                            );
                        }
                    }
                }
            }
        } else {
            // Opponent-dig pattern (Fire Lord Ozai, Xander's Pact)
            let opponent_ids: smallvec::SmallVec<[PlayerId; 4]> =
                self.players.iter().filter(|p| p.id != digger).map(|p| p.id).collect();

            for opponent_id in opponent_ids {
                let library = self
                    .player_zones
                    .iter()
                    .find(|(id, _)| *id == opponent_id)
                    .map(|(_, zones)| &zones.library);

                if let Some(library) = library {
                    let take_count = dig_count as usize;
                    // Library top is at end of Vec, so use .rev()
                    let card_ids: smallvec::SmallVec<[CardId; 4]> =
                        library.cards.iter().rev().take(take_count).copied().collect();

                    for card_id in card_ids {
                        let opponent_name = self.get_player(opponent_id)?.name.to_string();
                        let card_name = self.cards.get(card_id).map(|c| c.name.as_str()).unwrap_or("a card");

                        self.logger
                            .gamelog(&format!("{} exiled from {}'s library", card_name, opponent_name));

                        self.move_card(card_id, Zone::Library, destination, opponent_id)?;
                        moved_cards.push(card_id);
                    }
                }
            }
        }

        // If may_play is true, create persistent effect to allow playing exiled cards
        if may_play && !moved_cards.is_empty() {
            let mana_cost_text = if may_play_without_mana_cost {
                " without paying its mana cost"
            } else {
                ""
            };

            self.logger.gamelog(&format!(
                "Until end of turn, you may play one of those cards{}",
                mana_cost_text
            ));

            use crate::core::{CleanupCondition, PersistentEffectKind};

            if may_play_without_mana_cost {
                let source_card = moved_cards[0];
                let num_moved = moved_cards.len();

                self.persistent_effects.add(
                    PersistentEffectKind::MayPlayOneWithoutManaCost {
                        tracked_cards: std::mem::take(&mut moved_cards),
                        beneficiary: digger,
                    },
                    source_card,
                    digger,
                    CleanupCondition::EndOfTurn,
                );

                log::debug!(
                    target: "dig",
                    "Created MayPlayOneWithoutManaCost effect for {} cards, beneficiary: player {}",
                    num_moved,
                    digger.as_u32()
                );
            }
        }
        Ok(())
    }

    /// AI heuristic scoring a card for Dig selection: creatures by P/T + CMC,
    /// lands at a fixed 100, other cards by CMC. Higher = more desirable to keep.
    ///
    /// This heuristic reads the real (potentially hidden) card identity, so it
    /// is only safe on the SERVER side (authoritative game state). On a shadow
    /// game `execute_dig` bypasses this heuristic entirely — it uses the
    /// server-authoritative kept-list from `pending_dig_authoritative_decision`
    /// instead (mtg-677/mtg-908 fix).
    pub(in crate::game::actions) fn dig_card_score(&self, card_id: CardId) -> i32 {
        let Some(card) = self.cards.try_get(card_id) else {
            return 0;
        };
        let cmc = i32::from(card.definition.mana_cost.cmc());
        if card.is_creature() {
            let power = i32::from(card.current_power());
            let toughness = i32::from(card.current_toughness());
            80 + (power + toughness) * 10 + cmc * 5
        } else if card.is_land() {
            100
        } else {
            50 + 30 * cmc
        }
    }

    /// [`Effect::DestroyPermanent`]: destroy the target (CR 701.7), honoring
    /// indestructible (CR 702.12b) and regeneration shields (CR 701.15a) unless
    /// `no_regenerate` (NoRegen$ True — The Abyss / Terror, CR 701.15d). Fizzles
    /// on an unresolved/self-target sentinel.
    pub(in crate::game::actions) fn execute_destroy_permanent(
        &mut self,
        target: CardId,
        no_regenerate: bool,
    ) -> Result<()> {
        // Skip if target is still placeholder (0) or unresolved sentinel
        if target.is_placeholder() || target.is_self_target() {
            // Spell fizzles - no valid targets
            return Ok(());
        }
        // MTG Rules 702.12b: Permanents with indestructible can't be destroyed
        let (owner, has_indestructible, has_regen_shield) = {
            let card = self.cards.get(target)?;
            (card.owner, card.has_indestructible(), card.regeneration_shields > 0)
        };
        if has_indestructible {
            // Indestructible - can't be destroyed
        } else if has_regen_shield && !no_regenerate {
            // CR 701.15a: Regeneration replaces destruction.
            // When the destroy says "can't be regenerated" (NoRegen$ True,
            // e.g. The Abyss / Terror), the regeneration shield does NOT
            // apply (CR 701.15d) and the permanent is destroyed outright.
            self.apply_regeneration_shield(target)?;
        } else {
            let dest = self.death_destination_for_card(target);
            // Move the card to the graveyard/exile FIRST (per CR 704.3 — state-based actions move
            // the card before triggers fire), then fire death triggers with the card now in
            // its destination zone. This is required for triggers like Enduring Vitality whose
            // death trigger returns the card from the graveyard — the card must already be in
            // the graveyard when the trigger effect executes, so the graveyard→battlefield move
            // in execute_return_self_as_enchantment finds it there.
            self.move_card(target, Zone::Battlefield, dest, owner)?;
            let _ = self.check_death_triggers(target);
        }
        Ok(())
    }

    /// [`Effect::ExilePermanent`]: move the target from battlefield to exile.
    /// Fizzles on an unresolved/reuse-previous sentinel.
    pub(in crate::game::actions) fn execute_exile_permanent(&mut self, target: CardId) -> Result<()> {
        // Skip if target is still placeholder (0) or unresolved sentinel
        if target.is_placeholder() || target.is_reuse_previous() {
            return Ok(());
        }
        // Exile the permanent by moving it from battlefield to exile
        let owner = self.cards.get(target)?.owner;
        self.move_card(target, Zone::Battlefield, Zone::Exile, owner)?;
        Ok(())
    }

    /// [`Effect::ExileIfWouldDieThisTurn`]: Disintegrate's ReplaceDyingDefined
    /// clause — mark the targeted creature so that, if it would die this turn, it
    /// is exiled instead (CR 614). The flag is read by
    /// `death_destination_for_card` and cleared at cleanup.
    pub(in crate::game::actions) fn execute_exile_if_would_die_this_turn(&mut self, target: CardId) -> Result<()> {
        if target.is_placeholder() || target.is_reuse_previous() {
            return Ok(());
        }
        if let Ok(card) = self.cards.get_mut(target) {
            if card.is_creature() {
                card.exile_if_would_die_this_turn = true;
            }
        }
        Ok(())
    }

    /// [`Effect::PlayFromGraveyard`]: `AB$ Play | TgtZone$ Graveyard` — grant
    /// one-time permission to cast a targeted instant/sorcery from the graveyard
    /// this turn (Chandra, Acolyte of Flame −2). Creates a
    /// `PersistentEffectKind::CastTargetedSpellFromGraveyard` that the priority
    /// loop offers as a `SpellAbility::CastFromGraveyard`. If `exile_on_resolution`
    /// is set, also marks the card with `exile_if_would_go_to_graveyard_this_turn`
    /// so `resolve_spell_finalize` sends it to exile instead (CR 614 replacement).
    pub(in crate::game::actions) fn execute_play_from_graveyard(
        &mut self,
        target: CardId,
        exile_on_resolution: bool,
    ) -> Result<()> {
        if target.is_placeholder() {
            // Fizzle — no valid graveyard target was chosen.
            return Ok(());
        }

        let (owner, card_name) = {
            let card = self.cards.get(target)?;
            (card.owner, card.name.to_string())
        };

        // Mark the card so resolve_spell_finalize exiles it on resolution
        // (the ReplaceGraveyard$ Exile clause from the card script).
        if exile_on_resolution {
            if let Ok(card) = self.cards.get_mut(target) {
                card.exile_if_would_go_to_graveyard_this_turn = true;
            }
        }

        // Create the one-shot cast-permission persistent effect.
        use crate::core::{persistent_effect::CleanupCondition, persistent_effect::PersistentEffectKind, CardId};
        let cleanup = CleanupCondition::Any(vec![
            // Remove once the card is cast (it leaves the graveyard).
            CleanupCondition::TrackedCardIsCast { card: target },
            // Also remove at end of turn if it was never cast.
            CleanupCondition::EndOfTurn,
        ]);
        self.persistent_effects.add(
            PersistentEffectKind::CastTargetedSpellFromGraveyard {
                tracked_card: target,
                owner,
                exile_on_resolution,
            },
            // Source card: use a stable placeholder — the planewalker is on the
            // battlefield but we don't have its CardId at this call site. The
            // persistent effect store doesn't use source_card for tracking here.
            CardId::placeholder(),
            owner,
            cleanup,
        );

        self.logger.gamelog(&format!(
            "{} may cast {} from graveyard this turn{}",
            self.player_display_name(owner),
            card_name,
            if exile_on_resolution {
                " (exile instead of graveyard on resolution)"
            } else {
                ""
            },
        ));

        Ok(())
    }

    /// [`Effect::SelfExileFromStack`]: `SP$ ChangeZone | Origin$ Stack |
    /// Destination$ Exile` (All Hallow's Eve) — move the resolving spell from
    /// the stack to exile so the default sorcery resolution doesn't graveyard
    /// it. Optionally remembers the moved card for chained `Defined$ Remembered`
    /// sub-abilities.
    pub(in crate::game::actions) fn execute_self_exile_from_stack(
        &mut self,
        source: CardId,
        remember_changed: bool,
    ) -> Result<()> {
        if source.is_placeholder() || source.is_self_target() {
            // resolve_self_target should have patched the source CardId;
            // if it didn't (effect was placed in an unexpected context),
            // fizzle silently rather than panicking.
            log::debug!(
                target: "self_exile",
                "SelfExileFromStack: source still placeholder/sentinel, skipping"
            );
            return Ok(());
        }
        if !self.stack.contains(source) {
            log::debug!(
                target: "self_exile",
                "SelfExileFromStack: card {} no longer on stack",
                source.as_u32()
            );
            return Ok(());
        }
        let owner = self.cards.get(source)?.owner;
        self.move_card(source, Zone::Stack, Zone::Exile, owner)?;
        if remember_changed {
            // Make the just-exiled card available to chained
            // SubAbilities with `Defined$ Remembered` (e.g. the
            // PutCounter that places two scream counters on it).
            self.remembered_cards.push(source);
        }
        Ok(())
    }

    /// [`Effect::MoveSelfBetweenZones`]: `DB$ ChangeZone | Defined$ Self` from a
    /// triggered ability whose source lives outside the battlefield (e.g. All
    /// Hallow's Eve moving itself exile→graveyard once its last scream counter is
    /// removed). Verifies the card is actually in `origin` first (CR 400.7 /
    /// 608.2g object-no-longer-there) so it never double-moves.
    pub(in crate::game::actions) fn execute_move_self_between_zones(
        &mut self,
        source: CardId,
        origin: Zone,
        destination: Zone,
    ) -> Result<()> {
        if source.is_placeholder() || source.is_self_target() {
            log::debug!(
                target: "self_exile",
                "MoveSelfBetweenZones: source still placeholder/sentinel, skipping"
            );
            return Ok(());
        }
        // Verify the card is actually in the origin zone before moving so
        // we never double-move (CR 400.7 / 608.2g object-no-longer-there).
        let in_origin = self.find_card_zone(source) == Some(origin);
        if !in_origin {
            log::debug!(
                target: "self_exile",
                "MoveSelfBetweenZones: card {} not in {:?}, skipping",
                source.as_u32(),
                origin
            );
            return Ok(());
        }
        let owner = self.cards.get(source)?.owner;
        self.move_card(source, origin, destination, owner)?;
        Ok(())
    }

    /// [`Effect::DestroyAll`]: destroy every permanent matching `restriction`
    /// (Wrath of God), honoring indestructible and regeneration (unless
    /// `no_regenerate`). CR 701.7.
    pub(in crate::game::actions) fn execute_destroy_all(
        &mut self,
        restriction: &crate::core::effects::TargetRestriction,
        no_regenerate: bool,
    ) -> Result<()> {
        // Destroy all permanents matching the restriction (e.g., Wrath of God)
        let targets: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| {
                self.cards
                    .get(card_id)
                    .map(|card| restriction.matches(card))
                    .unwrap_or(false)
            })
            .collect();

        for card_id in targets {
            let (owner, has_indestructible, has_regen_shield) = {
                let card = self.cards.get(card_id)?;
                (card.owner, card.has_indestructible(), card.regeneration_shields > 0)
            };
            if has_indestructible {
                // Indestructible - can't be destroyed
            } else if has_regen_shield && !no_regenerate {
                // CR 701.15a: Regeneration replaces destruction
                self.apply_regeneration_shield(card_id)?;
            } else {
                let card_name = self
                    .cards
                    .get(card_id)
                    .map(|c| c.name.to_string())
                    .unwrap_or_else(|_| "Unknown".to_string());
                // Move the card first (CR 704.3), then fire death triggers so any
                // "return from graveyard" trigger sees the card in its destination zone.
                self.move_card(
                    card_id,
                    Zone::Battlefield,
                    self.death_destination_for_card(card_id),
                    owner,
                )?;
                let _ = self.check_death_triggers(card_id);
                self.logger
                    .gamelog(&format!("{} ({}) is destroyed", card_name, card_id));
            }
        }
        Ok(())
    }

    /// [`Effect::SacrificeAll`]: every permanent matching `restriction` is
    /// sacrificed (All is Dust). Sacrifice bypasses indestructible and
    /// regeneration (CR 701.17).
    pub(in crate::game::actions) fn execute_sacrifice_all(
        &mut self,
        restriction: &crate::core::effects::TargetRestriction,
    ) -> Result<()> {
        // Each player sacrifices all permanents matching the restriction (e.g., All is Dust)
        // Sacrifice bypasses indestructible and regeneration (CR 701.17)
        let targets: Vec<(CardId, PlayerId)> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter_map(|card_id| {
                let card = self.cards.try_get(card_id)?;
                if restriction.matches(card) {
                    Some((card_id, card.owner))
                } else {
                    None
                }
            })
            .collect();

        for (card_id, owner) in targets {
            let card_name = self
                .cards
                .try_get(card_id)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            // Move the card first (CR 704.3), then fire death triggers so any
            // "return from graveyard" trigger sees the card in its destination zone.
            self.move_card(
                card_id,
                Zone::Battlefield,
                self.death_destination_for_card(card_id),
                owner,
            )?;
            let _ = self.check_death_triggers(card_id);
            self.logger
                .gamelog(&format!("{} ({}) is sacrificed", card_name, card_id));
        }
        Ok(())
    }

    /// [`Effect::ForceSacrifice`]: force `player` to sacrifice `count`
    /// permanents matching the `SacValid$` `sac_type` (CR 701.17). The AI picks
    /// the least-valuable matching permanents (P/T sum for creatures, CMC else).
    ///
    /// mtg-907: `sac_type` is parsed with the canonical `TargetRestriction`,
    /// fixing the old hand-rolled `match` that mis-handled comma-lists, subtypes,
    /// and qualifiers (see the inline comment + commit rules review).
    ///
    /// KNOWN LIMITATION (NOT introduced here, deferred — see mtg-907): a few
    /// `SacValid$` strings carry DYNAMIC predicates that neither the old code nor
    /// `TargetRestriction::parse` resolves — `.attacking`, `.untapped`,
    /// `.withFlying`, `.sharesCardTypeWith…`, and the `Self` selector. The old
    /// `match` silently defaulted those to "any creature"; `TargetRestriction`
    /// silently ignores the unknown qualifier (so `Creature.attacking` degrades
    /// to bare `Creature`). Both are imperfect; extending `TargetRestriction` to
    /// model board-state predicates is the proper fix and is tracked in mtg-907.
    pub(in crate::game::actions) fn execute_force_sacrifice(
        &mut self,
        player: PlayerId,
        sac_type: &str,
        count: u8,
    ) -> Result<()> {
        // Force a player to sacrifice permanents matching a type
        // CR 701.17: "sacrifice a permanent" means its controller moves it to graveyard
        let player_name = self
            .get_player(player)
            .map(|p| p.name.clone())
            .unwrap_or_else(|_| "Unknown".to_string().into());

        // mtg-907: parse the `SacValid$` filter once with the canonical
        // TargetRestriction instead of the old hand-rolled `match sac_type`. The
        // old code only handled SINGLE bare types and fell through to
        // `is_creature()` for everything else — so it MIS-handled real shipping
        // filters: `SacValid$ Creature,Planeswalker` (comma-list — Planeswalkers
        // were never sacrificed), bare subtypes like `SacValid$ Food` / `Mountain`
        // (a Food artifact was treated as "creature" and skipped), and qualifiers
        // like `.nonArtifact`. TargetRestriction::matches handles all of these
        // correctly (CR 109.1/205 card types, multi-type OR, subtype membership,
        // nonArtifact). See the commit's rules review.
        let restriction = crate::core::TargetRestriction::parse(sac_type);
        // Find matching permanents controlled by the target player
        let mut candidates: Vec<(CardId, i32)> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter_map(|card_id| {
                let card = self.cards.get(card_id).ok()?;
                if card.controller != player {
                    return None;
                }
                if restriction.matches(card) {
                    // Score: lower value = sacrifice first
                    // Use P/T sum for creatures, CMC for non-creatures
                    let value = if card.is_creature() {
                        i32::from(card.current_power()) + i32::from(card.current_toughness())
                    } else {
                        i32::from(card.mana_cost.cmc())
                    };
                    Some((card_id, value))
                } else {
                    None
                }
            })
            .collect();

        // Sort by value ascending (sacrifice least valuable first)
        candidates.sort_by_key(|&(_, v)| v);

        let to_sac = (count as usize).min(candidates.len());
        for &(card_id, _) in candidates.iter().take(to_sac) {
            let card_name = self.cards.get(card_id).map(|c| c.name.to_string()).unwrap_or_default();
            let owner = self.cards.get(card_id).map(|c| c.owner).unwrap_or(player);
            let dest = self.death_destination_for_card(card_id);
            self.move_card(card_id, Zone::Battlefield, dest, owner)?;
            self.logger
                .gamelog(&format!("{} sacrifices {} ({})", player_name, card_name, card_id));
        }

        if to_sac == 0 {
            self.logger
                .gamelog(&format!("{} has no {} to sacrifice", player_name, sac_type));
        }
        Ok(())
    }

    /// [`Effect::ReturnCardsFromGraveyardToHand`]: Recall — return one card from
    /// `player`'s graveyard to hand for each card discarded this way
    /// (CR 400.7 / 701.25; count = `remembered_cards.len()` from the preceding
    /// DiscardCards). Cards are picked in stable lowest-CardId order so the
    /// choice is deterministic across server and both network clients (the
    /// graveyard is a public zone, so revealing CardIds leaks nothing).
    pub(in crate::game::actions) fn execute_return_cards_from_graveyard_to_hand(
        &mut self,
        player: PlayerId,
    ) -> Result<()> {
        let count = self.remembered_cards.len();
        if count == 0 {
            // Nothing was remembered (nothing discarded) — nothing to return.
            return Ok(());
        }
        // Collect graveyard cards once so we can mutate the zone in the loop.
        let graveyard_cards: smallvec::SmallVec<[CardId; 8]> = self
            .get_player_zones(player)
            .map(|z| z.graveyard.cards.iter().copied().collect())
            .unwrap_or_default();
        if graveyard_cards.is_empty() {
            self.logger.gamelog(&format!(
                "Recall effect: {} has no cards in graveyard to return",
                self.get_player(player).ok().map(|p| p.name.as_str()).unwrap_or("?")
            ));
            return Ok(());
        }
        // For each card to return, pick the AI-preferred card still in graveyard.
        let player_name = self
            .get_player(player)
            .ok()
            .map(|p| p.name.as_str())
            .unwrap_or("?")
            .to_string();
        let to_return = count.min(graveyard_cards.len());
        for _ in 0..to_return {
            // Re-snapshot graveyard each iteration (previous iteration may
            // have moved a card out).
            let remaining: smallvec::SmallVec<[CardId; 8]> = self
                .get_player_zones(player)
                .map(|z| z.graveyard.cards.iter().copied().collect())
                .unwrap_or_default();
            // Deterministic pick: lowest CardId (stable across server/clients).
            // `min_by_key` returns None only on empty iter; we guard
            // above with `if remaining.is_empty() { break }`, so the
            // `?`-style early-exit covers the impossible None case.
            let Some(&chosen) = remaining.iter().min_by_key(|id| id.as_u32()) else {
                break;
            };
            let card_name = self
                .cards
                .try_get(chosen)
                .map(|c| c.name.as_str())
                .unwrap_or("?")
                .to_string();
            self.move_card(chosen, Zone::Graveyard, Zone::Hand, player)?;
            self.logger
                .gamelog(&format!("{} returns {} from graveyard to hand", player_name, card_name));
        }
        Ok(())
    }

    /// [`Effect::PutCardsFromHandOnTopOfLibrary`]: non-interactive (fallback)
    /// path that moves `cards_to_put` from the player's hand to the top of their
    /// library.
    ///
    /// The interactive path (priority loop) asks the controller which cards to
    /// choose and calls this function with the chosen set.  The non-interactive
    /// fallback (e.g., execute_effect with no controller) uses a deterministic
    /// heuristic: pick the `count` cards with the smallest CMC in hand (proxy
    /// for "least valuable"), stable-sorted by `CardId` for server/client
    /// determinism.
    ///
    /// MTG CR 701.19b: the player puts the chosen cards on top of their library
    /// in any order they choose; we put them on top one by one (first chosen card
    /// ends up deepest, last on top) so the caller's ordering is respected.
    pub(crate) fn execute_put_cards_from_hand_on_top_of_library(
        &mut self,
        player: PlayerId,
        cards_to_put: &[CardId],
    ) -> Result<()> {
        let player_name = self
            .get_player(player)
            .ok()
            .map(|p| p.name.as_str())
            .unwrap_or("?")
            .to_string();
        for &card_id in cards_to_put {
            let card_name = self
                .cards
                .try_get(card_id)
                .map(|c| c.name.as_str())
                .unwrap_or("?")
                .to_string();
            self.move_card(card_id, crate::zones::Zone::Hand, crate::zones::Zone::Library, player)?;
            // Log the move so replay can reconstruct which card went back.
            self.logger
                .gamelog(&format!("{} puts {} on top of library", player_name, card_name));
        }
        Ok(())
    }

    /// Fallback heuristic for [`Effect::PutCardsFromHandOnTopOfLibrary`]: pick
    /// the `count` cards with the smallest CMC from `hand`, breaking ties by
    /// `CardId` (smallest first = deterministic across server/clients).  Returns
    /// up to `hand.len()` cards even if `count > hand.len()`.
    pub(crate) fn pick_cards_to_put_back_heuristic(
        &self,
        hand: &[CardId],
        count: usize,
    ) -> smallvec::SmallVec<[CardId; 7]> {
        let mut sorted: smallvec::SmallVec<[CardId; 16]> = hand.iter().copied().collect();
        sorted.sort_by_key(|&id| {
            let cmc = self.cards.try_get(id).map(|c| c.mana_cost.cmc()).unwrap_or(0);
            (cmc, id.as_u32())
        });
        sorted.into_iter().take(count).collect()
    }

    /// [`Effect::RevealCardsFromHand`]: reveal matching cards from a player's
    /// hand and optionally store the revealed count in
    /// [`GameState::remembered_amount`] for use by chained sub-abilities.
    ///
    /// Filter syntax follows Forge's `RevealValid$` field, e.g.
    /// `"Card.Artifact+YouCtrl"`.  Currently we match on card types contained
    /// in the filter string (`Artifact`, `Land`, `Creature`, …); the `+YouCtrl`
    /// qualifier is implicit (we always reveal *your own* hand cards here).
    ///
    /// Non-interactive: reveals ALL matching cards (maximises the mana gained
    /// from a Metalworker, which is the correct greedy play with the zero
    /// controller since Metalworker's mana scales with revealed artifacts).
    ///
    /// MTG CR 701.15 (Reveal): a player reveals a card by showing it to all
    /// other players.  The revealed cards stay in the hand after the reveal.
    pub(crate) fn execute_reveal_cards_from_hand(
        &mut self,
        player: PlayerId,
        filter: &str,
        remember_count: bool,
    ) -> Result<()> {
        let player_name = self
            .get_player(player)
            .ok()
            .map(|p| p.name.as_str())
            .unwrap_or("?")
            .to_string();

        // Collect matching card IDs from hand (cards stay in hand after reveal).
        let matching: smallvec::SmallVec<[CardId; 8]> = self
            .get_player_zones(player)
            .map(|z| {
                z.hand
                    .cards
                    .iter()
                    .copied()
                    .filter(|&id| {
                        self.cards
                            .try_get(id)
                            .map(|c| card_matches_reveal_filter(c, filter))
                            .unwrap_or(false)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let count = matching.len();

        // Log each revealed card (all players see the reveal — CR 701.15).
        for &id in &matching {
            let card_name = self
                .cards
                .try_get(id)
                .map(|c| c.name.as_str())
                .unwrap_or("?")
                .to_string();
            self.logger.gamelog(&format!("{player_name} reveals {card_name}"));
        }

        if count == 0 {
            self.logger
                .gamelog(&format!("{player_name} reveals no cards (no matching cards in hand)"));
        }

        // Optionally remember the count for the chained sub-ability.
        if remember_count {
            self.remembered_amount = Some(count as u32);
        }

        Ok(())
    }

    /// [`Effect::ReturnGraveyardCardToHand`]: return exactly one card matching
    /// `type_filter` from `player`'s graveyard to hand (Stormchaser's Talent
    /// level-2 "return target instant or sorcery"). The card is picked in stable
    /// lowest-CardId order (deterministic across server/clients).
    ///
    /// NOTE (mtg-907): `type_filter` is split + matched with raw `str` compares
    /// (`"Instant"`/`"Sorcery"`/…). Preserved verbatim from the inline arm; a
    /// candidate for the Valid$/filter consolidation.
    pub(in crate::game::actions) fn execute_return_graveyard_card_to_hand(
        &mut self,
        player: PlayerId,
        type_filter: &str,
    ) -> Result<()> {
        let graveyard_cards: smallvec::SmallVec<[CardId; 8]> = self
            .get_player_zones(player)
            .map(|z| z.graveyard.cards.iter().copied().collect())
            .unwrap_or_default();

        // Filter by type
        let filter_types: Vec<&str> = if type_filter.is_empty() {
            vec![]
        } else {
            type_filter.split(',').map(|s| s.trim()).collect()
        };

        let matching: smallvec::SmallVec<[CardId; 8]> = graveyard_cards
            .into_iter()
            .filter(|&id| {
                if filter_types.is_empty() {
                    return true;
                }
                if let Some(card) = self.cards.try_get(id) {
                    filter_types.iter().any(|&t| match t {
                        "Instant" => card.is_instant(),
                        "Sorcery" => card.is_sorcery(),
                        "Creature" => card.is_creature(),
                        "Land" => card.is_land(),
                        "Artifact" => card.is_artifact(),
                        _ => false,
                    })
                } else {
                    false
                }
            })
            .collect();

        if matching.is_empty() {
            let player_name = self
                .get_player(player)
                .ok()
                .map(|p| p.name.as_str())
                .unwrap_or("?")
                .to_string();
            self.logger.gamelog(&format!(
                "{} has no matching {} in graveyard to return",
                player_name,
                if type_filter.is_empty() {
                    "card".to_string()
                } else {
                    type_filter.to_string()
                }
            ));
        } else {
            // Deterministic pick: lowest CardId (stable across server/clients).
            let Some(&chosen) = matching.iter().min_by_key(|id| id.as_u32()) else {
                return Ok(());
            };
            let card_name = self
                .cards
                .try_get(chosen)
                .map(|c| c.name.as_str())
                .unwrap_or("?")
                .to_string();
            let player_name = self
                .get_player(player)
                .ok()
                .map(|p| p.name.as_str())
                .unwrap_or("?")
                .to_string();
            self.move_card(chosen, Zone::Graveyard, Zone::Hand, player)?;
            self.logger
                .gamelog(&format!("{} returns {} from graveyard to hand", player_name, card_name));
        }
        Ok(())
    }

    /// [`Effect::ReturnGraveyardCardToZone`]: return exactly one card matching
    /// `type_filter` from a player's graveyard to `destination` (Library,
    /// Battlefield, etc.), optionally under the caster's control
    /// (`gain_control`).
    ///
    /// Examples:
    /// - Reclaim: graveyard → top of library (any card the caster owns).
    ///   CR 700.4: putting a card on top of your library is a zone-change; the
    ///   card keeps its identity (it is NOT shuffled in unless another effect
    ///   says so). `library_position$ 0` = top.
    /// - Goryo's Vengeance: graveyard → battlefield, haste, GainControl$ True.
    ///   The spell's SubAbility chain handles the haste grant + EOT exile;
    ///   this effect handles only the zone move. CR 701.3: when a card is put
    ///   onto the battlefield with "under your control", its controller is
    ///   overridden to the casting player regardless of ownership.
    /// - Debtors' Knell trigger: graveyard → battlefield, GainControl$ True,
    ///   any creature from any graveyard.
    ///
    /// The AI picks the highest-power creature (for battlefield) or highest-CMC
    /// non-land (for library/hand) from the graveyard of `player` when
    /// `type_filter` has no ownership restriction, otherwise from any player's
    /// graveyard. This is deterministic across server/clients (graveyards are
    /// public, CR 400.2) so it is information-independent for network safety.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::actions) fn execute_return_graveyard_card_to_zone(
        &mut self,
        player: PlayerId,
        type_filter: &str,
        destination: Zone,
        gain_control: bool,
        library_position: u8,
    ) -> Result<()> {
        // Collect all player IDs so we can search across graveyards when needed.
        let all_players: smallvec::SmallVec<[PlayerId; 4]> = self.players.iter().map(|p| p.id).collect();

        // Build candidate list: (card_id, owner, score)
        // Cards are only reachable from graveyards (public zone, CR 400.2).
        let filter_types: Vec<&str> = if type_filter.is_empty() {
            vec![]
        } else {
            type_filter.split(',').map(|s| s.trim()).collect()
        };
        // Determine whether ownership is restricted to the caster.
        // Forge's `Card.YouCtrl` / `Card.YouOwn` suffix on ValidTgts$ restricts
        // to the casting player's graveyard; bare types (e.g. "Creature") do not.
        let own_graveyard_only = type_filter.contains("YouCtrl") || type_filter.contains("YouOwn");

        let search_players: smallvec::SmallVec<[PlayerId; 4]> = if own_graveyard_only {
            smallvec::smallvec![player]
        } else {
            all_players
        };

        // Helper: type matches a card by the base type token(s).
        let type_matches = |card: &crate::core::Card| -> bool {
            if filter_types.is_empty() {
                return true;
            }
            filter_types.iter().any(|&t| match t {
                "Card" => true,
                "Creature" => card.is_creature(),
                "Instant" => card.is_instant(),
                "Sorcery" => card.is_sorcery(),
                "Land" => card.is_land(),
                "Artifact" => card.is_artifact(),
                "Enchantment" => card.is_enchantment(),
                other => card.subtypes.iter().any(|st| st.as_str().eq_ignore_ascii_case(other)),
            })
        };

        // Collect candidates with a simple value score (power for creatures,
        // CMC for others). Deterministic low-to-high CardId tiebreak ensures
        // identical picks on server and both clients (CR 400.2 — graveyard is
        // public, no hidden info).
        let mut candidates: smallvec::SmallVec<[(CardId, PlayerId, i32); 8]> = smallvec::SmallVec::new();
        for &pid in &search_players {
            if let Some(zones) = self.get_player_zones(pid) {
                let gy: smallvec::SmallVec<[CardId; 8]> = zones.graveyard.cards.iter().copied().collect();
                for card_id in gy {
                    if let Some(card) = self.cards.try_get(card_id) {
                        if type_matches(card) {
                            let score = if card.is_creature() {
                                i32::from(card.current_power()) + i32::from(card.current_toughness())
                            } else {
                                i32::from(card.mana_cost.cmc())
                            };
                            candidates.push((card_id, pid, score));
                        }
                    }
                }
            }
        }

        if candidates.is_empty() {
            let player_name = self
                .get_player(player)
                .ok()
                .map(|p| p.name.as_str())
                .unwrap_or("?")
                .to_string();
            self.logger.gamelog(&format!(
                "{} has no matching {} in graveyard to return",
                player_name,
                if type_filter.is_empty() { "card" } else { type_filter }
            ));
            return Ok(());
        }

        // Pick: highest score, then lowest CardId for determinism.
        candidates.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.as_u32().cmp(&b.0.as_u32())));
        let (chosen, card_owner, _) = candidates[0];

        let card_name = self
            .cards
            .try_get(chosen)
            .map(|c| c.name.as_str())
            .unwrap_or("?")
            .to_string();
        let player_name = self
            .get_player(player)
            .ok()
            .map(|p| p.name.as_str())
            .unwrap_or("?")
            .to_string();

        // Move from graveyard → destination.
        // `gain_control` → move under `player`'s ownership for the purpose of
        // zone placement (CR 701.3). We pass `player` as the "owner" so the card
        // lands in the right player's Battlefield slot and the controller is set
        // correctly by `move_card`.
        let effective_owner = if gain_control { player } else { card_owner };
        self.move_card(chosen, Zone::Graveyard, destination, effective_owner)?;

        // For Library destination, honour `library_position`: 0 = top, 1 = bottom.
        // `move_card` appends to the top by default; for bottom, move the card to
        // position 0 (index 0 = bottom of library Vec).
        if destination == Zone::Library && library_position != 0 {
            if let Some(zones) = self.get_player_zones_mut(effective_owner) {
                zones.library.remove(chosen);
                zones.library.add_to_bottom(chosen);
            }
        }

        let dest_label = match destination {
            Zone::Library => {
                if library_position == 0 {
                    "the top of library"
                } else {
                    "the bottom of library"
                }
            }
            Zone::Battlefield => "the battlefield",
            Zone::Hand => "hand",
            Zone::Exile => "exile",
            Zone::Graveyard => "graveyard",
            Zone::Stack | Zone::Command => "another zone",
        };
        self.logger.gamelog(&format!(
            "{} returns {} from graveyard to {}",
            player_name, card_name, dest_label
        ));
        Ok(())
    }

    /// [`Effect::ReturnSelfAsEnchantment`]: return the card (that just died) from
    /// the graveyard to the battlefield under its owner's control, but strip all
    /// creature (and other non-enchantment) card types so the resulting permanent
    /// is purely an enchantment.
    ///
    /// This implements Enduring Vitality's death trigger:
    ///   "When Enduring Vitality dies, if it was a creature, return it to the
    ///    battlefield under its owner's control. It's an enchantment."
    ///
    /// Rules context (CR 400.7 / CR 110.5c):
    ///   - The card returns as a new object; its creature sub-types and creature
    ///     card type are stripped so it is now only an Enchantment.
    ///   - It will no longer trigger dies-as-creature triggers (the `+Creature`
    ///     guard in the trigger's ValidCard filter prevents re-triggering).
    pub(in crate::game::actions) fn execute_return_self_as_enchantment(
        &mut self,
        source: crate::core::CardId,
    ) -> crate::Result<()> {
        use crate::core::CardType;

        // Fizzle gracefully if the source ID is unresolved or not in graveyard.
        if source.is_placeholder() {
            log::debug!("ReturnSelfAsEnchantment: unresolved placeholder — fizzling");
            return Ok(());
        }

        // Find the card's owner (needed for move_card destination routing).
        let (card_name, owner) = {
            let card = match self.cards.try_get(source) {
                Some(c) => c,
                None => {
                    log::debug!("ReturnSelfAsEnchantment: card {:?} not found — fizzling", source);
                    return Ok(());
                }
            };
            (card.name.as_str().to_string(), card.owner)
        };

        // Verify the card is in the graveyard (it should be — we just came from
        // check_death_triggers, but guard defensively).
        let in_graveyard = self
            .get_player_zones(owner)
            .is_some_and(|z| z.graveyard.cards.contains(&source));
        if !in_graveyard {
            log::debug!(
                "ReturnSelfAsEnchantment: {} ({:?}) not in graveyard — fizzling",
                card_name,
                source
            );
            return Ok(());
        }

        // Move from graveyard → battlefield under the owner's control.
        self.move_card(source, Zone::Graveyard, Zone::Battlefield, owner)?;

        // Strip all card types except Enchantment so the permanent is purely
        // an enchantment and no longer a creature (CR 110.5c, CR 205.1a).
        // We preserve Legendary if present (rule 704.5j), Land type removal
        // doesn't apply here, but we do remove Creature + Artifact + Instant/Sorcery.
        {
            let card = self.cards.get_mut(source)?;
            // Keep only Enchantment (and Land, which shouldn't be present but is
            // harmless to preserve). Strip Creature, Artifact, Planeswalker, etc.
            card.types
                .retain(|t| matches!(t, CardType::Enchantment | CardType::Land));
            // Ensure Enchantment is present (it should already be, but be safe).
            if !card.types.contains(&CardType::Enchantment) {
                card.types.push(CardType::Enchantment);
            }
            // Refresh cached is_creature / is_enchantment / ... flags.
            card.refresh_type_cache();
        }

        self.logger.gamelog(&format!(
            "{} returns to the battlefield as an enchantment (no longer a creature)",
            card_name
        ));

        // Fire ETB triggers for the returning permanent (CR 603.6a).
        self.check_triggers(crate::core::TriggerEvent::EntersBattlefield, source)?;

        Ok(())
    }

    /// [`Effect::ChangeZoneAll`]: move every card matching `restriction` from
    /// each `origin` zone to `destination` (Timetwister / Wheel / mass bounce).
    /// `Shuffle$ True` into the library shuffles every library afterward
    /// (symmetric, replay-safe). On a SHADOW game an UNRESTRICTED mass move also
    /// moves instance-less reserved CardIds, so the opponent's library count
    /// stays in lockstep with the server (mtg-728 sig-2c — otherwise the
    /// subsequent shuffle consumes a different amount of RNG and desyncs).
    pub(in crate::game::actions) fn execute_change_zone_all(
        &mut self,
        restriction: &crate::core::effects::TargetRestriction,
        origins: &[Zone],
        destination: Zone,
        shuffle: bool,
    ) -> Result<()> {
        // Move all cards matching the restriction from EACH origin zone
        // to the destination. Timetwister/Diminishing-Returns style mass
        // shuffles list two origins (Hand + Graveyard); single-origin
        // mass bounce/exile list one. Track (card, owner, source-zone)
        // so each card moves from the zone it actually lives in.
        let mut cards_to_move: Vec<(CardId, PlayerId, Zone)> = Vec::new();
        // SHADOW determinism (mtg-728 sig-2c): the opponent's hidden
        // hand/library cards are late-bound reserved CardIds with NO
        // instance, so `try_get` returns None and `restriction.matches`
        // can't be evaluated. For an UNRESTRICTED mass move (Timetwister
        // / Wheel / Windfall shuffle-back — matches any card) those
        // reserved cards MUST still move; otherwise the opponent's
        // library ends up short on the shadow and its subsequent
        // shuffle consumes a different amount of RNG than the server's,
        // breaking server<->shadow RNG lockstep and desyncing every
        // later shuffle/draw. Only reached in shadow games (the server
        // and native clients always have real instances).
        let move_reserved_in_shadow = self.is_shadow_game && restriction.is_unrestricted();
        for &origin in origins {
            match origin {
                Zone::Battlefield => {
                    for card_id in self.battlefield.cards.iter().copied() {
                        if let Some(card) = self.cards.try_get(card_id) {
                            if restriction.matches(card) {
                                cards_to_move.push((card_id, card.owner, origin));
                            }
                        }
                    }
                }
                // Per-player private/owned zones: collect from each
                // player's own zone so ownership is exact.
                Zone::Graveyard | Zone::Hand | Zone::Library | Zone::Exile => {
                    for (player_id, zones) in &self.player_zones {
                        let zone_cards = match origin {
                            Zone::Graveyard => &zones.graveyard.cards,
                            Zone::Hand => &zones.hand.cards,
                            Zone::Library => &zones.library.cards,
                            Zone::Exile => &zones.exile.cards,
                            // Unreachable: outer match already narrowed to these four.
                            Zone::Battlefield | Zone::Stack | Zone::Command => continue,
                        };
                        for &card_id in zone_cards {
                            match self.cards.try_get(card_id) {
                                Some(card) => {
                                    if restriction.matches(card) {
                                        cards_to_move.push((card_id, *player_id, origin));
                                    }
                                }
                                // Reserved (instance-less) shadow card under an
                                // unrestricted mass move (mtg-728 sig-2c).
                                None if move_reserved_in_shadow => {
                                    cards_to_move.push((card_id, *player_id, origin));
                                }
                                None => {}
                            }
                        }
                    }
                }
                Zone::Stack | Zone::Command => {
                    // Mass zone changes don't originate from the stack or
                    // the command zone in any supported card.
                }
            }
        }

        for (card_id, owner, origin) in cards_to_move {
            self.move_card(card_id, origin, destination, owner)?;
        }

        // `Shuffle$ True` mass move into the library (Timetwister,
        // Mnemonic Nexus) requires shuffling the affected libraries so
        // the moved cards land in random order. Ordered moves
        // (`LibraryPosition$ -1`, e.g. Manifold Insights) set
        // shuffle=false and are left untouched. The effect is symmetric
        // across players, so shuffle every library; a library that
        // received no cards only advances RNG (deterministic /
        // replay-safe).
        if shuffle && matches!(destination, Zone::Library) {
            let player_ids: smallvec::SmallVec<[PlayerId; 4]> = self.players.iter().map(|p| p.id).collect();
            for pid in player_ids {
                self.shuffle_library(pid);
            }
        }
        Ok(())
    }

    /// [`Effect::SearchLibrary`]: search `player`'s library for the first card
    /// matching `card_type_filter`, move it to `destination` (tapped if
    /// `enters_tapped` and entering the battlefield), then shuffle if `shuffle`
    /// (CR 701.19). Fetch lands, tutors, Evolving Wilds.
    ///
    /// NOTE (mtg-907): the candidate match uses `card_matches_search_filter`, a
    /// raw string-based filter (comma-lists, dotted `Type.Subtype`, single-word
    /// type-vs-subtype disambiguation). Preserved verbatim; a candidate for the
    /// Valid$/filter consolidation.
    pub(in crate::game::actions) fn execute_search_library(
        &mut self,
        player: PlayerId,
        card_type_filter: &str,
        destination: Zone,
        enters_tapped: bool,
        shuffle: bool,
    ) -> Result<()> {
        // Search library for a card matching the filter and move it to destination
        // MTG Rules 701.19a: To search a zone, a player looks at all cards in that zone

        // Get the library zone for the player
        let library_cards = self
            .player_zones
            .iter()
            .find(|(id, _)| *id == player)
            .map(|(_, zones)| zones.library.cards.clone())
            .ok_or_else(|| crate::MtgError::InvalidAction(format!("Player {:?} has no library", player)))?;

        // Search for a card matching the filter
        // Filter format examples:
        // - "Land.Basic" = Land type + Basic subtype
        // - "Creature" = Any Creature
        // - "Plains,Island" = Land with Plains OR Island subtype (fetch lands)
        // - "Artifact.Equipment" = Artifact type + Equipment subtype
        let mut found_card = None;
        for &card_id in &library_cards {
            if let Some(card) = self.cards.try_get(card_id) {
                let card_matches = Self::card_matches_search_filter(card, card_type_filter);

                if card_matches {
                    found_card = Some(card_id);
                    break;
                }
            }
        }

        // If we found a matching card, move it to the destination
        if let Some(card_id) = found_card {
            // Move the card from library to destination
            self.move_card(card_id, Zone::Library, destination, player)?;

            // If destination is battlefield and enters_tapped is true, tap the card
            if destination == Zone::Battlefield && enters_tapped {
                // Use helper that handles tap + undo log + mana version
                let _ = self.tap_permanent(card_id);
            }
        }

        // Shuffle the library if required (MTG Rules 701.19b)
        if shuffle {
            self.shuffle_library(player);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Card;
    use crate::game::GameState;

    /// Regression for mtg-677/mtg-908: on the SERVER side `execute_dig` must
    /// enqueue the kept card IDs into `pending_dig_decisions` so
    /// `NetworkController` can broadcast the decision to both clients before
    /// the next `ChoiceRequest`. Without this, the shadow re-derives the kept
    /// list from hidden card data → different picks → fatal state-hash desync.
    #[test]
    fn test_execute_dig_server_enqueues_dig_decision() {
        let mut game = GameState::new_two_player("P1".into(), "P2".into(), 20);
        game.set_skip_reveals(false); // network mode
                                      // NOT a shadow game → we are the server / authoritative
        let p1_id = game.players[0].id;

        // Construct a 3-card library for P1.
        let card_a = game.next_card_id();
        let card_b = game.next_card_id();
        let card_c = game.next_card_id();
        for cid in [card_a, card_b, card_c] {
            game.cards.insert(cid, Card::new(cid, "Mountain".to_string(), p1_id));
        }
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            // bottom-to-top: c is bottom, a is top
            zones.library.cards = vec![card_c, card_b, card_a];
        }

        // execute_dig: look at top 2, keep 1, rest to bottom.
        // Empty change_valid → all cards valid (no filter).
        game.execute_dig(
            2,             // dig_count
            1,             // change_count
            false,         // change_all
            Zone::Hand,    // destination (kept)
            Zone::Library, // rest_destination
            false,         // may_play
            false,         // may_play_without_mana_cost
            true,          // target_self
            false,         // optional
            false,         // rest_random
            false,         // reveal
            &[],           // change_valid (empty = all cards valid)
        )
        .expect("execute_dig must not error");

        // The server must have queued exactly one pending_dig_decision.
        let decisions = game.sub_action_scratch.pending_dig_decisions.borrow().clone();
        assert_eq!(
            decisions.len(),
            1,
            "server execute_dig must enqueue exactly one pending_dig_decision (mtg-677/mtg-908)"
        );
        let (digger, kept, _ac) = &decisions[0];
        assert_eq!(*digger, p1_id, "digger must be P1");
        assert_eq!(kept.len(), 1, "kept must have exactly 1 card (change_count=1)");

        // The shadow must NOT have a pending_dig_authoritative_decision
        // (the field is only populated by apply_state_sync, never by execute_dig).
        assert!(
            game.sub_action_scratch.pending_dig_authoritative_decision.is_none(),
            "pending_dig_authoritative_decision must remain None after server-side execute_dig"
        );
    }

    /// Regression for mtg-677/mtg-908: on the SHADOW side `execute_dig` must
    /// consume `pending_dig_authoritative_decision` and use it as the kept list
    /// instead of re-deriving via the heuristic. This ensures the shadow picks
    /// exactly the same cards as the server.
    #[test]
    fn test_execute_dig_shadow_uses_authoritative_decision() {
        let mut game = GameState::new_two_player("P1".into(), "P2".into(), 20);
        game.set_skip_reveals(false); // network mode
        game.is_shadow_game = true; // we are a shadow
        let p1_id = game.players[0].id;

        // Construct a 3-card library for P1: card_a = top, card_b = next, card_c = bottom.
        let card_a = game.next_card_id();
        let card_b = game.next_card_id();
        let card_c = game.next_card_id();
        for cid in [card_a, card_b, card_c] {
            game.cards
                .insert(cid, Card::new(cid, "Lightning Bolt".to_string(), p1_id));
        }
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            // bottom-to-top: c is bottom, a is top
            zones.library.cards = vec![card_c, card_b, card_a];
        }

        // Pre-populate the authoritative decision: "keep card_b" (the second card
        // from top). The heuristic would have picked card_a (top). This verifies
        // the shadow respects the server decision rather than running the heuristic.
        game.sub_action_scratch.pending_dig_authoritative_decision = Some(smallvec::smallvec![card_b]);

        game.execute_dig(
            2,             // dig_count
            1,             // change_count
            false,         // change_all
            Zone::Hand,    // destination
            Zone::Library, // rest_destination
            false,         // may_play
            false,         // may_play_without_mana_cost
            true,          // target_self
            false,         // optional
            false,         // rest_random
            false,         // reveal
            &[],           // change_valid (all cards valid — no filter)
        )
        .expect("execute_dig must not error");

        // card_b must now be in P1's hand (the authoritative kept card).
        let hand_ids = game
            .get_player_zones(p1_id)
            .map(|z| z.hand.cards.clone())
            .unwrap_or_default();
        assert!(
            hand_ids.contains(&card_b),
            "shadow must keep card_b as directed by authoritative decision, but hand={:?}",
            hand_ids
        );

        // card_a must NOT be in the hand (the heuristic would have kept it, but
        // the shadow must use the server's decision).
        assert!(
            !hand_ids.contains(&card_a),
            "shadow must NOT keep card_a when authoritative decision says keep card_b"
        );

        // The authoritative decision must have been consumed (cleared).
        assert!(
            game.sub_action_scratch.pending_dig_authoritative_decision.is_none(),
            "pending_dig_authoritative_decision must be consumed (None) after shadow execute_dig"
        );

        // The shadow must NOT enqueue a pending_dig_decision (server-only path).
        assert!(
            game.sub_action_scratch.pending_dig_decisions.borrow().is_empty(),
            "shadow execute_dig must NOT enqueue pending_dig_decisions"
        );
    }
}
