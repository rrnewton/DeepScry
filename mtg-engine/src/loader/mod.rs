//! Card and deck loaders
//!
//! Parsers for the Forge card format (.txt) and deck format (.dck)

pub mod ability_parser;
pub mod card;
pub mod cardsfolder;
pub mod database_async;
pub mod deck;
pub mod deck_async;
pub mod effect_converter;
pub mod game_init;

pub use card::{CardDefinition, CardLoader};
pub use cardsfolder::{find_cardsfolder, require_cardsfolder};
pub use database_async::CardDatabase as AsyncCardDatabase;
pub use deck::{DeckEntry, DeckList, DeckLoader};
pub use deck_async::prefetch_deck_cards;
pub use game_init::GameInitializer;

// Re-export AsyncCardDatabase as CardDatabase for convenience
pub use database_async::CardDatabase;
