// Minimal test to demonstrate bincode incompatibility with skip_serializing_if
//
// Run with: cargo run --example test_bincode_skip

use serde::{Deserialize, Serialize};

// Simulating the CardDefinition structure with the problematic attribute
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct CardWithSkip {
    name: String,
    power: Option<i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    script_name: Option<String>, // This is the problematic field
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct CardWithoutSkip {
    name: String,
    power: Option<i8>,
    script_name: Option<String>, // Same field, no skip_serializing_if
}

fn main() {
    println!("=== Testing bincode with skip_serializing_if ===\n");

    // Test 1: Card with script_name = None
    let card_none = CardWithSkip {
        name: "Test Card".to_string(),
        power: Some(5),
        script_name: None,
    };

    let bytes_none = bincode::serialize(&card_none).unwrap();
    println!(
        "Card with script_name=None serialized to {} bytes: {:?}",
        bytes_none.len(),
        bytes_none
    );

    // Test 2: Card with script_name = Some("token")
    let card_some = CardWithSkip {
        name: "Test Card".to_string(),
        power: Some(5),
        script_name: Some("token".to_string()),
    };

    let bytes_some = bincode::serialize(&card_some).unwrap();
    println!(
        "Card with script_name=Some serialized to {} bytes: {:?}",
        bytes_some.len(),
        bytes_some
    );

    // Try to deserialize the None case
    println!("\nAttempting to deserialize script_name=None case...");
    match bincode::deserialize::<CardWithSkip>(&bytes_none) {
        Ok(card) => println!("SUCCESS: {:?}", card),
        Err(e) => println!("FAILED: {}", e),
    }

    // Try to deserialize the Some case
    println!("\nAttempting to deserialize script_name=Some case...");
    match bincode::deserialize::<CardWithSkip>(&bytes_some) {
        Ok(card) => println!("SUCCESS: {:?}", card),
        Err(e) => println!("FAILED: {}", e),
    }

    // Now test without skip_serializing_if
    println!("\n=== Testing bincode WITHOUT skip_serializing_if ===\n");

    let card_none2 = CardWithoutSkip {
        name: "Test Card".to_string(),
        power: Some(5),
        script_name: None,
    };

    let bytes_none2 = bincode::serialize(&card_none2).unwrap();
    println!(
        "Card with script_name=None serialized to {} bytes: {:?}",
        bytes_none2.len(),
        bytes_none2
    );

    let card_some2 = CardWithoutSkip {
        name: "Test Card".to_string(),
        power: Some(5),
        script_name: Some("token".to_string()),
    };

    let bytes_some2 = bincode::serialize(&card_some2).unwrap();
    println!(
        "Card with script_name=Some serialized to {} bytes: {:?}",
        bytes_some2.len(),
        bytes_some2
    );

    println!("\nAttempting to deserialize script_name=None case...");
    match bincode::deserialize::<CardWithoutSkip>(&bytes_none2) {
        Ok(card) => println!("SUCCESS: {:?}", card),
        Err(e) => println!("FAILED: {}", e),
    }

    println!("\nAttempting to deserialize script_name=Some case...");
    match bincode::deserialize::<CardWithoutSkip>(&bytes_some2) {
        Ok(card) => println!("SUCCESS: {:?}", card),
        Err(e) => println!("FAILED: {}", e),
    }

    // Compare byte sizes
    println!("\n=== Byte comparison ===");
    println!("WITH skip_serializing_if, None case: {} bytes", bytes_none.len());
    println!("WITHOUT skip_serializing_if, None case: {} bytes", bytes_none2.len());
    println!(
        "Difference: {} bytes",
        bytes_none2.len() as i32 - bytes_none.len() as i32
    );

    // The issue: when skip_serializing_if skips the field, bincode's byte stream
    // doesn't include it. But bincode doesn't self-describe its format, so on
    // deserialization it expects all fields in order. This causes the deserializer
    // to misalign and interpret garbage as enum tags.

    println!("\n=== CONCLUSION ===");
    println!("bincode is incompatible with #[serde(skip_serializing_if)] because:");
    println!("1. bincode is NOT a self-describing format (no field names)");
    println!("2. skip_serializing_if omits the field from the byte stream");
    println!("3. Deserializer expects fields in order, misaligns when field is missing");
    println!("\nFIX: Remove skip_serializing_if from CardDefinition::script_name");
}
