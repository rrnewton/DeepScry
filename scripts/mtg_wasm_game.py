#!/usr/bin/env python3
"""Drop-in `mtg tui` replacement that runs the game in headless WASM.

This drives the SAME WASM build the website uses (`web/pkg/mtg_engine.js`
via `web/tui_game.html` / `web/native_game.html`) inside a headless Chromium
tab (Playwright), so a user or agent can run a web/WASM game from the CLI as
easily as `mtg tui` — and get a gamelog + per-turn screenshots out for
visual inspection.

It is DRY by construction: the browser driver is the very same
`agentplay.lib.wasm_process.WasmPlaywrightProcess` the agentplay
`--driver=wasm` mode uses, and the common `mtg tui` flags / seed derivation /
free-port picker come from `agentplay.lib.web_game_common` (shared with
`scripts/mtg_tui_networked.py`).

Usage (mirrors `mtg tui`):
    scripts/mtg_wasm_game.py [--p1 C] [--p2 C] [--seed N] [--max-turns N] \\
        [--page fancy|game] [--out-dir DIR] [--headed] [--networked] \\
        PLAYER1_DECK [PLAYER2_DECK]

Examples:
    # Random vs random WASM game, screenshots + gamelog to a run dir:
    scripts/mtg_wasm_game.py --p1 random --p2 random --seed 42 \\
        decks/old_school2/the_deck_classic.dck

    # Heuristic mirror match against the card-style GUI page:
    scripts/mtg_wasm_game.py --page game --seed 7 decks/white_weenie.dck

    # Networked variant: run a native `mtg server` and connect the WASM
    # client to it over WebSocket (still headless, still captures artifacts):
    scripts/mtg_wasm_game.py --networked --p1 random --p2 random --seed 42 \\
        decks/old_school2/the_deck_classic.dck

Prerequisites (same as the agentplay WASM driver):
  * web/pkg/mtg_engine.js          (make wasm-dev / make wasm)
  * web/data/{decks.bin,...}       (mtg export-wasm)
  * python `playwright` + chromium (pip install playwright; playwright install chromium)
  * decks must be in the WASM-exported set (referenced by bare name)

Artifacts written to the run dir (default: debug/wasm_game_<timestamp>/):
  * game.log              — the full game log (one line per engine event)
  * snapshot.json         — final GuiViewModel JSON
  * wasm_transcript.log   — browser console / page errors
  * screenshots/          — per-turn + game-over full-page PNGs
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT))

from agentplay.lib.wasm_process import (  # noqa: E402
    WASM_PAGES,
    WasmLaunchConfig,
    WasmPlaywrightProcess,
    deck_path_to_wasm_name,
)
from agentplay.lib.web_game_common import (  # noqa: E402
    add_common_mtg_tui_args,
    find_free_port,
    parse_common_mtg_tui_args,
)

# WASM controllers the engine can run autonomously inside the tab.
_ENGINE_CONTROLLERS = {"zero", "random", "heuristic"}


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="mtg_wasm_game.py",
        description="Headless WASM drop-in for `mtg tui` (gamelog + screenshots).",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    add_common_mtg_tui_args(p)
    p.add_argument("--page", default="fancy", choices=list(WASM_PAGES),
                   help="Which WASM page to drive: 'fancy' (terminal-style) or 'game' (card GUI). Default: fancy.")
    p.add_argument("--out-dir", default=None,
                   help="Run-output directory (gamelog + screenshots). Default: debug/wasm_game_<timestamp>/.")
    p.add_argument("--headed", action="store_true",
                   help="Show the Chromium window (default: headless).")
    p.add_argument("--networked", action="store_true",
                   help="Run via a native `mtg server` + native AI peer + WASM WebSocket "
                        "client (the ?mode=network auto-match contract) instead of pure "
                        "local in-tab WASM. Needs target/release/mtg; P1 plays in the "
                        "browser, P2 is the native peer; both must be engine controllers.")
    p.add_argument("--verbose", action="store_true", help="Verbose driver logging to stderr.")
    return p


def main() -> int:
    parser = _build_parser()
    args = parser.parse_args()
    common = parse_common_mtg_tui_args(args)

    for ctrl in (common.p1_controller, common.p2_controller):
        if ctrl not in _ENGINE_CONTROLLERS:
            print(
                f"ERROR: controller {ctrl!r} is not engine-driven. "
                f"mtg_wasm_game.py auto-plays {sorted(_ENGINE_CONTROLLERS)}; "
                "for human/LLM-directed WASM play use "
                "`agentplay/agent_game.py --driver=wasm` instead.",
                file=sys.stderr,
            )
            return 2

    # Resolve run-output dir.
    if args.out_dir:
        out_dir = Path(args.out_dir)
    else:
        ts = time.strftime("%Y%m%d_%H%M%S")
        out_dir = REPO_ROOT / "debug" / f"wasm_game_{ts}"
    out_dir.mkdir(parents=True, exist_ok=True)
    screenshot_dir = out_dir / "screenshots"

    # Prerequisite checks (clear errors, hard fail — never silently skip).
    web_dir = REPO_ROOT / "web"
    if not (web_dir / "pkg" / "mtg_engine.js").exists():
        print(f"ERROR: WASM build missing at {web_dir/'pkg'}. Run: make wasm-dev", file=sys.stderr)
        return 1
    # tokens.bin + decks.bin are content-addressed (tokens+decks cache-skew
    # fix): their hashed names live in data/sets/index.json. Verify the manifest
    # resolves the decks bin to an on-disk file rather than the retired fixed
    # `data/decks.bin` path.
    import json as _json

    index_path = web_dir / "data" / "sets" / "index.json"
    if not index_path.exists():
        print(f"ERROR: WASM data missing at {web_dir/'data'}. Run: mtg export-wasm", file=sys.stderr)
        return 1
    try:
        _decks_rel = _json.loads(index_path.read_text())["decks"]
    except (KeyError, ValueError) as e:
        print(f"ERROR: index.json missing 'decks' entry ({e}). Re-run: mtg export-wasm", file=sys.stderr)
        return 1
    if not (web_dir / "data" / _decks_rel).exists():
        print(f"ERROR: decks bin {_decks_rel} missing under {web_dir/'data'}. Run: mtg export-wasm", file=sys.stderr)
        return 1

    if args.networked:
        if not (REPO_ROOT / "target" / "release" / "mtg").exists():
            print("ERROR: --networked needs the native `mtg` binary at "
                  "target/release/mtg. Build: cargo build --release --features network",
                  file=sys.stderr)
            return 1
        for ctrl in (common.p1_controller, common.p2_controller):
            if ctrl not in _ENGINE_CONTROLLERS:
                print(f"ERROR: --networked auto-play needs engine controllers "
                      f"({sorted(_ENGINE_CONTROLLERS)}); got {ctrl!r}.", file=sys.stderr)
                return 2

    p1_name = deck_path_to_wasm_name(common.p1_deck)
    p2_name = deck_path_to_wasm_name(common.p2_deck)
    seed = common.seed if common.seed is not None else 0

    mode = "networked" if args.networked else "local"
    print(f"[mtg_wasm_game] mode={mode} page={args.page} seed={seed} max_turns={common.max_turns}")
    print(f"[mtg_wasm_game] P1={common.p1_controller} deck={p1_name}  P2={common.p2_controller} deck={p2_name}")
    print(f"[mtg_wasm_game] run dir: {out_dir}")

    proc = WasmPlaywrightProcess(
        config=WasmLaunchConfig(
            p1_deck=p1_name,
            p2_deck=p2_name,
            p1_controller=common.p1_controller,
            p2_controller=common.p2_controller,
            seed=seed,
            page=args.page,
            headless=not args.headed,
            max_turns=common.max_turns,
        ),
        web_dir=web_dir,
        game_dir=out_dir,
        screenshot_dir=screenshot_dir,
        verbose=args.verbose,
        port=find_free_port(),
    )

    game_log_path = out_dir / "game.log"
    final_turn = None
    try:
        if args.networked:
            # Networked WASM: spawn a native `mtg server` + AI peer and boot
            # this browser tab as the second network client (proven
            # ?mode=network auto-match contract). Shared core in
            # WasmPlaywrightProcess.run_network_ui — the DRY counterpart to
            # the native-only scripts/mtg_tui_networked.py.
            import random as _random

            cardsfolder = REPO_ROOT / "cardsfolder"
            if not cardsfolder.exists():
                cardsfolder = REPO_ROOT / "mtg-engine" / "cardsfolder"
            result = proc.run_network_ui(
                mtg_binary=REPO_ROOT / "target" / "release" / "mtg",
                cardsfolder=cardsfolder,
                peer_deck=Path(common.p2_deck),
                password=f"test_{_random.randint(1000, 9999)}",
                max_turns=common.max_turns,
                server_seed=common.seed,
            )
        else:
            # Local WASM: boot an in-tab engine-vs-engine session from URL
            # params so screenshots show the rendered game. Engine
            # controllers auto-play.
            result = proc.run_autoplay_ui(max_turns=common.max_turns)
        final_turn = result["final_turn"]
        log_lines = result["log_lines"]
        game_log_path.write_text("\n".join(log_lines) + "\n", encoding="utf-8")
        if result["game_over"]:
            print("[mtg_wasm_game] GAME OVER")
        else:
            print(f"[mtg_wasm_game] stopped at turn cap ({common.max_turns})")
    finally:
        proc.close()

    # Report artifact locations.
    shots = sorted(screenshot_dir.glob("*.png")) if screenshot_dir.exists() else []
    print(f"[mtg_wasm_game] final turn: {final_turn}")
    print(f"[mtg_wasm_game] gamelog:    {game_log_path}")
    print(f"[mtg_wasm_game] snapshot:   {proc.snapshot_path}")
    print(f"[mtg_wasm_game] transcript: {proc.transcript_path}")
    print(f"[mtg_wasm_game] screenshots: {len(shots)} in {screenshot_dir}")
    for s in shots[:3]:
        print(f"[mtg_wasm_game]   {s}")
    if len(shots) > 3:
        print(f"[mtg_wasm_game]   ... and {len(shots) - 3} more")
    return 0


if __name__ == "__main__":
    sys.exit(main())
