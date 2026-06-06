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

# Per-player seed-derivation salts — the Python mirror of
# bug_finding/lib/seed_salts.sh, itself the mirror of the canonical Rust source
# mtg-engine/src/game/seed_derivation.rs:
#   const P1_SALT: u64 = 0x1234_5678_9ABC_DEF0;
#   const P2_SALT: u64 = 0xFEDC_BA98_7654_3210;
#   derive_player_seed(master, slot) = master.wrapping_add(SALT)  # wraps at 2^64
#
# Local-vs-network equivalence depends on these matching Rust EXACTLY. A LOCAL
# game (`mtg tui --seed-p1/--seed-p2`) takes the ALREADY-DERIVED per-player
# seeds, whereas a NETWORK client (`mtg connect --seed-player`) takes the master
# controller seed and derives internally. So run_local_game must pre-derive what
# the network side derives, or the two RandomController RNG streams diverge.
_U64_MASK = 0xFFFF_FFFF_FFFF_FFFF
P1_SALT = 0x1234_5678_9ABC_DEF0
P2_SALT = 0xFEDC_BA98_7654_3210


def derive_p1_seed(master: int) -> int:
    return (master + P1_SALT) & _U64_MASK


def derive_p2_seed(master: int) -> int:
    return (master + P2_SALT) & _U64_MASK

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


# Word-boundary ERROR/PANIC matcher. A bare substring test (`'ERROR' in
# line.upper()`) false-matches the card name "Terror" (-> "TERROR") and any word
# containing "error", flagging perfectly-deterministic gamelogs as failures
# (surfaced by the old_school2 corpus, which casts Terror). Matching ERROR/PANIC
# only at word boundaries keeps real `[ERROR ...]` log lines + panics while
# ignoring card text. (See CLAUDE.md "No Hacky String Operations On Structured
# Data".)
_ERROR_LINE_RE = re.compile(r'\b(?:ERROR|PANIC)', re.IGNORECASE)


def extract_error_lines(log_path: str) -> List[str]:
    """Extract last few ERROR/PANIC log lines from a log file."""
    errors = []
    if os.path.exists(log_path):
        with open(log_path, 'r') as f:
            for line in f:
                if _ERROR_LINE_RE.search(line):
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


# ═══════════════════════════════════════════════════════════════════════════
# PERSPECTIVE-AWARE GAMELOG ORACLE (server ↔ client ↔ client)
# ═══════════════════════════════════════════════════════════════════════════
#
# The local↔server comparison above is EXACT: both run with full information,
# so every [GAMELOG] line must be byte-identical. A *client* process, however,
# only has shadow state and legitimately renders some lines differently from
# the full-information server — it MUST NOT see hidden-zone identities. The
# server↔client oracle is therefore *weaker, perspective-aware*:
#
#   - PUBLIC-zone / public events (spells resolving, permanents entering or
#     leaving the battlefield, combat declares, damage dealt to a named
#     target, life totals on public events, public discards-to-graveyard,
#     etc.) MUST be byte-identical across server and every client. A
#     difference here is a real desync / information leak → a FINDING, never
#     silently tolerated.
#   - HIDDEN-zone lines that legitimately differ by perspective are TOLERATED
#     after collapsing the masked span to a canonical placeholder:
#       * per-card draws: the server logs `X draws CardName (id)`; a client
#         that does not (yet) know the identity logs `X draws a card (id)`.
#         (See GameState::draw_card_inner / GameLogger::gamelog_private and
#         the PrivateLogInfo::public_message masking in game/logger.rs.)
#       * a card NAME the client's shadow state cannot resolve, which the
#         client renders as the literal token `Unknown` (the
#         `.unwrap_or("Unknown")` fallback in the renderers) while the server
#         names it — e.g. `... earthbends Zhao (0)` vs `... earthbends Unknown
#         (0)`, or `delayed trigger on Zhao` vs `... on Unknown`. The object
#         *id* is public and still matches; only the masked name differs.
#       * the loss/win line at end of game: the server short-circuits the
#         lethal state-based-action check after the first lethal combat damage
#         (step `CD`, life snapshot at that instant) while a client applies all
#         combat damage before its own SBA check (step `CL`, lower life). The
#         public fact — who lost, who won — is identical; the STEP token and
#         the parenthesised life snapshot are a per-perspective timing artifact.
#
# This is the regex-normalisation fallback the brief allows "where the test
# harness only has rendered text" — the python/shell harness consumes the
# rendered stdout logs, not the in-memory LogEntry stream, so it cannot call
# LogEntry::message_for directly. The normalisation below mirrors exactly what
# message_for / the shadow-state name resolution produce, so a TOLERATED line
# is one the engine's own masking would have produced; anything else FAILS.

