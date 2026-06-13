//! Tap / untap effect-family handlers extracted from the `execute_effect`
//! dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the effects that tap or untap permanents (CR 701.21 / 701.22):
//! - [`Effect::TapPermanent`] / [`Effect::UntapPermanent`] / single-target,
//! - [`Effect::TapOrUntapPermanent`] (AI chooses),
//! - [`Effect::TapAll`] / [`Effect::UntapAll`] (mass, filtered),
//! - [`Effect::UntapOne`] (untap exactly one matching permanent, e.g. Hokori).
//!
//! All handlers route tap/untap through the `tap_permanent` / `untap_permanent`
//! helpers (NOT direct `card.tapped` writes) so the undo log, the
//! `ManaSourceCache` untapped counts, and `mana_state_version` stay consistent
//! — a direct write would desync server vs client shadow state (see
//! `docs/NETWORK_ARCHITECTURE.md`). Behavior-preserving: bodies moved verbatim.

use crate::core::{CardId, PlayerId, TriggerEvent};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::TapPermanent`]: tap the target and fire `Taps` triggers
    /// (CR 701.21). Fizzles on an unresolved/placeholder target.
    pub(in crate::game::actions) fn execute_tap_permanent(&mut self, target: CardId) -> Result<()> {
        // Skip if target is still placeholder (0) or unresolved sentinel
        if target.is_placeholder() || target.is_reuse_previous() {
            // Spell fizzles - no valid targets
            return Ok(());
        }
        // Use helper that handles tap + undo log + mana version
        self.tap_permanent(target)?;
        // Check for Taps triggers
        self.check_triggers(TriggerEvent::Taps, target)?;
        Ok(())
    }

    /// [`Effect::UntapPermanent`]: untap the target (CR 701.22). Fizzles on an
    /// unresolved/placeholder target.
    pub(in crate::game::actions) fn execute_untap_permanent(&mut self, target: CardId) -> Result<()> {
        // Skip if target is placeholder (0) or unresolved sentinel
        if target.is_placeholder() || target.is_reuse_previous() {
            return Ok(());
        }
        // Emit gamelog before untapping so the log is readable
        // (B23: Ley Druid / similar untap-land effects were silent).
        if let Some(card) = self.cards.try_get(target) {
            if card.tapped {
                let card_name = card.name.clone();
                self.logger
                    .gamelog(&format!("{} ({}) untaps", card_name, target.as_u32()));
            }
        }
        // Use helper that handles untap + undo log + mana version
        self.untap_permanent(target)?;
        Ok(())
    }

    /// [`Effect::TapOrUntapPermanent`]: tap OR untap the target, with the AI
    /// choosing — heuristic: untap our own permanents (free mana / ready to
    /// block), tap the opponent's (remove blocker / deny mana). Taps fire
    /// `Taps` triggers.
    pub(in crate::game::actions) fn execute_tap_or_untap_permanent(&mut self, target: CardId) -> Result<()> {
        // Tap or untap target permanent (AI chooses)
        // Heuristic: untap our own creatures, tap opponent's
        if target.is_placeholder() {
            return Ok(());
        }
        if let Some(card) = self.cards.try_get(target) {
            let is_ours = card.controller == self.turn.active_player;
            if is_ours {
                // Untap our own permanent (free mana, ready to block)
                self.untap_permanent(target)?;
            } else {
                // Tap opponent's permanent (remove blocker, deny mana)
                self.tap_permanent(target)?;
                self.check_triggers(TriggerEvent::Taps, target)?;
            }
        }
        Ok(())
    }

    /// [`Effect::TapAll`]: tap every untapped permanent matching `restriction`.
    pub(in crate::game::actions) fn execute_tap_all(
        &mut self,
        restriction: &crate::core::effects::TargetRestriction,
    ) -> Result<()> {
        // Tap all permanents matching the restriction
        let targets: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| {
                self.cards
                    .get(card_id)
                    .map(|card| !card.tapped && restriction.matches(card))
                    .unwrap_or(false)
            })
            .collect();

        for card_id in targets {
            // Route through tap_permanent so the undo log, ManaSourceCache
            // untapped counts, and mana_state_version all stay consistent.
            // Setting `card.tapped` directly would leave the mana cache
            // reporting these sources as still untapped, which can offer an
            // unaffordable spell as a legal play and diverge server vs client
            // shadow state (network desync). See docs/NETWORK_ARCHITECTURE.md.
            let card_name = self.cards.get(card_id)?.name.clone();
            self.tap_permanent(card_id)?;
            self.logger.gamelog(&format!("{} ({}) is tapped", card_name, card_id));
        }
        Ok(())
    }

    /// [`Effect::TapPermanentsMatchingFilter`]: tap up to `count` untapped
    /// permanents of the eligible types (e.g. `"Artifact,Creature,Land"`)
    /// controlled by `player`.
    ///
    /// Used by Tangle Wire: "that player taps an untapped artifact, creature,
    /// or land they control for each fade counter on Tangle Wire."
    ///
    /// AI heuristic: tap cheapest/least-impactful first — prefer artifacts over
    /// creatures over lands, matching the discard-least-valuable principle from
    /// the project coding guide.
    pub(in crate::game::actions) fn execute_tap_permanents_matching_filter(
        &mut self,
        player: PlayerId,
        choices_filter: &str,
        count: u8,
    ) -> Result<()> {
        if count == 0 {
            return Ok(());
        }

        // Parse filter: comma-separated type names, e.g. "Artifact,Creature,Land"
        let filter_types: Vec<&str> = choices_filter.split(',').map(str::trim).collect();

        // Collect all untapped permanents matching the filter controlled by the player
        let mut candidates: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&cid| {
                self.cards
                    .get(cid)
                    .map(|card| {
                        card.controller == player
                            && !card.tapped
                            && filter_types.iter().any(|&t| match t {
                                "Artifact" => card.is_artifact(),
                                "Creature" => card.is_creature(),
                                "Land" => card.is_land(),
                                _ => false,
                            })
                    })
                    .unwrap_or(false)
            })
            .collect();

        // AI heuristic: tap least-valuable first.
        // Priority (tap first → least valuable): Artifact > Creature > Land.
        // Within each type, no further ordering for now (stable sort preserves
        // insertion order as a tiebreaker).
        candidates.sort_by_key(|&cid| {
            self.cards
                .get(cid)
                .map(|card| {
                    if card.is_land() {
                        2u8 // tap lands last (most valuable for mana)
                    } else if card.is_creature() {
                        1u8 // creatures second
                    } else {
                        0u8 // tap artifacts first (least disruptive)
                    }
                })
                .unwrap_or(3)
        });

        let to_tap = candidates.into_iter().take(count as usize);
        for cid in to_tap {
            let card_name = self.cards.get(cid)?.name.clone();
            self.tap_permanent(cid)?;
            self.check_triggers(TriggerEvent::Taps, cid)?;
            self.logger
                .gamelog(&format!("{} (forced by Tangle Wire) is tapped", card_name));
        }
        Ok(())
    }

    /// [`Effect::UntapAll`]: untap every tapped permanent matching `restriction`
    /// (controller-aware).
    pub(in crate::game::actions) fn execute_untap_all(
        &mut self,
        restriction: &crate::core::effects::TargetRestriction,
    ) -> Result<()> {
        // Untap all permanents matching the restriction
        let spell_controller = self.turn.active_player;
        let targets: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| {
                self.cards
                    .get(card_id)
                    .map(|card| {
                        card.tapped && restriction.matches_with_controller(card, spell_controller, card.controller)
                    })
                    .unwrap_or(false)
            })
            .collect();

        for card_id in targets {
            // Route through untap_permanent so the undo log, ManaSourceCache
            // untapped counts, and mana_state_version stay consistent (see
            // the matching note in Effect::TapAll above).
            let card_name = self.cards.get(card_id)?.name.clone();
            self.untap_permanent(card_id)?;
            self.logger.gamelog(&format!("{} ({}) is untapped", card_name, card_id));
        }
        Ok(())
    }

    /// [`Effect::UntapOne`]: untap the first tapped permanent matching
    /// `restriction` (controller-aware). Used by Hokori, Dust Drinker's upkeep
    /// trigger — "that player untaps a land they control" (CR 701.22).
    pub(in crate::game::actions) fn execute_untap_one(
        &mut self,
        restriction: &crate::core::effects::TargetRestriction,
    ) -> Result<()> {
        let spell_controller = self.turn.active_player;
        let target = self.battlefield.cards.iter().copied().find(|&card_id| {
            self.cards
                .get(card_id)
                .map(|card| card.tapped && restriction.matches_with_controller(card, spell_controller, card.controller))
                .unwrap_or(false)
        });

        if let Some(card_id) = target {
            let card_name = self.cards.get(card_id)?.name.clone();
            self.untap_permanent(card_id)?;
            self.logger.gamelog(&format!("{} ({}) is untapped", card_name, card_id));
        }
        Ok(())
    }
}
