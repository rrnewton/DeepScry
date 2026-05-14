"""Unit tests for the WASM/Playwright bridge pieces.

Covers `agentplay/lib/text_formatter.py` and the parsing helpers in
`agentplay/lib/wasm_process.py`. The full Playwright/Chromium round-trip
is exercised by `debug/test_wasm_process.py` (not run in unit tests
because it requires a built WASM module + Chromium).
"""

from __future__ import annotations

import pytest

from agentplay.lib.text_formatter import (
    strip_menu_prefix,
    view_model_choice_context,
    view_model_choices,
    view_model_is_game_over,
    view_model_log_lines,
    view_model_priority_player,
    view_model_to_state_summary,
    view_model_turn_number,
)
from agentplay.lib.wasm_process import (
    WASM_PAGES,
    WASM_PAGE_FANCY,
    WASM_PAGE_GAME,
    WasmLaunchConfig,
    _diff_after,
    deck_path_to_wasm_name,
)


# ---------------------------------------------------------------------------
# strip_menu_prefix
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "raw,expected",
    [
        ("[0] pass", "pass"),
        ("[1] play Mountain", "play Mountain"),
        ("[12] cast Lightning Bolt", "cast Lightning Bolt"),
        ("  [3] activate Strip Mine  ", "activate Strip Mine"),
        # No prefix at all → stripped through unchanged
        ("pass", "pass"),
        ("cast Bolt", "cast Bolt"),
        # Bracketed but not a digit (defensive — current renderer never emits this)
        ("[ctx] play Mountain", "[ctx] play Mountain"),
    ],
)
def test_strip_menu_prefix(raw: str, expected: str) -> None:
    assert strip_menu_prefix(raw) == expected


# ---------------------------------------------------------------------------
# view_model_choices
# ---------------------------------------------------------------------------


def test_view_model_choices_filters_pass_and_strips_prefix() -> None:
    vm = {
        "choices": [
            {"index": 0, "text": "[0] pass"},
            {"index": 1, "text": "[1] play Mountain"},
            {"index": 2, "text": "[2] cast Lightning Bolt"},
        ]
    }
    assert view_model_choices(vm) == ["play Mountain", "cast Lightning Bolt"]


def test_view_model_choices_handles_empty_or_missing() -> None:
    assert view_model_choices({}) == []
    assert view_model_choices({"choices": None}) == []
    assert view_model_choices({"choices": []}) == []


def test_view_model_choices_skips_malformed_entries() -> None:
    vm = {
        "choices": [
            {"index": 0, "text": "[0] pass"},
            "garbage",
            {"index": 1},  # missing text
            {"index": 2, "text": "[2] play Forest"},
        ]
    }
    assert view_model_choices(vm) == ["play Forest"]


# ---------------------------------------------------------------------------
# view_model_to_state_summary
# ---------------------------------------------------------------------------


def test_view_model_to_state_summary_basic_shape() -> None:
    vm = {
        "turn_number": 3,
        "current_step": "Main1",
        "active_player_idx": 0,
        "our_player_idx": 0,
        "players": [
            {
                "player_id": 0,
                "name": "P1",
                "life": 18,
                "mana_pool": {"red": 1, "green": 0},
                "hand_size": 4,
                "graveyard_size": 1,
                "library_size": 50,
                "is_us": True,
                "hand": [
                    {"name": "Mountain"},
                    {"name": "Lightning Bolt"},
                ],
                "battlefield_sections": [
                    {
                        "label": "Lands",
                        "cards": [
                            {"name": "Mountain", "is_tapped": True},
                            {"name": "Forest", "is_tapped": False},
                        ],
                    }
                ],
                "graveyard": [{"name": "Shock"}],
            },
            {
                "player_id": 1,
                "name": "P2",
                "life": 12,
                "mana_pool": {},
                "hand_size": 5,
                "graveyard_size": 0,
                "library_size": 48,
                "is_us": False,
                "hand": [],
                "battlefield_sections": [],
                "graveyard": [],
            },
        ],
        "stack": [],
    }
    text = view_model_to_state_summary(vm)
    # Header line should match the native shape exactly
    assert text.startswith(
        "Turn: 3 | Phase: Pre-combat Main | Step: Main1 | Active player: P1 | Priority: P1"
    )
    # P1 (us / decision-maker) should show concrete hand cards
    assert "Mountain, Lightning Bolt" in text
    # P2 hand should be redacted (we're rendering from P1's perspective)
    assert "5 hidden card(s)" in text
    # Mana pool collapses zero entries; only red=1 should appear
    assert "R=1" in text
    assert "G=" not in text
    # Battlefield grouping by player
    assert "Mountain (tapped)" in text
    # Stack
    assert "Stack:\n- (empty)" in text


def test_view_model_to_state_summary_handles_missing_data() -> None:
    """An empty view model should not crash; it should fall through to the
    no-data placeholders so the prompt builder still gets something
    structurally valid."""

    text = view_model_to_state_summary({})
    # Should still produce the canonical section headers, just with
    # placeholders inside.
    assert "Players:" in text
    assert "Battlefield:" in text
    assert "Stack:" in text
    assert "(no player data)" in text
    # Non-dict input should hit the explicit error path.
    assert view_model_to_state_summary("nope") == "(no game state available)"  # type: ignore[arg-type]


