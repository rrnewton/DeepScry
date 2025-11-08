//! Centralized cardsfolder path resolution
//!
//! Single source of truth for locating the cardsfolder directory.
//! Future enhancements: support environment variables, installation paths, etc.

use std::path::{Path, PathBuf};

/// Find the cardsfolder directory
///
/// Searches in the following order:
/// 1. Environment variable CARDSFOLDER (TODO)
/// 2. ../cardsfolder (when running from mtg-engine/ subdirectory)
/// 3. cardsfolder (when running from repository root)
/// 4. Standard installation paths (TODO)
///
/// Returns None if cardsfolder cannot be found
pub fn find_cardsfolder() -> Option<PathBuf> {
    // TODO: Check CARDSFOLDER environment variable first
    // if let Ok(path) = std::env::var("CARDSFOLDER") {
    //     let p = PathBuf::from(path);
    //     if p.exists() && p.is_dir() {
    //         return Some(p);
    //     }
    // }

    // Check relative to current directory (for tests running from mtg-engine/)
    let relative_parent = PathBuf::from("../cardsfolder");
    if relative_parent.exists() && is_valid_cardsfolder(&relative_parent) {
        return Some(relative_parent);
    }

    // Check in current directory (for tests running from repository root)
    let relative_current = PathBuf::from("cardsfolder");
    if relative_current.exists() && is_valid_cardsfolder(&relative_current) {
        return Some(relative_current);
    }

    // TODO: Check standard installation paths
    // - /usr/share/mtg-forge-rs/cardsfolder
    // - ~/.local/share/mtg-forge-rs/cardsfolder
    // - etc.

    None
}

/// Find the cardsfolder directory or panic with helpful error message
///
/// Use this in tests where cardsfolder is required and skipping is not acceptable
pub fn require_cardsfolder() -> PathBuf {
    find_cardsfolder().expect(
        "cardsfolder not found! Searched:\n\
         - ../cardsfolder (relative to current directory)\n\
         - cardsfolder (in current directory)\n\
         \n\
         Please ensure the cardsfolder symlink exists:\n\
         - From repository root: ln -s forge-java/forge-gui/res/cardsfolder cardsfolder\n\
         \n\
         Future: set CARDSFOLDER environment variable to override search path.",
    )
}

/// Validate that a directory looks like a valid cardsfolder
///
/// Checks for the presence of subdirectories a-z which should contain card files
fn is_valid_cardsfolder(path: &Path) -> bool {
    // Quick heuristic: check if it has subdirectories like 'a', 'b', 'c', etc.
    // A valid cardsfolder should have at least a few letter subdirectories
    let letter_dirs = ['a', 'b', 'c', 'm', 'l']
        .iter()
        .filter(|&&letter| {
            let subdir = path.join(letter.to_string());
            subdir.exists() && subdir.is_dir()
        })
        .count();

    // If we find at least 3 of the common letter directories, it's probably valid
    letter_dirs >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_cardsfolder() {
        // This test should pass whether run from root or from mtg-engine/
        let cardsfolder = find_cardsfolder();

        // We expect to find it in development environment
        // If this fails, the developer needs to create the symlink
        assert!(
            cardsfolder.is_some(),
            "cardsfolder not found - please create symlink: \
             ln -s forge-java/forge-gui/res/cardsfolder cardsfolder"
        );

        let path = cardsfolder.unwrap();
        assert!(path.exists(), "cardsfolder path should exist");
        assert!(is_valid_cardsfolder(&path), "cardsfolder should be valid");
    }

    #[test]
    fn test_is_valid_cardsfolder() {
        // Test with actual cardsfolder if available
        if let Some(cardsfolder) = find_cardsfolder() {
            assert!(is_valid_cardsfolder(&cardsfolder));
        }
    }
}
