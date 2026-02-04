"""
Shared library for network game testing.

Provides reusable helpers for:
- Running LOCAL games (single process, two AIs)
- Running NETWORK games (server + two client processes)
- Extracting and comparing GAMELOG entries
- Error extraction and classification

Used by network_fuzz_test.py and can be used standalone for one-off tests.
"""

import subprocess
import os
import re
import tempfile
import time
from dataclasses import dataclass, field
from typing import Optional, List

WORKSPACE_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MTG_BIN = os.path.join(WORKSPACE_ROOT, "target/release/mtg")

DECKS = [
    os.path.join(WORKSPACE_ROOT, "decks/booster_draft/avatar/ryan_avatar_draft.dck"),
    os.path.join(WORKSPACE_ROOT, "decks/booster_draft/avatar/gabriel_avatar_draft.dck"),
]

CONTROLLERS = ["heuristic", "random", "zero"]

P1_NAME = "Ryan"
P2_NAME = "Gabriel"

# GAMELOG lines matching these patterns are filtered before comparison.
# These are noisy lines where server/local may format slightly differently.
GAMELOG_FILTER_PATTERNS = [
    re.compile(r'Tap.*for \{'),
    re.compile(r'resolves$'),
    re.compile(r'takes.*damage.*life:'),
    re.compile(r'deals.*damage.*life:'),
]


@dataclass
class TestConfig:
    """Configuration for a single test run."""
    seed: int
    controller_p1: str
    controller_p2: str
    deck1: str
    deck2: str
    seed_p1: int = 3
    seed_p2: int = 3

    def __str__(self):
        return f"seed={self.seed} p1={self.controller_p1} p2={self.controller_p2}"

    def reproducer_command(self) -> str:
        return (f"./tests/network_vs_local_equivalence_e2e.sh "
                f"{self.seed} {self.controller_p1} {self.controller_p2}")


@dataclass
class TestResult:
    """Result of a single test run."""
    config: TestConfig
    passed: bool
    duration: float
    error_signature: Optional[str] = None
    server_errors: List[str] = field(default_factory=list)
    client1_errors: List[str] = field(default_factory=list)
    client2_errors: List[str] = field(default_factory=list)
    local_errors: List[str] = field(default_factory=list)
    gamelog_diff_lines: int = 0
    gamelog_diff_sample: str = ""
    output_dir: Optional[str] = None


def extract_error_lines(log_path: str) -> List[str]:
    """Extract last few ERROR/PANIC lines from a log file."""
    errors = []
    if os.path.exists(log_path):
        with open(log_path, 'r') as f:
            for line in f:
                upper = line.upper()
                if 'ERROR' in upper or 'PANIC' in upper:
                    clean = re.sub(r'\x1b\[[0-9;]*m', '', line)
                    clean = re.sub(r'[^\x20-\x7e]', '', clean)  # ASCII only
                    clean = re.sub(r'^\[.*?\] ', '', clean)
                    clean = re.sub(r'\s+', ' ', clean)
                    errors.append(clean.strip())
    return errors[-3:] if errors else []


def classify_errors(server_errors: List[str], client1_errors: List[str],
                    client2_errors: List[str],
                    local_errors: Optional[List[str]] = None) -> str:
    """Create a signature from errors for bucketing."""
    all_errors = server_errors + client1_errors + client2_errors
    if local_errors:
        all_errors += local_errors
    if not all_errors:
        return "unknown"

    for error in all_errors:
        if 'unexpected OpponentChoice' in error:
            return "unexpected_opponent_choice"
        if 'action_count mismatch' in error:
            return "action_count_mismatch"
        if 'Connection reset' in error:
            return "connection_reset"
        if 'REVEAL VALIDATION FAILED' in error:
            return "reveal_validation"
        if 'Entity not found' in error:
            return "entity_not_found"
        if 'Creature must be on battlefield' in error:
            return "creature_not_on_battlefield"
        if 'Invalid game action' in error:
            return "invalid_game_action"
        if 'panic' in error.lower():
            return "panic"

    return all_errors[0][:50] if all_errors else "unknown"


def extract_gamelog(log_path: str) -> List[str]:
    """Extract filtered GAMELOG entries from a log file.

    Matches the same filtering as network_vs_local_equivalence_e2e.sh:
    - Lines containing [GAMELOG
    - Excluding: tap-for-mana, resolves, damage/life messages
    """
    lines = []
    if not os.path.exists(log_path):
        return lines
    with open(log_path, 'r') as f:
        for line in f:
            # Strip ANSI codes for clean comparison
            clean = re.sub(r'\x1b\[[0-9;]*m', '', line)
            if '[GAMELOG' not in clean:
                continue
            if any(pat.search(clean) for pat in GAMELOG_FILTER_PATTERNS):
                continue
            # Normalize: strip leading whitespace and trailing newline
            lines.append(clean.strip())
    return lines


