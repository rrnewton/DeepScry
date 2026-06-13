//! Mana-payment EXECUTION on the live game state.
//!
//! Split out of the former monolithic `game/actions/mod.rs` (this is a pure
//! structural refactor — no behavior change). Where the sibling resolver code
//! in this module's `mod.rs` decides *which* sources to tap to satisfy a cost
//! (the `ManaPaymentResolver` algorithms), THIS file performs the actual
//! state mutation: tapping permanents for mana, draining the mana pool to pay
//! a cost, and paying the non-mana parts of an activated ability's cost
//! (tap/sacrifice/counters/life/etc.).
//!
//! These are inherent methods on [`GameState`], added here via a dedicated
//! `impl GameState` block so the payment logic lives next to the resolver it
//! drives rather than buried in the 11k-line action dispatcher.
//!
//! See `mana_payment/README.md` for the module layout.

use crate::core::{CardId, PlayerId, TriggerEvent};
use crate::game::GameState;
use crate::zones::Zone;
use crate::{MtgError, Result};

impl GameState {
    /// Tap a land for mana (without cost hint)
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be tapped for mana.
    pub fn tap_for_mana(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        // Create an empty cost hint
        let empty_cost = crate::core::ManaCost::new();
        self.tap_for_mana_for_cost(player_id, card_id, &empty_cost)
    }

    /// Compute the set of colors that lands controlled by `player_id`'s
    /// opponents could produce — the "reflected" color set for Fellwar Stone's
    /// `AB$ ManaReflected | Valid$ Land.OppCtrl | ReflectProperty$ Produce`
    /// (CR 106.7 / Fellwar Stone Oracle). Derived purely from public battlefield
    /// state (each land's static mana-production cache), so it is
    /// information-independent and identical on server and every client.
    ///
    /// A land that itself produces "any color" contributes all five colors. A
    /// land that produces only colorless contributes nothing (colorless is not a
    /// color; CR 105.1). The result is the union across all matching lands.
    pub(crate) fn reflected_mana_colors(&self, player_id: PlayerId) -> crate::game::mana_colors::ManaColors {
        use crate::core::{ManaColor, ManaProductionKind};
        use crate::game::mana_colors::ManaColors;

        let mut colors = ManaColors::new();
        for &land_id in self.battlefield.cards.iter() {
            let Some(card) = self.cards.try_get(land_id) else {
                continue;
            };
            // Valid$ Land.OppCtrl: lands controlled by an opponent of player_id.
            if !card.is_land() || card.controller == player_id {
                continue;
            }
            match card.definition.cache.mana_production.kind {
                ManaProductionKind::Fixed(c) => colors.insert(c),
                ManaProductionKind::Choice(set) => {
                    for c in set.iter() {
                        colors.insert(c);
                    }
                }
                ManaProductionKind::AnyColor => {
                    for c in [
                        ManaColor::White,
                        ManaColor::Blue,
                        ManaColor::Black,
                        ManaColor::Red,
                        ManaColor::Green,
                    ] {
                        colors.insert(c);
                    }
                }
                // Colorless lands (e.g. Wastes) produce no color (CR 105.1).
                ManaProductionKind::Colorless => {}
            }
        }
        colors
    }

    /// Compute the mana production a source effectively offers *for payment
    /// resolution*, resolving the dynamic colour set of reflected-mana sources
    /// (Fellwar Stone) to the colours they can actually produce right now.
    ///
    /// The static `CardCache` models Fellwar Stone's `AB$ ManaReflected` ability
    /// as an unconstrained `AnyColor` upper bound, because the producible colour
    /// set depends on what lands opponents control at activation time and cannot
    /// be baked into the per-card cache. That upper bound is fine for a quick
    /// "could this ever produce colour X" check, but it makes the payment
    /// resolver (`GreedyManaResolver`) believe a Fellwar Stone can pay *any*
    /// coloured pip. The resolver then commits to a tap order that taps Fellwar
    /// Stone for, say, `{R}` — but the activation path (`tap_for_mana_for_cost`)
    /// honestly intersects with the reflected set and produces a *different*
    /// colour when red is not reflected. The coloured pip is never paid, the
    /// spell stays unpayable, and a heuristic/zero AI that keeps re-attempting it
    /// loops until the 1000-action priority guard trips (mtg-893).
    ///
    /// Resolving the reflected set HERE — to a concrete `Choice(colors)` over the
    /// colours opponents' lands could currently produce — makes the resolver's
    /// affordability decision exactly match what execution will produce, so the
    /// AI never commits to an unpayable cost and the loop cannot occur. It is
    /// derived purely from public battlefield state, so it is identical on the
    /// server and every client (no information leakage; CLAUDE.md network
    /// invariant) and is deterministic (canonical WUBRG bitset ordering).
    ///
    /// For non-reflected sources this returns the card's cached production
    /// unchanged.
    pub(crate) fn effective_production_for_resolution(
        &self,
        card_id: CardId,
        player_id: PlayerId,
    ) -> crate::core::ManaProduction {
        use crate::core::{ManaProduction, ManaProductionKind};

        let Some(card) = self.cards.try_get(card_id) else {
            return ManaProduction::default();
        };
        let cached = card.definition.cache.mana_production;

        let is_reflected = card
            .activated_abilities
            .iter()
            .any(|ab| ab.is_mana_ability && ab.produces_reflected_mana);
        if !is_reflected {
            return cached;
        }

        // Resolve the reflected colour set from the current board and model the
        // source as a `Choice` over exactly those colours. We deliberately keep
        // the `Choice` kind (rather than collapsing a single colour to `Fixed`
        // or an empty set to `Colorless`) so the source's complex/simple
        // CLASSIFICATION is unchanged from the static `AnyColor` cache — both the
        // live `read_from_cache` path and the debug `compute_from_scratch`
        // verifier agree on which bucket the source falls in. The resolver's
        // `GreedyManaResolver` handles `Choice` (including the empty set, which
        // simply produces no colour and therefore cannot pay a coloured pip)
        // exactly as it would any dual/multi source.
        let reflected = self.reflected_mana_colors(player_id);
        ManaProduction {
            kind: ManaProductionKind::Choice(reflected),
            ..cached
        }
    }

