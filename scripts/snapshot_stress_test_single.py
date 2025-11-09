#!/usr/bin/env python3
"""
Single-deck snapshot/resume stress test.

Runs a snapshot/resume stress test for a single deck with a single controller matchup.
Designed to be called by run_stress_tests.sh for parallel execution.

Usage:
    snapshot_stress_test_single.py <deck_path> <p1_controller> <p2_controller> [--seed SEED] [--replays N]

Examples:
    snapshot_stress_test_single.py decks/royal_assassin.dck heuristic heuristic
    snapshot_stress_test_single.py decks/white_aggro_4ed.dck random heuristic --seed 42 --replays 3
"""

import argparse
import subprocess
import sys
import os
import json
import random
import re
from pathlib import Path
from typing import List, Tuple, Dict, Optional

# ANSI color codes
RED = '\033[0;31m'
GREEN = '\033[0;32m'
YELLOW = '\033[1;33m'
CYAN = '\033[0;36m'
NC = '\033[0m'  # No Color

def print_color(color: str, message: str):
    """Print colored message"""
    print(f"{color}{message}{NC}")

def find_mtg_binary() -> Path:
    """Find the mtg binary (RELEASE ONLY). Error if not found.

    Snapshot stress tests must run against the optimized release build to ensure
    realistic performance and avoid debug-only behavior differences.
    """
    workspace = Path.cwd()
    release_bin = workspace / "target" / "release" / "mtg"

    if release_bin.exists():
        return release_bin

    print_color(RED, "Error: release mtg binary not found at target/release/mtg")
    print("Please build the project first with 'cargo build --release'")
    sys.exit(1)

def extract_choice_from_line(line: str) -> Optional[int]:
    """Extract the choice index from a RANDOM or HEURISTIC log line"""
    if "chose no attackers" in line:
        return 0  # No attackers is choice 0

    # Match "chose N" at the beginning of choice descriptions
    match = re.search(r'chose (\d+)', line)
    if match:
        return int(match.group(1))

    return None

def extract_choices_from_log(log: str) -> Tuple[List[int], List[int]]:
    """Extract P1 and P2 choice sequences from verbose game log."""
    p1_choices = []
    p2_choices = []
    current_player = None

    lines = log.split('\n')
    for line in lines:
        # Track which player's turn it is
        if "Alice's turn" in line or "Alice (active)" in line:
            current_player = 'p1'
        elif "Bob's turn" in line or "Bob (active)" in line:
            current_player = 'p2'

        # Look for choice lines
        if ">>> RANDOM:" in line or ">>> HEURISTIC:" in line or ">>> ZERO:" in line:
            choice = extract_choice_from_line(line)
            if choice is not None:
                if current_player == 'p1':
                    p1_choices.append(choice)
                elif current_player == 'p2':
                    p2_choices.append(choice)

    return p1_choices, p2_choices

def filter_game_actions(log: str) -> List[str]:
    """Filter log to only include meaningful game actions for comparison."""
    actions = []
    lines = log.split('\n')

    for line in lines:
        stripped = line.strip()

        # Skip empty lines
        if not stripped:
            continue

        # Skip snapshot/resume messages
        if "snapshot" in stripped.lower() or "resuming" in stripped.lower():
            continue

        # Skip "Stopping after" messages
        if "Stopping after" in stripped or "stopping" in stripped.lower():
            continue

        # Skip "Turn number:" messages (from snapshot resume)
        if "Turn number:" in stripped:
            continue

        # Skip diagnostic messages like [SUPPRESSED], emoji markers, etc.
        if "[SUPPRESSED]" in stripped or "🔄" in stripped or "📸" in stripped or "✅" in stripped:
            continue

        # Skip state hash debug lines (these are non-deterministic in stop/go tests)
        if "[STATE:" in stripped:
            continue

        # Keep game actions: draws, plays, attacks, damage, etc.
        if any(keyword in stripped for keyword in [
            "draws ",
            "plays ",
            "casts ",
            "attacks ",
            "blocks ",
            "damage ",
            "dies",
            "destroyed",
            "sacrificed",
            "discards",
            "to graveyard",
            "Turn ",
            "wins!",
            "Game Over",
            "Life:",
            "Turns played:"
        ]):
            actions.append(stripped)

    return actions

