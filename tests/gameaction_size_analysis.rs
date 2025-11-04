//! Analysis of GameAction variant sizes for undo log optimization
//!
//! This test measures the in-memory size of each GameAction variant to identify
//! size imbalances that might benefit from boxing or other optimizations.

use mtg_forge_rs::core::{CardId, CounterType, ManaCost, PlayerId};
use mtg_forge_rs::game::Step;
use mtg_forge_rs::undo::GameAction;
use mtg_forge_rs::zones::Zone;

#[test]
fn analyze_gameaction_variant_sizes() {
    use std::mem::size_of;
    use std::mem::size_of_val;

    println!("\n=== GameAction Variant Size Analysis ===\n");

    // Overall enum size (discriminant + largest variant)
    let enum_size = size_of::<GameAction>();
    println!("GameAction enum size: {} bytes\n", enum_size);

    // Create sample instances of each variant and measure
    let variants: Vec<(&str, GameAction)> = vec![
        (
            "MoveCard",
            GameAction::MoveCard {
                card_id: CardId::new(1),
                from_zone: Zone::Hand,
                to_zone: Zone::Battlefield,
                owner: PlayerId::new(0),
            },
        ),
        (
            "TapCard",
            GameAction::TapCard {
                card_id: CardId::new(1),
                tapped: true,
            },
        ),
        (
            "ModifyLife",
            GameAction::ModifyLife {
                player_id: PlayerId::new(0),
                delta: -3,
            },
        ),
        (
            "AddMana",
            GameAction::AddMana {
                player_id: PlayerId::new(0),
                mana: ManaCost::from_string("2RRG"),
            },
        ),
        (
            "EmptyManaPool",
            GameAction::EmptyManaPool {
                player_id: PlayerId::new(0),
                prev_white: 1,
                prev_blue: 2,
                prev_black: 3,
                prev_red: 4,
                prev_green: 5,
                prev_colorless: 6,
            },
        ),
        (
            "AddCounter",
            GameAction::AddCounter {
                card_id: CardId::new(1),
                counter_type: CounterType::P1P1,
                amount: 3,
            },
        ),
        (
            "RemoveCounter",
            GameAction::RemoveCounter {
                card_id: CardId::new(1),
                counter_type: CounterType::M1M1,
                amount: 1,
            },
        ),
        (
            "AdvanceStep",
            GameAction::AdvanceStep {
                from_step: Step::Untap,
                to_step: Step::Upkeep,
            },
        ),
        (
            "ChangeTurn (no RNG)",
            GameAction::ChangeTurn {
                from_player: PlayerId::new(0),
                to_player: PlayerId::new(1),
                turn_number: 5,
                rng_state: None,
            },
        ),
        (
            "ChangeTurn (with RNG)",
            GameAction::ChangeTurn {
                from_player: PlayerId::new(0),
                to_player: PlayerId::new(1),
                turn_number: 5,
                rng_state: Some(smallvec::SmallVec::from_vec(vec![0u8; 56])), // 56 bytes from bincode serialization
            },
        ),
        (
            "PumpCreature",
            GameAction::PumpCreature {
                card_id: CardId::new(1),
                power_delta: 3,
                toughness_delta: 3,
            },
        ),
        (
            "ChoicePoint (no choice)",
            GameAction::ChoicePoint {
                player_id: PlayerId::new(0),
                choice_id: 42,
                choice: None,
            },
        ),
    ];

    println!("Individual variant sizes (stack allocation):");
    println!("{:<25} {:>10}", "Variant", "Size (bytes)");
    println!("{}", "-".repeat(37));

    let mut max_size = 0;
    let mut max_variant = "";
    let mut sizes = Vec::new();

    for (name, variant) in &variants {
        let size = size_of_val(variant);
        sizes.push((name, size));
        println!("{:<25} {:>10}", name, size);

        if size > max_size {
            max_size = size;
            max_variant = name;
        }
    }

    println!("\n=== Analysis ===");
    println!("Largest variant: {} ({} bytes)", max_variant, max_size);
    println!("Enum overhead: {} bytes", enum_size - max_size);
    println!("Total enum size: {} bytes (discriminant + largest variant)", enum_size);

    // Calculate waste for each variant
    println!("\n=== Memory Waste Analysis ===");
    println!("Each variant is padded to {} bytes (enum size)", enum_size);
    println!("{:<25} {:>10} {:>10} {:>10}", "Variant", "Actual", "Padded", "Waste");
    println!("{}", "-".repeat(57));

    let mut total_waste_bytes = 0;
    for (name, size) in &sizes {
        let waste = enum_size - size;
        let waste_pct = (waste as f64 / enum_size as f64) * 100.0;
        println!("{:<25} {:>10} {:>10} {:>9} ({:.1}%)", name, size, enum_size, waste, waste_pct);
        total_waste_bytes += waste;
    }

    println!("\n=== Vec Overhead Analysis ===");
    // ChangeTurn with Vec<u8> has heap allocation overhead
    let vec_empty_size = size_of::<Vec<u8>>();
    let vec_with_data_stack = vec_empty_size; // Vec itself is always 24 bytes on stack
    println!("Vec<u8> stack size: {} bytes (ptr + len + cap)", vec_empty_size);
    println!("Vec<u8> with 56 bytes data:");
    println!("  Stack: {} bytes", vec_with_data_stack);
    println!("  Heap: 56 bytes (ChaCha12Rng bincode serialization)");
    println!("  Total: {} bytes", vec_with_data_stack + 56);

    println!("\n=== Recommendations ===");

    // Check for significant imbalances
    let avg_size = sizes.iter().map(|(_, s)| s).sum::<usize>() / sizes.len();
    let imbalance_ratio = max_size as f64 / avg_size as f64;

    println!("Average variant size: {} bytes", avg_size);
    println!("Max/avg ratio: {:.2}x", imbalance_ratio);

    if imbalance_ratio > 2.0 {
        println!("\n⚠ SIGNIFICANT IMBALANCE DETECTED");
        println!("  The largest variant ({}) is {:.1}x larger than average", max_variant, imbalance_ratio);
        println!("  Consider boxing the largest variant to reduce enum size");
        println!("  This would save ~{} bytes per smaller variant", max_size - avg_size);
    } else {
        println!("\n✓ Variants are reasonably balanced");
        println!("  Boxing optimization not recommended (max/avg ratio < 2.0)");
    }

    // Check for Vec usage
    println!("\n=== Heap Allocation Hotspots ===");
    println!("ChangeTurn variant allocates 56 bytes on heap for RNG state");
    println!("Optimization options:");
    println!("  1. Use SmallVec<[u8; 64]> - inline allocation for ≤64 bytes");
    println!("  2. Use Box<[u8; 56]> - heap allocation with fixed size");
    println!("  3. Use fixed-size array [u8; 56] - stack allocation (best)");
    println!("\nRecommendation: SmallVec<[u8; 64]> balances flexibility and performance");

    // Count field sizes
    println!("\n=== Component Type Sizes ===");
    println!("CardId: {} bytes", size_of::<CardId>());
    println!("PlayerId: {} bytes", size_of::<PlayerId>());
    println!("Zone: {} bytes", size_of::<Zone>());
    println!("Step: {} bytes", size_of::<Step>());
    println!("CounterType: {} bytes", size_of::<CounterType>());
    println!("ManaCost: {} bytes", size_of::<ManaCost>());
    println!("Option<Vec<u8>>: {} bytes", size_of::<Option<Vec<u8>>>());
    println!("i32: {} bytes", size_of::<i32>());
    println!("u32: {} bytes", size_of::<u32>());
    println!("u8: {} bytes", size_of::<u8>());
    println!("bool: {} bytes", size_of::<bool>());
}
