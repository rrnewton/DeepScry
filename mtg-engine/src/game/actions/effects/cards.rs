//! Card-flow effect-family handlers (draw / mill / scry / surveil) extracted
//! from the `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the effects that move cards between a player's library, hand, and
//! graveyard or reorder the top of the library:
//! - [`Effect::DrawCards`] (CR 120),
//! - [`Effect::Mill`] (CR 701.13),
//! - [`Effect::Scry`] (CR 701.18) — fallback path only,
//! - [`Effect::Surveil`] (CR 701.42) — fallback path only.
//!
//! NOTE on scry/surveil: the "real" controller-dispatched scry/surveil lives in
//! `game_loop/priority.rs` (it needs controller access to ask the player how to
//! order the revealed cards). The handlers here are the **fallback** path taken
//! only when execute_effect is reached without controller access (legacy v1
//! cast+resolve, or a direct test-harness call); they default to the safest
//! information-preserving no-op (keep every revealed card on top in order).
//!
//! Discard / Loot are NOT here: they already delegate to the shared
//! `execute_discard_effect` helper at the dispatch site (it threads the
//! discard "cause" for forced discards), so there is nothing to extract.

use crate::core::PlayerId;
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::DrawCards`]: the player (or each remembered player) draws
    /// `count` cards, firing card-drawn triggers per draw (CR 120).
    ///
    /// Each individual draw is first offered to the Dredge replacement
    /// (CR 702.52): if the player has a Dredge card in their graveyard and
    /// enough library cards, the draw is replaced by "mill N, return dredge
    /// card to hand". The AI always accepts this replacement.
    pub(in crate::game::actions) fn execute_draw_cards(&mut self, player: PlayerId, count: u8) -> Result<()> {
        if player.is_remembered_players() {
            // Draw for each player stored in remembered_players
            // Clone to avoid borrow conflict during mutation
            let players: smallvec::SmallVec<[PlayerId; 4]> = self.remembered_players.iter().copied().collect();
            for pid in players {
                for _ in 0..count {
                    if !self.try_apply_dredge(pid)? {
                        let (_, draw_num) = self.draw_card(pid)?;
                        self.check_card_drawn_triggers(pid, draw_num)?;
                    }
                }
            }
        } else {
            for _ in 0..count {
                if !self.try_apply_dredge(player)? {
                    let (_, draw_num) = self.draw_card(player)?;
                    // Check for "second card drawn" triggers
                    self.check_card_drawn_triggers(player, draw_num)?;
                }
            }
        }
        Ok(())
    }

    /// [`Effect::Mill`]: move `count` cards from the top of the player's library
    /// to their graveyard (CR 701.13).
    pub(in crate::game::actions) fn execute_mill(&mut self, player: PlayerId, count: u8) -> Result<()> {
        // Mill cards from library to graveyard
        self.mill_cards(player, count)?;
        Ok(())
    }

    /// [`Effect::RearrangeTopOfLibrary`]: look at the top `count` cards of
    /// `player`'s library, then put them back in any order (CR 701.22 "look
    /// at").
    ///
    /// **AI path**: the current library order is kept unchanged. Putting cards
    /// back in the same order is always a legal choice under the rules
    /// ("you may arrange them in any order" does not require re-ordering).
    /// The main benefit is that the ability resolves without emitting an
    /// `Unimplemented` warning, eliminating 15 k+ spurious warnings in the
    /// 2005 World Championship game set (Sensei's Divining Top — mtg-910 B2).
    ///
    /// **MTG rules**: CR 701.22a — "To 'look at' a card, you look at it
    /// without revealing it." The cards stay on top of the library in the
    /// chosen order; this executor leaves them in the same order, which is
    /// one valid outcome of the choice.
    pub(in crate::game::actions) fn execute_rearrange_top_of_library(
        &mut self,
        player: PlayerId,
        count: u8,
    ) -> Result<()> {
        let top_count = {
            let lib_len = self
                .get_player_zones(player)
                .map(|z| z.library.cards.len())
                .unwrap_or(0);
            (count as usize).min(lib_len)
        };

        if top_count == 0 {
            return Ok(());
        }

        let p = player.as_u32() + 1;
        self.logger.gamelog(&format!(
            "P{} looks at the top {} card{} of their library, puts them back in the same order",
            p,
            top_count,
            if top_count == 1 { "" } else { "s" }
        ));

        // The library order is not changed (valid choice per CR 701.22).
        // No undo log entry needed — nothing was mutated.
        Ok(())
    }

    /// [`Effect::Scry`] — fallback path (CR 701.18). The controller-dispatched
    /// path lives in `priority.rs`; reaching execute_effect means there is no
    /// controller access, so default to keeping every revealed card on top in
    /// its original order (a true no-op that never destroys information).
    pub(in crate::game::actions) fn execute_scry(&mut self, player: PlayerId, count: u8) -> Result<()> {
        let revealed = self.scry_snapshot_top_n(player, count);
        if !revealed.is_empty() {
            let decision = crate::game::ScryDecision::keep_all_on_top(&revealed);
            self.scry_apply_decision(player, &revealed, &decision)?;
        }
        Ok(())
    }

    /// [`Effect::Surveil`] — fallback path (CR 701.42). Same rationale as
    /// [`GameState::execute_scry`]: default to "no cards milled", preserving
    /// library order, when no controller is available.
    pub(in crate::game::actions) fn execute_surveil(&mut self, player: PlayerId, count: u8) -> Result<()> {
        let revealed = self.surveil_snapshot_top_n(player, count);
        if !revealed.is_empty() {
            let decision = crate::game::SurveilDecision::keep_all_on_top(&revealed);
            self.surveil_apply_decision(player, &revealed, &decision)?;
        }
        Ok(())
    }
}
