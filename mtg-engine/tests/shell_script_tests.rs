//! Integration tests that automatically discover and run shell scripts
//!
//! This module uses the `dir-test` crate to automatically discover and run all
//! `.sh` scripts in the workspace root's `tests/` directory. Each script becomes
//! a separate test case in `cargo test`.
//!
//! ## How it works:
//! - Shell scripts (*.sh) are run with `bash`
//! - All scripts are executed from the workspace root directory
//! - Test names are derived from filenames (e.g., `foo_e2e.sh` → `shell_scripts__foo_e2e`)
//! - Scripts should use `dirname $0` to determine their location and build absolute paths
//!
//! ## Adding new tests:
//! Simply add a new `.sh` file to the workspace root's `tests/` directory.
//! No code changes needed - the test will be automatically discovered!
//!
//! ## Currently discovered scripts:
//! Run `cargo test --test shell_script_tests -- --list` to see all discovered tests.

use dir_test::{dir_test, Fixture};
use std::path::PathBuf;
use std::process::Command;

/// Run a shell script test
fn run_shell_test(fixture: Fixture<&str>) {
    // Get the mtg-engine crate directory (CARGO_MANIFEST_DIR)
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Go up one level to workspace root
    let workspace_root = crate_dir.parent().expect("Failed to find workspace root");
    // Scripts are in workspace_root/tests/
    let script_path = workspace_root.join("tests").join(fixture.path());

    assert!(
        script_path.exists(),
        "Shell script not found: {}",
        script_path.display()
    );

    let output = Command::new("bash")
        .arg(&script_path)
        .current_dir(workspace_root)
        .output()
        .unwrap_or_else(|e| panic!("Failed to execute {}: {}", fixture.path(), e));

    if !output.status.success() {
        eprintln!("--- STDOUT ---");
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("--- STDERR ---");
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        panic!(
            "Shell script {} failed with exit code: {}",
            fixture.path(),
            output.status.code().unwrap_or(-1)
        );
    }
}

// Automatically discover and run all .sh files in workspace root's tests/
// Note: Uses *.sh (not **/*.sh) to avoid running stress tests in subdirectories
#[dir_test(
    dir: "$CARGO_MANIFEST_DIR/../tests",
    glob: "*.sh",
)]
fn shell_scripts(fixture: Fixture<&str>) {
    run_shell_test(fixture);
}
