"""
Shared library for network game testing.

Provides reusable helpers for:
- Running LOCAL games (single process, two AIs)
- Running NETWORK games (server + two client processes, native or WASM)
- Extracting and comparing GAMELOG entries
- Error extraction and classification

Used by network_fuzz_test.py and can be used standalone for one-off tests.

## WASM Client Support

WASM clients run in a headless Chromium browser (via Playwright) and connect
to the MTG server via WebSocket. Requires Playwright:
  pip install playwright && python3 -m playwright install chromium

The WASM AI harness page is served from `web/wasm_ai_harness.html` with
WASM artifacts from `web/pkg/` and card data from `web/data/`.

Use `client_p1="wasm"` or `client_p2="wasm"` in TestConfig to run a player
as a WASM client. The deck for the WASM player is auto-converted to JSON.
"""

import subprocess
import os
import re
import tempfile
import time
import json
import threading
from dataclasses import dataclass, field
from typing import Optional, List

WORKSPACE_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MTG_BIN = os.path.join(WORKSPACE_ROOT, "target/release/mtg")
WEB_DIR = os.path.join(WORKSPACE_ROOT, "web")

DECKS = [
    os.path.join(WORKSPACE_ROOT, "decks/booster_draft/avatar/ryan_avatar_draft.dck"),
    os.path.join(WORKSPACE_ROOT, "decks/booster_draft/avatar/gabriel_avatar_draft.dck"),
]

# Available controller types for native processes
CONTROLLERS = ["heuristic", "random", "zero"]

# Available client modes
CLIENT_MODES = ["native", "wasm"]

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
    # Client mode: "native" (subprocess) or "wasm" (headless browser)
    client_p1: str = "native"
    client_p2: str = "native"

    def __str__(self):
        parts = [f"seed={self.seed}"]
        p1 = f"{self.client_p1}/{self.controller_p1}"
        p2 = f"{self.client_p2}/{self.controller_p2}"
        parts.append(f"p1={p1} p2={p2}")
        return " ".join(parts)

    def has_wasm_client(self) -> bool:
        return self.client_p1 == "wasm" or self.client_p2 == "wasm"

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


# ═══════════════════════════════════════════════════════════════════════════
# WASM CLIENT HELPERS
# ═══════════════════════════════════════════════════════════════════════════

def parse_deck_file(dck_path: str) -> dict:
    """Parse a .dck file into a DeckSubmission-compatible dict.

    Returns {"main_deck": [["Card Name", count], ...], "sideboard": []}.
    """
    main_deck = []
    in_main = False
    with open(dck_path, 'r') as f:
        for line in f:
            line = line.strip()
            if line.lower() == '[main]':
                in_main = True
            elif line.startswith('['):
                in_main = False
            elif in_main and line:
                # Format: "N Card Name" or "N Card Name (set)"
                parts = line.split(' ', 1)
                if len(parts) == 2:
                    try:
                        count = int(parts[0])
                        name = parts[1].strip()
                        main_deck.append([name, count])
                    except ValueError:
                        pass
    return {"main_deck": main_deck, "sideboard": []}


# Module-level HTTP server for serving WASM files
_wasm_http_server = None
_wasm_http_port = None
_wasm_http_lock = threading.Lock()


def start_wasm_http_server() -> int:
    """Start (or reuse) a local HTTP server serving the web/ directory.

    Returns the port number. Thread-safe.
    """
    global _wasm_http_server, _wasm_http_port

    with _wasm_http_lock:
        if _wasm_http_server is not None:
            return _wasm_http_port

        import http.server
        import socketserver
        import random as _random

        port = _random.randint(19000, 29000)

        class QuietHandler(http.server.SimpleHTTPRequestHandler):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, directory=WEB_DIR, **kwargs)

            def log_message(self, format, *args):
                pass  # Suppress HTTP request logs

        _wasm_http_server = socketserver.TCPServer(("", port), QuietHandler)
        _wasm_http_server.allow_reuse_address = True

        thread = threading.Thread(target=_wasm_http_server.serve_forever, daemon=True)
        thread.start()
        _wasm_http_port = port
        return port


