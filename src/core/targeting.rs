'''//! Targeting specification parser

use crate::{MtgError, Result};

#[derive(Debug, PartialEq)]
pub enum TargetSpec {
    Any,
    Player,
    Creature,
    Land,
    Card,
    Complex(String),
}

pub fn parse(s: &str) -> Result<TargetSpec> {
    match s.to_lowercase().as_str() {
        "any" => Ok(TargetSpec::Any),
        "player" => Ok(TargetSpec::Player),
        "creature" => Ok(TargetSpec::Creature),
        "land" => Ok(TargetSpec::Land),
        "card" => Ok(TargetSpec::Card),
        _ => Ok(TargetSpec::Complex(s.to_string())),
    }
}
'''