# Strict tagged-gamelog prefix: `[GAMELOG TurnN STEP] ...`. Using this (rather
# than a bare `[GAMELOG` substring) drops the client's one-off
# `[INFO ...] Tag gamelogs ENABLED ...` startup line, whose message text
# happens to contain the substring `[GAMELOG]`.
_GAMELOG_PREFIX_RE = re.compile(r'\[GAMELOG Turn\d+ [A-Z0-9]+\]')

# `<who> draws <name> (<id>)` — mask the drawn card NAME, keep who + id.
_DRAW_MASK_RE = re.compile(r'(\bdraws )(.+?)( \(\d+\))')

# Loss/win line: capture the public fact (loser + winner), drop step + life.
_LOSS_LINE_RE = re.compile(
    r'\[GAMELOG Turn\d+ [A-Z0-9]+\]\s*(.+?) has lost the game '
    r'\(life: -?\d+\)\. (.+?) wins!')


def extract_gamelog_perspective(log_path: str) -> List[str]:
    """Extract strictly-tagged [GAMELOG TurnN STEP] lines from a process log.

    Like extract_gamelog() but uses the strict tagged prefix so a client's
    startup `[INFO ...] Tag gamelogs ENABLED` line (which contains the
    substring `[GAMELOG]`) is excluded. Applies the same shared noise filter
    (tap-for-mana / resolves / per-event damage-life lines).
    """
    lines = []
    if not os.path.exists(log_path):
        return lines
    with open(log_path, 'r') as f:
        for line in f:
            clean = re.sub(r'\x1b\[[0-9;]*m', '', line)
            if not _GAMELOG_PREFIX_RE.search(clean):
                continue
            if any(pat.search(clean) for pat in GAMELOG_FILTER_PATTERNS):
                continue
            lines.append(clean.strip())
    return lines


def _normalize_hidden(line: str) -> str:
    """Collapse legitimately-perspective-varying spans to a canonical form.

    Applied to BOTH server and client lines before the (tolerant) compare:
      - drawn card name  -> `<CARD>`   (so `draws Mountain (31)` == `draws a card (31)`)
      - loss/win line     -> step + life snapshot dropped, public fact kept.
    Public-zone content (spell/permanent/combat/damage text) is untouched, so
    any real public divergence still shows up as an inequality.
    """
    m = _LOSS_LINE_RE.search(line)
    if m:
        return f'<LOSS> {m.group(1)} lost; {m.group(2)} wins'
    return _DRAW_MASK_RE.sub(r'\1<CARD>\3', line)


def _tolerable_unknown_diff(server_line: str, client_line: str) -> bool:
    """True iff the ONLY difference is the client rendering a card NAME the
    server resolved, as the literal masked token `Unknown`.

    The client's shadow-state `.unwrap_or("Unknown")` fallback collapses an
    unresolved card name (which may be MULTIPLE words, e.g. "Zhao, Ruthless
    Admiral") to the single token `Unknown`. We therefore treat each client
    `Unknown` token as a wildcard that must match a NON-EMPTY span of server
    text that does NOT itself contain `Unknown`. Everything else must be
    byte-identical. The reverse (server `Unknown` vs a client-named card) is
    never tolerated — that would be a client leaking info the server lacks.
    """
    if 'Unknown' not in client_line or 'Unknown' in server_line:
        return False
    # Build a regex from the client line: literal everywhere except each
    # `Unknown` token becomes a non-greedy "1+ chars, no 'Unknown'" wildcard.
    parts = client_line.split('Unknown')
    if len(parts) < 2:
        return False
    pattern = '^' + r'(?:(?!Unknown).)+?'.join(re.escape(p) for p in parts) + '$'
    return re.match(pattern, server_line) is not None


