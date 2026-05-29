#!/usr/bin/env python3
"""
Network drop-in replacement for `mtg tui`.

This script emulates `mtg tui` by running a local server + two clients,
allowing us to test the networking stack with existing agentplay tests.

Usage:
    ./scripts/mtg_tui_networked.py [mtg tui args...]
    ./scripts/mtg_tui_networked.py --merge-logs <gamelog_dir>  # Merge logs by timestamp

Environment variables:
    MTG_BINARY: Path to mtg binary (default: ./target/release/mtg)
    MTG_CARDSFOLDER: Path to cardsfolder (default: mtg-engine/cardsfolder)
    RUST_LOG: Passed through to all processes
    MTG_GAMELOG_DIR: Directory to capture per-process gamelogs (for 4-way equivalence)

Supported options:
    - --seed (passed to server for deterministic games)
    - --p1, --p2 (controller types)
    - --seed-p1, --seed-p2 (controller seeds)
    - --verbosity, --visual-stacks, etc.
    - --tag-gamelogs (enables [GAMELOG] tagging on server AND clients)
    - --merge-logs (utility: merge captured logs by timestamp)

Limitations (will error if used):
    - --deck-seed (library ordering not supported)
    - --stop-on-choice (not implemented in network mode)
    - --start-state, --start-from (puzzles/snapshots not supported)
    - --p1-draw, --p2-draw (controlled draws not supported)
    - --save-final-gamestate (not supported)
    - --snapshot-output, --json (snapshots not supported)
    - --stop-when-fixed-exhausted (not supported)
    - --log-tail (not supported in network mode)
    - --debug-state-hash (different mechanism in network mode)

For 4-way gamelog equivalence testing:
    Set MTG_GAMELOG_DIR to capture gamelogs from all 3 processes (server, P1, P2)
    to separate files that can be compared for equivalence.

Log Analysis:
    After running with MTG_GAMELOG_DIR, use --merge-logs to view unified timeline:
        ./scripts/mtg_tui_networked.py --merge-logs /tmp/gamelogs
    This merges server.log, p1.log, p2.log into one timeline ordered by timestamps.
"""

import argparse
import json
import os
import random
import re
import signal
import subprocess
import sys
import time
from pathlib import Path


