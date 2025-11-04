//! Test to compare JSON vs bincode serialization for ChaCha12Rng
//!
//! This test measures the actual serialization sizes to inform optimization work.

use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;

#[test]
fn compare_chacha_serialization_sizes() {
    // Create a ChaCha12Rng with a seed
    let mut rng = ChaCha12Rng::seed_from_u64(42);

    // Generate some random numbers to advance the state
    use rand::Rng;
    for _ in 0..100 {
        let _ = rng.gen::<u32>();
    }

    // Serialize with JSON (current approach)
    let json_bytes = serde_json::to_vec(&rng).expect("JSON serialization failed");
    println!("JSON serialization: {} bytes", json_bytes.len());
    println!(
        "JSON content preview: {}",
        String::from_utf8_lossy(&json_bytes[..json_bytes.len().min(200)])
    );

    // Serialize with bincode (proposed approach)
    let bincode_bytes = bincode::serialize(&rng).expect("bincode serialization failed");
    println!("\nbincode serialization: {} bytes", bincode_bytes.len());

    // Verify both deserialize correctly
    let json_restored: ChaCha12Rng = serde_json::from_slice(&json_bytes).expect("JSON deserialization failed");
    let bincode_restored: ChaCha12Rng = bincode::deserialize(&bincode_bytes).expect("bincode deserialization failed");

    // Generate numbers from both to verify they're identical
    let mut json_rng = json_restored;
    let mut bincode_rng = bincode_restored;

    for i in 0..10 {
        let json_val = json_rng.gen::<u64>();
        let bincode_val = bincode_rng.gen::<u64>();
        assert_eq!(
            json_val, bincode_val,
            "RNG mismatch at iteration {}: JSON={} bincode={}",
            i, json_val, bincode_val
        );
    }

    println!("\n✓ Both serialization methods produce identical RNG states");
    println!(
        "Size reduction: {} bytes ({:.1}%)",
        json_bytes.len() - bincode_bytes.len(),
        100.0 * (1.0 - (bincode_bytes.len() as f64 / json_bytes.len() as f64))
    );
}

#[test]
fn measure_worst_case_bincode_size() {
    // Test multiple RNG states to find maximum bincode size
    let mut max_size = 0;

    for seed in 0..1000 {
        let mut rng = ChaCha12Rng::seed_from_u64(seed);

        // Advance to various positions
        use rand::Rng;
        for _ in 0..seed {
            let _ = rng.gen::<u64>();
        }

        let bincode_bytes = bincode::serialize(&rng).expect("bincode serialization");
        if bincode_bytes.len() > max_size {
            max_size = bincode_bytes.len();
        }
    }

    println!("Maximum bincode size over 1000 samples: {} bytes", max_size);
    assert!(
        max_size <= 100,
        "bincode size should be under 100 bytes for ChaCha12Rng"
    );
}