def compare_gamelogs_perspective(server_lines: List[str],
                                 client_lines: List[str],
                                 client_label: str = "client") -> tuple:
    """Perspective-aware server↔client gamelog comparison.

    PUBLIC-zone lines are compared exactly; hidden-zone lines that the engine's
    own masking (message_for / shadow-state name resolution) would render
    differently per perspective are tolerated. Returns
    (real_divergence_count, diff_sample). A non-zero count is a genuine
    public-zone desync / info-leak finding.
    """
    diff_parts = []
    real_count = 0
    max_len = max(len(server_lines), len(client_lines))
    for i in range(max_len):
        sv = server_lines[i] if i < len(server_lines) else "<missing>"
        cl = client_lines[i] if i < len(client_lines) else "<missing>"
        if sv == cl:
            continue
        # 1) Hidden-zone canonicalisation (draws / loss-win) on both sides.
        if _normalize_hidden(sv) == _normalize_hidden(cl):
            continue
        # 2) Token-level `Unknown` masking (client-masked-only direction).
        if _tolerable_unknown_diff(sv, cl):
            continue
        # Otherwise: a REAL public-zone divergence.
        real_count += 1
        if len(diff_parts) < 8:
            diff_parts.append(
                f"  line {i+1}:\n    SERVER: {sv[:140]}\n    {client_label.upper()}: {cl[:140]}")
    sample = "\n".join(diff_parts)
    if real_count > 8:
        sample += f"\n  ... and {real_count - 8} more real divergences"
    return real_count, sample


def oracle_self_test() -> None:
    """Assert the perspective oracle tolerates hidden-zone masking yet still
    catches a real public-zone divergence. Run as a fast gate before the live
    comparison so a future edit cannot silently neuter the oracle into a
    no-op (the failure mode the brief explicitly warns against). Raises
    AssertionError on regression.
    """
    # Hidden-zone masking is tolerated (draw name, multi-word Unknown, loss line).
    srv = [
        '[GAMELOG Turn3 DR] Ryan draws Mountain (32)',
        '[GAMELOG Turn4 M1] Lightning Bolt (5) deals 3 damage to Gabriel',
        '[GAMELOG Turn5 M1] Cracked Earth Technique (63) earthbends Zhao, Ruthless Admiral (0)',
        '[GAMELOG Turn6 CD] Ryan has lost the game (life: 0). Gabriel wins!',
    ]
    cl_ok = [
        '[GAMELOG Turn3 DR] Ryan draws a card (32)',           # draw masked
        '[GAMELOG Turn4 M1] Lightning Bolt (5) deals 3 damage to Gabriel',  # public, equal
        '[GAMELOG Turn5 M1] Cracked Earth Technique (63) earthbends Unknown (0)',  # name masked
        '[GAMELOG Turn7 CL] Ryan has lost the game (life: -7). Gabriel wins!',  # loss timing
    ]
    n_ok, _ = compare_gamelogs_perspective(srv, cl_ok, "selftest")
    assert n_ok == 0, f"oracle wrongly flagged tolerable masking ({n_ok})"

    # A real public-zone divergence MUST be caught (Bolt 3 -> 4 damage).
    cl_bad = list(cl_ok)
    cl_bad[1] = '[GAMELOG Turn4 M1] Lightning Bolt (5) deals 4 damage to Gabriel'
    n_bad, _ = compare_gamelogs_perspective(srv, cl_bad, "selftest")
    assert n_bad == 1, f"oracle failed to catch public-zone divergence ({n_bad})"

    # Reverse masking (server Unknown, client named) is NEVER tolerated.
    assert not _tolerable_unknown_diff('earthbends Unknown (0)',
                                       'earthbends Zhao (0)'), \
        "oracle tolerated client leaking info the server lacks"


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

    result_holder = [None]

    def browser_thread():
        try:
            with sync_playwright() as p:
                browser = p.chromium.launch(headless=True, args=['--no-sandbox', '--disable-setuid-sandbox'])
                page = browser.new_page()

                # Collect console output incrementally using Playwright's native event loop.
                # IMPORTANT: Do NOT use done_event.wait() here - that blocks the thread
                # and prevents Playwright's background event loop from delivering console
                # messages from async JavaScript code (like WASM initialization).
                # Instead, open the log file and write messages from on_console, then
                # use page.wait_for_function() which keeps Playwright's event loop running.
                with open(output_log_path, 'w', buffering=1) as log_f:
                    def on_console(msg):
                        text = msg.text
                        log_f.write(text + '\n')
                        log_f.flush()

                    page.on("console", on_console)

                    page.goto(harness_url)

                    # Poll for game completion using page.evaluate(), which keeps
                    # Playwright's event loop running so console messages are delivered.
                    # We check window.gameResult every 500ms up to timeout.
                    deadline = time.time() + timeout
                    while time.time() < deadline:
                        try:
                            result_js = page.evaluate("window.gameResult")
                            if result_js is not None:
                                break
                        except Exception:
                            break
                        time.sleep(0.5)

                    result = page.evaluate("window.getGameResult()")
                    result_holder[0] = result

                browser.close()
        except Exception as e:
            with open(output_log_path, 'a') as log_f:
                log_f.write(f"[PLAYWRIGHT_ERROR] {e}\n")

    t = threading.Thread(target=browser_thread, daemon=True)
    t.start()
    t.join(timeout=timeout + 10)

    return result_holder[0]