def merge_logs(gamelog_dir: str, output_file=None):
    """
    Merge server.log, p1.log, p2.log into a unified timeline ordered by action_count/timestamp.

    Each line with a timestamp or action_count is tagged with its source (SERVER/P1/P2)
    and sorted chronologically.

    The protocol includes timestamps in messages like:
    - ChoiceRequest: timestamp_ms, action_count, for_player
    - OpponentChoice: timestamp_ms, action_count, player
    - SubmitChoice: timestamp_ms, action_count

    This function extracts timing information and creates a unified view.
    """
    log_files = {
        'SERVER': os.path.join(gamelog_dir, 'server.log'),
        'P1': os.path.join(gamelog_dir, 'p1.log'),
        'P2': os.path.join(gamelog_dir, 'p2.log'),
    }

    entries = []

    # Regex patterns for extracting timing information
    # Match RUST_LOG style: "2025-01-02T12:34:56.789Z INFO ..."
    rust_log_pattern = re.compile(r'^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z)\s+(.*)$')
    # Match action_count in debug messages
    action_count_pattern = re.compile(r'action_count[=:]?\s*(\d+)', re.IGNORECASE)
    # Match [GAMELOG Turn N PHASE] prefix
    gamelog_pattern = re.compile(r'^\[GAMELOG\s+Turn\s*(\d+)\s+([^\]]+)\](.*)$')

    for source, log_path in log_files.items():
        if not os.path.exists(log_path):
            print(f"Warning: {log_path} not found", file=sys.stderr)
            continue

        with open(log_path, 'r') as f:
            line_num = 0
            for line in f:
                line_num += 1
                line = line.rstrip('\n')
                if not line:
                    continue

                # Try to extract timestamp from RUST_LOG format
                timestamp = None
                rust_match = rust_log_pattern.match(line)
                if rust_match:
                    timestamp = rust_match.group(1)

                # Try to extract action_count
                action_count = None
                ac_match = action_count_pattern.search(line)
                if ac_match:
                    action_count = int(ac_match.group(1))

                # Try to extract turn/phase from [GAMELOG] tags
                turn = None
                phase = None
                gamelog_match = gamelog_pattern.match(line)
                if gamelog_match:
                    turn = int(gamelog_match.group(1))
                    phase = gamelog_match.group(2).strip()

                # Create sort key: (action_count or 0, timestamp or '', line_num)
                sort_key = (
                    action_count if action_count is not None else 0,
                    turn if turn is not None else 0,
                    timestamp if timestamp else '',
                    line_num
                )

                entries.append({
                    'source': source,
                    'line_num': line_num,
                    'line': line,
                    'sort_key': sort_key,
                    'action_count': action_count,
                    'turn': turn,
                    'phase': phase,
                    'timestamp': timestamp,
                })

    # Sort by action_count, then turn, then timestamp, then line number
    entries.sort(key=lambda e: e['sort_key'])

    # Output merged logs
    out = open(output_file, 'w') if output_file else sys.stdout

    try:
        prev_action_count = None
        for entry in entries:
            # Add separator when action_count changes
            if entry['action_count'] is not None and entry['action_count'] != prev_action_count:
                if prev_action_count is not None:
                    print("", file=out)  # Empty line separator
                prev_action_count = entry['action_count']

            # Format: [SOURCE] line
            prefix = f"[{entry['source']:6s}]"
            if entry['action_count'] is not None:
                prefix += f" ac={entry['action_count']:4d}"
            if entry['turn'] is not None:
                prefix += f" T{entry['turn']}"

            print(f"{prefix} {entry['line']}", file=out)
    finally:
        if output_file:
            out.close()

    if output_file:
        print(f"Merged logs written to: {output_file}", file=sys.stderr)
    else:
        print(f"\n--- End of merged logs ({len(entries)} lines) ---", file=sys.stderr)


def find_free_port():
    """Find a free port to use for the server."""
    import socket
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(('', 0))
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        return s.getsockname()[1]


