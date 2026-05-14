//! Mana production types and logic
//!
//! This module defines the fundamental types for representing what mana a card can produce.
//! These types are part of the core domain model and are used by both the card cache
//! (for pre-computed mana production) and the game engine (for runtime mana payment).

use crate::core::ManaCost;
use crate::game::mana_colors::ManaColors;
use serde::{Deserialize, Serialize};

/// Represents a color of mana
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ManaColor {
    White,
    Blue,
    Black,
    Red,
    Green,
}

/// The five mana colors in WUBRG order
pub const ALL_MANA_COLORS: [ManaColor; 5] = [
    ManaColor::White,
    ManaColor::Blue,
    ManaColor::Black,
    ManaColor::Red,
    ManaColor::Green,
];

impl ManaColor {
    /// Returns iterator over the five mana colors in WUBRG order.
    ///
    /// This is a zero-cost abstraction: the array is const and
    /// `into_iter()` compiles to the same code as an inline array.
    #[inline]
    pub fn all_colors() -> impl Iterator<Item = ManaColor> {
        ALL_MANA_COLORS.into_iter()
    }

    /// Convert to single-character representation (W, U, B, R, G)
    pub fn to_char(self) -> char {
        match self {
            ManaColor::White => 'W',
            ManaColor::Blue => 'U',
            ManaColor::Black => 'B',
            ManaColor::Red => 'R',
            ManaColor::Green => 'G',
        }
    }

    /// Parse from single-character representation
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'W' | 'w' => Some(ManaColor::White),
            'U' | 'u' => Some(ManaColor::Blue),
            'B' | 'b' => Some(ManaColor::Black),
            'R' | 'r' => Some(ManaColor::Red),
            'G' | 'g' => Some(ManaColor::Green),
            _ => None,
        }
    }
}

/// The kind of mana a source can produce
///
/// This represents an UPPER BOUND on mana production (OR semantics, not AND).
/// For example, Choice([R, G]) means "can produce R OR G, choose one".
///
/// This does NOT include activation costs - those are stored separately in ManaProduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManaProductionKind {
    /// Produces exactly one specific color (e.g., Mountain → {R})
    Fixed(ManaColor),

    /// Can produce ONE of several colors (OR logic, choose one)
    /// Examples:
    /// - Taiga (dual land): Choice([R, G]) - tap for R OR G
    /// - Bloom Tender: Choice([W,U,B,R,G]) - tap for one of any you have
    Choice(ManaColors),

    /// Can produce any color (e.g., City of Brass, Birds of Paradise)
    AnyColor,

    /// Produces colorless mana (e.g., Wastes)
    Colorless,
}

impl Default for ManaProductionKind {
    /// Default is no mana production
    fn default() -> Self {
        ManaProductionKind::Choice(ManaColors::new())
    }
}

/// Activation-cost / preservation severity for a mana source.
///
/// This combines two concerns the resolver wants to bake into a single
/// orderable score:
/// 1. The non-mana cost paid to activate the mana ability itself.
/// 2. The strategic value of "saving" the source for a non-mana use.
///
/// Variants are deliberately ordered cheapest → most expensive so simple
/// `Ord` comparisons let the resolver tap the most expendable source last:
/// - **None** — basic lands, Moxen, dual lands. No cost, no other use.
/// - **Utility** — lands/permanents with *other* (non-mana) activated
///   abilities (Mishra's Factory, Strip Mine, Mutavault, manlands). Free to
///   tap *for mana*, but tapping prevents you from using the utility ability
///   that turn, so prefer plain lands first.
/// - **PayLife(n)** — pain lands such as City of Brass / Mana Confluence
///   (`n` = printed life cost).
/// - **Sacrifice** — destroys the source on tap (Black Lotus, Lotus Petal,
///   Treasure tokens). Almost always the worst choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum ManaSideCost {
    /// No side cost — free to tap (basic lands, Moxen, dual lands).
    #[default]
    None,
    /// Source has another activated ability the player may want to use later
    /// (Mishra's Factory, Strip Mine, Mutavault). Tapping for mana spends the
    /// ability slot, so prefer plain lands first.
    Utility,
    /// Pay life as part of activation (City of Brass, Mana Confluence).
    /// `amount` is the printed life cost (1 for City of Brass).
    PayLife(u8),
    /// Sacrifice the source as part of activation (Black Lotus, Lotus Petal,
    /// Treasure tokens). The card is destroyed when activated.
    Sacrifice,
}

