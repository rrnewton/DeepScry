#!/usr/bin/env python3
"""
Check for conflicting processes that might interfere with validation.

Returns 0 if environment is clean, non-zero if conflicting processes found.

This script checks for:
- MTG binary processes running from subdirectories of current directory
- validate.py (the make-validate entry point) processes for the current directory
- cargo test/build processes for the current directory
- chromium/playwright processes (E2E test remnants)
"""

import os
import sys
import subprocess
import re
from pathlib import Path


def get_current_dir():
    """Get the absolute path of the current working directory."""
    return os.path.abspath(os.getcwd())


def get_processes():
    """Get list of running processes with their command lines."""
    try:
        result = subprocess.run(
            ["ps", "aux"],
            capture_output=True,
            text=True,
            timeout=10
        )
        return result.stdout.splitlines()
    except Exception as e:
        print(f"Warning: Could not get process list: {e}", file=sys.stderr)
        return []


def proc_cwd(pid):
    """Resolve a process's working directory via /proc (Linux).

    Returns the absolute, symlink-resolved cwd string, or None when it cannot be
    determined — the process may have exited mid-scan, or the platform has no
    /proc (e.g. macOS). NEVER raises, so a transient/exited PID or a non-Linux
    host degrades gracefully instead of crashing the precheck.
    """
    try:
        return os.path.realpath(os.readlink(f"/proc/{pid}/cwd"))
    except (OSError, ValueError):
        return None


def _path_within(child, parent):
    """True if `child` is `parent` or a descendant of it.

    Compared with a trailing separator so a sibling worktree like
    `<parent>-other` does NOT match `<parent>`.
    """
    if not child or not parent:
        return False
    parent = parent.rstrip(os.sep)
    return child == parent or child.startswith(parent + os.sep)


def cargo_conflicts_with_worktree(pid, cmd, current_dir):
    """True if a cargo test/nextest/build process is operating WITHIN this
    worktree (`current_dir`).

    Scoping is by the process's REAL working directory (`/proc/<pid>/cwd`), NOT a
    cmdline substring. A `cargo test -p mtg-engine` running in a DIFFERENT
    worktree carries the same `-p mtg-engine` token but a different cwd, and must
    NOT be flagged here — the old global `-p mtg` substring match serialized
    every agent's validate across worktrees. When the cwd is unavailable
    (process exited mid-scan, or no /proc on macOS), FAIL SAFE to the original
    scoped cmdline check (`current_dir in cmd`) — never global, never crash.
    """
    cwd = proc_cwd(pid)
    if cwd is not None:
        return _path_within(cwd, os.path.realpath(current_dir))
    return current_dir in cmd


def is_conflicting_process(proc_line, current_dir):
    """
    Check if a process line represents a conflicting process.

    Returns (is_conflict, description) tuple.
    """
    # Skip header line
    if proc_line.startswith("USER"):
        return False, None

    # Skip our own process
    if "check_clean_environment.py" in proc_line:
        return False, None

    # Parse process info
    parts = proc_line.split(None, 10)
    if len(parts) < 11:
        return False, None

    pid = parts[1]
    cmd = parts[10] if len(parts) > 10 else ""

    # Check for MTG binary from current directory
    if "/mtg" in cmd and current_dir in cmd:
        if "target/release/mtg" in cmd or "target/debug/mtg" in cmd:
            return True, f"MTG binary (PID {pid}): {cmd[:100]}"

    # Check for validate.py (the make-validate entry point, formerly validate.sh)
    # for current directory
    if "validate.py" in cmd and current_dir in cmd:
        return True, f"validate.py (PID {pid}): {cmd[:100]}"

    # Check for cargo commands operating WITHIN this worktree. Scoped by the
    # process's real cwd (/proc/<pid>/cwd) rather than a global `-p mtg`
    # substring, so a cargo run in ANOTHER worktree (same `-p mtg-engine` token,
    # different cwd) no longer false-collides and serializes every agent's
    # validate. Fail-safe (no /proc) keeps the scoped `current_dir in cmd` check.
    if "cargo" in cmd and ("test" in cmd or "nextest" in cmd or "build" in cmd):
        if cargo_conflicts_with_worktree(pid, cmd, current_dir):
            return True, f"cargo (PID {pid}): {cmd[:100]}"

    # Check for chromium/playwright (E2E test remnants)
    if "chromium" in cmd.lower() or "playwright" in cmd.lower():
        # Only flag if it seems related to our tests
        if current_dir in cmd or "localhost" in cmd:
            return True, f"Browser/Playwright (PID {pid}): {cmd[:100]}"

    # Check for python mtg scripts
    if "python" in cmd and "mtg" in cmd and current_dir in cmd:
        return True, f"Python MTG script (PID {pid}): {cmd[:100]}"

    return False, None


def check_lock_file(current_dir):
    """Check if a validate lock file exists."""
    lock_file = Path(current_dir) / ".validate.lock"
    if lock_file.exists():
        try:
            with open(lock_file, "r") as f:
                content = f.read().strip()
            return True, f"Lock file exists: {lock_file} (content: {content[:50]})"
        except Exception:
            return True, f"Lock file exists: {lock_file}"
    return False, None


def main():
    current_dir = get_current_dir()
    print(f"Checking for conflicting processes in: {current_dir}")

    conflicts = []

    # Check for lock file
    has_lock, lock_desc = check_lock_file(current_dir)
    if has_lock:
        conflicts.append(lock_desc)

    # Check running processes
    processes = get_processes()
    for proc_line in processes:
        is_conflict, desc = is_conflicting_process(proc_line, current_dir)
        if is_conflict:
            conflicts.append(desc)

    if conflicts:
        print("\n" + "=" * 60)
        print("ERROR: Found conflicting processes!")
        print("=" * 60)
        for conflict in conflicts:
            print(f"  - {conflict}")
        print("\nTo clean up, run:")
        print(f"  python3 {current_dir}/scripts/kill_zombie_processes.py")
        print("=" * 60)
        return 1
    else:
        print("Environment is clean. No conflicting processes found.")
        return 0


if __name__ == "__main__":
    sys.exit(main())
