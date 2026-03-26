//! Shared card reveal processing for network clients
//!
//! This module provides the core logic for processing CardRevealed messages
//! in network clients. Both native and WASM clients use this to maintain
//! synchronized shadow game states.
//!
//! ## Design
//!
//! The reveal processing is identical across platforms, with one difference:
//! how card definitions are obtained. Native clients can fall back to a local
//! card database, while WASM clients require the server to provide definitions.
//!
//! This is abstracted via the `CardDefProvider` trait.

use crate::core::PlayerId;
use crate::game::GameState;
use crate::loader::CardDefinition;
use crate::network::{CardReveal, RevealReason};

/// Strategy for obtaining card definitions from reveals
///
/// Native clients can fall back to a database lookup.
/// WASM clients require the server to provide the definition.
pub trait CardDefProvider {
    /// Get a CardDefinition from a reveal
    ///
    /// Returns None only for dummy reveals (empty name = hidden opponent card).
    /// For real reveals, must return Some(definition) or panic.
    fn get_card_def(&self, reveal: &CardReveal) -> Option<CardDefinition>;
}

/// Check if a reveal is a "dummy" reveal (hidden opponent card)
///
/// Dummy reveals have empty names - the client knows the CardID exists
/// but doesn't know what card it is. Used for opponent's hand cards.
#[inline]
pub fn is_dummy_reveal(reveal: &CardReveal) -> bool {
    reveal.name.is_empty()
}

