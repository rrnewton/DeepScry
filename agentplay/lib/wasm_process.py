"""Persistent WASM/Playwright `mtg tui` browser process for agentplay.

This is the third backend behind the `GameProcess` interface used by
`agentplay/agent_game.py --driver=...`:

  * `--driver=stop-and-go`  (legacy)              → re-runs `mtg tui` per choice
  * `--driver=persistent`   (default, native)     → one `mtg tui` subprocess
  * `--driver=wasm`         (THIS FILE)           → one headless Chromium tab

The WASM driver loads `web/fancy.html` (or `web/game.html`) in a headless
Chromium, lets the page launch a game session via `launch_game_session(...)`,
and then reads/writes game state via `tui_get_gui_view_model_json()` /
`tui_set_choice_idx()` + `tui_select_choice()` + `tui_run_turn()`.

Both pages expose the same WASM exports — `wasm_process.py` is page-agnostic.
The page parameter only changes the URL we navigate to (and thus what visual
GUI a screenshot captures); the protocol is identical.

Architecture
------------

::

    ┌──────────────────┐  page.evaluate            ┌──────────────────────┐
    │ Python sync      │ ──────────────────────────▶│ Chromium (headless)  │
    │ playwright       │                            │   web/fancy.html     │
    │   wasm_process   │ ◀──── view model JSON ────│   ↓                  │
    └──────────────────┘                            │   pkg/mtg_forge_rs   │
              │                                     │      WASM            │
              │ subprocess.Popen                    └──────────────────────┘
              ▼                                              ▲
    ┌──────────────────┐                                     │ HTTP
    │ python3 -m       │ ────────────────────────────────────┘
    │   http.server    │
    └──────────────────┘

Why a sync Playwright API: the `GameProcess` protocol is request/response
(start → choice → choice → game over). Sync playwright slots in cleanly with
that flow; an async loop would just add ceremony with no benefit.
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import time
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Sequence

from .game_process import ChoicePoint, GameOver
from .text_formatter import (
    strip_menu_prefix,
    view_model_choices,
    view_model_choice_context,
    view_model_is_game_over,
    view_model_log_lines,
    view_model_priority_player,
    view_model_to_state_summary,
    view_model_turn_number,
)


WASM_PAGE_FANCY = "fancy"
WASM_PAGE_GAME = "game"
WASM_PAGES = (WASM_PAGE_FANCY, WASM_PAGE_GAME)

# Default poll interval used while waiting for the WASM session to surface
# the next choice point. Kept short because the WASM TUI runs heuristics
# synchronously in `tui_run_turn` — most "wait for next choice" gaps are
# milliseconds long.
_POLL_INTERVAL_MS = 25
_DEFAULT_TIMEOUT_S = 60.0


@dataclass
class WasmLaunchConfig:
    """Configuration for one Playwright/WASM `agent_play` session.

    Decks are referenced by NAME (matching `web/data/deck_index.json`), not
    by `.dck` path — the WASM build only ships the curated set produced by
    `mtg export-wasm`. The agent_game.py wrapper performs the path → name
    mapping (and surfaces a clear error if the requested deck wasn't
    exported) before constructing this struct.
    """

    p1_deck: str
    p2_deck: str
    p1_controller: str  # "human" / "zero" / "random" / "heuristic"
    p2_controller: str
    seed: int
    page: str = WASM_PAGE_FANCY
    headless: bool = True
    starting_life: int = 20


class WasmPlaywrightProcess:
    """A persistent WASM browser process for one agent game.

    Mirrors the `NativeTuiProcess` lifecycle: `start()` returns the first
    `ChoicePoint | GameOver`, `send_choice(player, text)` resolves the next
    one, `close()` tears down the browser + HTTP server.

    Each call also captures a full-page screenshot to `screenshot_dir` (if
    set) so the user can correlate the agent's chosen action with what the
    GUI looked like at that moment. Filenames follow the
    `choice_NNNN_<player>.png` convention.
    """

    def __init__(
        self,
        *,
        config: WasmLaunchConfig,
        web_dir: Path,
        game_dir: Path,
        screenshot_dir: Path | None = None,
        verbose: bool = False,
        port: int | None = None,
    ) -> None:
        self.config = config
        self.web_dir = web_dir
        self.game_dir = game_dir
        self.screenshot_dir = screenshot_dir
        self.verbose = verbose
        self._port = port

        # Engine snapshot path so downstream tooling can find the latest
        # view-model JSON; matches `NativeTuiProcess.snapshot_path` so the
        # agent_game.py shared bookkeeping works for either driver.
        self.snapshot_path = self.game_dir / "snapshot.json"
        self.transcript_path = self.game_dir / "wasm_transcript.log"

        # Lazy-imported to avoid hard-failing on import for environments
        # without playwright installed (the persistent + stop-and-go drivers
        # don't need it).
        self._pw = None
        self._browser = None
        self._page = None
        self._http_proc: subprocess.Popen[str] | None = None
        self._console_lines: list[str] = []
        self._cumulative_log_lines: list[str] = []
        self._decision_count = 0

    # ------------------------------------------------------------------
    # Public API (matches GameProcess)
    # ------------------------------------------------------------------

    def start(self) -> ChoicePoint | GameOver:
        if self.config.page not in WASM_PAGES:
            raise ValueError(f"unknown WASM page: {self.config.page!r}")

        self.game_dir.mkdir(parents=True, exist_ok=True)
        if self.screenshot_dir is not None:
            self.screenshot_dir.mkdir(parents=True, exist_ok=True)

        self._start_http_server()
        self._launch_browser()
        self._navigate_and_init()
        self._launch_game_session()
        return self._wait_for_next_event()

    def send_choice(self, expected_player: str, choice_text: str) -> ChoicePoint | GameOver:
        if self._page is None:
            raise RuntimeError("send_choice() called before start() or after close()")

        # Resolve the choice text → choice index by inspecting the current
        # view model. The text command convention is what agent_game.py
        # writes to pN_choices.txt across all drivers (parity with the
        # native subprocess), so we have to translate to the WASM TUI's
        # idx-based API ourselves.
        idx = self._resolve_choice_idx(choice_text)
        if self.verbose:
            print(
                f"[wasm] resolving choice text {choice_text!r} → idx {idx}",
                file=sys.stderr,
            )

        # Drive the choice through the WASM exports. Same JS the GUI runs
        # when a human clicks an action: tui_set_choice_idx + tui_select_choice
        # + tui_run_turn (game.html:1882).
        self._page.evaluate(
            """
            async (idx) => {
                const m = await window.__mtgEnsureBridge();
                m.tui_set_choice_idx(idx);
                m.tui_select_choice();
                m.tui_run_turn();
                return true;
            }
            """,
            idx,
        )

        return self._wait_for_next_event()

    def close(self) -> None:
        # Persist the view-model history so we can debug post-mortem.
        with suppress(Exception):
            if self._console_lines:
                self.transcript_path.write_text(
                    "\n".join(self._console_lines) + "\n", encoding="utf-8"
                )

        with suppress(Exception):
            if self._page is not None:
                self._page.close()
        self._page = None

        with suppress(Exception):
            if self._browser is not None:
                self._browser.close()
        self._browser = None

        with suppress(Exception):
            if self._pw is not None:
                self._pw.stop()
        self._pw = None

        if self._http_proc is not None:
            with suppress(Exception):
                self._http_proc.terminate()
                self._http_proc.wait(timeout=5)
            self._http_proc = None

    # ------------------------------------------------------------------
    # Setup helpers
    # ------------------------------------------------------------------

    def _start_http_server(self) -> None:
        if self._port is None:
            self._port = _pick_free_port()
        cmd = ["python3", "-m", "http.server", str(self._port)]
        if self.verbose:
            print(f"[wasm] http server: $ {' '.join(cmd)} (cwd={self.web_dir})", file=sys.stderr)
        self._http_proc = subprocess.Popen(
            cmd,
            cwd=str(self.web_dir),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        # Wait for the server to start accepting connections.
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", self._port), timeout=0.5):
                    return
            except OSError:
                time.sleep(0.1)
        raise RuntimeError(f"WASM driver: http.server on port {self._port} failed to start")

    def _launch_browser(self) -> None:
        # Lazy import so consumers without playwright installed don't break
        # at module import time.
        try:
            from playwright.sync_api import sync_playwright
        except ImportError as exc:
            raise RuntimeError(
                "WASM driver requires the python `playwright` package. "
                "Install with: python3 -m pip install playwright && python3 -m playwright install chromium"
            ) from exc

        self._pw = sync_playwright().start()
        # `--no-sandbox` mirrors what `web/test_fancy_tui.js` uses so this
        # works inside CI containers that don't allow user namespaces.
        self._browser = self._pw.chromium.launch(
            headless=self.config.headless,
            args=["--no-sandbox", "--enable-unsafe-swiftshader"],
        )
        context = self._browser.new_context()
        self._page = context.new_page()
        self._page.on("console", self._on_console)
        self._page.on("pageerror", self._on_pageerror)

    def _on_console(self, msg: Any) -> None:
        try:
            line = f"[browser:{msg.type}] {msg.text}"
        except Exception:
            line = "[browser:?] (unparseable console message)"
        self._console_lines.append(line)
        if self.verbose:
            print(line, file=sys.stderr)

    def _on_pageerror(self, err: Any) -> None:
        line = f"[browser:error] {getattr(err, 'message', err)}"
        self._console_lines.append(line)
        if self.verbose:
            print(line, file=sys.stderr)

    def _navigate_and_init(self) -> None:
        url = f"http://127.0.0.1:{self._port}/{self.config.page}.html"
        if self.verbose:
            print(f"[wasm] navigating to {url}", file=sys.stderr)
        self._page.goto(url, wait_until="networkidle", timeout=60_000)

        # Install a tiny shim on `window` that exposes (or lazily imports)
        # the WASM module bindings every page.evaluate call needs. This
        # avoids requiring fancy.html / game.html to have specific
        # `window.__mtg` exports — we go straight to the JS module.
        #
        # We cache the imported module on `window.__mtgBridge` so subsequent
        # calls don't re-import.
        self._page.evaluate(
            """
            window.__mtgEnsureBridge = async () => {
                if (!window.__mtgBridge) {
                    window.__mtgBridge = await import('/pkg/mtg_forge_rs.js');
                }
                return window.__mtgBridge;
            };
            """
        )
        # Wait for WASM init to finish (the bridge module's `init()` runs
        # at first call). We invoke it eagerly so subsequent waits aren't
        # racing with WASM compilation.
        self._page.evaluate(
            """
            async () => {
                const m = await window.__mtgEnsureBridge();
                if (typeof m.default === 'function') {
                    try { await m.default(); } catch (_) { /* already initialized */ }
                }
                return m.is_ready ? m.is_ready() : true;
            }
            """
        )

    def _launch_game_session(self) -> None:
        cfg = self.config
        # Run the page's own card-database wiring (decks/cards loaders) by
        # calling its main() if needed, then invoke launch_game_session
        # via the bridge.
        #
        # We use `WasmCardDatabase` directly via the bridge module so this
        # works on either page without depending on page-specific globals.
        result = self._page.evaluate(
            """
            async (cfg) => {
                const m = await window.__mtgEnsureBridge();

                // Build a card database and load the WASM decks.bin if needed.
                const cardDb = new m.WasmCardDatabase();
                const resp = await fetch('/data/decks.bin');
                if (!resp.ok) {
                    return { error: `fetch decks.bin failed: ${resp.status}` };
                }
                const bytes = new Uint8Array(await resp.arrayBuffer());
                const deckCount = cardDb.load_decks(bytes);
                const deckNames = JSON.parse(cardDb.get_deck_names_json());
                if (!deckNames.includes(cfg.p1_deck) || !deckNames.includes(cfg.p2_deck)) {
                    return {
                        error: `deck not found in WASM data`,
                        requested_p1: cfg.p1_deck,
                        requested_p2: cfg.p2_deck,
                        available: deckNames,
                    };
                }

                // Load the cards needed for both decks. The WASM card DB
                // requires cards.bin to be loaded before launching a game
                // (so card definitions are resolvable).
                const cardsResp = await fetch('/data/cards.bin');
                if (cardsResp.ok) {
                    const cardsBytes = new Uint8Array(await cardsResp.arrayBuffer());
                    if (typeof cardDb.load_cards === 'function') {
                        cardDb.load_cards(cardsBytes);
                    } else if (typeof cardDb.load_cards_bin === 'function') {
                        cardDb.load_cards_bin(cardsBytes);
                    }
                }

                // Map controller names to the WASM enum.
                const ctrlMap = {
                    'human': m.WasmControllerType.Human,
                    'zero': m.WasmControllerType.Zero,
                    'random': m.WasmControllerType.Random,
                    'heuristic': m.WasmControllerType.Heuristic,
                };
                const p1c = ctrlMap[cfg.p1_controller];
                const p2c = ctrlMap[cfg.p2_controller];
                if (p1c === undefined || p2c === undefined) {
                    return { error: `unknown controller: p1=${cfg.p1_controller} p2=${cfg.p2_controller}` };
                }

                m.launch_game_session(
                    cardDb, cfg.p1_deck, cfg.p2_deck,
                    cfg.starting_life, BigInt(cfg.seed),
                    p1c, p2c,
                );
                // Stash for subsequent calls.
                window.__mtgCardDb = cardDb;
                // Tick once to advance to the first choice point.
                m.tui_run_turn();
                return { ok: true, deck_count: deckCount };
            }
            """,
            {
                "p1_deck": cfg.p1_deck,
                "p2_deck": cfg.p2_deck,
                "p1_controller": cfg.p1_controller,
                "p2_controller": cfg.p2_controller,
                "seed": cfg.seed,
                "starting_life": cfg.starting_life,
            },
        )
        if not isinstance(result, dict) or result.get("error"):
            raise RuntimeError(
                f"WASM driver: launch_game_session failed: {result!r}\n"
                f"Available decks: {result.get('available', '<unknown>') if isinstance(result, dict) else '<n/a>'}"
            )
        if self.verbose:
            print(f"[wasm] launch_game_session ok ({result})", file=sys.stderr)

    # ------------------------------------------------------------------
    # Polling / event loop
    # ------------------------------------------------------------------

    def _wait_for_next_event(self, timeout_s: float = _DEFAULT_TIMEOUT_S) -> ChoicePoint | GameOver:
        """Poll the WASM view model until either there's a pending choice we
        need to resolve OR the game is over."""

        deadline = time.monotonic() + timeout_s
        while True:
            view = self._read_view_model()
            if view_model_is_game_over(view):
                return self._build_game_over(view)
            choices = view_model_choices(view)
            if choices:
                return self._build_choice_point(view, choices)
            if time.monotonic() > deadline:
                raise RuntimeError(
                    f"WASM driver: timed out after {timeout_s:.0f}s waiting for next event "
                    f"(turn={view.get('turn_number')}, step={view.get('current_step')!r})"
                )
            # Game is mid-resolve — kick the engine forward and back off.
            self._page.evaluate(
                """
                async () => {
                    const m = await window.__mtgEnsureBridge();
                    m.tui_run_turn();
                    return true;
                }
                """
            )
            self._page.wait_for_timeout(_POLL_INTERVAL_MS)

    def _read_view_model(self) -> dict[str, Any]:
        try:
            text = self._page.evaluate(
                """
                async () => {
                    const m = await window.__mtgEnsureBridge();
                    return m.tui_get_gui_view_model_json();
                }
                """
            )
        except Exception as exc:
            raise RuntimeError(f"WASM driver: failed to read view model: {exc}") from exc
        if not isinstance(text, str):
            return {}
        try:
            data = json.loads(text)
        except json.JSONDecodeError:
            return {}
        # Mirror what NativeTuiProcess does: persist the raw view model so
        # downstream tooling has a snapshot file to look at.
        with suppress(OSError):
            self.snapshot_path.write_text(text, encoding="utf-8")
        return data

    def _build_choice_point(self, view: dict[str, Any], choices: list[str]) -> ChoicePoint:
        player = view_model_priority_player(view) or "p1"
        turn_number = view_model_turn_number(view)
        choice_context = view_model_choice_context(view)

        # Compute the log delta since the previous decision. Mirrors the
        # incremental dedup the NativeTuiProcess does in `_maybe_record_log_line`.
        all_log_lines = view_model_log_lines(view)
        new_lines = _diff_after(all_log_lines, self._cumulative_log_lines)
        self._cumulative_log_lines = all_log_lines

        # Take a screenshot if requested.
        self._maybe_screenshot(player)

        # The WASM view model already gives us a structured snapshot — we
        # bundle the raw view model as `snapshot` so callers can inspect it,
        # and a precomputed text summary that the prompt builder consumes.
        cp = ChoicePoint(
            player=player,
            choices=choices,
            snapshot=view,
            log_lines=new_lines,
            fresh_output="\n".join(new_lines),
            choice_context=choice_context,
            turn_number=turn_number,
        )
        # Stash the precomputed text summary on the dataclass via the
        # `snapshot` dict so agent_game.py's wasm runner picks it up
        # without needing a new ChoicePoint field.
        cp.snapshot["_state_summary_text"] = view_model_to_state_summary(view)
        self._decision_count += 1
        return cp

    def _build_game_over(self, view: dict[str, Any]) -> GameOver:
        all_log_lines = view_model_log_lines(view)
        new_lines = _diff_after(all_log_lines, self._cumulative_log_lines)
        self._cumulative_log_lines = all_log_lines
        self._maybe_screenshot("game-over")
        return GameOver(
            fresh_output="\n".join(new_lines),
            log_lines=new_lines,
            return_code=0,
            reason="WASM view model reports game_over=true",
        )

    def _maybe_screenshot(self, label: str) -> None:
        if self.screenshot_dir is None or self._page is None:
            return
        filename = f"choice_{self._decision_count:04d}_{label}.png"
        path = self.screenshot_dir / filename
        with suppress(Exception):
            self._page.screenshot(path=str(path), full_page=True)

    def _resolve_choice_idx(self, choice_text: str) -> int:
        """Map the agent_game.py text choice (e.g. "play Mountain", "pass")
        to the WASM TUI's `selected_choice_idx`.

        The WASM TUI puts pass at some position in `choices[]` (often as
        text "pass"); other choices have stable text matching what the
        native menu prints (`format_spell_ability_choice`). We do an
        exact + prefix match to find the right `index` field.
        """

        view = self._read_view_model()
        choices = view.get("choices") or []
        if not isinstance(choices, list) or not choices:
            raise RuntimeError(
                f"WASM driver: no choices available when trying to send {choice_text!r}"
            )

        target = choice_text.strip().lower()
        # The WASM `ChoiceView.text` contains the full menu line with the
        # `[N] ` prefix baked in (e.g. "[0] pass", "[1] play Mountain"). The
        # native CLI driver strips that prefix before recording choices to
        # `pN_choices.txt`, so the agent_game.py text command we're given
        # here is unprefixed (e.g. "pass", "play Mountain"). Strip the
        # prefix on the WASM side before comparing.
        for c in choices:
            if not isinstance(c, dict):
                continue
            text = strip_menu_prefix(c.get("text") or "").lower()
            idx = c.get("index")
            if not isinstance(idx, int):
                continue
            if text == target:
                return idx
        # Fallback: prefix match (handles "cast Lightning Bolt" vs.
        # "cast Lightning Bolt (R)" rendering differences).
        for c in choices:
            if not isinstance(c, dict):
                continue
            text = strip_menu_prefix(c.get("text") or "").lower()
            idx = c.get("index")
            if not isinstance(idx, int):
                continue
            if text.startswith(target) or target.startswith(text):
                return idx
        raise RuntimeError(
            f"WASM driver: could not resolve choice text {choice_text!r}; "
            f"available: {[strip_menu_prefix(c.get('text') or '') for c in choices if isinstance(c, dict)]!r}"
        )


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _pick_free_port() -> int:
    """Bind to port 0 to let the OS hand us a free ephemeral port, then
    release it. There's a TOCTOU race between release-and-rebind, but it's
    short enough for our purposes (test runs, not production)."""

    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _diff_after(current: list[str], previous: list[str]) -> list[str]:
    """Return the suffix of `current` that doesn't overlap with the tail of
    `previous` — used for incremental log delta extraction.

    The view-model log buffer caps at `log_tail_size` (200) and gets new
    entries appended to the end. Two cases:

    * Simple growth (the common case): `previous` is a prefix of `current`,
      so the new lines are just `current[len(previous):]`.
    * Buffer roll-off: `current` no longer starts with the previous list
      (older entries fell off the front). In that case we search inside
      `current` for the longest tail of `previous` that's still present,
      and return everything after that match.

    If even the smallest tail of `previous` doesn't appear in `current`,
    we conservatively return the full `current` list (better to over-report
    new lines than to drop them silently).
    """

    if not previous:
        return list(current)
    # Simple-growth fast path.
    n = len(previous)
    if len(current) >= n and current[:n] == previous:
        return list(current[n:])
    # Roll-off path: try progressively shorter tails of `previous` until we
    # find one that occurs in `current`. We start from the longest plausible
    # overlap (capped at 32 to keep this O(K * len(current)) tiny).
    max_tail = min(len(previous), len(current), 32)
    for tail_len in range(max_tail, 0, -1):
        needle = previous[-tail_len:]
        # Find the LAST occurrence of `needle` in `current` (anything after
        # it is genuinely new).
        for offset in range(len(current) - tail_len, -1, -1):
            if current[offset : offset + tail_len] == needle:
                return list(current[offset + tail_len :])
    # No overlap at all — assume heavy buffer rolloff and emit everything.
    return list(current)


def deck_path_to_wasm_name(path: str | os.PathLike[str]) -> str:
    """Map "decks/foo_bar.dck" → "foo_bar" — the WASM data uses bare deck
    names while agent_game.py CLI consumers always pass `.dck` paths."""

    name = Path(path).stem
    return name
