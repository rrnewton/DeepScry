//! Centralized cardsfolder path resolution
//!
//! Single source of truth for locating the cardsfolder directory.
//! Searches in current directory, binary location, and parent directories.

use std::path::{Path, PathBuf};

/// Find the cardsfolder directory
///
/// Searches in the following order:
/// 1. Environment variable CARDSFOLDER (if set)
/// 2. ./cardsfolder (in current working directory)
/// 3. Directory containing the `mtg` binary, then parent directories up to root
///
/// Returns None if cardsfolder cannot be found
pub fn find_cardsfolder() -> Option<PathBuf> {
    // 1. Check CARDSFOLDER environment variable first
    if let Ok(path) = std::env::var("CARDSFOLDER") {
        let p = PathBuf::from(&path);
        if p.exists() && is_valid_cardsfolder(&p) {
            return Some(p);
        }
    }

    // 2. Check in current working directory
    let cwd_cardsfolder = PathBuf::from("cardsfolder");
    if cwd_cardsfolder.exists() && is_valid_cardsfolder(&cwd_cardsfolder) {
        return Some(cwd_cardsfolder);
    }

    // 3. Search from binary directory up to root
    if let Some(path) = find_cardsfolder_from_binary() {
        return Some(path);
    }

    // 4. Fall back to searching from current directory up to root
    // (handles cases where we can't determine binary location)
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(path) = search_parent_directories(&cwd) {
            return Some(path);
        }
    }

    None
}

/// Search for cardsfolder starting from the binary's directory
fn find_cardsfolder_from_binary() -> Option<PathBuf> {
    // Get the path to the current executable
    let exe_path = std::env::current_exe().ok()?;

    // Get the directory containing the executable
    let exe_dir = exe_path.parent()?;

    // Search from exe directory up to root
    search_parent_directories(exe_dir)
}

/// Search for cardsfolder in the given directory and all parent directories
fn search_parent_directories(start_dir: &Path) -> Option<PathBuf> {
    let mut current = start_dir.to_path_buf();

    loop {
        // Check for cardsfolder in current directory
        let candidate = current.join("cardsfolder");
        if candidate.exists() && is_valid_cardsfolder(&candidate) {
            return Some(candidate);
        }

        // Move to parent directory
        match current.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                current = parent.to_path_buf();
            }
            _ => {
                // Reached root (or empty path), not found
                return None;
            }
        }
    }
}

/// Find the cardsfolder directory or panic with helpful error message
///
/// Use this in tests where cardsfolder is required and skipping is not acceptable
///
/// # Panics
///
/// Panics if the cardsfolder cannot be found (provides helpful error message with setup instructions).
pub fn require_cardsfolder() -> PathBuf {
    find_cardsfolder().expect(
        "cardsfolder not found! Searched:\n\
         - CARDSFOLDER environment variable\n\
         - ./cardsfolder (in current working directory)\n\
         - Binary directory and all parent directories up to root\n\
         - Current directory and all parent directories up to root\n\
         \n\
         Please ensure the cardsfolder symlink exists:\n\
         - From repository root: ln -s forge-java/forge-gui/res/cardsfolder cardsfolder\n\
         \n\
         Or set the CARDSFOLDER environment variable to the path.",
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
