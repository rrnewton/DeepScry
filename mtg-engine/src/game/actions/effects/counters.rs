//! Counter-manipulation effect-family handlers extracted from the
//! `execute_effect` dispatcher (see `game/actions/mod.rs`).
//!
//! Groups the effects that add, remove, multiply, or proliferate counters on
//! permanents (CR 122, CR 701.34):
//! - [`Effect::PutCounter`] / [`Effect::PutCounterAll`],
//! - [`Effect::RemoveCounter`],
//! - [`Effect::MultiplyCounter`] (Doubling Season-style),
//! - [`Effect::Proliferate`] (CR 701.34a).
//!
//! Each handler is a thin `impl GameState` method; `execute_effect` matches the
//! variant and delegates here. Behavior-preserving: bodies moved verbatim.

use crate::core::{CardId, CounterType};
use crate::game::GameState;
use crate::Result;

impl GameState {
    /// [`Effect::PutCounter`]: add `amount` counters of `counter_type` to the
    /// target. Handles `Defined$ Remembered` (apply to every card in
    /// `remembered_cards`) and fizzles on an unresolved/placeholder target.
    pub(in crate::game::actions) fn execute_put_counter(
        &mut self,
        target: CardId,
        counter_type: CounterType,
        amount: u8,
    ) -> Result<()> {
        // Skip if target is still placeholder (0) or unresolved sentinel
        if target.is_placeholder() || target.is_reuse_previous() {
            return Ok(());
        }
        // `Defined$ Remembered` (e.g. All Hallow's Eve's chained PutCounter after
        // a RememberChanged self-exile) — apply the counters to every card
        // currently in `remembered_cards`. Clone first to avoid the &self borrow
        // held by `iter()` conflicting with `add_counters`'s &mut self.
        if target.is_remembered_card() {
            let remembered: smallvec::SmallVec<[CardId; 4]> = self.remembered_cards.iter().copied().collect();
            if remembered.is_empty() {
                log::debug!(
                    target: "put_counter",
                    "PutCounter Defined$ Remembered with empty remembered_cards list, skipping"
                );
                return Ok(());
            }
            for cid in remembered {
                self.add_counters(cid, counter_type, amount)?;
            }
            return Ok(());
        }
        // Add counters using the GameState method (which logs for undo)
        self.add_counters(target, counter_type, amount)?;
        Ok(())
    }

    /// [`Effect::MultiplyCounter`]: multiply the counters on the target by
    /// `multiplier` — a specific `counter_type` if given, else every counter
    /// type the card has (Doubling Season / Vorinclex-style).
    pub(in crate::game::actions) fn execute_multiply_counter(
        &mut self,
        target: CardId,
        counter_type: Option<CounterType>,
        multiplier: u8,
    ) -> Result<()> {
        if target.is_placeholder() {
            return Ok(());
        }
        // Multiply counters on the target card
        if let Some(card) = self.cards.try_get(target) {
            let counters_to_add: smallvec::SmallVec<[(CounterType, u8); 4]> = if let Some(ct) = counter_type {
                // Multiply specific counter type
                let current = card.get_counter(ct);
                if current > 0 {
                    let to_add = current.saturating_mul(multiplier - 1);
                    smallvec::smallvec![(ct, to_add)]
                } else {
                    smallvec::SmallVec::new()
                }
            } else {
                // Multiply ALL counter types on the card
                card.counters
                    .iter()
                    .filter_map(|(ct, count)| {
                        if *count > 0 {
                            Some((*ct, count.saturating_mul(multiplier - 1)))
                        } else {
                            None
                        }
                    })
                    .collect()
            };

            for (ct, amount) in counters_to_add {
                if amount > 0 {
                    self.add_counters(target, ct, amount)?;
                }
            }
        }
        Ok(())
    }

    /// [`Effect::PutCounterAll`]: put `amount` counters of `counter_type` on
    /// every permanent matching `restriction` (controller-aware).
    pub(in crate::game::actions) fn execute_put_counter_all(
        &mut self,
        restriction: &crate::core::effects::TargetRestriction,
        counter_type: CounterType,
        amount: u8,
    ) -> Result<()> {
        // Put counters on all permanents matching the restriction
        let spell_controller = self.turn.active_player;
        let targets: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| {
                self.cards
                    .try_get(card_id)
                    .is_some_and(|card| restriction.matches_with_controller(card, spell_controller, card.controller))
            })
            .collect();

        for card_id in targets {
            self.add_counters(card_id, counter_type, amount)?;
        }
        Ok(())
    }

    /// [`Effect::Proliferate`] (CR 701.34a): give each permanent that already
    /// has a counter one additional counter of each kind it has. Automated play
    /// proliferates every eligible permanent; the controller's choice of which
    /// to skip is handled earlier (at the should-cast level).
    pub(in crate::game::actions) fn execute_proliferate(&mut self) -> Result<()> {
        let permanents_with_counters: Vec<(CardId, Vec<CounterType>)> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter_map(|card_id| {
                let card = self.cards.try_get(card_id)?;
                if card.has_counters() {
                    let counter_types: Vec<CounterType> = card
                        .counters
                        .iter()
                        .filter(|(_, count)| *count > 0)
                        .map(|(ct, _)| *ct)
                        .collect();
                    if counter_types.is_empty() {
                        None
                    } else {
                        Some((card_id, counter_types))
                    }
                } else {
                    None
                }
            })
            .collect();

        for (card_id, counter_types) in permanents_with_counters {
            for ct in counter_types {
                self.add_counters(card_id, ct, 1)?;
            }
        }
        Ok(())
    }

    /// [`Effect::RemoveCounter`]: remove `amount` counters from the target — a
    /// specific `counter_type` if given, else up to `amount` total across all
    /// counter types present (`CounterType$ Any`). Fizzles on a placeholder
    /// target.
    pub(in crate::game::actions) fn execute_remove_counter(
        &mut self,
        target: CardId,
        counter_type: Option<CounterType>,
        amount: u8,
    ) -> Result<()> {
        // Skip if target is still placeholder (0) - no valid targets found
        if target.is_placeholder() {
            // Spell fizzles - no valid targets
            return Ok(());
        }
        // Remove counters using the GameState method (which logs for undo)
        if let Some(ct) = counter_type {
            // Specific counter type
            self.remove_counters(target, ct, amount)?;
        } else {
            // CounterType$ Any - remove counters of any type. Get all counter
            // types present on the card and remove up to `amount` total.
            let mut remaining = amount;
            let counter_types: smallvec::SmallVec<[CounterType; 4]> = {
                let card = self.cards.get(target)?;
                card.counters.iter().map(|(ct, _)| *ct).collect()
            };

            for ct in counter_types {
                if remaining == 0 {
                    break;
                }
                let removed = self.remove_counters(target, ct, remaining)?;
                remaining = remaining.saturating_sub(removed);
            }
        }
        Ok(())
    }
}