def run_wasm_client(server_port: int, player_name: str, controller: str,
                    seed: int, deck_path: str,
                    output_log_path: str, timeout: int = 180) -> Optional[dict]:
    """Run a WASM AI client in a headless browser using Playwright.

    The browser connects to the MTG game server at ws://localhost:server_port,
    runs the AI game loop, and returns when the game ends.

    Args:
        server_port: Port of the running MTG game server
        player_name: Player display name for authentication
        controller: AI controller type ("random", "heuristic", "zero")
        seed: RNG seed for random controller
        deck_path: Path to the .dck file to submit as deck
        output_log_path: Path to write browser console logs
        timeout: Timeout in seconds

    Returns:
        dict with game result {"winner": int, "choices": int} or None on error.
    """
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        raise RuntimeError(
            "Playwright not installed. Run: pip install playwright && "
            "python3 -m playwright install chromium"
        )

    # Parse the deck file and write deck_submission.json for the harness
    deck_data = parse_deck_file(deck_path)
    deck_submission_path = os.path.join(WEB_DIR, "data", "deck_submission.json")
    deck_submission_path_tmp = deck_submission_path + ".tmp"

    # Write atomically (the HTTP server may be serving simultaneously)
    with open(deck_submission_path_tmp, 'w') as f:
        json.dump(deck_data, f)
    os.replace(deck_submission_path_tmp, deck_submission_path)

    harness_port = start_wasm_http_server()
    server_url = f"ws://localhost:{server_port}"
    harness_url = (f"http://localhost:{harness_port}/wasm_ai_harness.html"
                   f"?server={server_url}&controller={controller}&seed={seed}&name={player_name}")

    console_lines = []
    result_holder = [None]
    done_event = threading.Event()

    def browser_thread():
        try:
            with sync_playwright() as p:
                browser = p.chromium.launch(headless=True)
                page = browser.new_page()

                # Collect console messages
                def on_console(msg):
                    text = msg.text
                    console_lines.append(text)
                    if '[WASM_DONE]' in text:
                        done_event.set()

                page.on("console", on_console)

                page.goto(harness_url)

                # Wait for game completion or timeout
                done_event.wait(timeout=timeout)

                # Get the game result
                result = page.evaluate("window.getGameResult()")
                result_holder[0] = result

                browser.close()
        except Exception as e:
            console_lines.append(f"[PLAYWRIGHT_ERROR] {e}")
            done_event.set()

    t = threading.Thread(target=browser_thread, daemon=True)
    t.start()
    t.join(timeout=timeout + 10)

    # Write console output to log file
    with open(output_log_path, 'w') as f:
        f.write('\n'.join(console_lines))
        f.write('\n')

    return result_holder[0]


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


def _start_native_client(port: int, deck_path: str, controller: str,
                          seed_player: int, name: str,
                          log_path: str) -> subprocess.Popen:
    """Start a native MTG connect process."""
    return subprocess.Popen(
        [MTG_BIN, "connect",
         deck_path,
         "--server", f"localhost:{port}",
         "--controller", controller,
         "--seed-player", str(seed_player),
         "--name", name,
         "--tag-gamelogs"],
        stdout=open(log_path, 'w'),
        stderr=subprocess.STDOUT,
        cwd=WORKSPACE_ROOT
    )


def run_network_game(config: TestConfig, output_dir: str,
                     timeout: int = 180) -> Optional[int]:
    """Run a NETWORK game (server + 2 clients, native or WASM).

    Always uses --network-debug for strict reveal validation and full state
    hashing. Returns server exit code, or None on timeout.
    Writes output to output_dir/{server,client1,client2}.log.

    When config.client_p1 or config.client_p2 is "wasm", that player runs
    as a headless browser WASM client instead of a native subprocess.
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

    # Start clients (native or WASM) in connection order
    wasm_threads = []

    def start_client(deck, controller, seed_player, name, log_path, client_mode):
        """Start a client (native or WASM) and return (proc_or_None, wasm_result_holder)."""
        if client_mode == "wasm":
            result_holder = [None]
            def wasm_runner():
                result_holder[0] = run_wasm_client(
                    server_port=port,
                    player_name=name,
                    controller=controller,
                    seed=seed_player,
                    deck_path=deck,
                    output_log_path=log_path,
                    timeout=timeout,
                )
            t = threading.Thread(target=wasm_runner, daemon=True)
            t.start()
            return None, t, result_holder
        else:
            proc = _start_native_client(port, deck, controller, seed_player, name, log_path)
            return proc, None, None

    proc1, t1, res1 = start_client(
        config.deck1, config.controller_p1, config.seed_p1, P1_NAME,
        client1_log, config.client_p1,
    )

    time.sleep(0.5)

    proc2, t2, res2 = start_client(
        config.deck2, config.controller_p2, config.seed_p2, P2_NAME,
        client2_log, config.client_p2,
    )

    try:
        server_proc.wait(timeout=timeout)

        # Wait for native clients to finish
        for proc in [proc1, proc2]:
            if proc is not None:
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=2)

        # Wait for WASM threads to finish
        for t in [t1, t2]:
            if t is not None:
                t.join(timeout=10)

        return server_proc.returncode
    except subprocess.TimeoutExpired:
        _kill_procs(server_proc, proc1, proc2)
        for t in [t1, t2]:
            if t is not None:
                t.join(timeout=5)
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