    /// Tap a permanent for mana with a cost hint to guide color production
    ///
    /// This method handles both:
    /// - Lands with implicit mana abilities (based on subtypes)
    /// - Creatures/artifacts with explicit mana abilities (e.g., "Guy in the Chair", Black Lotus)
    ///
    /// For mana abilities with sacrifice costs (e.g., Black Lotus), this will also
    /// sacrifice the permanent after activating the mana ability.
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be tapped for mana or is already tapped.
    pub fn tap_for_mana_and_update_hint(
        &mut self,
        player_id: PlayerId,
        card_id: CardId,
        remaining_hint: &mut crate::core::ManaCost,
    ) -> Result<()> {
        let pool_before = self.get_player(player_id)?.mana_pool;
        self.tap_for_mana_for_cost(player_id, card_id, remaining_hint)?;
        let pool_after = self.get_player(player_id)?.mana_pool;

        let produced_white = pool_after.white.saturating_sub(pool_before.white);
        let produced_blue = pool_after.blue.saturating_sub(pool_before.blue);
        let produced_black = pool_after.black.saturating_sub(pool_before.black);
        let produced_red = pool_after.red.saturating_sub(pool_before.red);
        let produced_green = pool_after.green.saturating_sub(pool_before.green);
        let produced_colorless = pool_after.colorless.saturating_sub(pool_before.colorless);

        let take_white = remaining_hint.white.min(produced_white);
        remaining_hint.white -= take_white;
        let unused_white = produced_white - take_white;

        let take_blue = remaining_hint.blue.min(produced_blue);
        remaining_hint.blue -= take_blue;
        let unused_blue = produced_blue - take_blue;

        let take_black = remaining_hint.black.min(produced_black);
        remaining_hint.black -= take_black;
        let unused_black = produced_black - take_black;

        let take_red = remaining_hint.red.min(produced_red);
        remaining_hint.red -= take_red;
        let unused_red = produced_red - take_red;

        let take_green = remaining_hint.green.min(produced_green);
        remaining_hint.green -= take_green;
        let unused_green = produced_green - take_green;

        let take_colorless = remaining_hint.colorless.min(produced_colorless);
        remaining_hint.colorless -= take_colorless;
        let unused_colorless = produced_colorless - take_colorless;

        let total_unused = unused_white + unused_blue + unused_black + unused_red + unused_green + unused_colorless;
        let take_generic = remaining_hint.generic.min(total_unused);
        remaining_hint.generic -= take_generic;

        Ok(())
    }