/// Process a card reveal in the client's shadow game state
///
/// This is the core reveal processing logic shared by native and WASM clients.
/// It handles:
/// - Dummy reveals (hidden opponent cards) - skipped
/// - Draw/OpeningHand reveals - instantiate card if not already known
/// - Played reveals - instantiate and add to hand if needed
/// - TokenCreated reveals - instantiate and add to battlefield
///
/// # Arguments
/// * `game` - The client's shadow game state to update
/// * `provider` - Implementation of CardDefProvider for obtaining card definitions
/// * `owner` - The player who owns the revealed card
/// * `reveal` - The card reveal information from the server
/// * `reason` - Why the card was revealed
/// * `log_prefix` - Prefix for log messages (e.g., "Native" or "WASM")
/// * `local_player` - The player this client controls. Used to determine if we should
///   manipulate hand zones (only for our own cards). For opponent cards, the shadow
///   state doesn't have their library populated, so we can't reliably add to hand.
///
/// Note: Wildcard match is intentional - RevealReason has 7 variants; we handle
/// specific ones (Draw, OpeningHand, Played, TokenCreated) and log the rest.
#[allow(clippy::wildcard_enum_match_arm)]
pub fn process_card_reveal<P: CardDefProvider>(
    game: &mut GameState,
    provider: &P,
    owner: PlayerId,
    reveal: CardReveal,
    reason: RevealReason,
    log_prefix: &str,
    local_player: Option<PlayerId>,
) {
    let card_id = reveal.card_id;

    // Check for dummy reveal (empty name = hidden opponent card)
    if is_dummy_reveal(&reveal) {
        log::debug!(
            "{}: Dummy reveal for CardID {} owned by {:?} ({:?}) - skipping instantiation",
            log_prefix,
            card_id.as_u32(),
            owner,
            reason
        );
        return;
    }

    // Get card definition from provider
    let Some(card_def) = provider.get_card_def(&reveal) else {
        // This shouldn't happen for non-dummy reveals, but handle gracefully
        log::warn!(
            "{}: Could not get card_def for {} (id={}) - skipping",
            log_prefix,
            reveal.name,
            card_id.as_u32()
        );
        return;
    };

    // Rebuild parsed_svars which is skipped during serialization
    let mut card_def = card_def;
    card_def.rebuild_parsed_svars();

    match reason {
        RevealReason::Draw | RevealReason::OpeningHand => {
            let card_already_known = game.cards.get(card_id).is_ok();
            log::debug!(
                "{} {:?}: {} (id={}) for {:?} card_already_known={}",
                log_prefix,
                reason,
                reveal.name,
                card_id.as_u32(),
                owner,
                card_already_known
            );

            if !card_already_known {
                let card_instance = card_def.instantiate(card_id, owner);
                game.cards.insert(card_id, card_instance);
                log::debug!(
                    "{}: Instantiated {} for {:?}: {} ({:?})",
                    log_prefix,
                    if matches!(reason, RevealReason::Draw) {
                        "drawn card"
                    } else {
                        "opening hand card"
                    },
                    owner,
                    reveal.name,
                    card_id
                );

                // For Draw/OpeningHand, add to hand ONLY if:
                // 1. This is OUR card (owner == local_player), AND
                // 2. Card is NOT in hand, AND
                // 3. Card is NOT in library (i.e., WASM clients with empty game state), AND
                // 4. Card is NOT in graveyard/exile (already drawn+discarded/exiled)
                //
                // CRITICAL: For OPPONENT cards, we must NOT try to add to hand!
                // The opponent's library is empty in our shadow state, so the
                // "empty library mode" check would incorrectly trigger.
                // Opponent draws are handled by the GameLoop when it processes
                // the DrawCard action - we just need to instantiate the card.
                //
                // Native clients have populated libraries (from init_game_reserve_only),
                // so draw_card() will properly move the card from library to hand.
                // We must NOT add to hand here or we'll get duplicates.
                //
                // WASM clients may start with empty game state where libraries are empty,
                // so we need to add to hand directly for them (only for LOCAL player).
                //
                // CRITICAL FIX: Also check graveyard and exile zones. When a card is
                // drawn and then discarded in the same ability (e.g., Bazaar of Baghdad:
                // "draw 2, discard 3"), the reveal for the drawn card may arrive AFTER
                // the discard has already moved it to graveyard. Without this check,
                // the "empty library mode" condition (!in_hand && !in_library) would
                // incorrectly re-add the card to hand, causing a network desync.
                let is_our_card = local_player.is_some_and(|lp| lp == owner);
                if !is_our_card {
                    log::debug!(
                        "{}: {} {} (id={}) is opponent's card, skipping hand zone manipulation",
                        log_prefix,
                        if matches!(reason, RevealReason::Draw) {
                            "Drawn card"
                        } else {
                            "Opening hand card"
                        },
                        reveal.name,
                        card_id.as_u32()
                    );
                } else {
                    let card_in_hand = game.get_player_zones(owner).is_some_and(|z| z.hand.contains(card_id));
                    let card_in_library = game
                        .get_player_zones(owner)
                        .is_some_and(|z| z.library.contains(card_id));
                    // Check if card has already moved to another zone (graveyard, exile).
                    // This happens when draw+discard effects execute in the same ability
                    // and the reveal is processed after the discard.
                    let card_in_graveyard = game
                        .get_player_zones(owner)
                        .is_some_and(|z| z.graveyard.contains(card_id));
                    let card_in_exile = game.get_player_zones(owner).is_some_and(|z| z.exile.contains(card_id));
                    let card_on_battlefield = game.battlefield.contains(card_id);
                    let card_elsewhere = card_in_graveyard || card_in_exile || card_on_battlefield;
                    if !card_in_hand && !card_in_library && !card_elsewhere {
                        if let Some(zones) = game.get_player_zones_mut(owner) {
                            zones.hand.add(card_id);
                            log::debug!(
                                "{}: Added {} to hand for {:?}: {} (id={}) [empty library mode]",
                                log_prefix,
                                if matches!(reason, RevealReason::Draw) {
                                    "drawn card"
                                } else {
                                    "opening hand card"
                                },
                                owner,
                                reveal.name,
                                card_id.as_u32()
                            );
                        }
                    } else if card_elsewhere {
                        log::debug!(
                            "{}: {} {} (id={}) already in another zone (gy={} exile={} bf={}), not re-adding to hand",
                            log_prefix,
                            if matches!(reason, RevealReason::Draw) {
                                "Drawn card"
                            } else {
                                "Opening hand card"
                            },
                            reveal.name,
                            card_id.as_u32(),
                            card_in_graveyard,
                            card_in_exile,
                            card_on_battlefield,
                        );
                    } else if !card_in_hand && card_in_library {
                        log::debug!(
                            "{}: {} {} (id={}) is in library, letting draw_card() handle zone movement",
                            log_prefix,
                            if matches!(reason, RevealReason::Draw) {
                                "Drawn card"
                            } else {
                                "Opening hand card"
                            },
                            reveal.name,
                            card_id.as_u32()
                        );
                    }
                }
            }
        }
        RevealReason::Played => {
            // Played reveals tell us what card the opponent is playing FROM hand.
            // We only instantiate the card so it can be recognized when the GameLoop
            // executes the action. We do NOT add it to hand - the card is being
            // played FROM hand, and the GameLoop will move it to stack/battlefield.
            let card_already_known = game.cards.get(card_id).is_ok();
            log::debug!(
                "{} Played: {} (id={}) card_already_known={}",
                log_prefix,
                reveal.name,
                card_id.as_u32(),
                card_already_known
            );

            if !card_already_known {
                let card_instance = card_def.instantiate(card_id, owner);
                game.cards.insert(card_id, card_instance);
                log::debug!(
                    "{}: Instantiated played card for {:?}: {} ({:?})",
                    log_prefix,
                    owner,
                    reveal.name,
                    card_id
                );
            }
        }
        RevealReason::TokenCreated => {
            let card_instance = card_def.instantiate(card_id, owner);
            if game.cards.insert_if_vacant(card_id, card_instance) {
                game.battlefield.add(card_id);
                log::debug!(
                    "{}: Created token for {:?}: {} ({:?})",
                    log_prefix,
                    owner,
                    reveal.name,
                    card_id
                );
            }
        }
        RevealReason::Searched => {
            // Library search result - instantiate the card so it can be moved to hand
            let card_already_known = game.cards.get(card_id).is_ok();
            log::debug!(
                "{} Searched: {} (id={}) for {:?} card_already_known={}",
                log_prefix,
                reveal.name,
                card_id.as_u32(),
                owner,
                card_already_known
            );

            if !card_already_known {
                let card_instance = card_def.instantiate(card_id, owner);
                game.cards.insert(card_id, card_instance);
                log::debug!(
                    "{}: Instantiated searched card for {:?}: {} ({:?})",
                    log_prefix,
                    owner,
                    reveal.name,
                    card_id
                );
            }
        }
        RevealReason::Effect | RevealReason::Targeting => {
            // Effect reveals are used for cards moving to public zones (graveyard, exile)
            // from hidden zones (library). Common cases:
            // - Mill: library -> graveyard
            // - Dig (Fire Lord Ozai): library -> exile
            //
            // For OUR cards: library is populated, game loop's move_card will find the card.
            // For OPPONENT cards: we just instantiate the card entity. The shadow game's
            // zone tracking for opponent's hidden zones (library) is inherently incomplete,
            // so we don't try to add cards to their library. Zone move operations for
            // opponent cards may fail silently, but that's acceptable since:
            // 1. Server is authoritative for zone contents
            // 2. State hash comparison catches real desync
            //
            // When casting from exile (Fire Lord Ozai's "may play"), the Played reveal
            // will instantiate the card if needed before the cast operation.
            let card_already_known = game.cards.get(card_id).is_ok();
            log::debug!(
                "{} {:?}: {} (id={}) for {:?} card_already_known={}",
                log_prefix,
                reason,
                reveal.name,
                card_id.as_u32(),
                owner,
                card_already_known
            );

            if !card_already_known {
                let card_instance = card_def.instantiate(card_id, owner);
                game.cards.insert(card_id, card_instance);
                log::debug!(
                    "{}: Instantiated effect-revealed card for {:?}: {} ({:?})",
                    log_prefix,
                    owner,
                    reveal.name,
                    card_id
                );
            }
        }
    }
}

