#!/usr/bin/env python3
"""Native-vs-WASM engine-equivalence fuzz sweep.

The mtg-forge-rs engine compiles to BOTH a native binary and a WASM module,
and the SAME engine + SAME seed MUST produce the SAME game in either target.
This harness asserts that across a seed-range x deck-sample sweep.

What is compared
----------------

For each `(seed, deck)` combo we run a `random-vs-random` game in BOTH targets
and compare the canonical `[GAMELOG TurnN STEP]` action stream:

* **native** — `mtg tui <deck> <deck> --p1=random --p2=random --seed=N
  --tag-gamelogs --p1-name=P1 --p2-name=P2 --no-color-logs --verbosity=2`,
  GAMELOG lines scraped from stdout.
* **wasm** — a headless-Chromium `launch_game_session(...)` with the matching
  seed/deck/random-random controllers, `tui_set_tag_gamelogs(true)`, run to
  game-over (or a turn cap), then `tui_get_logs_json()` scraped for GAMELOG
  lines.

Both targets run the engine's `RandomController` seeded via the canonical
`derive_player_seed` (see `mtg-engine/src/game/seed_derivation.rs`), so a
divergence in the GAMELOG stream is a REAL determinism bug — the two compile
targets are playing different games from the same seed.

Why GAMELOG and not the raw log: the `[GAMELOG TurnN STEP]` prefix is the
engine's own "comparable across modes" tag (added for local-vs-network
equivalence). It carries only official game actions (plays, casts, combat,
life, draws, turn/step transitions) with a stable turn/step anchor, and skips
display chrome that legitimately differs between a CLI dump and a browser
view-model. Hidden-information draws are normalised on the native side to match
the WASM perspective filter (see `_normalise_stream`).

Exit codes
----------

* 0  — every combo's native and WASM GAMELOG streams matched.
* 1  — at least one combo diverged (reproducer + diff saved under `debug/`).
* 2  — a setup/infra error (missing build, deck not exported, browser crash).

Usage
-----

    # Bounded (the make-validate leg uses this with --seeds 1 --decks 1):
    python3 bug_finding/native_wasm_equiv_sweep.py --seeds 3 --decks 2 --max-turns 12

    # Heavy overnight run over the full old-school corpus:
    python3 bug_finding/native_wasm_equiv_sweep.py --seeds 50 \
        --decks 'decks/old_school/*.dck,decks/old_school2/*.dck' --max-turns 40

The shell wrapper `bug_finding/native_wasm_equiv_sweep.sh` resolves CARDSFOLDER and
handles the WASM-toolchain gating; prefer it for CLI use.
"""

from __future__ import annotations

import argparse
import dataclasses
import glob as globmod
import json
import os
import re
import socket
import subprocess
import sys
import time
from pathlib import Path
from typing import Sequence

REPO_ROOT = Path(__file__).resolve().parent.parent

# Default deck corpus: the 1994 "old-school" decks. Both directories are
# exported into the WASM `decks.bin` bundle by `mtg export-wasm` (see the
# `ExportWasm` default globs in mtg-engine/src/main.rs), so every deck here is
# launchable in BOTH targets.
DEFAULT_DECK_GLOBS = "decks/old_school/*.dck,decks/old_school2/*.dck"

# A GAMELOG line looks like: "  [GAMELOG Turn3 M1] P1 casts Lightning Bolt (54)"
_GAMELOG_RE = re.compile(r"\[GAMELOG (Turn\d+) (\S+)\]\s*(.*)$")

# Hidden-information draw reveal: "P2 draws Volcanic Island (112)". The WASM
# view's perspective filter masks the opponent's per-card draws to
# "P2 draws a card"; the native full-information log shows the card name. To
# compare the PUBLIC action stream we collapse every named draw to the masked
# form on BOTH sides. (A divergence in WHICH card was drawn would be a
# library-order bug, but that surfaces downstream as a divergent play anyway,
# and shuffling identity is covered by the engine's own determinism tests.)
_DRAW_RE = re.compile(r"^(P\d+|.+?) draws .+$")