    /// Taps a permanent for mana to pay a cost.
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be tapped for mana or is already tapped.
    pub fn tap_for_mana_for_cost(
        &mut self,
        player_id: PlayerId,
        card_id: CardId,
        cost_hint: &crate::core::ManaCost,
    ) -> Result<()> {
        let card = self.cards.get_mut(card_id)?;

        // Check if card is untapped
        if card.tapped {
            return Err(MtgError::InvalidAction("Permanent is already tapped".to_string()));
        }

        // Check if card can produce mana (either land or has mana ability)
        let is_land = card.is_land();
        let has_mana_ability = card.activated_abilities.iter().any(|ab| ab.is_mana_ability);

        if !is_land && !has_mana_ability {
            return Err(MtgError::InvalidAction("Permanent cannot produce mana".to_string()));
        }

        // Check for explicit mana ability and its cost before tapping
        // We need both the mana production and the full cost (for sacrifice, etc.)
        let (explicit_mana, mana_ability_cost) = if !is_land && has_mana_ability {
            // For non-lands (creatures, artifacts) with mana abilities,
            // extract the mana from the activated ability's AddMana effect
            // and also capture the full cost for non-tap costs (like sacrifice)
            card.activated_abilities
                .iter()
                .find(|ab| ab.is_mana_ability)
                .map(|ab| {
                    let mana = ab.effects.iter().find_map(|effect| {
                        if let crate::core::Effect::AddMana { mana, .. } = effect {
                            Some(*mana)
                        } else {
                            None
                        }
                    });
                    (mana, Some(ab.cost.clone()))
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        // Capture log size before tap
        let prior_log_size = self.logger.log_count();

        // Tap the permanent
        card.tap();

        // Log the tap
        self.undo_log.log(
            crate::undo::GameAction::TapCard { card_id, tapped: true },
            prior_log_size,
        );

        // Update mana caches (event-driven incremental update)
        // Read card data to avoid borrow conflicts
        if let Some(card) = self.cards.try_get(card_id) {
            for (_, cache) in &mut self.mana_caches {
                cache.on_tap(card_id, card);
            }
        }

        // Increment mana state version to invalidate ManaEngine cache
        self.increment_mana_version();

        // Check for Taps triggers (e.g., Gran-Gran: "Whenever ~ becomes tapped")
        self.check_triggers(TriggerEvent::Taps, card_id)?;
        self.check_taps_for_mana_triggers(card_id, player_id)?;

        // Handle non-land mana sources with explicit mana abilities
        if let Some(mana_to_add) = explicit_mana {
            // For creatures with "Add mana of any color", we need to choose based on cost hint
            // Check if this is an any-color source using the pre-computed cache
            // (derived from parsed abilities, not text)
            let is_any_color = self
                .cards
                .get(card_id)
                .map(|c| {
                    matches!(
                        c.definition.cache.mana_production.kind,
                        crate::core::ManaProductionKind::AnyColor
                    )
                })
                .unwrap_or(false);

            // For multi-mana any-color sources like Black Lotus (`Amount$ 3`),
            // each activation produces N mana of the chosen colour, not 1. Read
            // the per-activation amount from the cached production BEFORE
            // borrowing the player mutably (otherwise we'd hold incompatible
            // borrows on `self.cards` and `self.players`).
            let any_color_amount = if is_any_color {
                self.cards
                    .get(card_id)
                    .map(|c| c.definition.cache.mana_production.amount.max(1))
                    .unwrap_or(1)
            } else {
                1
            };

            // Reflected mana (Fellwar Stone): the produced color is constrained
            // to the set of colors opponents' lands could produce. Detect the
            // flag and compute the reflected color set BEFORE borrowing the
            // player mutably (the helper borrows self immutably).
            let is_reflected = self
                .cards
                .get(card_id)
                .map(|c| {
                    c.activated_abilities
                        .iter()
                        .any(|ab| ab.is_mana_ability && ab.produces_reflected_mana)
                })
                .unwrap_or(false);
            let reflected_colors = if is_reflected {
                Some(self.reflected_mana_colors(player_id))
            } else {
                None
            };

            // Capture log size before mana addition (before get_player_mut to avoid borrow issues)
            let prior_log_size = self.logger.log_count();

            let player = self.get_player_mut(player_id)?;

            if is_any_color {
                // Choose color based on cost hint. For reflected sources, the
                // choice is restricted to colors the opponents' lands could
                // produce (CR 106.7); a cost-hint color outside that set falls
                // back to the cheapest available reflected color so we never
                // fabricate a color the source could not produce.
                use crate::core::ManaColor;
                let hint_color = if cost_hint.white > 0 {
                    Some(crate::core::Color::White)
                } else if cost_hint.blue > 0 {
                    Some(crate::core::Color::Blue)
                } else if cost_hint.black > 0 {
                    Some(crate::core::Color::Black)
                } else if cost_hint.red > 0 {
                    Some(crate::core::Color::Red)
                } else if cost_hint.green > 0 {
                    Some(crate::core::Color::Green)
                } else {
                    None
                };

                // Map between ManaColor (reflected set) and Color (mana pool).
                let to_color = |mc: ManaColor| match mc {
                    ManaColor::White => crate::core::Color::White,
                    ManaColor::Blue => crate::core::Color::Blue,
                    ManaColor::Black => crate::core::Color::Black,
                    ManaColor::Red => crate::core::Color::Red,
                    ManaColor::Green => crate::core::Color::Green,
                };
                let hint_mana_color = hint_color.and_then(|c| match c {
                    crate::core::Color::White => Some(ManaColor::White),
                    crate::core::Color::Blue => Some(ManaColor::Blue),
                    crate::core::Color::Black => Some(ManaColor::Black),
                    crate::core::Color::Red => Some(ManaColor::Red),
                    crate::core::Color::Green => Some(ManaColor::Green),
                    crate::core::Color::Colorless => None,
                });

                let color = match &reflected_colors {
                    Some(set) => {
                        // Prefer the hinted color if the reflected set allows it,
                        // else the first reflected color in canonical WUBRG order,
                        // else colorless when the set is empty (opponents control
                        // only colorless lands or none). Producing COLORLESS rather
                        // than a fabricated green keeps this consistent with
                        // `effective_production_for_resolution`, which models an
                        // empty reflected set as `Colorless`: the resolver never
                        // counts an empty-reflected Fellwar Stone toward a coloured
                        // pip, so it never commits to a cost the activation can't
                        // pay (the mtg-893 Lightning Bolt loop).
                        if let Some(hint) = hint_mana_color {
                            if set.contains(hint) {
                                to_color(hint)
                            } else {
                                set.iter().next().map(to_color).unwrap_or(crate::core::Color::Colorless)
                            }
                        } else {
                            set.iter().next().map(to_color).unwrap_or(crate::core::Color::Colorless)
                        }
                    }
                    // Non-reflected any-color source: honor hint, default green.
                    None => hint_color.unwrap_or(crate::core::Color::Green),
                };

                // Use the per-activation amount captured above before the
                // mutable borrow.
                let amount = any_color_amount;

                let mut mana = crate::core::ManaCost::new();
                let color_symbol = match color {
                    crate::core::Color::White => {
                        player.mana_pool.white += amount;
                        mana.white = amount;
                        "W"
                    }
                    crate::core::Color::Blue => {
                        player.mana_pool.blue += amount;
                        mana.blue = amount;
                        "U"
                    }
                    crate::core::Color::Black => {
                        player.mana_pool.black += amount;
                        mana.black = amount;
                        "B"
                    }
                    crate::core::Color::Red => {
                        player.mana_pool.red += amount;
                        mana.red = amount;
                        "R"
                    }
                    crate::core::Color::Green => {
                        player.mana_pool.green += amount;
                        mana.green = amount;
                        "G"
                    }
                    crate::core::Color::Colorless => {
                        player.mana_pool.colorless += amount;
                        mana.colorless = amount;
                        "C"
                    }
                };
                self.undo_log
                    .log(crate::undo::GameAction::AddMana { player_id, mana }, prior_log_size);

                // Log visible message (use gamelog for official action)
                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    let card = self.cards.get(card_id).ok();
                    let name = card.map(|c| c.name.as_str()).unwrap_or("Unknown");
                    // Render amount-many pips, e.g. Black Lotus → "Tap Black Lotus for {G}{G}{G}".
                    let pip = format!("{{{}}}", color_symbol);
                    let pips: String = pip.repeat(amount as usize);
                    let message = format!("Tap {} for {}", name, pips);
                    self.logger.gamelog(&message);
                }
            } else {
                // Add the specific mana from the ability
                if mana_to_add.white > 0 {
                    player.mana_pool.white += mana_to_add.white;
                }
                if mana_to_add.blue > 0 {
                    player.mana_pool.blue += mana_to_add.blue;
                }
                if mana_to_add.black > 0 {
                    player.mana_pool.black += mana_to_add.black;
                }
                if mana_to_add.red > 0 {
                    player.mana_pool.red += mana_to_add.red;
                }
                if mana_to_add.green > 0 {
                    player.mana_pool.green += mana_to_add.green;
                }
                if mana_to_add.colorless > 0 {
                    player.mana_pool.colorless += mana_to_add.colorless;
                }

                self.undo_log.log(
                    crate::undo::GameAction::AddMana {
                        player_id,
                        mana: mana_to_add,
                    },
                    prior_log_size,
                );

                // Log visible message (use gamelog for official action)
                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    let card = self.cards.get(card_id).ok();
                    let name = card.map(|c| c.name.as_str()).unwrap_or("Unknown");
                    let message = format!("Tap {} for mana", name);
                    self.logger.gamelog(&message);
                }
            }

            // Pay any additional costs from the mana ability (e.g., sacrifice for Black Lotus)
            // For non-land mana sources, handle sacrifice costs before returning
            if let Some(cost) = mana_ability_cost {
                use crate::core::Cost;
                match cost {
                    Cost::Tap => {
                        // Already handled above
                    }
                    Cost::SacrificePattern { .. } | Cost::Sacrifice { .. } => {
                        // Pay the sacrifice cost (moves permanent to graveyard)
                        self.pay_ability_cost(player_id, card_id, &cost)?;
                    }
                    Cost::Composite(costs) => {
                        // For composite costs, pay everything except tap (already paid)
                        for sub_cost in costs {
                            if !matches!(sub_cost, Cost::Tap) {
                                self.pay_ability_cost(player_id, card_id, &sub_cost)?;
                            }
                        }
                    }
                    // Other costs not yet handled by mana abilities:
                    Cost::Untap
                    | Cost::Mana(_)
                    | Cost::TapAndMana(_)
                    | Cost::PayLife { .. }
                    | Cost::Discard { .. }
                    | Cost::DiscardHand
                    | Cost::Waterbend { .. }
                    | Cost::AddLoyalty { .. }
                    | Cost::SubLoyalty { .. }
                    | Cost::SubCounter { .. } => {
                        // These cost types aren't currently used in mana ability costs
                    }
                }
            }

            return Ok(());
        }

        // Add mana to player's pool based on land type
        // For basic lands and simple cases, check subtypes
        // For dual lands (e.g., Underground Sea = Island Swamp), we need smarter logic
        // First, check subtypes and mana production cache before we borrow player_mut
        // Get mana production info and build available colors from BOTH subtypes AND mana production cache
        // This handles both basic lands (with subtypes) and non-basic dual lands (with Choice abilities)
        let (is_any_color_land, produces_colorless, available_colors) = {
            let card = self.cards.get(card_id)?;
            // Use pre-computed cache for mana production type (derived from abilities, not text)
            let is_any_color = matches!(
                card.definition.cache.mana_production.kind,
                crate::core::ManaProductionKind::AnyColor
            );
            let is_colorless = matches!(
                card.definition.cache.mana_production.kind,
                crate::core::ManaProductionKind::Colorless
            );

            // Build available_colors from BOTH sources:
            // 1. Land subtypes (Island, Forest, etc.) - for basic/dual lands with land types
            // 2. ManaProductionKind::Choice - for non-basic duals like Blooming Marsh
            let mut colors = Vec::new();

            // First, add colors from land subtypes
            if card.definition.cache.has_plains_subtype {
                colors.push(crate::core::Color::White);
            }
            if card.definition.cache.has_island_subtype {
                colors.push(crate::core::Color::Blue);
            }
            if card.definition.cache.has_swamp_subtype {
                colors.push(crate::core::Color::Black);
            }
            if card.definition.cache.has_mountain_subtype {
                colors.push(crate::core::Color::Red);
            }
            if card.definition.cache.has_forest_subtype {
                colors.push(crate::core::Color::Green);
            }

            // Second, add colors from mana production cache (for non-basic lands)
            // This handles lands without basic land subtypes
            use crate::core::ManaColor;
            match &card.definition.cache.mana_production.kind {
                crate::core::ManaProductionKind::Fixed(mana_color) => {
                    // Non-basic land that produces a fixed color (e.g., Ba Sing Se produces {G})
                    let color = match mana_color {
                        ManaColor::White => crate::core::Color::White,
                        ManaColor::Blue => crate::core::Color::Blue,
                        ManaColor::Black => crate::core::Color::Black,
                        ManaColor::Red => crate::core::Color::Red,
                        ManaColor::Green => crate::core::Color::Green,
                    };
                    if !colors.contains(&color) {
                        colors.push(color);
                    }
                }
                crate::core::ManaProductionKind::Choice(mana_colors) => {
                    // Dual/multi lands (e.g., Blooming Marsh)
                    if mana_colors.contains(ManaColor::White) && !colors.contains(&crate::core::Color::White) {
                        colors.push(crate::core::Color::White);
                    }
                    if mana_colors.contains(ManaColor::Blue) && !colors.contains(&crate::core::Color::Blue) {
                        colors.push(crate::core::Color::Blue);
                    }
                    if mana_colors.contains(ManaColor::Black) && !colors.contains(&crate::core::Color::Black) {
                        colors.push(crate::core::Color::Black);
                    }
                    if mana_colors.contains(ManaColor::Red) && !colors.contains(&crate::core::Color::Red) {
                        colors.push(crate::core::Color::Red);
                    }
                    if mana_colors.contains(ManaColor::Green) && !colors.contains(&crate::core::Color::Green) {
                        colors.push(crate::core::Color::Green);
                    }
                }
                crate::core::ManaProductionKind::AnyColor | crate::core::ManaProductionKind::Colorless => {
                    // Handled by is_any_color and is_colorless checks
                }
            }

            // Third, add chosen_color for lands like Thriving Grove
            // (cards with "choose a color" ETB effects that produce mana of that color)
            if let Some(chosen) = card.chosen_color {
                if !colors.contains(&chosen) {
                    colors.push(chosen);
                }
            }

            (is_any_color, is_colorless, colors)
        };

        // Per-activation amount for lands whose mana ability produces more than
        // one mana per tap (e.g. Mishra's Workshop `Produced$ C | Amount$ 3`).
        // Most lands produce exactly 1; read the cached amount derived from the
        // parsed `AB$ Mana` effect so multi-mana lands aren't silently clamped
        // to a single pip. Captured before the mutable player borrow.
        let land_amount = self
            .cards
            .get(card_id)
            .map(|c| c.definition.cache.mana_production.amount.max(1))
            .unwrap_or(1);

        // Capture log size before mana addition (before get_player_mut to avoid borrow issues)
        let prior_log_size = self.logger.log_count();

        let player = self.get_player_mut(player_id)?;

        let color = if is_any_color_land || available_colors.len() > 1 {
            // Multi-color or any-color land: choose based on cost hint
            // Produce the first color needed by the cost that this land can produce
            if cost_hint.white > 0 && (is_any_color_land || available_colors.contains(&crate::core::Color::White)) {
                Some(crate::core::Color::White)
            } else if cost_hint.blue > 0 && (is_any_color_land || available_colors.contains(&crate::core::Color::Blue))
            {
                Some(crate::core::Color::Blue)
            } else if cost_hint.black > 0
                && (is_any_color_land || available_colors.contains(&crate::core::Color::Black))
            {
                Some(crate::core::Color::Black)
            } else if cost_hint.red > 0 && (is_any_color_land || available_colors.contains(&crate::core::Color::Red)) {
                Some(crate::core::Color::Red)
            } else if cost_hint.green > 0
                && (is_any_color_land || available_colors.contains(&crate::core::Color::Green))
            {
                Some(crate::core::Color::Green)
            } else {
                // Cost doesn't need a specific color - produce the first available color
                available_colors.first().copied().or(Some(crate::core::Color::White))
            }
        } else if available_colors.len() == 1 {
            // Single-color land
            available_colors.first().copied()
        } else if produces_colorless {
            // Colorless mana land (e.g., Mishra's Factory, Wastes)
            Some(crate::core::Color::Colorless)
        } else {
            // Unknown land type
            None
        };

        if let Some(color) = color {
            // Multi-mana lands (e.g. Mishra's Workshop) add `land_amount` mana of
            // the chosen colour per tap; ordinary lands keep the amount of 1.
            let amount = land_amount;

            // Log the mana addition
            let mut mana = crate::core::ManaCost::new();
            let color_symbol = match color {
                crate::core::Color::White => {
                    player.mana_pool.white += amount;
                    mana.white = amount;
                    "W"
                }
                crate::core::Color::Blue => {
                    player.mana_pool.blue += amount;
                    mana.blue = amount;
                    "U"
                }
                crate::core::Color::Black => {
                    player.mana_pool.black += amount;
                    mana.black = amount;
                    "B"
                }
                crate::core::Color::Red => {
                    player.mana_pool.red += amount;
                    mana.red = amount;
                    "R"
                }
                crate::core::Color::Green => {
                    player.mana_pool.green += amount;
                    mana.green = amount;
                    "G"
                }
                crate::core::Color::Colorless => {
                    player.mana_pool.colorless += amount;
                    mana.colorless = amount;
                    "C"
                }
            };
            self.undo_log
                .log(crate::undo::GameAction::AddMana { player_id, mana }, prior_log_size);

            // Log visible message for mana tapping (use gamelog for official action).
            // Render amount-many pips, e.g. Mishra's Workshop → "Tap ... for {C}{C}{C}".
            if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                let card_name = self.cards.get(card_id).map(|c| c.name.as_str()).unwrap_or("Unknown");
                let pip = format!("{{{}}}", color_symbol);
                let pips: String = pip.repeat(amount as usize);
                let message = format!("Tap {} for {}", card_name, pips);
                self.logger.gamelog(&message);
            }
        }

        // Note: For lands, mana_ability_cost is None (set at line 1623), so no additional
        // costs need to be paid. Non-land mana sources with sacrifice costs are handled
        // in the explicit_mana path above (lines 1760-1784), which returns early.

        Ok(())
    }

