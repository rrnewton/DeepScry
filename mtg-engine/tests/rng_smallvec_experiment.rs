//! Comprehensive experiment to measure bincode serialization size of ChaCha12Rng
//! across many different states to determine optimal SmallVec inline capacity.

use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;

#[test]
fn comprehensive_bincode_size_measurement() {
    let mut sizes = std::collections::HashMap::new();
    let mut max_size = 0;
    let mut min_size = usize::MAX;

    println!("\n=== Comprehensive ChaCha12Rng bincode serialization size experiment ===\n");

    // Test 1: Fresh seeds with different values
    println!("Test 1: Fresh seeds (no RNG calls)");
    for seed in 0..1000 {
        let rng = ChaCha12Rng::seed_from_u64(seed);
        let bytes = bincode::serialize(&rng).expect("bincode serialization");
        let size = bytes.len();
        *sizes.entry(size).or_insert(0) += 1;
        max_size = max_size.max(size);
        min_size = min_size.min(size);
    }
    println!("  Sizes observed: {:?}", sizes.keys().collect::<Vec<_>>());
    println!("  Min: {}, Max: {}", min_size, max_size);

    // Test 2: Advanced RNG states (many random draws)
    println!("\nTest 2: Advanced RNG states (1000-10000 draws)");
    sizes.clear();
    for seed in 0..100 {
        let mut rng = ChaCha12Rng::seed_from_u64(seed);
        use rand::Rng;
        // Advance the RNG state significantly
        for _ in 0..(seed * 100 + 1000) {
            let _ = rng.gen::<u64>();
        }
        let bytes = bincode::serialize(&rng).expect("bincode serialization");
        let size = bytes.len();
        *sizes.entry(size).or_insert(0) += 1;
        max_size = max_size.max(size);
        min_size = min_size.min(size);
    }
    println!("  Sizes observed: {:?}", sizes.keys().collect::<Vec<_>>());
    println!("  Min: {}, Max: {}", min_size, max_size);

    // Test 3: Different stream values (using from_seed with full 32 bytes)
    println!("\nTest 3: Different seed patterns (full 32-byte seeds)");
    sizes.clear();
    for i in 0..100 {
        let seed = [i as u8; 32]; // All bytes same value
        let rng = ChaCha12Rng::from_seed(seed);
        let bytes = bincode::serialize(&rng).expect("bincode serialization");
        let size = bytes.len();
        *sizes.entry(size).or_insert(0) += 1;
        max_size = max_size.max(size);
        min_size = min_size.min(size);
    }
    println!("  Sizes observed: {:?}", sizes.keys().collect::<Vec<_>>());
    println!("  Min: {}, Max: {}", min_size, max_size);

    // Test 4: Alternating seed patterns
    println!("\nTest 4: Alternating seed patterns");
    sizes.clear();
    for pattern in 0..100 {
        let mut seed = [0u8; 32];
        for (i, byte) in seed.iter_mut().enumerate() {
            *byte = if i % 2 == 0 { pattern as u8 } else { 255 - pattern as u8 };
        }
        let mut rng = ChaCha12Rng::from_seed(seed);
        use rand::Rng;
        // Mix of fresh and advanced states
        for _ in 0..(pattern * 10) {
            let _ = rng.gen::<u128>();
        }
        let bytes = bincode::serialize(&rng).expect("bincode serialization");
        let size = bytes.len();
        *sizes.entry(size).or_insert(0) += 1;
        max_size = max_size.max(size);
        min_size = min_size.min(size);
    }
    println!("  Sizes observed: {:?}", sizes.keys().collect::<Vec<_>>());
    println!("  Min: {}, Max: {}", min_size, max_size);

    // Test 5: Extreme word_pos values (many u128 draws to advance word_pos)
    println!("\nTest 5: Extreme word_pos values (massive advancement)");
    sizes.clear();
    for seed in 0..20 {
        let mut rng = ChaCha12Rng::seed_from_u64(seed);
        use rand::Rng;
        // Draw many u128 values to advance word_pos significantly
        for _ in 0..100000 {
            let _ = rng.gen::<u128>();
        }
        let bytes = bincode::serialize(&rng).expect("bincode serialization");
        let size = bytes.len();
        *sizes.entry(size).or_insert(0) += 1;
        max_size = max_size.max(size);
        min_size = min_size.min(size);

        if seed < 3 {
            println!("    Seed {} after 100k u128 draws: {} bytes", seed, size);
        }
    }
    println!("  Sizes observed: {:?}", sizes.keys().collect::<Vec<_>>());
    println!("  Min: {}, Max: {}", min_size, max_size);

    println!("\n=== Final Summary ===");
    println!("Overall min size: {} bytes", min_size);
    println!("Overall max size: {} bytes", max_size);
    println!("\nConclusion:");
    if max_size == min_size {
        println!(
            "  ✓ ChaCha12Rng bincode serialization is FIXED SIZE: {} bytes",
            max_size
        );
        println!(
            "  ✓ Recommended SmallVec inline capacity: {} bytes (next power of 2)",
            max_size.next_power_of_two()
        );
    } else {
        println!(
            "  ✗ ChaCha12Rng bincode serialization is VARIABLE SIZE: {}-{} bytes",
            min_size, max_size
        );
        println!(
            "  ✓ Recommended SmallVec inline capacity: {} bytes (max + margin)",
            (max_size + 8).next_power_of_two()
        );
    }

    // Verify against user expectation
    if max_size > 200 {
        println!(
            "\n⚠ User expected ~224 bytes, found {} bytes - expectation CONFIRMED",
            max_size
        );
    } else if max_size < 100 {
        println!(
            "\n⚠ User expected ~224 bytes, but found only {} bytes - DISCREPANCY",
            max_size
        );
        println!("   This suggests ChaCha12Rng structure is smaller than expected.");
    }

    assert!(max_size <= 300, "Sanity check: bincode size should be under 300 bytes");
}

#[test]
fn inspect_chacha_structure_size() {
    use std::mem::size_of;

    println!("\n=== ChaCha12Rng structure size analysis ===\n");

    // Check the actual in-memory size
    let in_memory_size = size_of::<ChaCha12Rng>();
    println!("ChaCha12Rng in-memory size: {} bytes", in_memory_size);

    // Expected from documentation: seed [u8;32] + stream u64 + word_pos u128
    let expected = 32 + 8 + 16;
    println!("Expected from docs (32 + 8 + 16): {} bytes", expected);

    // Try a fresh RNG
    let rng = ChaCha12Rng::seed_from_u64(42);
    let bytes = bincode::serialize(&rng).expect("serialization");
    println!("Fresh RNG bincode size: {} bytes", bytes.len());
    println!("Raw bytes (first 64): {:?}", &bytes[..bytes.len().min(64)]);

    if bytes.len() != expected {
        println!(
            "\n⚠ Discrepancy: bincode size ({}) != expected ({})",
            bytes.len(),
            expected
        );
        println!("  This could be due to:");
        println!("  - Additional fields in ChaCha12Rng struct");
        println!("  - bincode serialization overhead");
        println!("  - Alignment padding");
    }
}