def parse_args():
    """Parse mtg tui arguments and translate to network equivalents."""
    # We need to handle the positional deck arguments specially
    # mtg tui [OPTIONS] [PLAYER1_DECK] [PLAYER2_DECK]

    parser = argparse.ArgumentParser(
        description='Network drop-in for mtg tui',
        add_help=False  # We'll handle --help ourselves
    )

    # Positional args
    parser.add_argument('player1_deck', nargs='?', default=None)
    parser.add_argument('player2_deck', nargs='?', default=None)

    # Supported options
    parser.add_argument('--p1', default='heuristic')
    parser.add_argument('--p2', default='heuristic')
    parser.add_argument('--p1-name', default='Player1')
    parser.add_argument('--p2-name', default='Player2')
    parser.add_argument('--p1-fixed-inputs', default='')
    parser.add_argument('--p2-fixed-inputs', default='')
    parser.add_argument('--seed-p1', default=None)
    parser.add_argument('--seed-p2', default=None)
    parser.add_argument('-v', '--verbosity', default='normal')
    parser.add_argument('--visual-stacks', action='store_true')
    parser.add_argument('--numeric-choices', action='store_true')
    parser.add_argument('--load-all-cards', action='store_true')
    parser.add_argument('--tag-gamelogs', action='store_true')
    parser.add_argument('--network-debug', action='store_true',
                        help='Enable network debug mode (server validates client hashes)')

    # Help
    parser.add_argument('-h', '--help', action='store_true')

    # Utility mode: merge logs
    parser.add_argument('--merge-logs', metavar='DIR',
                        help='Merge server/p1/p2 logs from DIR into unified timeline')
    parser.add_argument('--merge-output', metavar='FILE',
                        help='Output file for merged logs (default: stdout)')

    # Unsupported options (we'll detect and error)
    parser.add_argument('--seed', default=None)
    parser.add_argument('--deck-seed', default=None)
    parser.add_argument('--start-state', default=None)
    parser.add_argument('--start-from', default=None)
    parser.add_argument('--stop-on-choice', default=None)
    parser.add_argument('--p1-draw', default=None)
    parser.add_argument('--p2-draw', default=None)
    parser.add_argument('--save-final-gamestate', default=None)
    parser.add_argument('--snapshot-output', default=None)
    parser.add_argument('--json', action='store_true')
    parser.add_argument('--stop-when-fixed-exhausted', action='store_true')
    parser.add_argument('--log-tail', default=None)
    parser.add_argument('--debug-state-hash', action='store_true')
    parser.add_argument('--screenshot-width', default=None)
    parser.add_argument('--screenshot-height', default=None)

    args = parser.parse_args()

    if args.help:
        print(__doc__)
        print("\nThis is a network drop-in replacement. Supported options:")
        print("  --seed: Game RNG seed (passed to server)")
        print("  --p1, --p2: Controller types (zero, random, heuristic, fixed)")
        print("  --p1-name, --p2-name: Player names")
        print("  --p1-fixed-inputs, --p2-fixed-inputs: Fixed script inputs")
        print("  --seed-p1, --seed-p2: Controller seeds")
        print("  --verbosity: Output verbosity")
        print("  --visual-stacks: Enable visual stacking")
        print("  --tag-gamelogs: Tag game actions with [GAMELOG] prefix (passed to server)")
        print("  --network-debug: Enable network debug mode (validates client state hashes)")
        print("\nUtility mode:")
        print("  --merge-logs DIR: Merge server/p1/p2 logs into unified timeline")
        print("  --merge-output FILE: Output file for merged logs (default: stdout)")
        print("\nExample:")
        print("  MTG_GAMELOG_DIR=/tmp/gamelogs ./scripts/mtg_tui_networked.py deck.dck")
        print("  ./scripts/mtg_tui_networked.py --merge-logs /tmp/gamelogs")
        sys.exit(0)

    # Check for unsupported options
    unsupported = []
    # Note: --seed IS supported (passed to server)
    if args.deck_seed:
        unsupported.append(f'--deck-seed={args.deck_seed}')
    if args.start_state:
        unsupported.append(f'--start-state={args.start_state}')
    if args.start_from:
        unsupported.append(f'--start-from={args.start_from}')
    if args.stop_on_choice:
        unsupported.append(f'--stop-on-choice={args.stop_on_choice}')
    if args.p1_draw:
        unsupported.append(f'--p1-draw={args.p1_draw}')
    if args.p2_draw:
        unsupported.append(f'--p2-draw={args.p2_draw}')
    if args.save_final_gamestate:
        unsupported.append(f'--save-final-gamestate={args.save_final_gamestate}')
    if args.snapshot_output and args.snapshot_output != 'game.snapshot':
        unsupported.append(f'--snapshot-output={args.snapshot_output}')
    if args.json:
        unsupported.append('--json')
    if args.stop_when_fixed_exhausted:
        unsupported.append('--stop-when-fixed-exhausted')
    if args.log_tail:
        unsupported.append(f'--log-tail={args.log_tail}')
    if args.debug_state_hash:
        unsupported.append('--debug-state-hash')

    if unsupported:
        print(f"ERROR: Network mode does not support these options:", file=sys.stderr)
        for opt in unsupported:
            print(f"  {opt}", file=sys.stderr)
        print("\nSet MTG_NETWORK_MODE=0 to use local mode instead.", file=sys.stderr)
        sys.exit(2)  # Exit code 2 = unsupported options

    return args