def run_local_game(config: TestConfig, output_dir: str,
                   timeout: int = 180) -> Optional[int]:
    """Run a LOCAL game (single process, two AIs).

    Returns exit code, or None on timeout.
    Writes output to output_dir/local.log.
    """
    log_path = os.path.join(output_dir, "local.log")
    rayon_env = os.environ.copy()
    rayon_env.setdefault('RAYON_NUM_THREADS', '2')
    proc = subprocess.Popen(
        [MTG_BIN, "tui",
         config.deck1, config.deck2,
         "--p1", config.controller_p1,
         "--p2", config.controller_p2,
         "--p1-name", P1_NAME,
         "--p2-name", P2_NAME,
         "--seed", str(config.seed),
         # LOCAL takes pre-derived per-player seeds; the NETWORK side derives the
         # same values internally from --seed-player. Pre-derive here so the two
         # RandomController RNG streams match (matches
         # tests/network_vs_local_equivalence_e2e.sh's $P{1,2}_DERIVED_SEED).
         "--seed-p1", str(derive_p1_seed(config.seed_p1)),
         "--seed-p2", str(derive_p2_seed(config.seed_p2)),
         "--tag-gamelogs",
         "--verbosity", "normal"],
        stdout=open(log_path, 'w'),
        stderr=subprocess.STDOUT,
        cwd=WORKSPACE_ROOT,
        env=rayon_env
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
                          log_path: str,
                          env: Optional[dict] = None) -> subprocess.Popen:
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
        cwd=WORKSPACE_ROOT,
        env=env
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

    # Limit rayon worker threads to avoid exhausting system thread limits on
    # high-CPU-count machines (rayon defaults to nCPUs which can be 64+).
    rayon_env = os.environ.copy()
    rayon_env.setdefault('RAYON_NUM_THREADS', '2')

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
        cwd=WORKSPACE_ROOT,
        env=rayon_env
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
            proc = _start_native_client(port, deck, controller, seed_player, name, log_path, env=rayon_env)
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

    # The server is a LONG-LIVED multi-game lobby: it intentionally outlives any
    # single game and will NOT exit on its own (see the note in
    # tests/network_vs_local_equivalence_e2e.sh). So we wait for the CLIENTS to
    # finish the game, then shut the server down — NOT the other way round
    # (waiting for the server to exit would always time out on a clean game).
    deadline = time.time() + timeout

    def _remaining() -> int:
        return max(1, int(deadline - time.time()))

    # Wait for native clients to finish the game.
    for proc in [proc1, proc2]:
        if proc is not None:
            try:
                proc.wait(timeout=_remaining())
            except subprocess.TimeoutExpired:
                _kill_procs(server_proc, proc1, proc2)
                for t in [t1, t2]:
                    if t is not None:
                        t.join(timeout=5)
                return None

    # Wait for WASM threads to finish.
    # WASM browser (Playwright) needs time to detect server disconnect, close
    # cleanly, and write its log file. Bound by the remaining game budget (min
    # 45s) so client2.log is always written before we try to read it.
    for t in [t1, t2]:
        if t is not None:
            t.join(timeout=max(45, _remaining()))

    # Clients are done. If the server already exited on its own (e.g. it crashed
    # or hit a fatal desync) surface its exit code; otherwise it is still the
    # idle lobby — shut it down and report success. Game correctness is judged by
    # the gamelog comparison + the error-log scan in the callers, NOT by this
    # exit code.
    server_rc = server_proc.poll()
    if server_rc is None:
        _kill_procs(server_proc)
        return 0
    return server_rc


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

    # --- Perspective-aware server↔client↔client comparison ---
    # Public-zone lines must be identical across server and every client;
    # hidden-zone per-perspective masking is tolerated. A non-zero count here
    # is a genuine public-zone desync / info-leak finding.
    server_persp = extract_gamelog_perspective(os.path.join(output_dir, "server.log"))
    client1_persp = extract_gamelog_perspective(os.path.join(output_dir, "client1.log"))
    client2_persp = extract_gamelog_perspective(os.path.join(output_dir, "client2.log"))

    persp_diff_count = 0
    persp_diff_sample = ""
    if server_persp and client1_persp:
        c1_diff, c1_sample = compare_gamelogs_perspective(
            server_persp, client1_persp, "client1")
        if c1_diff:
            persp_diff_count += c1_diff
            persp_diff_sample += (f"server↔client1: {c1_diff} divergence(s)\n"
                                  + c1_sample + "\n")
    if server_persp and client2_persp:
        c2_diff, c2_sample = compare_gamelogs_perspective(
            server_persp, client2_persp, "client2")
        if c2_diff:
            persp_diff_count += c2_diff
            persp_diff_sample += (f"server↔client2: {c2_diff} divergence(s)\n"
                                  + c2_sample + "\n")
    # Fold the perspective divergence into the headline diff_count so callers
    # that only inspect diff_count still fail on a public-zone client desync.
    diff_count += persp_diff_count
    if persp_diff_sample:
        diff_sample = (diff_sample + "\n" if diff_sample else "") + persp_diff_sample

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


