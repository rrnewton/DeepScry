"""Shared formatting helpers for start_game.py / continue_game.py output.

These helpers exist to keep the two CLI scripts in sync: any change to
the human / agent facing output format should land here, not in either
script. The format intentionally mirrors `agent_game.py`'s decision
banners (`--- p1 (...) | Turn N | K choices ---`) so that operators
who switch between the scripted and agent-driven harnesses see the
same vocabulary.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any


def player_label(active_player: object) -> str:
    """Map the snapshot's active_player int / str to `p1` / `p2`."""
    if str(active_player) == "0":
        return "p1"
    if str(active_player) == "1":
        return "p2"
    return "?"


def print_log_segment(text: str) -> None:
    """Echo a chunk of game log to stdout, preceded by a header.

    No-op when `text` is empty so we don't emit a useless header at
    the very first choice point of a fresh game.
    """
    body = (text or "").strip()
    if not body:
        return
    print("--- game log ---")
    print(body)
    print("--- end game log ---")


def record_log_segment(path: Path, text: str) -> None:
    """Append the same chunk to `<game-dir>/game.log` for replay-dedup.

    `continue_game.py` reads this file back on the next invocation and
    diffs it against the engine's freshly-replayed `log_tail` so that
    only newly-emitted lines are shown to the operator.
    """
    body = (text or "").strip()
    if not body:
        return
    with path.open("a", encoding="utf-8") as handle:
        handle.write(body)
        handle.write("\n")


def print_choice_block(
    snapshot: dict[str, Any],
    *,
    choice_number: int,
    game_dir_name: str,
) -> None:
    """Render the upcoming choice menu plus a `continue_game` hint."""
    choices = snapshot.get("choices", []) or []
    player = player_label(snapshot.get("active_player"))
    turn_number = snapshot.get("turn_number")
    current_step = snapshot.get("current_step")
    choice_context = snapshot.get("choice_context")

    bits: list[str] = [
        f"Turn {turn_number if turn_number is not None else '?'}",
        f"choice #{choice_number}",
    ]
    # `choice_context` (e.g. `Your_Main2`) is the live step that the engine
    # is paused at and is more reliable than the snapshot's `current_step`,
    # which can lag a phase or two behind the actual choice point. Prefer
    # it when present and fall back to `current_step` otherwise.
    if choice_context:
        bits.append(f"[{choice_context}]")
    elif current_step:
        bits.append(f"step {current_step}")
    print(f"=== {' | '.join(bits)} ===")

    print(f"{player}'s turn. {len(choices)} choice(s) available:")
    print("  [0] pass")
    for i, c in enumerate(choices, 1):
        print(f"  [{i}] {c}")
    print()
    print(
        "Continue with: "
        f"./agentplay/continue_game.py --game-dir={game_dir_name} {player} <choice>"
    )


# Re-export `sys` so callers can reuse the same I/O surface; helps when
# tests want to monkeypatch stdout. (Kept at module level rather than
# imported in callers to avoid scattering `import sys` boilerplate.)
__all__ = [
    "player_label",
    "print_choice_block",
    "print_log_segment",
    "record_log_segment",
    "sys",
]