def compare_gamelogs(local_lines: List[str],
                     network_lines: List[str]) -> tuple:
    """Compare two sets of gamelog lines.

    Returns (diff_count, diff_sample) where diff_sample shows first few diffs.
    """
    if local_lines == network_lines:
        return 0, ""

    # Find first divergence point and count total diffs
    diff_parts = []
    max_len = max(len(local_lines), len(network_lines))
    diff_count = 0
    for i in range(max_len):
        local = local_lines[i] if i < len(local_lines) else "<missing>"
        network = network_lines[i] if i < len(network_lines) else "<missing>"
        if local != network:
            diff_count += 1
            if len(diff_parts) < 5:
                diff_parts.append(f"  line {i+1}:\n    LOCAL:   {local[:120]}\n    NETWORK: {network[:120]}")

    sample = "\n".join(diff_parts)
    if diff_count > 5:
        sample += f"\n  ... and {diff_count - 5} more differences"
    return diff_count, sample


def _kill_procs(*procs):
    """Kill processes, ignoring errors."""
    for p in procs:
        if p is not None:
            try:
                p.kill()
            except OSError:
                pass
            try:
                p.wait(timeout=2)
            except Exception:
                pass


def run_local_game(config: TestConfig, output_dir: str,
                   timeout: int = 180) -> Optional[int]:
    """Run a LOCAL game (single process, two AIs).

    Returns exit code, or None on timeout.
    Writes output to output_dir/local.log.
    """
    log_path = os.path.join(output_dir, "local.log")
    proc = subprocess.Popen(
        [MTG_BIN, "tui",
         config.deck1, config.deck2,
         "--p1", config.controller_p1,
         "--p2", config.controller_p2,
         "--p1-name", P1_NAME,
         "--p2-name", P2_NAME,
         "--seed", str(config.seed),
         "--seed-p1", str(config.seed_p1),
         "--seed-p2", str(config.seed_p2),
         "--tag-gamelogs",
         "--verbosity", "normal"],
        stdout=open(log_path, 'w'),
        stderr=subprocess.STDOUT,
        cwd=WORKSPACE_ROOT
    )
    try:
        proc.wait(timeout=timeout)
        return proc.returncode
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=2)
        return None


def run_network_game(config: TestConfig, output_dir: str,
                     timeout: int = 180) -> Optional[int]:
    """Run a NETWORK game (server + 2 clients).

    Always uses --network-debug for strict reveal validation and full state
    hashing. Returns server exit code, or None on timeout.
    Writes output to output_dir/{server,client1,client2}.log.
    """
    import random as _random
    port = _random.randint(17800, 27800)

    server_log = os.path.join(output_dir, "server.log")
    client1_log = os.path.join(output_dir, "client1.log")
    client2_log = os.path.join(output_dir, "client2.log")

    server_proc = subprocess.Popen(
        [MTG_BIN, "server",
         "--port", str(port),
         "--seed", str(config.seed),
         "--network-debug",
         "--tag-gamelogs",
         "--verbosity", "normal",
         "--no-color-logs"],
        stdout=open(server_log, 'w'),
        stderr=subprocess.STDOUT,
        cwd=WORKSPACE_ROOT
    )

    time.sleep(1.5)

    if server_proc.poll() is not None:
        return server_proc.returncode

    client1_proc = subprocess.Popen(
        [MTG_BIN, "connect",
         config.deck1,
         "--server", f"localhost:{port}",
         "--controller", config.controller_p1,
         "--seed-player", str(config.seed_p1),
         "--name", P1_NAME,
         "--tag-gamelogs"],
        stdout=open(client1_log, 'w'),
        stderr=subprocess.STDOUT,
        cwd=WORKSPACE_ROOT
    )

    time.sleep(0.5)

    client2_proc = subprocess.Popen(
        [MTG_BIN, "connect",
         config.deck2,
         "--server", f"localhost:{port}",
         "--controller", config.controller_p2,
         "--seed-player", str(config.seed_p2),
         "--name", P2_NAME,
         "--tag-gamelogs"],
        stdout=open(client2_log, 'w'),
        stderr=subprocess.STDOUT,
        cwd=WORKSPACE_ROOT
    )

    try:
        server_proc.wait(timeout=timeout)
        client1_proc.wait(timeout=5)
        client2_proc.wait(timeout=5)
        return server_proc.returncode
    except subprocess.TimeoutExpired:
        _kill_procs(server_proc, client1_proc, client2_proc)
        return None


