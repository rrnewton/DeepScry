//! Zone-movement effect-family handlers extracted from the `execute_effect`
//! dispatcher (see `game/actions/mod.rs`).
//!
//! This module will eventually hold the whole zone-change family
//! (Destroy/Exile/Return/Sacrifice/Search/ChangeZoneAll/Balance/Dig). It starts
//! with [`Effect::Dig`] — the "look at top N, keep some, put the rest
//! elsewhere" effect.
//!
//! ## mtg-908 — Dig keep-decision is information-INDEPENDENT (positional)
//!
//! [`Effect::Dig`]'s "which cards to keep" decision used to run an INLINE AI
//! value-ranking that peeked at the actual top-of-library card identities. On a
//! network game the server ranked the real top-N while a client SHADOW (where
//! those cards are hidden / unmaterialised) scored them all 0 and kept a
//! DIFFERENT subset → fatal state-hash desync (mtg-908: the user's 2025 04-vs-02
//! game died this way). CLAUDE.md requires controllers/effects to "produce
//! identical decisions whether running on the server (full state) or on a client
//! (shadow state)".
//!
//! The keep decision is now PURELY POSITIONAL — keep the first `max_select`
//! filter-matching cards in revealed (library top-down) order. That depends only
//! on the server-authoritative library ORDER, which the shadow learns via
//! LibraryReordered / reveals, so server and shadow agree.
//!
//! PARTIAL: this closes the UNFILTERED-dig desync class (Stock Up, Accumulate
//! Wisdom). FILTERED digs (e.g. Thundertrap Trainer's `ChangeValid$
//! Card.nonCreature+nonLand`) still desync because the shadow cannot apply the
//! filter to hidden cards at all — that needs the server's kept-set broadcast to
//! the shadow (a server-local-decision → shadow side-channel), tracked under
//! mtg-908 / mtg-677. mtg-908 stays OPEN for that case.

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
    /// NOTE (mtg-908): the self-dig keep decision is POSITIONAL (keep the first
    /// `max_select` matching cards in revealed order) — information-independent,
    /// so server and network-shadow agree. See the module doc.
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
        // mtg-908: `optional` no longer affects the keep COUNT (the old
        // hidden-info "skip if low value" heuristic was removed; the AI now
        // always takes `max_select`). Kept in the signature for the full
        // Effect::Dig parameter set / future controller-routed use.
        _optional: bool,
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

                // Determine how many cards to select.
                let max_select = if change_all {
                    valid_ids.len()
                } else {
                    (change_count as usize).min(valid_ids.len())
                };

                // mtg-908: keep the first `max_select` valid cards in REVEALED
                // (library top-down) order — a POSITIONAL, information-INDEPENDENT
                // decision. The previous code ranked `valid_ids` by
                // `dig_card_score` (creature P/T, land, CMC) and, for an optional
                // dig, skipped when the best card scored low. BOTH read the real
                // (hidden) top-of-library card identities, which a network
                // client's SHADOW cannot reproduce: there those cards are
                // unmaterialised, so the shadow scored them all 0, did NOT
                // reorder, and kept a DIFFERENT subset than the server — a FATAL
                // state-hash desync (mtg-908: the user's 2025 04-vs-02 game died
                // here). CLAUDE.md requires controllers/effects to "produce
                // identical decisions whether running on the server (full state)
                // or on a client (shadow state)". A positional keep depends only
                // on the server-authoritative library ORDER (which the shadow
                // learns via LibraryReordered / reveals), so both sides agree.
                //
                // Tradeoff (acceptable): the AI no longer keeps the "best"-valued
                // cards, just the first matching ones. Determinism outranks dig
                // keep-quality for automated play, and a HUMAN still chooses which
                // cards to keep through the UI (this fallback only drives AI /
                // no-controller resolution). For an `optional` dig the AI always
                // takes `max_select` (an information-independent default) rather
                // than the old hidden-info "skip if low value" heuristic.
                let select_count = max_select;

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
}
// NOTE (mtg-908): the old `dig_card_score` value-ranking heuristic was REMOVED.
// It read hidden top-of-library card identities to rank which dug cards to keep,
// which a network client's shadow cannot reproduce (those cards are
// unmaterialised there) — the root of the mtg-908 desync. The Dig keep decision
// is now purely positional (keep the first `max_select` matching cards in
// revealed order), which is information-independent.
