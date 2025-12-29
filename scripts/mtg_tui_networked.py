#!/usr/bin/env python3
"""
Network drop-in replacement for `mtg tui`.

This script emulates `mtg tui` by running a local server + two clients,
allowing us to test the networking stack with existing agentplay tests.

Usage:
    ./scripts/mtg_tui_networked.py [mtg tui args...]

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
"""

import argparse
import os
import random
import signal
import subprocess
import sys
import time
from pathlib import Path


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

    # Help
    parser.add_argument('-h', '--help', action='store_true')

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
            # TODO(mtg-037fw): Add --tag-gamelogs-clients flag for 4-way testing

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
