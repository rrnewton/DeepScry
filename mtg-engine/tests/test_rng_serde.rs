//! Test RNG serialization/deserialization fidelity
//!
//! This test verifies that Xoshiro256PlusPlus RNG state can be serialized
//! and deserialized correctly, producing identical random sequences.

use rand::Rng;
use rand::SeedableRng;
use rand_xoshiro::Xoshiro256PlusPlus;

#[test]
fn test_rng_serialize_deserialize_fidelity() {
    // Create RNG with a specific seed
    let mut rng1 = Xoshiro256PlusPlus::seed_from_u64(42);

    // Generate some initial numbers to advance the state
    for _ in 0..10 {
        rng1.gen::<u64>();
    }

    // Serialize the RNG state
    let json = serde_json::to_string(&rng1).expect("Failed to serialize RNG");
    println!("Serialized RNG state: {}", json);

    // Deserialize to a new RNG
    let mut rng2: Xoshiro256PlusPlus = serde_json::from_str(&json).expect("Failed to deserialize RNG");

    // Both RNGs should now generate identical sequences
    for i in 0..100 {
        let val1 = rng1.gen::<u64>();
        let val2 = rng2.gen::<u64>();
        assert_eq!(
            val1, val2,
            "RNG divergence at iteration {}: rng1={}, rng2={}",
            i, val1, val2
        );
    }

    println!("✓ RNG serialization/deserialization is faithful for 100 iterations");
}

#[test]
fn test_rng_serialize_deserialize_with_choices() {
    // Simulate making controller choices with an RNG
    let mut rng1 = Xoshiro256PlusPlus::seed_from_u64(12345);

    // Make some "game choices"
    let choices_before: Vec<usize> = (0..5).map(|_| rng1.gen_range(0..10)).collect();

    // Serialize
    let json = serde_json::to_string(&rng1).expect("Failed to serialize");

    // Deserialize
    let mut rng2: Xoshiro256PlusPlus = serde_json::from_str(&json).expect("Failed to deserialize");

    // Make more choices - should be identical
    let choices_after1: Vec<usize> = (0..10).map(|_| rng1.gen_range(0..10)).collect();

    let choices_after2: Vec<usize> = (0..10).map(|_| rng2.gen_range(0..10)).collect();

    println!("Before serialize: {:?}", choices_before);
    println!("After serialize (rng1): {:?}", choices_after1);
    println!("After serialize (rng2): {:?}", choices_after2);

    assert_eq!(
        choices_after1, choices_after2,
        "RNG choices diverged after serialization"
    );
}