def test_view_model_to_state_summary_step_to_phase_mapping() -> None:
    for step, phase in [
        ("Main1", "Pre-combat Main"),
        ("Main2", "Post-combat Main"),
        ("DeclareAttackers", "Combat"),
        ("Untap", "Beginning"),
        ("End", "Ending"),
    ]:
        vm = {"turn_number": 1, "current_step": step, "players": []}
        text = view_model_to_state_summary(vm)
        assert f"Phase: {phase}" in text, f"step={step!r}"


# ---------------------------------------------------------------------------
# view_model_log_lines
# ---------------------------------------------------------------------------


def test_view_model_log_lines_drops_choice_entries() -> None:
    vm = {
        "logs": [
            {"text": "P1 plays Mountain", "is_choice": False},
            {"text": "<Choice> P1 chose pass", "is_choice": True},
            {"text": "P2 draws a card", "is_choice": False},
        ]
    }
    assert view_model_log_lines(vm) == [
        "P1 plays Mountain",
        "P2 draws a card",
    ]


def test_view_model_log_lines_handles_missing() -> None:
    assert view_model_log_lines({}) == []
    assert view_model_log_lines({"logs": None}) == []


# ---------------------------------------------------------------------------
# Misc accessors
# ---------------------------------------------------------------------------


def test_view_model_priority_player_with_choices() -> None:
    vm = {"choices": [{"index": 0, "text": "[0] pass"}], "our_player_idx": 0}
    assert view_model_priority_player(vm) == "p1"
    vm2 = {"choices": [{"index": 0, "text": "[0] pass"}], "our_player_idx": 1}
    assert view_model_priority_player(vm2) == "p2"


def test_view_model_priority_player_without_choices_is_none() -> None:
    assert view_model_priority_player({"choices": [], "our_player_idx": 0}) is None
    assert view_model_priority_player({}) is None


def test_view_model_choice_context_filters_None_string() -> None:
    assert view_model_choice_context({"choice_context": "PlayingSpell"}) == "PlayingSpell"
    assert view_model_choice_context({"choice_context": "None"}) is None
    assert view_model_choice_context({"choice_context": ""}) is None
    assert view_model_choice_context({}) is None


def test_view_model_turn_number() -> None:
    assert view_model_turn_number({"turn_number": 5}) == 5
    assert view_model_turn_number({"turn_number": "5"}) is None  # only int counts
    assert view_model_turn_number({}) is None


def test_view_model_is_game_over() -> None:
    assert view_model_is_game_over({"game_over": True}) is True
    assert view_model_is_game_over({"game_over": False}) is False
    assert view_model_is_game_over({}) is False


# ---------------------------------------------------------------------------
# wasm_process helpers
# ---------------------------------------------------------------------------


def test_deck_path_to_wasm_name() -> None:
    assert deck_path_to_wasm_name("decks/simple_bolt.dck") == "simple_bolt"
    assert deck_path_to_wasm_name("decks/old_school2/ur_burn.dck") == "ur_burn"
    assert deck_path_to_wasm_name("ur_burn") == "ur_burn"


def test_wasm_pages_constant_complete() -> None:
    """If we add a new WASM page, it should be discoverable through the
    `WASM_PAGES` tuple so argparse choices stay in sync."""

    assert WASM_PAGE_FANCY in WASM_PAGES
    assert WASM_PAGE_GAME in WASM_PAGES


def test_wasm_launch_config_defaults() -> None:
    cfg = WasmLaunchConfig(
        p1_deck="ur_burn",
        p2_deck="ur_burn",
        p1_controller="human",
        p2_controller="heuristic",
        seed=7,
    )
    # Defaults should match what the agent_game.py wrapper relies on.
    assert cfg.page == WASM_PAGE_FANCY
    assert cfg.headless is True
    assert cfg.starting_life == 20


# ---------------------------------------------------------------------------
# _diff_after — incremental log delta
# ---------------------------------------------------------------------------


def test_diff_after_simple_growth() -> None:
    prev = ["a", "b", "c"]
    curr = ["a", "b", "c", "d", "e"]
    assert _diff_after(curr, prev) == ["d", "e"]


def test_diff_after_first_call_returns_everything() -> None:
    assert _diff_after(["a", "b"], []) == ["a", "b"]


def test_diff_after_no_change_returns_empty() -> None:
    assert _diff_after(["a", "b"], ["a", "b"]) == []


def test_diff_after_buffer_rolloff_uses_tail_search() -> None:
    """If `previous` is no longer a prefix of `current` (the buffer rolled
    over), we should still find the right offset by searching for the tail."""

    prev = ["x1", "x2", "x3", "y1", "y2"]
    curr = ["x3", "y1", "y2", "z1", "z2"]
    # `current` doesn't start with the previous prefix, but its tail
    # matches the previous tail (`y1, y2`), so we should emit just the
    # genuinely new lines that came after.
    assert _diff_after(curr, prev) == ["z1", "z2"]


def test_diff_after_total_rolloff_emits_full_current() -> None:
    """If there's no overlap at all (e.g. heavy log_tail truncation), we
    should fall back to the full current list rather than erroring."""

    assert _diff_after(["new1", "new2"], ["old1", "old2"]) == ["new1", "new2"]