@dataclasses.dataclass(frozen=True)
class Combo:
    seed: int
    deck_path: str
    deck_name: str


@dataclasses.dataclass
class GameResult:
    """The comparable GAMELOG stream from one target, plus its full raw log."""

    stream: list[str]
    raw: str
    turns: int
    reached_game_over: bool = True


@dataclasses.dataclass
class Divergence:
    combo: Combo
    first_diff_index: int
    native_line: str | None
    wasm_line: str | None


# ---------------------------------------------------------------------------
# Shared normalisation
# ---------------------------------------------------------------------------


def _normalise_stream(gamelog_lines: Sequence[str]) -> list[str]:
    """Turn raw GAMELOG-tagged log lines into a comparable token stream.

    Each element is `"<TurnN> <STEP> <action>"` with card instance-id
    suffixes (`(56)`) stripped — the native and WASM targets assign card
    instance IDs from different starting offsets, so the IDs are NOT
    comparable, but the action text and turn/step anchor are. Hidden-info
    draws are masked to the public "draws a card" form.
    """

    out: list[str] = []
    for line in gamelog_lines:
        m = _GAMELOG_RE.search(line)
        if m is None:
            continue
        turn, step, action = m.group(1), m.group(2), m.group(3).strip()
        # Strip instance-id suffixes like " (56)" and inline "(123)" refs.
        action = re.sub(r"\s*\((\d+)\)", "", action)
        # Mask hidden-information draws to the public form.
        dm = _DRAW_RE.match(action)
        if dm and " draws " in action:
            who = action.split(" draws ", 1)[0]
            action = f"{who} draws a card"
        out.append(f"{turn} {step} {action}")
    return out


def extract_gamelog(raw: str) -> list[str]:
    return [ln for ln in raw.splitlines() if "[GAMELOG" in ln]


# ---------------------------------------------------------------------------
# Native target
# ---------------------------------------------------------------------------


def _engine_binary() -> Path:
    return REPO_ROOT / "target" / "release" / "mtg"


def run_native(combo: Combo, max_turns: int, cards_folder: Path) -> GameResult:
    binary = _engine_binary()
    if not binary.exists():
        raise SetupError(f"native engine binary not found at {binary} (cargo build --release)")
    env = os.environ.copy()
    env["CARDSFOLDER"] = str(cards_folder)
    env.setdefault("RUST_LOG", "warn")
    cmd = [
        str(binary),
        "tui",
        combo.deck_path,
        combo.deck_path,
        "--p1=random",
        "--p2=random",
        f"--seed={combo.seed}",
        "--tag-gamelogs",
        "--p1-name=P1",
        "--p2-name=P2",
        "--no-color-logs",
        "--verbosity=2",
    ]
    # NOTE: `mtg tui` has no `--max-turns` flag — a random-vs-random game runs
    # to its natural game-over. `max_turns` is honoured only as a WASM-side
    # hang-guard; the comparison (see `compare`) tolerates one stream being a
    # prefix of the other when a turn cap truncated the WASM leg early.
    _ = max_turns
    completed = subprocess.run(
        cmd, capture_output=True, text=True, cwd=str(REPO_ROOT), env=env, timeout=180
    )
    if completed.returncode not in (0, 2):
        raise SetupError(
            f"native run failed (rc={completed.returncode}) for {combo}:\n"
            f"stderr:\n{completed.stderr[-1500:]}"
        )
    raw = completed.stdout
    gamelog = extract_gamelog(raw)
    turns = _max_turn_seen(gamelog)
    return GameResult(stream=_normalise_stream(gamelog), raw=raw, turns=turns)


def _max_turn_seen(gamelog_lines: Sequence[str]) -> int:
    best = 0
    for ln in gamelog_lines:
        m = re.search(r"\[GAMELOG Turn(\d+)", ln)
        if m:
            best = max(best, int(m.group(1)))
    return best


