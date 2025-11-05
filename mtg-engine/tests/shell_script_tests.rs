//! Integration tests that automatically discover and run shell scripts
//!
//! This module uses the `dir-test` crate to automatically discover and run all
//! `.sh` scripts in the `tests/` directory. Each script becomes a separate test
//! case in `cargo test`.
//!
//! ## How it works:
//! - Shell scripts (*.sh) are run with `bash`
//! - All scripts are executed from the workspace root directory
//! - Test names are derived from filenames (e.g., `foo_e2e.sh` → `shell_scripts__foo_e2e`)
//!
//! ## Adding new tests:
//! Simply add a new `.sh` file to the `tests/` directory.
//! No code changes needed - the test will be automatically discovered!
//!
//! ## Currently discovered scripts:
//! Run `cargo test --test shell_script_tests -- --list` to see all discovered tests.

use dir_test::{dir_test, Fixture};
use std::path::PathBuf;
use std::process::Command;

/// Run a shell script test
fn run_shell_test(fixture: Fixture<&str>) {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script_path = workspace_root.join("tests").join(fixture.path());

    assert!(
        script_path.exists(),
        "Shell script not found: {}",
        script_path.display()
    );

    let output = Command::new("bash")
        .arg(&script_path)
        .current_dir(&workspace_root)
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

// Automatically discover and run all .sh files in tests/
#[dir_test(
    dir: "$CARGO_MANIFEST_DIR/tests",
    glob: "**/*.sh",
)]
fn shell_scripts(fixture: Fixture<&str>) {
    run_shell_test(fixture);
}
