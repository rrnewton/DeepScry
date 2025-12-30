//! Card and deck loaders
//!
//! Parsers for the Forge card format (.txt) and deck format (.dck)

pub mod ability_parser;
pub mod card;
#[cfg(feature = "native")]
pub mod cardsfolder;
#[cfg(feature = "native")]
pub mod database_async;
pub mod deck;
#[cfg(feature = "native")]
pub mod deck_async;
#[cfg(feature = "native")]
pub mod edition;
pub mod effect_converter;
#[cfg(feature = "native")]
pub mod game_init;

pub use card::{CardDefinition, CardLoader};
#[cfg(feature = "native")]
pub use cardsfolder::{find_cardsfolder, require_cardsfolder};
#[cfg(feature = "native")]
pub use database_async::CardDatabase as AsyncCardDatabase;
pub use deck::{DeckEntry, DeckList, DeckLoader};
#[cfg(feature = "native")]
pub use deck_async::prefetch_deck_cards;
#[cfg(feature = "native")]
pub use edition::CardEditionIndex;
#[cfg(feature = "native")]
pub use game_init::GameInitializer;

// Re-export AsyncCardDatabase as CardDatabase for convenience
#[cfg(feature = "native")]
pub use database_async::CardDatabase;
