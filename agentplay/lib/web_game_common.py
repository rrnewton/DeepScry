"""Shared infra for web / networked / WASM game runners (DRY).

Both `scripts/mtg_tui_networked.py` (native server + 2 clients) and
`scripts/mtg_wasm_game.py` (headless WASM/Playwright) — and the agentplay
WASM driver (`agentplay/lib/wasm_process.py`) — need the SAME small pieces:

  * a free-port picker,
  * the `mtg tui --seed N` → per-controller-seed derivation (so a network /
    WASM run is a true drop-in for `mtg tui --seed N`),
  * the common `mtg tui` CLI surface (deck(s), --p1/--p2, --seed,
    --max-turns) parsed into one struct,
  * deck path → WASM deck-name mapping.

Centralizing these here keeps the runners thin and guarantees they agree on
seed semantics and argument names. The runners add their own backend-specific
flags on top of `add_common_mtg_tui_args`.
"""

from __future__ import annotations

import argparse
import socket
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence

# Per-controller seed salts. MUST match `mtg tui` (mtg-engine/src/main.rs):
#   p1_seed = master_seed.wrapping_add(0x1234_5678_9ABC_DEF0)
#   p2_seed = master_seed.wrapping_add(0xFEDC_BA98_7654_3210)
P1_SEED_SALT = 0x1234_5678_9ABC_DEF0
P2_SEED_SALT = 0xFEDC_BA98_7654_3210
U64_MASK = 0xFFFF_FFFF_FFFF_FFFF


def find_free_port() -> int:
    """Bind to port 0 to let the OS hand us a free ephemeral port, release
    it, and return the number. There is a short TOCTOU window between release
    and the caller re-binding, but that is acceptable for test/dev runners."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        return s.getsockname()[1]


def derive_controller_seeds(master_seed: int) -> tuple[int, int]:
    """Derive (p1_seed, p2_seed) from a master `--seed` using the exact same
    salt formula as native `mtg tui`. Returns u64 values."""
    m = master_seed & U64_MASK
    return ((m + P1_SEED_SALT) & U64_MASK, (m + P2_SEED_SALT) & U64_MASK)


def deck_path_to_wasm_name(path: str | Path) -> str:
    """Map "decks/foo_bar.dck" → "foo_bar" — the WASM data ships bare deck
    names (keys of `web/data/decks.bin`) while CLI consumers pass `.dck`
    paths. Shared so wasm_process.py and the WASM CLI agree."""
    return Path(path).stem


@dataclass
class MtgTuiArgs:
    """The subset of `mtg tui` arguments common to every backend.

    `p1_deck` / `p2_deck` are whatever the user passed (a `.dck` path for
    native/networked, or a `.dck` path that the WASM backend maps to a bare
    name). `p2_deck` defaults to `p1_deck` (mirror match) when omitted.
    """

    p1_deck: str
    p2_deck: str
    p1_controller: str
    p2_controller: str
    seed: int | None
    max_turns: int


def add_common_mtg_tui_args(parser: argparse.ArgumentParser) -> None:
    """Register the common `mtg tui` flags on `parser`. Backends call this,
    then add their own flags, then call `parse_common_mtg_tui_args`."""
    parser.add_argument("player1_deck", nargs="?", default=None,
                        help="Player 1 deck (.dck path).")
    parser.add_argument("player2_deck", nargs="?", default=None,
                        help="Player 2 deck (.dck path); defaults to P1's deck (mirror match).")
    parser.add_argument("--p1", default="heuristic",
                        help="P1 controller: zero | random | heuristic | human (default: heuristic).")
    parser.add_argument("--p2", default="heuristic",
                        help="P2 controller: zero | random | heuristic | human (default: heuristic).")
    parser.add_argument("--seed", default=None,
                        help="Master RNG seed (u64). Per-controller seeds are derived to match `mtg tui --seed`.")
    parser.add_argument("--max-turns", type=int, default=100,
                        help="Stop the game after this many turns (default: 100).")


def parse_common_mtg_tui_args(args: argparse.Namespace) -> MtgTuiArgs:
    """Validate + normalize the common flags into an `MtgTuiArgs`. Raises
    `SystemExit` via the standard argparse error path if a deck is missing."""
    if not args.player1_deck:
        raise SystemExit("ERROR: at least one deck file is required (PLAYER1_DECK).")
    p1_deck = args.player1_deck
    p2_deck = args.player2_deck or args.player1_deck
    seed: int | None = None
    if args.seed is not None and str(args.seed) != "from_entropy":
        seed = int(args.seed) & U64_MASK
    return MtgTuiArgs(
        p1_deck=p1_deck,
        p2_deck=p2_deck,
        p1_controller=args.p1,
        p2_controller=args.p2,
        seed=seed,
        max_turns=args.max_turns,
    )


def deck_paths_from_mtg_args(mtg_args: Sequence[str]) -> list[str]:
    """Extract `.dck` paths (in order) from a raw `mtg tui` argv tail. Used by
    the agentplay WASM driver which receives a passthrough arg list. Mirrors
    the native default: if only one deck is given, mirror it for both seats."""
    decks = [str(a) for a in mtg_args if str(a).endswith(".dck")]
    if len(decks) == 1:
        return [decks[0], decks[0]]
    return decks