    /// Pay the cost for an activated ability
    ///
    /// This method pays costs in the correct order:
    /// 1. Tap costs (must happen before zone changes)
    /// 2. Mana costs (pay from mana pool)
    /// 3. Other costs (sacrifice, discard, etc.) - TODO
    ///
    /// Returns Ok(()) if costs were successfully paid, Err otherwise.
    ///
    /// Note: This is a simplified implementation. Full implementation would:
    /// - Support cost refund if payment fails midway
    /// - Handle cost ordering more comprehensively
    /// - Support all cost types (sacrifice, discard, pay life, etc.)
    ///
    /// # Errors
    ///
    /// Pay a mana cost for `player_id` by automatically tapping their mana
    /// sources, then deducting the cost from their pool. Returns `true` if the
    /// cost was fully paid, `false` (with no state change to the pool balance)
    /// otherwise.
    ///
    /// This is the controller-free auto-payment used for OPTIONAL mana costs
    /// that are decided during effect resolution rather than during casting —
    /// notably the "may pay {R}{R}" gate on a `CopySpellAbility` SubAbility
    /// (Chain Lightning, mtg-152). The casting path performs the same
    /// compute-tap-order → tap-each-source → deduct-from-pool dance via the
    /// controller's `choose_mana_sources_to_pay`; here we pick sources greedily
    /// (the GreedyManaResolver tap order) since there is no priority window to
    /// route the choice through. The source selection is a pure function of the
    /// payer's own public mana sources, so it is identical on server and client
    /// (no information leakage, deterministic — CLAUDE.md network invariant).
    ///
    /// Crucially this DEDUCTS real mana, so a recursive copy chain (Chain
    /// Lightning copied back and forth) terminates when a player runs out of
    /// red sources, exactly as it would in paper.
    pub fn pay_mana_cost_by_tapping(&mut self, player_id: PlayerId, cost: &crate::core::ManaCost) -> bool {
        use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

        // If the pool already covers it, just deduct (no tapping needed).
        let pool_covers = self
            .try_get_player(player_id)
            .map(|p| p.mana_pool.can_pay(cost))
            .unwrap_or(false);
        if !pool_covers {
            // Compute which sources to tap. Build the resolver state read-only,
            // then collect the tap order into an OWNED Vec so the immutable
            // borrow of `self` is released before we start tapping.
            let mut mana_engine = crate::game::mana_engine::ManaEngine::new();
            mana_engine.update(self, player_id);
            let mut tap_order: Vec<CardId> = Vec::new();
            let resolver = GreedyManaResolver::new();
            let ok = resolver.compute_tap_order(cost, mana_engine.all_sources(), &mut tap_order);
            if !ok {
                return false;
            }
            // Tap each chosen source, accumulating mana into the pool.
            let mut remaining_hint = *cost;
            for source_id in tap_order {
                if self
                    .tap_for_mana_and_update_hint(player_id, source_id, &mut remaining_hint)
                    .is_err()
                {
                    return false;
                }
            }
        }

        // Deduct the cost from the pool (snapshotting for undo via pay_ability_cost).
        self.pay_ability_cost(player_id, CardId::new(0), &crate::core::Cost::Mana(*cost))
            .is_ok()
    }

