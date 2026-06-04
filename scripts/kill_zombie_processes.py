#!/usr/bin/env python3
"""
Kill zombie processes that might interfere with validation.

This script kills:
- MTG binary processes running from subdirectories of current directory
- validate.py (the make-validate entry point) processes for the current directory
- cargo test/build processes for the current directory
- chromium/playwright processes (E2E test remnants)
- Removes stale lock files
"""

import os
import sys
import subprocess
import signal
import time
from pathlib import Path


def get_current_dir():
    """Get the absolute path of the current working directory."""
    return os.path.abspath(os.getcwd())


def get_processes():
    """Get list of running processes with their PIDs and command lines."""
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


def should_kill_process(proc_line, current_dir):
    """
    Check if a process should be killed.

    Returns (should_kill, pid, description) tuple.
    """
    # Skip header line
    if proc_line.startswith("USER"):
        return False, None, None

    # Skip our own process
    if "kill_zombie_processes.py" in proc_line:
        return False, None, None

    # Parse process info
    parts = proc_line.split(None, 10)
    if len(parts) < 11:
        return False, None, None

    try:
        pid = int(parts[1])
    except ValueError:
        return False, None, None

    cmd = parts[10] if len(parts) > 10 else ""

    # Skip our own PID
    if pid == os.getpid():
        return False, None, None

    # Check for MTG binary from current directory
    if "/mtg" in cmd and current_dir in cmd:
        if "target/release/mtg" in cmd or "target/debug/mtg" in cmd:
            return True, pid, f"MTG binary: {cmd[:80]}"

    # Check for validate.py (the make-validate entry point, formerly validate.sh)
    # for current directory
    if "validate.py" in cmd and current_dir in cmd:
        return True, pid, f"validate.py: {cmd[:80]}"

    # Check for cargo commands for current directory
    if "cargo" in cmd and ("test" in cmd or "nextest" in cmd):
        if current_dir in cmd:
            return True, pid, f"cargo test: {cmd[:80]}"

    # Check for chromium/playwright (E2E test remnants)
    if "chromium" in cmd.lower() or "playwright" in cmd.lower():
        if current_dir in cmd or "localhost" in cmd:
            return True, pid, f"Browser/Playwright: {cmd[:80]}"

    # Check for python mtg scripts
    if "python" in cmd and "mtg" in cmd and current_dir in cmd:
        return True, pid, f"Python MTG script: {cmd[:80]}"

    # Check for shell test scripts
    if "gamelog_equivalence" in cmd or "shell_script_tests" in cmd:
        if current_dir in cmd:
            return True, pid, f"Test script: {cmd[:80]}"

    return False, None, None


def kill_process(pid, description):
    """Kill a process, first with SIGTERM, then SIGKILL if needed."""
    print(f"  Killing PID {pid}: {description}")
    try:
        os.kill(pid, signal.SIGTERM)
        time.sleep(0.5)
        # Check if still alive
        try:
            os.kill(pid, 0)  # Signal 0 just checks if process exists
            # Still alive, use SIGKILL
            os.kill(pid, signal.SIGKILL)
            print(f"    (used SIGKILL)")
        except ProcessLookupError:
            pass  # Process already dead
        return True
    except ProcessLookupError:
        print(f"    (already dead)")
        return False
    except PermissionError:
        print(f"    (permission denied)")
        return False
    except Exception as e:
        print(f"    (error: {e})")
        return False


def remove_lock_file(current_dir):
    """Remove stale lock file if it exists."""
    lock_file = Path(current_dir) / ".validate.lock"
    if lock_file.exists():
        try:
            lock_file.unlink()
            print(f"  Removed lock file: {lock_file}")
            return True
        except Exception as e:
            print(f"  Failed to remove lock file: {e}")
            return False
    return False


def main():
    current_dir = get_current_dir()
    print(f"Killing zombie processes in: {current_dir}")
    print()

    killed_count = 0
    failed_count = 0

    # Kill processes
    processes = get_processes()
    pids_to_kill = []

    for proc_line in processes:
        should_kill, pid, desc = should_kill_process(proc_line, current_dir)
        if should_kill:
            pids_to_kill.append((pid, desc))

    if pids_to_kill:
        print(f"Found {len(pids_to_kill)} process(es) to kill:")
        for pid, desc in pids_to_kill:
            if kill_process(pid, desc):
                killed_count += 1
            else:
                failed_count += 1
    else:
        print("No zombie processes found.")

    # Remove lock file
    print()
    if remove_lock_file(current_dir):
        print("Lock file removed.")
    else:
        print("No lock file to remove.")

    # Summary
    print()
    print("=" * 40)
    if killed_count > 0 or failed_count > 0:
        print(f"Killed: {killed_count}, Failed: {failed_count}")
    print("Environment cleanup complete.")
    print("=" * 40)

    # Return error if we failed to kill anything
    return 1 if failed_count > 0 else 0


if __name__ == "__main__":
    sys.exit(main())