def run_normal_game(mtg_bin: Path, deck1: str, deck2: str,
                    p1_controller: str, p2_controller: str, seed: int,
                    save_gamestate: Optional[Path] = None, json_format: bool = False) -> Tuple[str, List[int], List[int]]:
    """Run a normal game and extract choices."""
    cmd = [
        str(mtg_bin), "tui",
        deck1, deck2,
        f"--p1={p1_controller}",
        f"--p2={p2_controller}",
        f"--seed={seed}",
        "--verbosity=3",
        "--debug-state-hash"
    ]

    if save_gamestate:
        cmd.append(f"--save-final-gamestate={save_gamestate}")

    if json_format:
        cmd.append("--json")

    result = subprocess.run(cmd, capture_output=True, text=True, timeout=120)

    if result.returncode != 0:
        print_color(RED, f"Normal game failed with code {result.returncode}")
        print("STDOUT:", result.stdout[:2000])
        print("STDERR:", result.stderr[:2000])
        sys.exit(1)

    # Extract choices
    p1_choices, p2_choices = extract_choices_from_log(result.stdout)

    return result.stdout, p1_choices, p2_choices

def run_stop_and_go_game(mtg_bin: Path, deck1: str, deck2: str,
                         p1_controller: str, p2_controller: str,
                         p1_choices: List[int], p2_choices: List[int],
                         seed: int, num_stops: int = 3,
                         save_gamestate: Optional[Path] = None,
                         keep_snapshots: bool = False,
                         snapshot_dir: Optional[Path] = None,
                         test_name: str = "",
                         work_dir: Optional[Path] = None,
                         use_fixed_controllers: bool = False,
                         json_format: bool = False) -> Tuple[str, List[Path]]:
    """Run a stop-and-go game with randomized stop points.

    If use_fixed_controllers is True, uses fixed controllers with the provided choice sequences
    instead of the original p1_controller/p2_controller types.

    Returns: (accumulated_log, list_of_snapshot_paths)
    """
    accumulated_log = ""
    saved_snapshots = []

    # Use unique snapshot file within the work directory
    if work_dir:
        snapshot_file = work_dir / "snapshot.json"
    else:
        # Fallback to temp file if no work_dir provided
        import tempfile
        snapshot_file = Path(tempfile.mktemp(suffix="_snapshot.json"))

    # Calculate stop points
    all_player_choices = []
    all_player_choices.extend(p1_choices)
    all_player_choices.extend(p2_choices)

    total_choices = len(all_player_choices)
    if total_choices == 0:
        # Fall back to running game normally
        cmd = [
            str(mtg_bin), "tui",
            deck1, deck2,
            f"--p1={p1_controller}",
            f"--p2={p2_controller}",
            f"--seed={seed}",
            "--verbosity=3",
            "--debug-state-hash"
        ]
        if json_format:
            cmd.append("--json")
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        return result.stdout, []  # No snapshots in fallback case

    # Generate random stop points
    actual_num_stops = min(num_stops, max(1, total_choices - 1))

    choices_per_stop = max(1, total_choices // (actual_num_stops + 1))
    stop_points = []
    cumulative = 0
    for i in range(actual_num_stops):
        remaining = total_choices - cumulative - (actual_num_stops - i)
        if remaining <= 0:
            break
        advance = random.randint(1, min(choices_per_stop * 2, remaining))
        if advance > 0:
            cumulative += advance
            stop_points.append(advance)

    if not stop_points:
        stop_points = [1]  # At least one stop

    # Determine controller types and fixed inputs
    if use_fixed_controllers:
        actual_p1_controller = "fixed"
        actual_p2_controller = "fixed"
        # Convert choice lists to comma-separated strings
        p1_fixed_inputs = ",".join(map(str, p1_choices))
        p2_fixed_inputs = ",".join(map(str, p2_choices))
    else:
        actual_p1_controller = p1_controller
        actual_p2_controller = p2_controller
        p1_fixed_inputs = None
        p2_fixed_inputs = None

    # Run segments
    for i, stop_after in enumerate(stop_points + [0]):  # 0 = run to completion
        if i == 0:
            # First segment: start from beginning
            cmd = [
                str(mtg_bin), "tui",
                deck1, deck2,
                f"--p1={actual_p1_controller}",
                f"--p2={actual_p2_controller}",
                f"--seed={seed}",
                "--verbosity=3",
                "--debug-state-hash"
            ]

            # Add fixed inputs if using fixed controllers
            if use_fixed_controllers:
                cmd.extend([
                    f"--p1-fixed-inputs={p1_fixed_inputs}",
                    f"--p2-fixed-inputs={p2_fixed_inputs}",
                ])

            if stop_after > 0:
                cmd.extend([
                    f"--stop-every=both:choice:{stop_after}",
                    f"--snapshot-output={snapshot_file}",
                ])

            # Add json flag if requested
            if json_format:
                cmd.append("--json")
        else:
            # Resume from snapshot using 'mtg resume' subcommand
            # This restores controllers from snapshot (including RNG state) for proper determinism
            if not snapshot_file.exists():
                print_color(RED, f"✗ Snapshot file missing at segment {i+1}")
                return "", saved_snapshots

            cmd = [
                str(mtg_bin), "resume",
                str(snapshot_file),
                "--verbosity=3",
                "--debug-state-hash"
            ]

            if stop_after > 0:
                cmd.extend([
                    f"--stop-every=both:choice:{stop_after}",
                    f"--snapshot-output={snapshot_file}",
                ])
            elif save_gamestate:
                # Final segment - save gamestate
                cmd.append(f"--save-final-gamestate={save_gamestate}")

            # Add json flag if requested
            if json_format:
                cmd.append("--json")

        result = subprocess.run(cmd, capture_output=True, text=True, timeout=120)

        if result.returncode != 0:
            print_color(RED, f"✗ Segment {i+1} failed with code {result.returncode}")
            print("STDOUT:", result.stdout[:2000])
            print("STDERR:", result.stderr[:2000])
            return "", saved_snapshots

        accumulated_log += result.stdout

        # Save snapshot if requested and it exists
        if keep_snapshots and snapshot_file.exists() and snapshot_dir:
            snapshot_dir.mkdir(parents=True, exist_ok=True)
            saved_path = snapshot_dir / f"{test_name}_snapshot_{i+1}.json"
            import shutil
            shutil.copy(snapshot_file, saved_path)
            saved_snapshots.append(saved_path)

        # Check if game ended
        if "Game Over" in result.stdout or "wins!" in result.stdout:
            break

    # Cleanup temp snapshot file
    if snapshot_file.exists():
        snapshot_file.unlink()

    return accumulated_log, saved_snapshots

def compare_game_logs_via_tool(normal_log: str, stopgo_log: str, verbose: bool = False,
                                save_logs: bool = False, log_dir: Optional[Path] = None,
                                test_name: str = "") -> Tuple[bool, Optional[Path], Optional[Path]]:
    """Compare game action logs using the diff_logs.py tool.

    Returns: (match_success, normal_log_path, stopgo_log_path)
    """
    # Always filter and save logs to temp or permanent location
    normal_actions = filter_game_actions(normal_log)
    stopgo_actions = filter_game_actions(stopgo_log)

    # Determine where to save logs
    import tempfile
    if save_logs and log_dir:
        log_dir.mkdir(parents=True, exist_ok=True)
        safe_name = test_name.replace(" ", "_").replace("/", "_")
        normal_log_path = log_dir / f"{safe_name}_normal.log"
        stopgo_log_path = log_dir / f"{safe_name}_stopgo.log"
    else:
        # Use temp files for comparison
        normal_log_path = Path(tempfile.mktemp(suffix="_normal.log"))
        stopgo_log_path = Path(tempfile.mktemp(suffix="_stopgo.log"))

    # Write filtered actions to files
    with open(normal_log_path, 'w') as f:
        f.write('\n'.join(normal_actions))
    with open(stopgo_log_path, 'w') as f:
        f.write('\n'.join(stopgo_actions))

    # Call diff_logs.py tool
    script_dir = Path(__file__).parent
    diff_logs_script = script_dir / "diff_logs.py"

    cmd = [sys.executable, str(diff_logs_script), str(normal_log_path), str(stopgo_log_path)]
    if verbose:
        cmd.append("--verbose")

    result = subprocess.run(cmd, capture_output=True, text=True)

    # Print output from diff tool
    if result.stdout:
        print(result.stdout, end='')
    if result.stderr:
        print(result.stderr, end='', file=sys.stderr)

    # Clean up temp files if not saving
    if not save_logs:
        normal_log_path.unlink()
        stopgo_log_path.unlink()
        normal_log_path = None
        stopgo_log_path = None

    # Return code 0 means match
    return result.returncode == 0, normal_log_path, stopgo_log_path

def compare_gamestates_via_tool(normal_state_file: Path, stopgo_state_file: Path, verbose: bool = False) -> bool:
    """Compare GameState files using the diff_gamestate.py tool."""
    script_dir = Path(__file__).parent
    diff_gamestate_script = script_dir / "diff_gamestate.py"

    cmd = [sys.executable, str(diff_gamestate_script), str(normal_state_file), str(stopgo_state_file)]
    if verbose:
        cmd.append("--verbose")

    result = subprocess.run(cmd, capture_output=True, text=True)

    # Print output from diff tool
    if result.stdout:
        print(result.stdout, end='')
    if result.stderr:
        print(result.stderr, end='', file=sys.stderr)

    # Return code 0 means match
    return result.returncode == 0

def run_test_for_deck(mtg_bin: Path, deck_path: str,
                      p1_controller: str, p2_controller: str, seed: int,
                      num_replays: int = 3, verbose: bool = False,
                      keep_artifacts: bool = False, artifact_dir: Optional[Path] = None,
                      switch_fixed: bool = False, json_format: bool = False) -> bool:
    """Run complete test for a specific deck with multiple replay runs.

    If switch_fixed is True, runs the stop-and-go game with fixed controllers
    using the extracted choices from the initial run.
    """
    import tempfile
    import shutil

    # Create a unique temp directory for this test run to avoid conflicts
    work_dir = Path(tempfile.mkdtemp(prefix="mtg_test_", dir="/tmp"))

    try:
        # Create temp files for gamestates within the work directory
        normal_state_file = work_dir / "normal.gamestate"

        # Run normal game and extract choices
        normal_log, p1_choices, p2_choices = run_normal_game(
            mtg_bin, deck_path, deck_path, p1_controller, p2_controller, seed,
            save_gamestate=normal_state_file, json_format=json_format
        )

        # Extract deck name for test naming
        deck_name = Path(deck_path).stem

        # Run multiple stop-and-go games with different random stop points
        all_success = True

        for replay_num in range(num_replays):
            # Use different random seed for stop point generation each replay
            random.seed(seed + replay_num + 1000)

            stopgo_state_file = work_dir / f"stopgo_{replay_num}.gamestate"
            test_name = f"{deck_name}_{p1_controller}v{p2_controller}_seed{seed}_replay{replay_num+1}"

            # Run stop-and-go game with randomized stop points (5 stops)
            stopgo_log, saved_snapshots = run_stop_and_go_game(
                mtg_bin, deck_path, deck_path,
                p1_controller, p2_controller,
                p1_choices, p2_choices, seed, num_stops=5,
                save_gamestate=stopgo_state_file,
                keep_snapshots=keep_artifacts,
                snapshot_dir=artifact_dir,
                test_name=test_name,
                work_dir=work_dir,
                use_fixed_controllers=switch_fixed,
                json_format=json_format
            )

            if not stopgo_log:
                all_success = False
                continue

            # Compare logs using diff_logs.py tool
            log_success, normal_log_path, stopgo_log_path = compare_game_logs_via_tool(
                normal_log, stopgo_log, verbose=verbose,
                save_logs=keep_artifacts, log_dir=artifact_dir, test_name=test_name
            )

            # Compare final gamestates using diff_gamestate.py tool
            gamestate_success = True
            normal_state_saved = None
            stopgo_state_saved = None
            if normal_state_file.exists() and stopgo_state_file.exists():
                gamestate_success = compare_gamestates_via_tool(normal_state_file, stopgo_state_file, verbose=verbose)

                # Save gamestate files if requested
                if keep_artifacts and artifact_dir:
                    artifact_dir.mkdir(parents=True, exist_ok=True)
                    normal_state_saved = artifact_dir / f"{test_name}_normal.gamestate"
                    stopgo_state_saved = artifact_dir / f"{test_name}_stopgo.gamestate"

                    shutil.copy(normal_state_file, normal_state_saved)
                    shutil.copy(stopgo_state_file, stopgo_state_saved)

            # Check if this replay succeeded
            # BOTH logs AND gamestate must match for deterministic correctness
            replay_success = log_success and gamestate_success
            all_success = all_success and replay_success

            if verbose:
                if replay_success:
                    print_color(GREEN, f"  ✓ Replay {replay_num+1}/{num_replays} PASSED")
                else:
                    print_color(RED, f"  ✗ Replay {replay_num+1}/{num_replays} FAILED")
                    if not log_success:
                        print_color(RED, "    - Log comparison failed")
                    if not gamestate_success:
                        print_color(RED, "    - GameState comparison failed")

            # Report saved artifact paths
            if keep_artifacts:
                if not verbose and not replay_success:
                    # Always show paths for failures, even without verbose
                    print_color(CYAN, f"  Saved artifacts for replay {replay_num+1}:")
                    if normal_log_path and stopgo_log_path:
                        print(f"    Logs:       {normal_log_path} / {stopgo_log_path}")
                    if normal_state_saved and stopgo_state_saved:
                        print(f"    GameStates: {normal_state_saved} / {stopgo_state_saved}")
                    if saved_snapshots:
                        print(f"    Snapshots:  {len(saved_snapshots)} files in {artifact_dir}")
                elif verbose:
                    # In verbose mode, show paths for all replays
                    print_color(CYAN, f"  Saved artifacts for replay {replay_num+1}:")
                    if normal_log_path and stopgo_log_path:
                        print(f"    Normal log:  {normal_log_path}")
                        print(f"    Stop-go log: {stopgo_log_path}")
                    if normal_state_saved and stopgo_state_saved:
                        print(f"    Normal state:  {normal_state_saved}")
                        print(f"    Stop-go state: {stopgo_state_saved}")
                    if saved_snapshots:
                        for snap_path in saved_snapshots:
                            print(f"    Snapshot: {snap_path}")

        return all_success

    finally:
        # Always cleanup the unique temp directory
        if work_dir.exists():
            if verbose:
                print_color(CYAN, f"  Cleaning up work directory: {work_dir}")
            shutil.rmtree(work_dir, ignore_errors=True)

def parse_args():
    """Parse command line arguments"""
    parser = argparse.ArgumentParser(
        description='Single-deck snapshot/resume stress test',
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )

    parser.add_argument(
        'deck_path',
        type=str,
        help='Path to the deck file (.dck)'
    )

    parser.add_argument(
        'p1_controller',
        type=str,
        choices=['random', 'heuristic'],
        help='Controller type for player 1'
    )

    parser.add_argument(
        'p2_controller',
        type=str,
        choices=['random', 'heuristic'],
        help='Controller type for player 2'
    )

    parser.add_argument(
        '--seed',
        type=int,
        default=42,
        help='Random seed for the game (default: 42)'
    )

    parser.add_argument(
        '--replays',
        type=int,
        default=3,
        help='Number of stop-and-go replay runs (default: 3)'
    )

    parser.add_argument(
        '--quiet',
        action='store_true',
        help='Suppress output except for errors'
    )

    parser.add_argument(
        '--verbose', '-v',
        action='store_true',
        help='Show detailed comparison output including log differences'
    )

    parser.add_argument(
        '--keep', '--keep-logs',
        action='store_true',
        dest='keep_artifacts',
        help='Save all artifacts (logs, gamestates, snapshots) for inspection (default: artifacts are not saved)'
    )

    parser.add_argument(
        '--artifact-dir',
        type=str,
        default='test_artifacts',
        help='Directory to save artifacts when --keep is used (default: test_artifacts/)'
    )

    parser.add_argument(
        '--switch-fixed',
        action='store_true',
        help='Run stop-go reproducer with fixed controllers using extracted choices from initial run'
    )

    parser.add_argument(
        '--json',
        action='store_true',
        help='Use JSON format for snapshots (default is binary format)'
    )

    return parser.parse_args()


def main():
    args = parse_args()

    # Check for cardsfolder
    if not Path("cardsfolder").exists():
        if not args.quiet:
            print_color(YELLOW, "Warning: cardsfolder not found, skipping test")
        sys.exit(0)

    # Check if deck exists
    if not Path(args.deck_path).exists():
        print_color(RED, f"Error: Deck file not found: {args.deck_path}")
        sys.exit(1)

    # Find binary
    mtg_bin = find_mtg_binary()

    # Extract deck name for display
    deck_name = Path(args.deck_path).stem

    if not args.quiet:
        print(f"{CYAN}Testing: {deck_name} ({args.p1_controller} vs {args.p2_controller}){NC}")

    # Run test
    passed = run_test_for_deck(
        mtg_bin, args.deck_path, args.p1_controller, args.p2_controller,
        args.seed, num_replays=args.replays, verbose=args.verbose,
        keep_artifacts=args.keep_artifacts,
        artifact_dir=Path(args.artifact_dir) if args.keep_artifacts else None,
        switch_fixed=args.switch_fixed,
        json_format=args.json
    )

    if passed:
        if not args.quiet:
            print_color(GREEN, f"✓ PASSED: {deck_name} ({args.p1_controller} vs {args.p2_controller})")
        sys.exit(0)
    else:
        print_color(RED, f"✗ FAILED: {deck_name} ({args.p1_controller} vs {args.p2_controller})")
        sys.exit(1)

if __name__ == "__main__":
    main()