/// What mana a source can produce and at what cost
///
/// This struct represents the UPPER BOUND on what mana a permanent can produce.
/// - `kind`: What colors/types of mana can be produced (OR semantics)
/// - `activation_cost`: Optional cost to activate (e.g., pay {2} to produce mana)
///
/// The `kind` field does NOT account for costs - it represents the maximum theoretical
/// mana production assuming you can pay any activation cost.
///
/// Use `net_delta()` to check if this source produces net positive mana after costs.
///
/// This is Copy-eligible: 2 bytes (ManaProductionKind) + 9 bytes (Option<ManaCost>)
/// + 1 byte (amount) = 12 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManaProduction {
    /// The type of mana this source produces (upper bound, OR semantics)
    pub kind: ManaProductionKind,

    /// Optional activation cost (e.g., pay {2} to produce mana)
    /// None means no mana cost (tap-only or free ability)
    pub activation_cost: Option<ManaCost>,

    /// How many mana this source produces per activation
    ///
    /// For most permanents (basic lands, single-color Moxes) this is 1. Cards
    /// like Sol Ring (`Amount$ 2`) and Black Lotus (`Amount$ 3`) produce more
    /// than one mana per tap and use a higher value.
    ///
    /// For `Choice`/`AnyColor`, the amount is the number of mana of the chosen
    /// color produced (e.g. Black Lotus → AnyColor, amount 3 = "add three of any
    /// one color"). For `Fixed`/`Colorless`, the amount is the count of that
    /// specific color.
    ///
    /// Defaults to 1 to keep older snapshots / tests working transparently when
    /// they predate this field.
    #[serde(default = "default_amount")]
    pub amount: u8,

    /// Non-mana cost paid alongside tapping this source. Used by the mana
    /// resolver to deprioritize destructive sources (Black Lotus → Sacrifice,
    /// City of Brass → PayLife) so cheap lands are tapped first.
    ///
    /// Defaults to `ManaSideCost::None` to keep older snapshots/tests working
    /// transparently when they predate this field.
    #[serde(default)]
    pub side_cost: ManaSideCost,
}

fn default_amount() -> u8 {
    1
}

impl Default for ManaProduction {
    /// Default is no mana production
    fn default() -> Self {
        Self {
            kind: ManaProductionKind::default(),
            activation_cost: None,
            amount: 1,
            side_cost: ManaSideCost::None,
        }
    }
}

impl ManaProduction {
    /// Create a new mana production with no activation cost
    pub fn free(kind: ManaProductionKind) -> Self {
        Self {
            kind,
            activation_cost: None,
            amount: 1,
            side_cost: ManaSideCost::None,
        }
    }

    /// Create a new mana production with an activation cost
    pub fn with_cost(kind: ManaProductionKind, cost: ManaCost) -> Self {
        Self {
            kind,
            activation_cost: Some(cost),
            amount: 1,
            side_cost: ManaSideCost::None,
        }
    }

    /// Create a new mana production with a specific output amount
    ///
    /// Use this for permanents like Sol Ring (`Amount$ 2`) or Black Lotus
    /// (`Amount$ 3`) that produce multiple mana per activation.
    pub fn with_amount(kind: ManaProductionKind, amount: u8) -> Self {
        Self {
            kind,
            activation_cost: None,
            amount: amount.max(1),
            side_cost: ManaSideCost::None,
        }
    }

    /// Builder helper: attach a non-mana side cost (sacrifice / pay life).
    pub fn with_side_cost(mut self, side_cost: ManaSideCost) -> Self {
        self.side_cost = side_cost;
        self
    }

    /// Numeric severity score for the side cost, used by the mana resolver to
    /// order sources from cheapest to most expensive. Lower = tap first.
    ///
    /// We weight `Sacrifice` heavily — losing the source itself is almost
    /// always worse than paying a few life. PayLife scales with the printed
    /// life cost so a 2-life ability is preferred over a 3-life ability.
    #[inline]
    pub fn side_cost_score(&self) -> u16 {
        match self.side_cost {
            ManaSideCost::None => 0,
            // Utility lands sit just above plain lands — preferred over pain
            // and sacrifice but tapped after vanilla lands.
            ManaSideCost::Utility => 5,
            // Pay 1 life ≈ score 20, scale linearly. Stays well below sacrifice.
            ManaSideCost::PayLife(n) => 20u16.saturating_mul(u16::from(n)).min(900),
            ManaSideCost::Sacrifice => 1000,
        }
    }

    /// Get the net mana delta (production - cost) for total mana bounds checking
    /// This is an i8 because you can have negative delta (pay more than you produce)
    pub fn net_delta(&self) -> i8 {
        let production = self.amount as i8;
        let cost = self.activation_cost.as_ref().map(|c| c.cmc() as i8).unwrap_or(0);
        production - cost
    }

    /// Check if this production is non-zero (produces at least some mana)
    #[inline(always)]
    pub fn produces_mana(&self) -> bool {
        match &self.kind {
            ManaProductionKind::Fixed(_) => true,
            ManaProductionKind::Choice(colors) => !colors.is_empty(),
            ManaProductionKind::AnyColor => true,
            ManaProductionKind::Colorless => true,
        }
    }
}
