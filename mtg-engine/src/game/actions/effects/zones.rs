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
            // Self-dig: look at top N cards of YOUR library.
            //
            // mtg-908: the "which cards to keep" decision below is made by the
            // CONTROLLER-LESS FALLBACK heuristic (`dig_default_decision`). On a
            // network game this site is only reached when the Dig was NOT routed
            // through the controller-interception path in `priority.rs`
            // (`resolve_dig_with_controller`). When it IS routed there, the
            // SERVER's controller picks and `dig_apply_self_decision` is called
            // directly with the server-authoritative kept-set — so server and
            // client never re-decide from divergent hidden-library views.
            let Some((card_ids, valid_ids, invalid_ids)) =
                self.dig_self_snapshot(digger, dig_count, change_valid, reveal)?
            else {
                return Ok(());
            };

            let decision = self.dig_default_decision(&valid_ids, change_count, change_all, optional);
            self.dig_apply_self_decision(
                digger,
                &decision,
                &card_ids,
                &invalid_ids,
                destination,
                rest_destination,
                reveal,
                rest_random,
                &mut moved_cards,
            )?;
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
        self.dig_apply_may_play(digger, may_play, may_play_without_mana_cost, &mut moved_cards);
        Ok(())
    }

    /// Apply the Dig `MayPlay$` rider: if `may_play` and any cards were moved,
    /// announce the "until end of turn you may play one of those cards" grant,
    /// and (for `may_play_without_mana_cost`) install the
    /// `MayPlayOneWithoutManaCost` persistent effect over `moved_cards`
    /// (Impulse-style). Shared by [`GameState::execute_dig`] (no-controller
    /// fallback) and the priority.rs controller-routed Dig interception so the
    /// rider behaves identically on both paths (mtg-908).
    pub(crate) fn dig_apply_may_play(
        &mut self,
        digger: PlayerId,
        may_play: bool,
        may_play_without_mana_cost: bool,
        moved_cards: &mut Vec<CardId>,
    ) {
        if !may_play || moved_cards.is_empty() {
            return;
        }
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
                    tracked_cards: std::mem::take(moved_cards),
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

    /// Snapshot the top `dig_count` cards of `digger`'s library for a self-dig,
    /// emit the "looks at / reveals the top N" gamelog, and partition them into
    /// the filter-matching `valid` set and the non-matching `invalid` set.
    ///
    /// Returns `None` only when the digger has no library zone (nothing to do).
    /// Returns the full revealed top-N (`card_ids`, top-down) plus the partition.
    /// This is the server-authoritative "reveal" that BOTH the controller-routed
    /// path (priority.rs) and the no-controller fallback (`execute_dig`) share —
    /// so the revealed CardIds shipped to the client match what the server saw.
    pub(crate) fn dig_self_snapshot(
        &mut self,
        digger: PlayerId,
        dig_count: u8,
        change_valid: &[DigFilter],
        reveal: bool,
    ) -> Result<
        Option<(
            smallvec::SmallVec<[CardId; 8]>,
            smallvec::SmallVec<[CardId; 8]>,
            smallvec::SmallVec<[CardId; 8]>,
        )>,
    > {
        let Some(library) = self
            .player_zones
            .iter()
            .find(|(id, _)| *id == digger)
            .map(|(_, zones)| &zones.library)
        else {
            return Ok(None);
        };

        let take_count = dig_count as usize;
        // Library top is at the end of the Vec, so use .rev()
        let card_ids: smallvec::SmallVec<[CardId; 8]> = library.cards.iter().rev().take(take_count).copied().collect();

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

        // Separate cards into valid (matchable) and invalid (rest).
        // If change_valid is empty, all cards are valid.
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

        Ok(Some((card_ids, valid_ids, invalid_ids)))
    }

    /// The controller-LESS fallback Dig decision: rank the `valid` cards by
    /// [`GameState::dig_card_score`] and keep the best `change_count` (all when
    /// `change_all`), with the existing `optional`-skip heuristic. This is the
    /// behavior the engine used inline before mtg-908; it is kept as the
    /// fallback used by [`GameState::execute_dig`] (when Dig is not routed
    /// through a controller) and mirrored by the heuristic controller.
    ///
    /// NOTE (mtg-908): this reads hidden top-of-library identities, so it is
    /// information-DEPENDENT and MUST NOT be the deciding authority on a client
    /// shadow. The controller-routed path makes the SERVER's result authoritative
    /// and ships it to the client; this fallback only runs server-side or in
    /// pure-local games where there is no shadow to diverge.
    pub(crate) fn dig_default_decision(
        &self,
        valid_ids: &[CardId],
        change_count: u8,
        change_all: bool,
        _optional: bool,
    ) -> crate::game::controller::DigDecision {
        // mtg-908: keep the first `change_count` valid cards in REVEALED order —
        // POSITIONAL and information-INDEPENDENT. The previous `dig_card_score`
        // value-ranking read hidden top-of-library identities, which a network
        // client's shadow cannot see, so server and shadow kept different cards →
        // FATAL desync. A positional keep depends only on the server-authoritative
        // library ORDER (synced to the shadow), so both sides agree. Shared with
        // the heuristic controller via `DigDecision::keep_first_n` (one rule).
        crate::game::controller::DigDecision::keep_first_n(valid_ids, change_count as usize, change_all)
    }

    /// Apply a self-dig `decision` (the server-authoritative kept-set): move the
    /// kept cards to `destination`, then everything else (non-kept valid + all
    /// invalid) to `rest_destination`, honoring `rest_random` and emitting the
    /// reveal-timing-independent gamelog lines.
    ///
    /// `card_ids` is the full revealed top-N (top-down) from
    /// [`GameState::dig_self_snapshot`]; `decision.kept` is a subset of the
    /// valid cards. The "rest" is computed here as "every revealed card not in
    /// `kept`", preserving revealed order — identical to the pre-mtg-908 result
    /// (non-selected valid cards, in their order, followed by invalid cards).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dig_apply_self_decision(
        &mut self,
        digger: PlayerId,
        decision: &crate::game::controller::DigDecision,
        card_ids: &[CardId],
        invalid_ids: &[CardId],
        destination: Zone,
        rest_destination: Zone,
        reveal: bool,
        rest_random: bool,
        moved_cards: &mut Vec<CardId>,
    ) -> Result<()> {
        let digger_name = self.get_player(digger)?.name.to_string();
        let kept = &decision.kept;

        // Move selected (kept) cards to destination, in selection order.
        for &card_id in kept.iter() {
            let card_name = self
                .cards
                .get(card_id)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|_| format!("card#{}", card_id.as_u32()));

            self.move_card(card_id, Zone::Library, destination, digger)?;

            // mtg-212: the DISPLAYED name depends on async reveal timing on a
            // network shadow; supply the rewind/replay verifier a reveal-timing-
            // INDEPENDENT id form so the presentation asymmetry is not flagged as
            // a fatal desync. Same mechanism as the discard-into-graveyard line.
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

        // Handle rest: every revealed card NOT kept, in revealed order. This
        // reproduces the pre-mtg-908 "non-selected valid cards then invalid
        // cards" set — `card_ids` is valid-and-invalid interleaved in reveal
        // order, but the kept set is drawn only from valid cards, so filtering
        // `card_ids` by "not kept" yields exactly the same multiset; we order it
        // as (non-kept valid in reveal order) ++ (invalid in reveal order) to
        // match the legacy concatenation `rest_from_valid ++ invalid_ids`.
        let mut rest_cards: smallvec::SmallVec<[CardId; 8]> = smallvec::SmallVec::new();
        for &card_id in card_ids {
            if !kept.contains(&card_id) && !invalid_ids.contains(&card_id) {
                rest_cards.push(card_id);
            }
        }
        rest_cards.extend(invalid_ids.iter().copied());

        if !rest_cards.is_empty() {
            // Shuffle rest if RestRandomOrder$ True.
            if rest_random {
                // Deterministic shuffle based on game state (card IDs provide
                // enough entropy for reasonable shuffling).
                let len = rest_cards.len();
                for i in (1..len).rev() {
                    let j = (rest_cards[i].as_u32() as usize + i) % (i + 1);
                    rest_cards.swap(i, j);
                }
            }

            // Move rest to rest_destination.
            if rest_destination == Zone::Library {
                // Capture pre-reorder library order so a rewind can restore it
                // (mtg-ba6uq #2): the raw remove/add_to_bottom below is not
                // otherwise undo-logged.
                self.log_library_reorder(digger, false);
                if let Some(zones) = self.get_player_zones_mut(digger) {
                    for &card_id in &rest_cards {
                        zones.library.remove(card_id);
                        zones.library.add_to_bottom(card_id);
                    }
                }
                // NETWORK SYNC (mtg-908, mirrors scry_apply_decision): on the
                // SERVER in network mode, signal that the digger's new library
                // order must be broadcast to clients so their shadow libraries
                // re-sync the bottomed cards. The NetworkController drains this
                // queue into the next ChoiceRequest. `reorder_ac` is the undo-log
                // length right after `log_library_reorder` logged the
                // ReorderLibrary action (the raw reorder ops above log nothing).
                if !self.skip_reveals && !self.is_shadow_game {
                    let reorder_ac = self.undo_log.len() as u64;
                    self.sub_action_scratch
                        .pending_library_reorders
                        .borrow_mut()
                        .push((digger, reorder_ac));
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
                    let stable = format!("{} puts card#{} into {}", digger_name, card_id.as_u32(), dest_name);
                    self.logger.gamelog_reveal_stable(
                        &format!("{} puts {} into {}", digger_name, card_name, dest_name),
                        &stable,
                    );
                }
            }
        }
        Ok(())
    }
}
// NOTE (mtg-908): the old `dig_card_score` value-ranking heuristic was REMOVED.
// It read hidden top-of-library card identities to rank which cards to keep,
// which a network client's shadow cannot reproduce (the cards are unmaterialised
// there) — the root of the mtg-908 desync. The Dig keep decision is now purely
// positional (`DigDecision::keep_first_n`), which is information-independent.
