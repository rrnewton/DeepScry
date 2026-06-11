//! Zone-movement effect-family handlers extracted from the `execute_effect`
//! dispatcher (see `game/actions/mod.rs`).
//!
//! This module will eventually hold the whole zone-change family
//! (Destroy/Exile/Return/Sacrifice/Search/ChangeZoneAll/Balance/Dig). It starts
//! with [`Effect::Dig`] — the "look at top N, keep some, put the rest
//! elsewhere" effect — because that extraction is the structural prerequisite
//! for the mtg-908 network-desync fix.
//!
//! ## mtg-908 follow-on (READ BEFORE editing `execute_dig`)
//!
//! [`Effect::Dig`]'s "which cards to keep" decision currently runs an INLINE AI
//! heuristic ([`GameState::dig_card_score`]) that peeks at the actual (hidden)
//! library contents. On a network game the server scores the real top-N while
//! the client shadow scores its hidden-shadowed top-N, so the two pick
//! different cards → fatal state-hash desync (mtg-908: the user's 2025 04-vs-02
//! game died this way at turn 13).
//!
//! The fix (tracked in mtg-908, NOT done here) mirrors how Scry is handled:
//! intercept Dig in the `priority.rs` effect-resolution loop where the
//! `controller` handle is in scope, call a new
//! `controller.choose_dig_partition(...)` (server-authoritative, sent to the
//! client), and reduce THIS `execute_dig` to a controller-less fallback
//! (default keep-first-N), exactly like [`GameState::execute_scry`].
//!
//! THIS slice is purely structural / behavior-preserving: `execute_dig` keeps
//! the existing hidden-info heuristic verbatim. Keeping the whole self-dig
//! decision + application in one cohesive method is deliberate — it makes the
//! mtg-908 swap (decision → controller, application → a `dig_apply_decision`
//! helper) a clean follow-on rather than surgery on the giant dispatcher.

use crate::core::{CardId, DigFilter, PlayerId};
use crate::game::GameState;
use crate::zones::Zone;
use crate::Result;

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
    /// NOTE (mtg-908): the self-dig "which cards to keep" ranking below uses
    /// [`GameState::dig_card_score`] against the real (hidden) library — a
    /// network-desync hazard slated to move behind a server-authoritative
    /// controller choice. Preserved verbatim here (behavior-preserving
    /// extraction); see the module doc.
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

                // AI heuristic: rank valid cards by value, pick best ones
                // Score: creatures by (power+toughness)*10 + cmc*5 + 80,
                //        lands by 100, others by 50 + cmc*30
                if valid_ids.len() > 1 && max_select < valid_ids.len() {
                    valid_ids.sort_by(|&a, &b| {
                        let score_a = self.dig_card_score(a);
                        let score_b = self.dig_card_score(b);
                        score_b.cmp(&score_a) // Descending: best first
                    });
                }

                // If optional and no good cards, AI may choose to skip
                let select_count = if optional && max_select > 0 {
                    // Simple heuristic: skip only if best card scores very low
                    let best_score = valid_ids.first().map(|&id| self.dig_card_score(id)).unwrap_or(0);
                    if best_score < 30 {
                        0
                    } else {
                        max_select
                    }
                } else {
                    max_select
                };

                // Move selected cards to destination
                let selected: smallvec::SmallVec<[CardId; 8]> = valid_ids.iter().take(select_count).copied().collect();
                let rest_from_valid: smallvec::SmallVec<[CardId; 8]> =
                    valid_ids.iter().skip(select_count).copied().collect();

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
    /// NOTE (mtg-908): this reads the real (potentially hidden) card identity,
    /// which is the network-desync hazard. On the fix it becomes a legitimate
    /// controller-side decision over the controller's OWN view (server-
    /// authoritative). Kept here verbatim for the behavior-preserving extraction.
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
            // Check death triggers BEFORE moving the card (trigger still has access to card data)
            let _ = self.check_death_triggers(target);
            self.move_card(target, Zone::Battlefield, dest, owner)?;
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
                let _ = self.check_death_triggers(card_id);
                let card_name = self
                    .cards
                    .get(card_id)
                    .map(|c| c.name.to_string())
                    .unwrap_or_else(|_| "Unknown".to_string());
                self.move_card(
                    card_id,
                    Zone::Battlefield,
                    self.death_destination_for_card(card_id),
                    owner,
                )?;
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
            let _ = self.check_death_triggers(card_id);
            let card_name = self
                .cards
                .try_get(card_id)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            self.move_card(
                card_id,
                Zone::Battlefield,
                self.death_destination_for_card(card_id),
                owner,
            )?;
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