def run_determinism_test(config: TestConfig, timeout: int = 180) -> TestResult:
    """Run the SAME local game twice and assert byte-identical gamelogs.

    The native-determinism invariant: `mtg tui D1 D2 --seed K --tag-gamelogs`
    run twice yields identical [GAMELOG ...] streams. This is the Python
    counterpart of the determinism leg in
    bug_finding/fuzz_determinism_netequiv.sh (which uses the bash shared
    gamelog filter); here we reuse run_local_game + extract_gamelog so the one
    Python CLI can run determinism as a mode without shelling out.

    Both runs use the SAME TestConfig (same seed/decks/controllers); any
    divergence is a real native nondeterminism bug.
    """
    start_time = time.time()
    output_dir = tempfile.mkdtemp(prefix="determinism_fuzz_")
    run_a = os.path.join(output_dir, "run_a")
    run_b = os.path.join(output_dir, "run_b")
    os.makedirs(run_a, exist_ok=True)
    os.makedirs(run_b, exist_ok=True)

    try:
        exit_a = run_local_game(config, run_a, timeout=timeout)
        exit_b = run_local_game(config, run_b, timeout=timeout)
    except Exception as e:
        return TestResult(
            config=config, passed=False,
            duration=time.time() - start_time,
            error_signature=f"determinism_exception:{str(e)[:30]}",
            output_dir=output_dir,
        )

    duration = time.time() - start_time

    if exit_a is None or exit_b is None:
        return TestResult(
            config=config, passed=False, duration=duration,
            error_signature="determinism_timeout", output_dir=output_dir,
        )

    errors_a = extract_error_lines(os.path.join(run_a, "local.log"))
    errors_b = extract_error_lines(os.path.join(run_b, "local.log"))
    gamelog_a = extract_gamelog(os.path.join(run_a, "local.log"))
    gamelog_b = extract_gamelog(os.path.join(run_b, "local.log"))

    diff_count, diff_sample = compare_gamelogs(gamelog_a, gamelog_b)

    passed = (exit_a == 0 and exit_b == 0
              and not errors_a and not errors_b
              and diff_count == 0
              and len(gamelog_a) > 0)

    if passed:
        error_sig = None
    elif diff_count > 0:
        error_sig = f"determinism_divergence({diff_count}_lines)"
    elif exit_a != 0 or exit_b != 0:
        error_sig = f"local_exit_{exit_a}_{exit_b}"
    elif len(gamelog_a) == 0:
        error_sig = "no_gamelog"
    else:
        error_sig = classify_errors([], errors_a, errors_b)

    return TestResult(
        config=config, passed=passed, duration=duration,
        error_signature=error_sig,
        local_errors=errors_a + errors_b,
        gamelog_diff_lines=diff_count,
        gamelog_diff_sample=diff_sample,
        output_dir=output_dir,
    )