    /// # Errors
    ///
    /// Returns an error if the cost cannot be paid.
    pub fn pay_ability_cost(&mut self, player_id: PlayerId, card_id: CardId, cost: &crate::core::Cost) -> Result<()> {
        use crate::core::{Cost, ManaCost};

        match cost {
            Cost::Tap => {
                // Tap the permanent (this updates cache and increments mana_version)
                self.tap_permanent(card_id)?;
                // Check for Taps triggers (e.g., Gran-Gran: "Whenever ~ becomes tapped")
                self.check_triggers(TriggerEvent::Taps, card_id)?;
                Ok(())
            }

            Cost::Mana(mana_cost) => {
                // Pay mana from pool. Snapshot the pool for undo (mtg-733).
                self.log_mana_pool(player_id);
                let player = self.get_player_mut(player_id)?;
                if !player.mana_pool.can_pay(mana_cost) {
                    return Err(MtgError::InvalidAction("Cannot pay mana cost".to_string()));
                }
                player.mana_pool.pay_cost(mana_cost).map_err(MtgError::InvalidAction)?;
                Ok(())
            }

            Cost::TapAndMana(mana_cost) => {
                // Pay both tap and mana
                // Tap first (must happen before zone changes)
                // Check if already tapped
                {
                    let card = self.cards.get(card_id)?;
                    if card.tapped {
                        return Err(MtgError::InvalidAction("Permanent is already tapped".to_string()));
                    }
                }

                // Tap the permanent (this updates cache and increments mana_version)
                self.tap_permanent(card_id)?;
                // Check for Taps triggers (e.g., Gran-Gran: "Whenever ~ becomes tapped")
                self.check_triggers(TriggerEvent::Taps, card_id)?;

                // Then pay mana. Snapshot the pool for undo (mtg-733).
                self.log_mana_pool(player_id);
                let player = self.get_player_mut(player_id)?;
                if !player.mana_pool.can_pay(mana_cost) {
                    // TODO(mtg-733): Should refund the tap here (undo the source taps if pool payment fails)
                    return Err(MtgError::InvalidAction("Cannot pay mana cost".to_string()));
                }
                player.mana_pool.pay_cost(mana_cost).map_err(MtgError::InvalidAction)?;
                Ok(())
            }

            Cost::PayLife { amount } => {
                // Pay life
                let player = self.get_player_mut(player_id)?;
                if player.life < *amount {
                    return Err(MtgError::InvalidAction("Not enough life".to_string()));
                }
                player.life -= amount;
                Ok(())
            }

            Cost::Untap => {
                // Untap the permanent
                let card = self.cards.get_mut(card_id)?;
                if !card.tapped {
                    return Err(MtgError::InvalidAction("Permanent is not tapped".to_string()));
                }
                card.untap();
                Ok(())
            }

            Cost::SacrificePattern { count, card_type } => {
                // Find permanents matching the pattern and sacrifice them
                // For now, automatically choose without asking the controller
                // TODO(mtg-144): Let controller choose which permanents to sacrifice

                let mut to_sacrifice = Vec::new();

                // Special case: CARDNAME means the card with this ability
                if card_type == "CARDNAME" {
                    to_sacrifice.push(card_id);
                } else {
                    // Find permanents on battlefield matching the type
                    // Collect IDs first to avoid borrowing issues
                    let battlefield_cards = self.battlefield.cards.to_vec();

                    for permanent_id in battlefield_cards {
                        if to_sacrifice.len() >= *count as usize {
                            break;
                        }

                        let card = self.cards.get(permanent_id)?;

                        // Check ownership
                        if card.owner != player_id {
                            continue;
                        }

                        // Check if it matches the pattern.
                        // Use card_matches_type_filter_static which handles both
                        // main types (Land, Creature, Artifact) AND subtypes
                        // (Forest, Island, Mountain, etc.) correctly.
                        // Special-case "Creature.Other" which means the creature
                        // is not the card holding the ability.
                        let matches = if card_type == "Creature.Other" {
                            card.is_creature() && permanent_id != card_id
                        } else {
                            crate::game::GameState::card_matches_type_filter_static(card, card_type)
                        };

                        if matches {
                            to_sacrifice.push(permanent_id);
                        }
                    }
                }

                // Check if we found enough permanents to sacrifice
                if to_sacrifice.len() < *count as usize {
                    return Err(MtgError::InvalidAction(format!(
                        "Not enough permanents of type {} to sacrifice (need {}, found {})",
                        card_type,
                        count,
                        to_sacrifice.len()
                    )));
                }

                // Record the LAST sacrificed permanent so an immediately-following
                // dynamic effect can read its last-known characteristics (Diamond
                // Valley: gain life = sacrificed creature's toughness, CR 608.2g).
                // Set BEFORE the zone move so the card's data is still its
                // battlefield state; reading is LKI either way. Cleared after the
                // ability's effects run (priority.rs). Provably None at every
                // serialize boundary — see SubActionScratch::sacrificed_for_cost.
                // The last of the `count` permanents we are about to sacrifice
                // (index count-1; `to_sacrifice` holds at least `count` entries
                // per the check above). For the single-creature case (Diamond
                // Valley) this is exactly the sacrificed creature.
                if let Some(last) = to_sacrifice.get((*count as usize).saturating_sub(1)) {
                    self.sub_action_scratch.sacrificed_for_cost = Some(*last);
                }

                // Sacrifice the permanents (move to graveyard or exile if finality) and check triggers
                for sac_id in to_sacrifice.iter().take(*count as usize) {
                    // Capture the toughness BEFORE the card leaves the battlefield
                    // (Diamond Valley: "gain life equal to the sacrificed creature's toughness").
                    // Stored in GameState::last_sacrificed_toughness for GainLifeDynamic
                    // (DynamicAmount::SacrificedToughness) to read at resolution time.
                    if let Ok(sac_card) = self.cards.get(*sac_id) {
                        if sac_card.is_creature() {
                            self.last_sacrificed_toughness = Some(i32::from(sac_card.current_toughness()));
                        }
                    }
                    let owner = self.cards.get(*sac_id)?.owner;
                    let dest = self.death_destination_for_card(*sac_id);
                    self.move_card(*sac_id, Zone::Battlefield, dest, owner)?;
                    // Check sacrifice triggers (e.g., Pirate Peddlers)
                    self.check_triggers(TriggerEvent::Sacrificed, *sac_id)?;
                }

                Ok(())
            }

            Cost::Sacrifice { card_id: sac_id } => {
                // Sacrifice a specific permanent (move to graveyard or exile if finality)
                let owner = self.cards.get(*sac_id)?.owner;
                let dest = self.death_destination_for_card(*sac_id);
                self.move_card(*sac_id, Zone::Battlefield, dest, owner)?;
                // Check sacrifice triggers
                self.check_triggers(TriggerEvent::Sacrificed, *sac_id)
            }

            Cost::Discard { card_id: _ } => {
                // TODO(mtg-32f9h): Implement discard cost for specific card
                Err(MtgError::InvalidAction(format!(
                    "Cost type {cost:?} not yet implemented"
                )))
            }

            Cost::DiscardHand => {
                // Discard entire hand (e.g., Slate of Ancestry)
                if let Some(zones) = self.get_player_zones(player_id) {
                    let hand_cards: Vec<CardId> = zones.hand.cards.clone();
                    for &hand_card_id in &hand_cards {
                        self.move_card(hand_card_id, Zone::Hand, Zone::Graveyard, player_id)?;
                    }
                    self.logger.normal(&format!(
                        "{} discards their hand ({} cards)",
                        self.get_player(player_id)?.name,
                        hand_cards.len()
                    ));
                }
                Ok(())
            }

            Cost::Composite(costs) => {
                // Pay each cost in order
                for sub_cost in costs {
                    self.pay_ability_cost(player_id, card_id, sub_cost)?;
                }
                Ok(())
            }

            Cost::Waterbend { amount } => {
                // Waterbend cost - Avatar set mechanic (like Convoke)
                // Player can tap untapped creatures/artifacts to pay for {1} each.
                // Player can also tap lands to produce mana.
                // Total payment = mana from lands + tapped creatures/artifacts + floating mana

                // Get current floating mana
                let floating_mana = {
                    let player = self.get_player(player_id)?;
                    player.mana_pool.total()
                };

                // Find untapped mana sources (lands) controlled by this player
                let battlefield_cards = self.battlefield.cards.to_vec();
                let mana_sources: Vec<CardId> = battlefield_cards
                    .iter()
                    .filter(|&&cid| {
                        if cid == card_id {
                            return false; // Can't tap the source to pay its own cost
                        }
                        if let Some(card) = self.cards.try_get(cid) {
                            // Must be untapped land controlled by player with mana ability
                            !card.tapped && card.controller == player_id && card.is_land()
                        } else {
                            false
                        }
                    })
                    .copied()
                    .collect();

                // Find untapped creatures and artifacts controlled by this player
                // (excluding the source card and mana sources - they're counted above)
                let tappable_permanents: Vec<CardId> = battlefield_cards
                    .into_iter()
                    .filter(|&cid| {
                        if cid == card_id {
                            return false; // Can't tap the source to pay its own cost
                        }
                        if mana_sources.contains(&cid) {
                            return false; // Already counted as mana source
                        }
                        if let Some(card) = self.cards.try_get(cid) {
                            // Must be untapped, controlled by player, and be creature or artifact
                            !card.tapped && card.controller == player_id && (card.is_creature() || card.is_artifact())
                        } else {
                            false
                        }
                    })
                    .collect();

                let total_available = floating_mana + mana_sources.len() as u8 + tappable_permanents.len() as u8;

                if total_available < *amount {
                    return Err(MtgError::InvalidAction(format!(
                        "Cannot pay Waterbend {}: only {} available (floating: {}, lands: {}, tappable: {})",
                        amount,
                        total_available,
                        floating_mana,
                        mana_sources.len(),
                        tappable_permanents.len()
                    )));
                }

                // Payment strategy: prefer tapping creatures/artifacts first, then lands
                // This preserves mana sources for future use when possible
                let mut remaining = *amount;

                // First use floating mana
                if remaining > 0 && floating_mana > 0 {
                    let use_from_pool = remaining.min(floating_mana);
                    let mana_cost = ManaCost::from_string(&use_from_pool.to_string());
                    // Snapshot the pool for undo before spending floating mana (mtg-733).
                    self.log_mana_pool(player_id);
                    let player = self.get_player_mut(player_id)?;
                    player.mana_pool.pay_cost(&mana_cost).map_err(MtgError::InvalidAction)?;
                    remaining -= use_from_pool;
                }

                // Then tap creatures/artifacts for waterbend. Route through
                // tap_permanent so the undo log, ManaSourceCache untapped counts,
                // and mana_state_version stay consistent (see TapAll note above).
                for &perm_id in &tappable_permanents {
                    if remaining == 0 {
                        break;
                    }
                    if self.cards.try_get(perm_id).is_some() {
                        self.tap_permanent(perm_id)?;
                        remaining -= 1;
                    }
                }

                // Finally tap lands to produce mana
                for &land_id in &mana_sources {
                    if remaining == 0 {
                        break;
                    }
                    if self.cards.try_get(land_id).is_some() {
                        self.tap_permanent(land_id)?;
                        remaining -= 1;
                        // Note: We're not adding mana to pool since we're directly counting
                        // each land tap as {1} payment for simplicity
                    }
                }

                Ok(())
            }

            Cost::AddLoyalty { amount } => {
                // Planeswalker +N loyalty ability: add N loyalty counters
                use crate::core::CounterType;
                let prior_log_size = self.logger.log_count();
                let card = self.cards.get_mut(card_id)?;
                card.add_counter(CounterType::Loyalty, *amount);
                let old_loyalty_flag = card.loyalty_activated_this_turn;
                card.loyalty_activated_this_turn = true; // MTG CR 606.3: once per turn
                self.undo_log.log(
                    crate::undo::GameAction::SetLoyaltyActivated {
                        card_id,
                        old_value: old_loyalty_flag,
                        new_value: true,
                    },
                    prior_log_size,
                );
                let new_loyalty = card.get_counter(CounterType::Loyalty);
                self.logger
                    .verbose(&format!("{} gains {} loyalty (now {})", card.name, amount, new_loyalty));
                Ok(())
            }

            Cost::SubLoyalty { amount } => {
                // Planeswalker -N loyalty ability: remove N loyalty counters
                use crate::core::CounterType;
                let prior_log_size = self.logger.log_count();
                let current = self.cards.get(card_id)?.get_counter(CounterType::Loyalty);
                if current < *amount {
                    return Err(MtgError::InvalidAction(format!(
                        "Not enough loyalty counters ({} < {}) on {}",
                        current,
                        amount,
                        self.cards.get(card_id)?.name
                    )));
                }
                let card = self.cards.get_mut(card_id)?;
                card.remove_counter(CounterType::Loyalty, *amount);
                let old_loyalty_flag = card.loyalty_activated_this_turn;
                card.loyalty_activated_this_turn = true; // MTG CR 606.3: once per turn
                self.undo_log.log(
                    crate::undo::GameAction::SetLoyaltyActivated {
                        card_id,
                        old_value: old_loyalty_flag,
                        new_value: true,
                    },
                    prior_log_size,
                );
                let new_loyalty = card.get_counter(CounterType::Loyalty);
                let card_name = card.name.to_string();
                self.logger
                    .verbose(&format!("{} loses {} loyalty (now {})", card_name, amount, new_loyalty));

                // Check if loyalty reaches 0 - planeswalker dies (MTG CR 704.5i)
                if new_loyalty == 0 {
                    self.logger
                        .normal(&format!("{} has 0 loyalty and is put into the graveyard", card_name));
                    let owner = self.cards.get(card_id)?.owner;
                    let dest = self.death_destination_for_card(card_id);
                    self.move_card(card_id, Zone::Battlefield, dest, owner)?;
                }
                Ok(())
            }

            Cost::SubCounter { amount, counter_type } => {
                // Generic counter-removal cost (e.g. Triskelion's
                // SubCounter<1/P1P1>). Distinct from Cost::SubLoyalty so we
                // don't tag the activation as the once-per-turn planeswalker
                // ability and don't enforce "0 counters → graveyard" — Triskelion
                // happily lives at 1/1 with zero P1P1 counters.
                let current = self.cards.get(card_id)?.get_counter(*counter_type);
                if current < *amount {
                    return Err(MtgError::InvalidAction(format!(
                        "Not enough {:?} counters ({} < {}) on {}",
                        counter_type,
                        current,
                        amount,
                        self.cards.get(card_id)?.name
                    )));
                }
                // Route through the LOGGED counter-removal so this cost is a
                // faithful undo-log inverse (mtg-728 sig-2e). The previous
                // direct `card.remove_counter(...)` mutated the card with NO
                // GameAction::RemoveCounter entry, so a rewind+replay (network
                // shadow / MCTS / undo) left the counters stale — the WASM
                // replay verifier caught it as "turn-start state hash changed
                // across rewinds" with a `cards[N].counters` field diff
                // (Triskelion paying its SubCounter<1/P1P1> ping cost). The
                // earlier `current < amount` guard already validated the cost,
                // and `remove_counters` does NOT enforce the loyalty "0 -> die"
                // rule, so Triskelion still happily lives at 1/1 with zero
                // P1P1 counters.
                let card_name = self.cards.get(card_id)?.name.to_string();
                self.remove_counters(card_id, *counter_type, *amount)?;
                let new_count = self.cards.get(card_id)?.get_counter(*counter_type);
                self.logger.verbose(&format!(
                    "{} loses {} {:?} counter(s) (now {})",
                    card_name, amount, counter_type, new_count
                ));
                Ok(())
            }
        }
    }
}