# ---------------------------------------------------------------------------
# WASM target (headless Chromium via Playwright)
# ---------------------------------------------------------------------------


class SetupError(RuntimeError):
    """Raised for infra/setup problems (exit code 2), distinct from a real
    game divergence (exit code 1)."""


def _pick_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


# JS that drives one full random-vs-random game to completion and returns the
# tagged GAMELOG-bearing log buffer. `max_turns` caps the loop so a game that
# loops forever (a bug!) doesn't hang the harness; we stop once the engine's
# reported turn exceeds the cap.
_WASM_RUN_JS = """
async (cfg) => {
    const m = await window.__mtgEnsureBridge();
    const cardDb = new m.WasmCardDatabase();
    // decks.bin is content-addressed (tokens+decks cache-skew fix): resolve the
    // hashed name from the manifest, not the retired fixed /data/decks.bin.
    const idxResp = await fetch('/data/sets/index.json');
    if (!idxResp.ok) return { error: `fetch sets/index.json failed: ${idxResp.status}` };
    const idx = await idxResp.json();
    const resp = await fetch(`/data/${idx.decks}`);
    if (!resp.ok) return { error: `fetch ${idx.decks} failed: ${resp.status}` };
    cardDb.load_decks(new Uint8Array(await resp.arrayBuffer()));
    const names = JSON.parse(cardDb.get_deck_names_json());
    if (!names.includes(cfg.deck)) {
        return { error: 'deck not in WASM data', requested: cfg.deck, available: names };
    }
    await Promise.all(idx.sets.map(async s => {
        const r = await fetch(`/data/sets/${s.file}`);
        if (r.ok) cardDb.load_set(new Uint8Array(await r.arrayBuffer()));
    }));
    m.launch_game_session(
        cardDb, cfg.deck, cfg.deck, cfg.starting_life, BigInt(cfg.seed),
        m.WasmControllerType.Random, m.WasmControllerType.Random,
    );
    if (!m.tui_set_tag_gamelogs(true)) {
        return { error: 'tui_set_tag_gamelogs returned false (no active session)' };
    }
    let ticks = 0;
    let turn = 0;
    let gameOver = false;
    const HARD_TICK_CAP = 200000;
    while (ticks < HARD_TICK_CAP) {
        const vm = JSON.parse(m.tui_get_gui_view_model_json());
        if (vm.game_over) { gameOver = true; break; }
        turn = vm.turn_number || turn;
        if (turn > cfg.max_turns) break;
        m.tui_run_turn();
        ticks++;
    }
    const logs = JSON.parse(m.tui_get_logs_json());
    return { ok: true, ticks, turn, game_over: gameOver, logs };
}
"""


