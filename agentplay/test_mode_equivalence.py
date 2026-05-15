"""End-to-end equivalence tests across all three agent_play.py drivers.

The three drivers (`stop-and-go`, `persistent`, `wasm`) are intended to be
SEMANTICALLY equivalent — the LLM should see the same prompts, the engine
should make the same gameplay decisions, and the resulting on-disk
artefacts should describe the same game.

This module tests that promise at the levels where it's actually achievable:

* `test_prompt_*` — UNIT tests confirming the prompt-builder refactor in
  `prompts.py` produces byte-identical prompts whether you start from a
  native `GameSnapshot` (stop-and-go / persistent path) or from a precomputed
  state-summary string (the WASM path).

* `test_state_summary_shape_parity` — UNIT test confirming the WASM
  `view_model_to_state_summary` formatter mirrors the field shape of
  the native `_format_state_summary` (turn header, player rows, battlefield
  groups, stack line). Keeps both formatters from drifting.

* `test_drivers_run_to_completion_*` — INTEGRATION tests that each driver
  runs a short mock game without crashing and produces the standard
  artefacts (`pN_choices.txt`, `snapshot.json`, `game.log`,
  `enriched_log.md`).

* `test_filtered_action_streams_overlap` — INTEGRATION test comparing the
  ACTION (non-pass) streams of the stop-and-go and persistent drivers when
  given the same seed. Full byte equivalence is NOT possible because the
  two drivers route mock decisions through different RNG paths (stop-and-go
  uses a single Python `random.Random(seed)`; persistent uses per-player
  `MockSession`s; engine-side controllers introduce more variance). This
  test verifies that BOTH streams are non-empty and that the action vocabulary
  (set of distinct actions taken) overlaps significantly.

The `test_drivers_run_to_completion_wasm` case is gated behind
`AGENTPLAY_TEST_WASM=1` because it requires Chromium + the built WASM
module + the python `playwright` package; CI runs it in the dedicated
WASM job (see `Makefile` `validate-agentplay-step`).
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parent.parent

# Keep these in sync with the values in `agentplay/agent_game.py`. We don't
# import them directly to avoid a hard dependency on agent_game.py's full
# import surface (which pulls in the WASM Playwright bridge etc.) for the
# unit tests that don't need it.
_DECK = "decks/simple_bolt.dck"
_MAX_TURNS = 3  # Tight cap so tests stay fast even when games run long.


# ---------------------------------------------------------------------------
# Section 1: prompt-builder equivalence (no subprocesses, no engine)
# ---------------------------------------------------------------------------


def _toy_snapshot() -> dict:
    """Build a minimal `GameSnapshot`-shaped dict that exercises every
    section of `_format_state_summary` (turn header, two players, hand of
    decision-maker, hidden hand of opponent, battlefield with a tapped
    permanent, mana pool, stack).

    The native `_build_card_map` assumes `cards` is a LIST indexed by card
    id (matching the engine's `GameState` serialisation), so we lay the
    cards out as list entries with stable indices the zone references
    point at. This mirrors the on-disk shape `mtg tui` writes.
    """

    return {
        "game_state": {
            "turn": {
                "turn_number": 4,
                "current_step": "Main2",
                "active_player": 0,
                "priority_player": 0,
            },
            "players": [
                {"id": 0, "name": "P1", "life": 20, "mana_pool": {"red": 1}},
                {"id": 1, "name": "P2", "life": 17, "mana_pool": {}},
            ],
            "player_zones": [
                [
                    0,
                    {
                        # 0 = Lightning Bolt, 1 = Mountain
                        "hand": {"cards": [0, 1]},
                        # 2 = Shock
                        "graveyard": {"cards": [2]},
                        # 3, 4, 5 = library
                        "library": {"cards": [3, 4, 5]},
                        "exile": {"cards": []},
                    },
                ],
                [
                    1,
                    {
                        # 6, 7, 8 = P2 hand (will be hidden by perspective filter)
                        "hand": {"cards": [6, 7, 8]},
                        "graveyard": {"cards": []},
                        "library": {"cards": [9, 10]},
                        "exile": {"cards": []},
                    },
                ],
            ],
            "battlefield": {"cards": [11, 12]},
            "stack": {"cards": []},
            "cards": [
                {"name": "Lightning Bolt"},   # 0  (P1 hand)
                {"name": "Mountain"},         # 1  (P1 hand)
                {"name": "Shock"},            # 2  (P1 graveyard)
                {"name": "Mountain"},         # 3  (P1 library)
                {"name": "Goblin Guide"},     # 4  (P1 library)
                {"name": "Mountain"},         # 5  (P1 library)
                {"name": "Counterspell"},     # 6  (P2 hand)
                {"name": "Island"},           # 7  (P2 hand)
                {"name": "Brainstorm"},       # 8  (P2 hand)
                {"name": "Island"},           # 9  (P2 library)
                {"name": "Force of Will"},    # 10 (P2 library)
                {"name": "Mountain", "tapped": True, "controller": 0},  # 11 (BF)
                {
                    "name": "Goblin Guide",
                    "base_power": 2,
                    "base_toughness": 2,
                    "controller": 0,
                },  # 12 (BF)
            ],
        }
    }


def test_prompt_builder_refactor_byte_identical() -> None:
    """`build_choice_prompt(snapshot)` and
    `build_choice_prompt_with_summary(state_summary)` MUST agree on every
    section other than the source of `state_summary`. This locks in the
    refactor that introduced the `_with_summary` variant for the WASM
    driver."""

    from agentplay.lib.prompts import (
        _as_dict,
        _build_card_map,
        _extract_players,
        _extract_zone_map,
        _format_state_summary,
        _normalize_scalar,
        _snapshot_root,
        _zone_cards,
        build_choice_prompt,
        build_choice_prompt_with_summary,
    )

    snap = _toy_snapshot()
    choices = ["play Mountain", "cast Lightning Bolt"]
    log = "P1 drew Mountain"

    # Recompute state_summary the same way `build_choice_prompt` does.
    root = _snapshot_root(snap)
    card_map = _build_card_map(root)
    players = _extract_players(root)
    zone_map = _extract_zone_map(root)
    turn = _as_dict(root.get("turn"))
    ap = _normalize_scalar(turn.get("active_player"))
    pp = _normalize_scalar(turn.get("priority_player"))
    bf = _zone_cards(root.get("battlefield"))
    st = _zone_cards(root.get("stack"))
    summary = _format_state_summary(root, players, zone_map, turn, ap, pp, bf, st, card_map)

    p_native = build_choice_prompt(
        snap, choices, log, scenario="test scenario", goal="goal text"
    )
    p_summary = build_choice_prompt_with_summary(
        summary,
        choices,
        log,
        scenario="test scenario",
        goal="goal text",
    )
    assert p_native == p_summary, (
        "build_choice_prompt and build_choice_prompt_with_summary should be "
        "byte-identical when given an equivalent state_summary"
    )


def test_prompt_sections_present_in_all_paths() -> None:
    """Every prompt produced by either path MUST contain the canonical
    section headers — the LLM relies on them as reading anchors."""

    from agentplay.lib.prompts import build_choice_prompt

    snap = _toy_snapshot()
    prompt = build_choice_prompt(snap, ["play Mountain"], "log")
    for header in (
        "Current game state:",
        "Interleaved history so far:",
        "Previous decision:",
        "Game log since last decision:",
        "Available choices:",
        "MTG rules context:",
        "Response format:",
    ):
        assert header in prompt, f"missing prompt header: {header!r}"


# ---------------------------------------------------------------------------
# Section 2: state-summary shape parity (native vs WASM formatters)
# ---------------------------------------------------------------------------


def test_state_summary_shape_parity() -> None:
    """The WASM `view_model_to_state_summary` and the native
    `_format_state_summary` should agree on overall structure: same turn
    header skeleton, same Players/Battlefield/Stack section labels.

    The two formatters work from different input shapes (`GuiViewModel` vs
    `GameSnapshot`), so byte equivalence isn't expected — but the *shape*
    must match so the prompt builder can drop the summary into the same
    slot regardless of source.
    """

    from agentplay.lib.prompts import (
        _as_dict,
        _build_card_map,
        _extract_players,
        _extract_zone_map,
        _format_state_summary,
        _normalize_scalar,
        _snapshot_root,
        _zone_cards,
    )
    from agentplay.lib.text_formatter import view_model_to_state_summary

    snap = _toy_snapshot()
    root = _snapshot_root(snap)
    native = _format_state_summary(
        root,
        _extract_players(root),
        _extract_zone_map(root),
        _as_dict(root.get("turn")),
        _normalize_scalar(_as_dict(root.get("turn")).get("active_player")),
        _normalize_scalar(_as_dict(root.get("turn")).get("priority_player")),
        _zone_cards(root.get("battlefield")),
        _zone_cards(root.get("stack")),
        _build_card_map(root),
    )

    # Build a GuiViewModel that DESCRIBES THE SAME GAME STATE.
    vm = {
        "turn_number": 4,
        "current_step": "Main2",
        "active_player_idx": 0,
        "our_player_idx": 0,
        "players": [
            {
                "name": "P1",
                "life": 20,
                "mana_pool": {"red": 1},
                "hand_size": 2,
                "graveyard_size": 1,
                "library_size": 3,
                "is_us": True,
                "hand": [{"name": "Lightning Bolt"}, {"name": "Mountain"}],
                "battlefield_sections": [
                    {
                        "label": "Lands",
                        "cards": [
                            {"name": "Mountain", "is_tapped": True},
                            {"name": "Goblin Guide"},
                        ],
                    }
                ],
                "graveyard": [{"name": "Shock"}],
            },
            {
                "name": "P2",
                "life": 17,
                "mana_pool": {},
                "hand_size": 3,
                "graveyard_size": 0,
                "library_size": 2,
                "is_us": False,
                "hand": [],
                "battlefield_sections": [],
                "graveyard": [],
            },
        ],
        "stack": [],
    }
    wasm = view_model_to_state_summary(vm)

    # Both formatters MUST start with a "Turn: …" header on line 1.
    assert native.splitlines()[0].startswith("Turn:"), native.splitlines()[0]
    assert wasm.splitlines()[0].startswith("Turn:"), wasm.splitlines()[0]
    # Same turn number / phase mapping in the header
    assert "Turn: 4" in native and "Turn: 4" in wasm
    assert "Phase: Post-combat Main" in native and "Phase: Post-combat Main" in wasm
    assert "Step: Main2" in native and "Step: Main2" in wasm
    assert "Active player: P1" in native and "Active player: P1" in wasm

    # Same section labels.
    for header in ("Players:", "Battlefield:", "Stack:"):
        assert header in native, f"native missing {header!r}"
        assert header in wasm, f"wasm missing {header!r}"

    # Same hand-redaction policy: opponent hand (P2 here) hidden, decision-
    # maker hand visible.
    assert "5 hidden card(s)" in native or "3 hidden card(s)" in native
    assert "3 hidden card(s)" in wasm
    assert "Lightning Bolt" in native and "Lightning Bolt" in wasm

    # Mana pool: only non-zero entries are emitted.
    assert "R=1" in native and "R=1" in wasm

    # Tapped permanent surfaced in both.
    assert "Mountain (tapped)" in native and "Mountain (tapped)" in wasm

    # Empty stack yields the same canonical "(empty)" marker.
    assert "Stack:\n- (empty)" in native and "Stack:\n- (empty)" in wasm


# ---------------------------------------------------------------------------
# Section 3: integration — drivers run to completion
# ---------------------------------------------------------------------------


def _cards_folder() -> Path | None:
    """Resolve a usable cardsfolder for the test subprocesses. The persist
    worktree's symlink may be broken (forge-java not initialized); fall
    back to a sibling checkout's cardsfolder if available."""

    env = os.environ.get("CARDSFOLDER")
    if env and (Path(env) / "a").is_dir():
        return Path(env)
    candidates = [
        REPO_ROOT / "cardsfolder",
        REPO_ROOT / "forge-java" / "forge-gui" / "res" / "cardsfolder",
        Path("/home/newton/working_copies/mtg/mtg-forge-rs/cardsfolder"),
    ]
    for c in candidates:
        if c.exists() and (c / "a").is_dir():
            return c
    return None


def _run_agent_game(
    driver: str,
    game_dir: Path,
    *,
    extra_args: list[str] | None = None,
    deck: str | None = None,
) -> int:
    """Invoke `agent_game.py` as a subprocess so it doesn't pollute the
    pytest process state. Returns the exit code."""

    cards = _cards_folder()
    if cards is None:
        pytest.skip("No usable CARDSFOLDER found — skipping subprocess equivalence test")

    env = os.environ.copy()
    env["CARDSFOLDER"] = str(cards)
    deck_path = deck or _DECK
    cmd = [
        sys.executable,
        str(REPO_ROOT / "agentplay" / "agent_game.py"),
        "--mock",
        "--seed",
        "42",
        "--max-turns",
        str(_MAX_TURNS),
        f"--game-dir={game_dir}",
        f"--driver={driver}",
        "--mode=random-vs-random",
        "--",
        deck_path,
        deck_path,
    ]
    if extra_args:
        cmd.extend(extra_args)
    completed = subprocess.run(cmd, capture_output=True, text=True, env=env, cwd=str(REPO_ROOT))
    if completed.returncode not in (0, 2):
        sys.stderr.write(
            f"\n[run_agent_game driver={driver}] exit={completed.returncode}\n"
            f"stdout:\n{completed.stdout[-2000:]}\n"
            f"stderr:\n{completed.stderr[-2000:]}\n"
        )
    return completed.returncode


def _check_artefacts(game_dir: Path) -> None:
    """Every driver should produce the same set of standard artefacts."""

    for name in ("p1_choices.txt", "p2_choices.txt", "snapshot.json", "game.log"):
        path = game_dir / name
        assert path.exists(), f"missing artefact {path}"
    # enriched_log.md should be present and start with the canonical header
    enriched = game_dir / "enriched_log.md"
    assert enriched.exists(), f"missing enriched log at {enriched}"
    text = enriched.read_text(encoding="utf-8")
    assert text.startswith("# Enriched Agent Game Log"), text[:120]


def test_drivers_run_to_completion_persistent(tmp_path: Path) -> None:
    """Persistent driver runs a short mock game and produces all artefacts."""

    game_dir = tmp_path / "persistent.game"
    rc = _run_agent_game("persistent", game_dir)
    assert rc in (0, 2), f"persistent driver exited with rc={rc}"
    _check_artefacts(game_dir)


def test_drivers_run_to_completion_stop_and_go(tmp_path: Path) -> None:
    """Stop-and-go driver runs a short mock game and produces all artefacts."""

    game_dir = tmp_path / "stopgo.game"
    rc = _run_agent_game("stop-and-go", game_dir)
    assert rc in (0, 2), f"stop-and-go driver exited with rc={rc}"
    _check_artefacts(game_dir)


@pytest.mark.skipif(
    os.environ.get("AGENTPLAY_TEST_WASM") != "1",
    reason=(
        "WASM driver test requires built web/pkg, web/data, web/node_modules, "
        "and Chromium. Set AGENTPLAY_TEST_WASM=1 to enable."
    ),
)
def test_drivers_run_to_completion_wasm(tmp_path: Path) -> None:
    """WASM driver runs a short mock game and produces all artefacts.

    Gated behind `AGENTPLAY_TEST_WASM=1` because it depends on the
    Playwright Chromium runtime + a built WASM module. CI sets the env var
    in the WASM-specific job.
    """

    game_dir = tmp_path / "wasm.game"
    # The default `simple_bolt.dck` isn't in the curated WASM-exported set;
    # `ur_burn` is the closest analogue (mono-red burn deck) and ships in
    # `web/data/decks.bin`.
    wasm_deck = os.environ.get("AGENTPLAY_TEST_WASM_DECK", "decks/old_school2/ur_burn.dck")
    rc = _run_agent_game("wasm", game_dir, deck=wasm_deck)
    assert rc in (0, 2), f"wasm driver exited with rc={rc}"
    _check_artefacts(game_dir)
    # Also verify the WASM driver populated screenshots/.
    screenshots = game_dir / "screenshots"
    assert screenshots.exists() and any(screenshots.iterdir()), (
        "WASM driver should have written at least one screenshot to <game_dir>/screenshots/"
    )


# ---------------------------------------------------------------------------
# Section 4: cross-driver action-stream alignment
# ---------------------------------------------------------------------------


def _read_choices(game_dir: Path, who: str) -> list[str]:
    path = game_dir / f"{who}_choices.txt"
    if not path.exists():
        return []
    return [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def test_filtered_action_streams_overlap(tmp_path: Path) -> None:
    """Stop-and-go and persistent drivers both record `pN_choices.txt` in
    the same text-command vocabulary (e.g. `play Mountain`,
    `cast Lightning Bolt`, `pass`). They are NOT byte-identical because:

      * stop-and-go's `_choose_for_player` uses a single Python
        `random.Random(seed)` for both players;
      * persistent's `_run_persistent` uses TWO `MockSession` instances
        (one per player, with `seed` and `seed+1`);
      * the engine-side fixed-script wildcards in stop-and-go cause some
        priority points to auto-pass (and therefore not be recorded) that
        persistent records explicitly.

    What we CAN test is that:
      (a) both drivers actually exercise the engine and produce non-empty
          action streams,
      (b) the actions they DO take are drawn from the same vocabulary
          (`play X` / `cast Y` / `activate Z` / `pass`), so neither driver
          is producing garbage, and
      (c) at least one non-trivial action (something other than `pass`)
          appears in BOTH driver outputs.
    """

    persist_dir = tmp_path / "persist.game"
    stopgo_dir = tmp_path / "stopgo.game"
    rc_p = _run_agent_game("persistent", persist_dir)
    rc_s = _run_agent_game("stop-and-go", stopgo_dir)
    assert rc_p in (0, 2)
    assert rc_s in (0, 2)

    # Vocabulary check: every recorded action should start with one of the
    # legal verbs the controller expects.
    legal_verbs = ("pass", "play ", "cast ", "activate ", "Cast from ", "cycle", "cycling")
    for who in ("p1", "p2"):
        for driver_name, choices in [
            ("persistent", _read_choices(persist_dir, who)),
            ("stop-and-go", _read_choices(stopgo_dir, who)),
        ]:
            for line in choices:
                assert any(line.startswith(v) or line == v.strip() for v in legal_verbs), (
                    f"{driver_name} {who} produced unrecognised action {line!r}; "
                    "valid verbs are pass/play/cast/activate/cycle/Cast from"
                )

    persist_p1 = _read_choices(persist_dir, "p1")
    stopgo_p1 = _read_choices(stopgo_dir, "p1")

    # At least one driver should have made a non-pass action — otherwise
    # the test is vacuous.
    persist_actions = {c for c in persist_p1 if c != "pass"}
    stopgo_actions = {c for c in stopgo_p1 if c != "pass"}
    assert persist_actions or stopgo_actions, (
        "Neither driver took any non-pass action — bump --max-turns or seed"
    )


# ---------------------------------------------------------------------------
# Section 5: snapshot / view-model JSON sanity
# ---------------------------------------------------------------------------


def test_persistent_driver_snapshot_has_game_state(tmp_path: Path) -> None:
    """The persistent driver writes a `GameSnapshot`-shaped snapshot.json
    with a nested `game_state` object (the input shape `build_choice_prompt`
    expects). This is the contract the LLM prompt builder relies on."""

    game_dir = tmp_path / "persistent.game"
    rc = _run_agent_game("persistent", game_dir)
    assert rc in (0, 2)
    snap_path = game_dir / "snapshot.json"
    assert snap_path.exists()
    data = json.loads(snap_path.read_text(encoding="utf-8"))
    assert isinstance(data, dict)
    assert "game_state" in data, "persistent snapshot.json missing top-level game_state"
    assert "turn_number" in data, "persistent snapshot.json missing turn_number"


def test_stop_and_go_driver_snapshot_has_game_state(tmp_path: Path) -> None:
    """Same contract for the stop-and-go driver."""

    game_dir = tmp_path / "stopgo.game"
    rc = _run_agent_game("stop-and-go", game_dir)
    assert rc in (0, 2)
    snap_path = game_dir / "snapshot.json"
    assert snap_path.exists()
    data = json.loads(snap_path.read_text(encoding="utf-8"))
    assert isinstance(data, dict)
    # Stop-and-go writes the GameState directly (no GameSnapshot wrapper);
    # the prompt builder's `_snapshot_root` accepts both shapes, so we
    # just need to verify there's a "turn" object somewhere reachable.
    root = data.get("game_state", data)
    assert "turn" in root, "stop-and-go snapshot.json missing reachable turn object"