def run_network_test(config: TestConfig, timeout: int = 120) -> TestResult:
    """Run a network-only test (no local equivalence check).

    This is the original fuzz test mode: just run the network game and check
    for errors/crashes.
    """
    start_time = time.time()
    output_dir = tempfile.mkdtemp(prefix="network_fuzz_")

    try:
        exit_code = run_network_game(config, output_dir, timeout=timeout)
    except Exception as e:
        return TestResult(
            config=config, passed=False,
            duration=time.time() - start_time,
            error_signature=f"exception:{str(e)[:30]}",
            output_dir=output_dir
        )

    duration = time.time() - start_time

    if exit_code is None:
        return TestResult(
            config=config, passed=False, duration=duration,
            error_signature="timeout", output_dir=output_dir
        )

    server_errors = extract_error_lines(os.path.join(output_dir, "server.log"))
    client1_errors = extract_error_lines(os.path.join(output_dir, "client1.log"))
    client2_errors = extract_error_lines(os.path.join(output_dir, "client2.log"))

    passed = (exit_code == 0
              and not server_errors
              and not client1_errors
              and not client2_errors)

    error_sig = None if passed else classify_errors(
        server_errors, client1_errors, client2_errors)

    return TestResult(
        config=config, passed=passed, duration=duration,
        error_signature=error_sig,
        server_errors=server_errors,
        client1_errors=client1_errors,
        client2_errors=client2_errors,
        output_dir=output_dir
    )


def run_equivalence_test(config: TestConfig, timeout: int = 180) -> TestResult:
    """Run both LOCAL and NETWORK games, compare gamelogs for equivalence.

    This is the Python port of tests/network_vs_local_equivalence_e2e.sh.
    Both games run sequentially (local first, then network) to avoid port
    contention when running many tests in parallel.
    """
    start_time = time.time()
    output_dir = tempfile.mkdtemp(prefix="equiv_fuzz_")

    # --- Run LOCAL game ---
    try:
        local_exit = run_local_game(config, output_dir, timeout=timeout)
    except Exception as e:
        return TestResult(
            config=config, passed=False,
            duration=time.time() - start_time,
            error_signature=f"local_exception:{str(e)[:30]}",
            output_dir=output_dir
        )

    if local_exit is None:
        return TestResult(
            config=config, passed=False,
            duration=time.time() - start_time,
            error_signature="local_timeout",
            output_dir=output_dir
        )

    # --- Run NETWORK game ---
    try:
        net_exit = run_network_game(config, output_dir, timeout=timeout)
    except Exception as e:
        return TestResult(
            config=config, passed=False,
            duration=time.time() - start_time,
            error_signature=f"network_exception:{str(e)[:30]}",
            output_dir=output_dir
        )

    if net_exit is None:
        return TestResult(
            config=config, passed=False,
            duration=time.time() - start_time,
            error_signature="network_timeout",
            output_dir=output_dir
        )

    duration = time.time() - start_time

    # --- Collect errors ---
    local_errors = extract_error_lines(os.path.join(output_dir, "local.log"))
    server_errors = extract_error_lines(os.path.join(output_dir, "server.log"))
    client1_errors = extract_error_lines(os.path.join(output_dir, "client1.log"))
    client2_errors = extract_error_lines(os.path.join(output_dir, "client2.log"))

    has_errors = bool(local_errors or server_errors
                      or client1_errors or client2_errors)

    # --- Compare gamelogs ---
    local_gamelog = extract_gamelog(os.path.join(output_dir, "local.log"))
    server_gamelog = extract_gamelog(os.path.join(output_dir, "server.log"))

    diff_count, diff_sample = compare_gamelogs(local_gamelog, server_gamelog)

    # Check for panics specifically (not just ERROR lines)
    has_panics = False
    for log_file in ["local.log", "server.log", "client1.log", "client2.log"]:
        path = os.path.join(output_dir, log_file)
        if os.path.exists(path):
            with open(path, 'r') as f:
                content = f.read()
                if re.search(r'thread.*panicked|RUST_BACKTRACE|panicked at|fatal error', content):
                    has_panics = True
                    break

    # Determine pass/fail
    passed = (local_exit == 0
              and net_exit == 0
              and not has_errors
              and not has_panics
              and diff_count == 0
              and len(local_gamelog) > 0
              and len(server_gamelog) > 0)

    # Determine error signature
    if not passed:
        if diff_count > 0:
            error_sig = f"gamelog_divergence({diff_count}_lines)"
        elif has_panics:
            error_sig = "panic"
        elif has_errors:
            error_sig = classify_errors(
                server_errors, client1_errors, client2_errors, local_errors)
        elif local_exit != 0:
            error_sig = f"local_exit_{local_exit}"
        elif net_exit != 0:
            error_sig = f"network_exit_{net_exit}"
        elif len(local_gamelog) == 0:
            error_sig = "no_local_gamelog"
        elif len(server_gamelog) == 0:
            error_sig = "no_server_gamelog"
        else:
            error_sig = "unknown"
    else:
        error_sig = None

    return TestResult(
        config=config, passed=passed, duration=duration,
        error_signature=error_sig,
        server_errors=server_errors,
        client1_errors=client1_errors,
        client2_errors=client2_errors,
        local_errors=local_errors,
        gamelog_diff_lines=diff_count,
        gamelog_diff_sample=diff_sample,
        output_dir=output_dir
    )