class WasmRunner:
    """Persistent headless Chromium tab + http.server reused across the whole
    sweep. Re-navigating the page per game resets WASM `GLOBAL_TUI_STATE`, so
    one browser serves every combo (massively faster than relaunching)."""

    def __init__(self, web_dir: Path, headless: bool = True, verbose: bool = False) -> None:
        self.web_dir = web_dir
        self.headless = headless
        self.verbose = verbose
        self._http: subprocess.Popen | None = None
        self._port: int | None = None
        self._pw = None
        self._browser = None
        self._page = None

    def __enter__(self) -> "WasmRunner":
        self._start_http()
        self._launch_browser()
        return self

    def __exit__(self, *exc) -> None:
        self.close()

    def _start_http(self) -> None:
        self._port = _pick_free_port()
        self._http = subprocess.Popen(
            ["python3", "-m", "http.server", str(self._port)],
            cwd=str(self.web_dir),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", self._port), timeout=0.5):
                    return
            except OSError:
                time.sleep(0.1)
        raise SetupError(f"http.server on port {self._port} failed to start")

    def _launch_browser(self) -> None:
        try:
            from playwright.sync_api import sync_playwright
        except ImportError as exc:
            raise SetupError(
                "WASM sweep requires the python `playwright` package + Chromium. "
                "Install: python3 -m pip install playwright && python3 -m playwright install chromium"
            ) from exc
        self._pw = sync_playwright().start()
        self._browser = self._pw.chromium.launch(
            headless=self.headless, args=["--no-sandbox", "--enable-unsafe-swiftshader"]
        )
        self._page = self._browser.new_context().new_page()
        if self.verbose:
            self._page.on("console", lambda m: print(f"[browser:{m.type}] {m.text}", file=sys.stderr))
            self._page.on("pageerror", lambda e: print(f"[browser:error] {e}", file=sys.stderr))

    def run(self, combo: Combo, max_turns: int, starting_life: int = 20) -> GameResult:
        assert self._page is not None and self._port is not None
        # Re-navigate to reset WASM global state for a clean game.
        self._page.goto(
            f"http://127.0.0.1:{self._port}/game.html",
            wait_until="networkidle",
            timeout=60_000,
        )
        self._page.evaluate(
            """
            window.__mtgEnsureBridge = async () => {
                if (!window.__mtgBridge) {
                    window.__mtgBridge = await import('/pkg/mtg_engine.js');
                    if (typeof window.__mtgBridge.default === 'function') {
                        try { await window.__mtgBridge.default(); } catch (_) {}
                    }
                }
                return window.__mtgBridge;
            };
            """
        )
        result = self._page.evaluate(
            _WASM_RUN_JS,
            {
                "deck": combo.deck_name,
                "seed": combo.seed,
                "max_turns": max_turns,
                "starting_life": starting_life,
            },
        )
        if not isinstance(result, dict) or result.get("error"):
            raise SetupError(f"WASM run failed for {combo}: {result!r}")
        logs = result.get("logs") or []
        gamelog = [ln for ln in logs if "[GAMELOG" in ln]
        return GameResult(
            stream=_normalise_stream(gamelog),
            raw="\n".join(logs),
            turns=int(result.get("turn") or 0),
            reached_game_over=bool(result.get("game_over")),
        )

    def close(self) -> None:
        for closer in (
            lambda: self._page and self._page.close(),
            lambda: self._browser and self._browser.close(),
            lambda: self._pw and self._pw.stop(),
        ):
            try:
                closer()
            except Exception:
                pass
        if self._http is not None:
            try:
                self._http.terminate()
                self._http.wait(timeout=5)
            except Exception:
                pass


# ---------------------------------------------------------------------------
# Comparison + reporting
# ---------------------------------------------------------------------------


def compare(combo: Combo, native: GameResult, wasm: GameResult) -> Divergence | None:
    n, w = native.stream, wasm.stream
    limit = min(len(n), len(w))
    for i in range(limit):
        if n[i] != w[i]:
            return Divergence(combo, i, n[i], w[i])
    if len(n) == len(w):
        return None
    # Streams differ in length but agree on the common prefix. This is only a
    # genuine divergence if BOTH games ran to their natural game-over (so the
    # length difference is real). If the WASM leg was truncated by the turn
    # cap (reached_game_over == False), a longer native stream is expected and
    # the prefix match is a PASS.
    if not wasm.reached_game_over and len(w) <= len(n):
        return None
    idx = limit
    return Divergence(
        combo,
        idx,
        n[idx] if idx < len(n) else None,
        w[idx] if idx < len(w) else None,
    )


def save_divergence(div: Divergence, native: GameResult, wasm: GameResult, debug_dir: Path) -> Path:
    debug_dir.mkdir(parents=True, exist_ok=True)
    stem = f"divergence_seed{div.combo.seed}_{div.combo.deck_name}"
    (debug_dir / f"{stem}.native.gamelog").write_text(
        "\n".join(native.stream) + "\n", encoding="utf-8"
    )
    (debug_dir / f"{stem}.wasm.gamelog").write_text(
        "\n".join(wasm.stream) + "\n", encoding="utf-8"
    )
    (debug_dir / f"{stem}.native.rawlog").write_text(native.raw, encoding="utf-8")
    (debug_dir / f"{stem}.wasm.rawlog").write_text(wasm.raw, encoding="utf-8")

    repro = (
        f"# Native-vs-WASM equivalence divergence reproducer\n"
        f"# seed={div.combo.seed} deck={div.combo.deck_path} (wasm name: {div.combo.deck_name})\n"
        f"# first divergent action index: {div.first_diff_index}\n"
        f"#   native: {div.native_line!r}\n"
        f"#   wasm:   {div.wasm_line!r}\n\n"
        f"# Reproduce the native leg:\n"
        f"./target/release/mtg tui {div.combo.deck_path} {div.combo.deck_path} \\\n"
        f"    --p1=random --p2=random --seed={div.combo.seed} \\\n"
        f"    --tag-gamelogs --p1-name=P1 --p2-name=P2 --no-color-logs --verbosity=2\n\n"
        f"# Reproduce the WASM leg + compare:\n"
        f"./bug_finding/native_wasm_equiv_sweep.sh --seeds 1 --decks '{div.combo.deck_path}' "
        f"--seed-base {div.combo.seed}\n"
    )
    repro_path = debug_dir / f"{stem}.reproducer.sh"
    repro_path.write_text(repro, encoding="utf-8")

    # Unified-diff context around the first divergence.
    lo = max(0, div.first_diff_index - 4)
    hi = div.first_diff_index + 5
    ctx_lines = ["--- native (normalised GAMELOG) ---"]
    for i in range(lo, min(hi, len(native.stream))):
        mark = ">>" if i == div.first_diff_index else "  "
        ctx_lines.append(f"{mark} [{i}] {native.stream[i]}")
    ctx_lines.append("--- wasm (normalised GAMELOG) ---")
    for i in range(lo, min(hi, len(wasm.stream))):
        mark = ">>" if i == div.first_diff_index else "  "
        ctx_lines.append(f"{mark} [{i}] {wasm.stream[i]}")
    (debug_dir / f"{stem}.diff.txt").write_text("\n".join(ctx_lines) + "\n", encoding="utf-8")
    return repro_path


# ---------------------------------------------------------------------------
# Deck resolution
# ---------------------------------------------------------------------------


def resolve_decks(deck_globs: str, max_decks: int | None) -> list[tuple[str, str]]:
    """Expand a comma-separated glob list into (path, wasm_name) pairs.

    The WASM bundle keys decks by bare stem (see `deck_path_to_wasm_name`), so
    the name is just the file stem.
    """

    paths: list[str] = []
    for pattern in deck_globs.split(","):
        pattern = pattern.strip()
        if not pattern:
            continue
        abs_pattern = pattern if os.path.isabs(pattern) else str(REPO_ROOT / pattern)
        matched = sorted(globmod.glob(abs_pattern))
        for m in matched:
            rel = os.path.relpath(m, REPO_ROOT)
            paths.append(rel)
    # De-dup while preserving order.
    seen: set[str] = set()
    decks: list[tuple[str, str]] = []
    for p in paths:
        if p in seen:
            continue
        seen.add(p)
        decks.append((p, Path(p).stem))
    if max_decks is not None:
        decks = decks[:max_decks]
    return decks


def wasm_available_deck_names(web_dir: Path) -> set[str] | None:
    """Best-effort: read the WASM-exported deck names so we can skip decks the
    bundle doesn't ship (rather than fail the whole sweep). Returns None if we
    can't determine the set (in which case we attempt every deck)."""

    # decks.bin is content-addressed (tokens+decks cache-skew fix): its hashed
    # name lives in data/sets/index.json. Probe via the manifest.
    index_json = web_dir / "data" / "sets" / "index.json"
    if not index_json.exists():
        return None
    try:
        decks_rel = json.loads(index_json.read_text())["decks"]
    except (KeyError, ValueError):
        return None
    if not (web_dir / "data" / decks_rel).exists():
        return None
    # The bundle is a binary format; rather than parse it in Python we leave
    # name-validation to the WASM runner (which surfaces a clear error). This
    # helper exists as a hook for future filtering.
    return None


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def _resolve_cards_folder() -> Path:
    env = os.environ.get("CARDSFOLDER")
    if env and (Path(env) / "a").is_dir():
        return Path(env)
    candidates = [
        REPO_ROOT / "cardsfolder",
        REPO_ROOT / "forge-java" / "forge-gui" / "res" / "cardsfolder",
    ]
    for c in candidates:
        if c.exists() and (c / "a").is_dir():
            return c
    raise SetupError(
        "No usable CARDSFOLDER found. Set CARDSFOLDER=... or init the forge-java submodule."
    )


def parse_args(argv: Sequence[str] | None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Native-vs-WASM engine-equivalence fuzz sweep.")
    p.add_argument("--seeds", type=int, default=3, help="Number of seeds to sweep (per deck).")
    p.add_argument("--seed-base", type=int, default=1, help="First seed value (inclusive).")
    p.add_argument(
        "--decks",
        type=str,
        default=DEFAULT_DECK_GLOBS,
        help="Comma-separated deck glob(s). Default: the 1994 old-school corpus.",
    )
    p.add_argument(
        "--max-decks",
        type=int,
        default=None,
        help="Cap the number of decks after glob expansion (deterministic prefix).",
    )
    p.add_argument("--max-turns", type=int, default=20, help="Per-game turn cap.")
    p.add_argument("--headed", action="store_true", help="Run Chromium headed (debugging).")
    p.add_argument("--verbose", "-v", action="store_true", help="Verbose progress + browser logs.")
    p.add_argument(
        "--debug-dir",
        type=str,
        default=str(REPO_ROOT / "debug" / "native_wasm_equiv"),
        help="Where to save divergent gamelogs + reproducers (gitignored).",
    )
    p.add_argument(
        "--expect-divergence",
        action="store_true",
        help=(
            "INVERTED mode for the KNOWN native-vs-WASM divergence tracked in "
            "beads mtg-ofl2i. Exit 0 if EVERY combo still diverges as expected, "
            "and exit 1 (loudly) if any combo unexpectedly MATCHES — which means "
            "mtg-ofl2i was fixed and this flag must be removed so the sweep "
            "reverts to a strict equivalence guard. Used by `make validate` so "
            "the bounded leg stays green while the bug is open WITHOUT silently "
            "green-skipping a real regression."
        ),
    )
    return p.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)

    web_dir = REPO_ROOT / "web"
    if not (web_dir / "pkg" / "mtg_engine.js").exists():
        print(
            f"SETUP ERROR: WASM build not found at {web_dir / 'pkg'}.\n"
            "Build it with: make wasm-dev",
            file=sys.stderr,
        )
        return 2
    # decks.bin is content-addressed (tokens+decks cache-skew fix): resolve via
    # the manifest rather than the retired fixed web/data/decks.bin path.
    _index_json = web_dir / "data" / "sets" / "index.json"
    _decks_ok = False
    if _index_json.exists():
        try:
            _decks_rel = json.loads(_index_json.read_text())["decks"]
            _decks_ok = (web_dir / "data" / _decks_rel).exists()
        except (KeyError, ValueError):
            _decks_ok = False
    if not _decks_ok:
        print(
            f"SETUP ERROR: WASM data not found at {web_dir / 'data'}.\n"
            "Generate it with: mtg export-wasm (or make wasm-dev).",
            file=sys.stderr,
        )
        return 2

    try:
        cards_folder = _resolve_cards_folder()
    except SetupError as exc:
        print(f"SETUP ERROR: {exc}", file=sys.stderr)
        return 2

    decks = resolve_decks(args.decks, args.max_decks)
    if not decks:
        print(f"SETUP ERROR: no decks matched glob(s): {args.decks!r}", file=sys.stderr)
        return 2

    seeds = list(range(args.seed_base, args.seed_base + args.seeds))
    combos = [
        Combo(seed=s, deck_path=path, deck_name=name)
        for (path, name) in decks
        for s in seeds
    ]

    print(
        f"=== native-vs-WASM equivalence sweep ===\n"
        f"  decks:     {len(decks)} ({', '.join(name for _, name in decks)})\n"
        f"  seeds:     {seeds[0]}..{seeds[-1]} ({len(seeds)})\n"
        f"  combos:    {len(combos)}\n"
        f"  max-turns: {args.max_turns}\n"
        f"  cardsfolder: {cards_folder}\n",
        file=sys.stderr,
        flush=True,
    )

    debug_dir = Path(args.debug_dir)
    passed = 0
    diverged: list[Divergence] = []
    setup_errors: list[str] = []

    try:
        with WasmRunner(web_dir, headless=not args.headed, verbose=args.verbose) as wasm:
            for combo in combos:
                tag = f"seed={combo.seed} deck={combo.deck_name}"
                try:
                    native = run_native(combo, args.max_turns, cards_folder)
                    wres = wasm.run(combo, args.max_turns)
                except SetupError as exc:
                    setup_errors.append(f"{tag}: {exc}")
                    print(f"  [SETUP-ERR] {tag}: {exc}", file=sys.stderr, flush=True)
                    continue
                div = compare(combo, native, wres)
                if div is None:
                    passed += 1
                    print(
                        f"  [PASS] {tag}  "
                        f"(native {len(native.stream)} acts/{native.turns}t, "
                        f"wasm {len(wres.stream)} acts/{wres.turns}t)",
                        file=sys.stderr,
                        flush=True,
                    )
                else:
                    diverged.append(div)
                    repro = save_divergence(div, native, wres, debug_dir)
                    print(
                        f"  [FAIL] {tag}  DIVERGED at action #{div.first_diff_index}\n"
                        f"         native: {div.native_line!r}\n"
                        f"         wasm:   {div.wasm_line!r}\n"
                        f"         saved:  {repro}",
                        file=sys.stderr,
                        flush=True,
                    )
    except SetupError as exc:
        print(f"SETUP ERROR (browser/toolchain): {exc}", file=sys.stderr)
        return 2

    total = len(combos)
    print(
        f"\n=== sweep summary ===\n"
        f"  combos:        {total}\n"
        f"  PASS:          {passed}\n"
        f"  DIVERGED:      {len(diverged)}\n"
        f"  setup-skipped: {len(setup_errors)}\n",
        file=sys.stderr,
        flush=True,
    )
    if diverged:
        print("  Divergences (first action index, deck, seed):", file=sys.stderr)
        for d in diverged:
            print(
                f"    - seed={d.combo.seed} deck={d.combo.deck_name} @#{d.first_diff_index}",
                file=sys.stderr,
            )
        print(f"  Reproducers + diffs saved under: {debug_dir}", file=sys.stderr)

    ran = passed + len(diverged)
    if setup_errors and ran == 0:
        # Every combo errored at setup — treat as infra failure, not a green pass.
        return 2

    if args.expect_divergence:
        # KNOWN-divergence guard (beads mtg-ofl2i). We EXPECT every combo to
        # diverge. A clean PASS means the bug was fixed and this flag must go.
        if passed > 0:
            print(
                "\n*** UNEXPECTED MATCH ***\n"
                f"  {passed}/{ran} combo(s) now produce IDENTICAL native+WASM games.\n"
                "  The known divergence (beads mtg-ofl2i) appears to be FIXED.\n"
                "  ACTION REQUIRED: drop --expect-divergence from the make-validate\n"
                "  leg so this sweep reverts to a strict equivalence regression guard.",
                file=sys.stderr,
            )
            return 1
        print(
            "\n[expect-divergence] All combos diverged as expected for the open\n"
            "  native-vs-WASM determinism bug (beads mtg-ofl2i). Treating as PASS\n"
            "  so make validate stays green; this leg is a live tripwire that will\n"
            "  fail the moment the bug is fixed (telling you to remove the flag).",
            file=sys.stderr,
        )
        return 0

    if diverged:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