/// WASM card definition provider
///
/// WASM clients require the server to provide card definitions with reveals.
/// This provider panics if the server doesn't include the definition.
#[derive(Debug, Default)]
pub struct WasmCardDefProvider;

impl CardDefProvider for WasmCardDefProvider {
    fn get_card_def(&self, reveal: &CardReveal) -> Option<CardDefinition> {
        if reveal.name.is_empty() {
            return None; // Dummy reveal
        }

        // Server MUST provide card_def for real reveals
        reveal
            .card_def
            .clone()
            .map(|mut def| {
                def.rebuild_parsed_svars();
                def
            })
            .or_else(|| {
                panic!(
                    "WASM DESYNC: Server didn't provide card_def for {} (id={}) - this is a server bug",
                    reveal.name,
                    reveal.card_id.as_u32()
                )
            })
    }
}

/// Native card definition provider with database fallback
///
/// Native clients prefer server-provided definitions but can fall back to
/// a local card database if needed.
#[cfg(feature = "network")]
pub struct NativeCardDefProvider<'a> {
    card_db: &'a crate::loader::AsyncCardDatabase,
}

#[cfg(feature = "network")]
impl<'a> NativeCardDefProvider<'a> {
    /// Create a new native provider with database fallback
    pub fn new(card_db: &'a crate::loader::AsyncCardDatabase) -> Self {
        Self { card_db }
    }
}

#[cfg(feature = "network")]
impl<'a> CardDefProvider for NativeCardDefProvider<'a> {
    fn get_card_def(&self, reveal: &CardReveal) -> Option<CardDefinition> {
        if reveal.name.is_empty() {
            return None; // Dummy reveal
        }

        // Prefer the CardDefinition sent by the server (enables DB-free clients)
        if let Some(ref card_def) = reveal.card_def {
            let mut def = card_def.clone();
            def.rebuild_parsed_svars();
            return Some(def);
        }

        // Fallback to local database lookup
        match futures_executor::block_on(self.card_db.get_card(&reveal.name)) {
            Ok(Some(def)) => Some((*def).clone()),
            Ok(None) => panic!(
                "Native DESYNC: Card '{}' not in database and server didn't provide definition",
                reveal.name
            ),
            Err(e) => panic!(
                "Native DESYNC: Failed to load card '{}' from database: {}",
                reveal.name, e
            ),
        }
    }
}
