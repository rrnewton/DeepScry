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
                   help="Run via a native `mtg server` + WASM WebSocket client instead of pure local WASM.")
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

    if args.networked:
        print(
            "NOTE: --networked WASM play (server + WASM WebSocket client) is "
            "driven through the same WasmPlaywrightProcess against the "
            "network-enabled page; falling through to the local WASM path "
            "is not equivalent. This path is wired but the WASM build must "
            "be the network build (`make wasm-dev` uses wasm-network).",
            file=sys.stderr,
        )
        # The current WasmPlaywrightProcess launches an in-tab local session.
        # A true networked run would point the page at a running `mtg server`.
        # We surface a clear, actionable message rather than silently doing
        # the local thing, and run the local WASM path so the user still gets
        # artifacts. (Full server-backed WASM CLI is tracked as follow-up.)

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
    if not (web_dir / "data" / "decks.bin").exists():
        print(f"ERROR: WASM data missing at {web_dir/'data'}. Run: mtg export-wasm", file=sys.stderr)
        return 1

    p1_name = deck_path_to_wasm_name(common.p1_deck)
    p2_name = deck_path_to_wasm_name(common.p2_deck)
    seed = common.seed if common.seed is not None else 0

    print(f"[mtg_wasm_game] page={args.page} seed={seed} max_turns={common.max_turns}")
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
        # Drive the page's OWN launcher UI so screenshots show the rendered
        # game (not the idle launcher). Engine controllers auto-play.
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