def main():
    args = parse_args()

    # Handle utility mode: merge logs
    if args.merge_logs:
        if not os.path.isdir(args.merge_logs):
            print(f"ERROR: {args.merge_logs} is not a directory", file=sys.stderr)
            sys.exit(1)
        merge_logs(args.merge_logs, args.merge_output)
        sys.exit(0)

    # Get configuration from environment
    mtg_binary = os.environ.get('MTG_BINARY', './target/release/mtg')
    cardsfolder = os.environ.get('MTG_CARDSFOLDER', 'mtg-engine/cardsfolder')
    rust_log = os.environ.get('RUST_LOG', 'warn')

    # Validate decks
    if not args.player1_deck:
        print("ERROR: At least one deck file is required", file=sys.stderr)
        sys.exit(1)

    p1_deck = args.player1_deck
    p2_deck = args.player2_deck or args.player1_deck  # Default to mirror match

    # Derive per-controller seeds from --seed using the SAME formula as
    # `mtg tui` (see mtg-engine/src/main.rs around line 1556):
    #   p1_seed = master_seed.wrapping_add(0x1234_5678_9ABC_DEF0)
    #   p2_seed = master_seed.wrapping_add(0xFEDC_BA98_7654_3210)
    # Without this, controllers seed from entropy and the network run is not
    # a drop-in replacement for `mtg tui --seed N` — it produces a different
    # (non-deterministic) game log.
    # Explicit --seed-p1 / --seed-p2 flags still take precedence.
    P1_SEED_SALT = 0x1234_5678_9ABC_DEF0
    P2_SEED_SALT = 0xFEDC_BA98_7654_3210
    U64_MASK = 0xFFFF_FFFF_FFFF_FFFF
    if args.seed and args.seed != 'from_entropy':
        try:
            master_seed_u64 = int(args.seed) & U64_MASK
            if args.seed_p1 is None:
                args.seed_p1 = str((master_seed_u64 + P1_SEED_SALT) & U64_MASK)
                print(f"[mtg_tui_networked] Derived P1 controller seed from --seed: {args.seed_p1}")
            if args.seed_p2 is None:
                args.seed_p2 = str((master_seed_u64 + P2_SEED_SALT) & U64_MASK)
                print(f"[mtg_tui_networked] Derived P2 controller seed from --seed: {args.seed_p2}")
        except ValueError:
            print(f"[mtg_tui_networked] WARNING: --seed={args.seed!r} is not a u64; "
                  "cannot derive per-controller seeds. Controllers may use entropy.",
                  file=sys.stderr)

    # Find a free port
    port = find_free_port()
    password = f"test_{random.randint(1000, 9999)}"

    print(f"[mtg_tui_networked] Starting network game on port {port}")
    print(f"[mtg_tui_networked] P1: {args.p1} ({args.p1_name}) deck={p1_deck}")
    print(f"[mtg_tui_networked] P2: {args.p2} ({args.p2_name}) deck={p2_deck}")

    env = os.environ.copy()
    env['RUST_LOG'] = rust_log

    # Start server
    # NOTE: --deck-visibility is REQUIRED for synchronized GameLoop mode
    # Without it, clients don't receive opponent decklists and fall back to
    # using their own deck for both players, causing entity ID mismatches.
    server_cmd = [
        mtg_binary, 'server',
        '--port', str(port),
        '--password', password,
        '--cardsfolder', cardsfolder,
        '--verbosity', args.verbosity,
        '--deck-visibility',  # Required for entity ID synchronization
    ]
    if args.seed:
        server_cmd.extend(['--seed', args.seed])
    if args.tag_gamelogs:
        server_cmd.append('--tag-gamelogs')
    if args.network_debug:
        server_cmd.append('--network-debug')

    print(f"[mtg_tui_networked] Starting server: {' '.join(server_cmd)}")

    # Check for gamelog capture mode (4-way equivalence testing)
    gamelog_dir = os.environ.get('MTG_GAMELOG_DIR')
    gamelog_files = {}
    if gamelog_dir:
        os.makedirs(gamelog_dir, exist_ok=True)
        gamelog_files['server'] = open(os.path.join(gamelog_dir, 'server.log'), 'w')
        gamelog_files['p1'] = open(os.path.join(gamelog_dir, 'p1.log'), 'w')
        gamelog_files['p2'] = open(os.path.join(gamelog_dir, 'p2.log'), 'w')
        print(f"[mtg_tui_networked] Capturing gamelogs to {gamelog_dir}/")

    # If tag_gamelogs is enabled, output server logs so they can be captured
    # Otherwise discard to prevent blocking
    if gamelog_dir:
        server_stdout = gamelog_files['server']
    elif args.tag_gamelogs:
        server_stdout = sys.stdout
    else:
        server_stdout = subprocess.DEVNULL

    server_stderr = sys.stderr if args.tag_gamelogs else subprocess.DEVNULL

    server_proc = subprocess.Popen(
        server_cmd,
        env=env,
        stdout=server_stdout,
        stderr=server_stderr,
    )

    # Wait for server to start
    time.sleep(1.0)

    if server_proc.poll() is not None:
        print("ERROR: Server failed to start", file=sys.stderr)
        sys.exit(1)

    processes = [server_proc]

    def cleanup():
        """Clean up all processes."""
        for proc in processes:
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    proc.kill()

    def signal_handler(signum, frame):
        cleanup()
        sys.exit(128 + signum)

    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)

    try:
        # Build client commands
        # Clients run a synchronized GameLoop that stays in sync with the server
        def build_client_cmd(deck, controller, name, fixed_inputs, seed_player, is_p1):
            cmd = [
                mtg_binary, 'connect',
                '--server', f'localhost:{port}',
                '--password', password,
                '--name', name,
                '--controller', controller,
                '--cardsfolder', cardsfolder,
                '--verbosity', args.verbosity,
                deck,
            ]

            if controller == 'fixed' and fixed_inputs:
                cmd.extend(['--fixed-inputs', fixed_inputs])

            if seed_player:
                cmd.extend(['--seed-player', seed_player])

            if args.visual_stacks:
                cmd.append('--visual-stacks')

            # Note: We don't pass --tag-gamelogs to clients by default because:
            # 1. The 2-way equivalence test (local vs server) only needs server logs
            # 2. Client logs are for 4-way testing which requires separate file outputs
            # TODO(mtg-193): Add --tag-gamelogs-clients flag for 4-way testing

            return cmd

        # Start client 1
        p1_cmd = build_client_cmd(
            p1_deck, args.p1, args.p1_name,
            args.p1_fixed_inputs, args.seed_p1, True
        )
        print(f"[mtg_tui_networked] Starting P1: {' '.join(p1_cmd)}")
        p1_stdout = gamelog_files.get('p1', sys.stdout)
        p1_proc = subprocess.Popen(
            p1_cmd,
            env=env,
            stdout=p1_stdout,
            stderr=sys.stderr,
        )
        processes.append(p1_proc)

        # Small delay between clients
        time.sleep(0.5)

        # Start client 2
        p2_cmd = build_client_cmd(
            p2_deck, args.p2, args.p2_name,
            args.p2_fixed_inputs, args.seed_p2, False
        )
        print(f"[mtg_tui_networked] Starting P2: {' '.join(p2_cmd)}")
        p2_stdout = gamelog_files.get('p2', sys.stdout)
        p2_proc = subprocess.Popen(
            p2_cmd,
            env=env,
            stdout=p2_stdout,
            stderr=sys.stderr,
        )
        processes.append(p2_proc)

        # Wait for both clients to finish
        p1_exit = p1_proc.wait()
        p2_exit = p2_proc.wait()

        print(f"[mtg_tui_networked] P1 exited with code {p1_exit}")
        print(f"[mtg_tui_networked] P2 exited with code {p2_exit}")

        # Return worst exit code
        exit_code = max(p1_exit, p2_exit)

    finally:
        cleanup()
        # Close gamelog files
        for f in gamelog_files.values():
            f.close()

    sys.exit(exit_code)


if __name__ == '__main__':
    main()
