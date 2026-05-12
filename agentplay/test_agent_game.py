from __future__ import annotations

import random
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from agentplay.agent_game import _choose_for_player, _query_agent, build_parser
from agentplay.lib.engine import GameEngine
from agentplay.lib.prompts import (
    build_choice_prompt,
    build_intro_section,
    format_deck_preamble,
    parse_agent_decision,
    parse_agent_response,
)


def test_build_choice_prompt_includes_state_choices_log_and_goal() -> None:
    game_state = {
        "game_state": {
            "turn": {
                "turn_number": 3,
                "current_step": "Main1",
                "active_player": 0,
                "priority_player": 0,
            },
            "players": [
                {"id": 0, "name": "Alice", "life": 20, "mana_pool": {"red": 1}},
                {"id": 1, "name": "Bob", "life": 15, "mana_pool": {}},
            ],
            "player_zones": [
                [0, {"hand": {"cards": [0]}, "graveyard": {"cards": []}, "library": {"cards": [2, 3]}, "exile": {"cards": []}}],
                [1, {"hand": {"cards": [1, 4]}, "graveyard": {"cards": [5]}, "library": {"cards": [6]}, "exile": {"cards": []}}],
            ],
            "battlefield": {"cards": [7]},
            "stack": {"cards": []},
            "cards": [
                {"name": "Lightning Bolt"},
                {"name": "Counterspell"},
                {"name": "Mountain"},
                {"name": "Mountain"},
                {"name": "Island"},
                {"name": "Shock"},
                {"name": "Island"},
                {"name": "Goblin Guide", "base_power": 2, "base_toughness": 2, "controller": 0},
            ],
        }
    }

    prompt = build_choice_prompt(
        game_state,
        ["play Mountain", "cast Lightning Bolt"],
        "Alice drew a card",
        goal="Win this turn.",
        scenario="Cast Lightning Bolt after Bob attacks.",
        interleaved_history="## Decision #1\nAlice chose pass because she had no plays.",
        previous_decision="Decision #1: Alice chose pass.",
    )

    assert "Scenario to reproduce: Cast Lightning Bolt after Bob attacks." in prompt
    assert "Goal directive: Win this turn." in prompt
    assert "Turn: 3 | Phase: Pre-combat Main | Step: Main1" in prompt
    assert "[0] pass" in prompt
    assert "[1] play Mountain" in prompt
    assert "[2] cast Lightning Bolt" in prompt
    assert "Interleaved history so far:" in prompt
    assert "Alice chose pass because she had no plays." in prompt
    assert "Previous decision:" in prompt
    assert "Decision #1: Alice chose pass." in prompt
    assert "Game log since last decision:" in prompt
    assert "Alice drew a card" in prompt
    assert "Bob: life 15" in prompt
    assert "2 hidden card(s)" in prompt
    assert "either choose the strongest legal action or STOP" in prompt
    assert "BUG_REPORT must explain" in prompt


def test_build_choice_prompt_supports_pure_play_mode() -> None:
    prompt = build_choice_prompt({}, ["pass priority"], "", bug_detection=False)

    assert "Pure play mode is enabled" in prompt
    assert "To report a gameplay bug" not in prompt
    assert "Put the choice number alone on the final line" in prompt


@pytest.mark.parametrize(
    ("response", "expected"),
    [
        ("0", 0),
        ("2\n", 2),
        ("I choose 3\n3", 3),
        ("Choice: [4]", 4),
    ],
)
def test_parse_agent_response_variants(response: str, expected: int) -> None:
    assert parse_agent_response(response) == expected


def test_parse_agent_response_rejects_missing_number() -> None:
    with pytest.raises(ValueError):
        parse_agent_response("pass please")


def test_parse_agent_decision_stops_for_bug_report_in_bug_detection_mode() -> None:
    decision = parse_agent_decision(
        "STOP\n\nBUG_REPORT: The menu offers an illegal block.",
        bug_detection=True,
    )

    assert decision.stopped_for_bug is True
    assert decision.choice_number is None
    assert "illegal block" in str(decision.bug_report)


def test_parse_agent_decision_requires_number_in_pure_play_mode() -> None:
    with pytest.raises(ValueError):
        parse_agent_decision("STOP\n\nBUG_REPORT: suspicious", bug_detection=False)


def test_game_engine_start_game_creates_dir_and_choice_files(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    engine = GameEngine(seed=42, game_dir=tmp_path / "session.game", verbose=False)
    engine.set_initial_args(["decks/simple_bolt.dck", "decks/simple_bolt.dck"])

    monkeypatch.setattr(engine, "_run_game", lambda stop_on_choice: {"stop": stop_on_choice})

    result = engine.start_game()

    assert result == {"stop": 1}
    assert engine.game_dir.is_dir()
    assert engine.initial_args_path.read_text(encoding="utf-8").splitlines() == [
        "decks/simple_bolt.dck",
        "decks/simple_bolt.dck",
    ]
    assert engine.p1_choices_path.exists()
    assert engine.p2_choices_path.exists()


def test_game_engine_append_choice_writes_expected_player_file(tmp_path: Path) -> None:
    engine = GameEngine(seed=42, game_dir=tmp_path / "session.game", verbose=False)
    engine.game_dir.mkdir(parents=True, exist_ok=True)
    engine.p1_choices_path.touch()
    engine.p2_choices_path.touch()

    engine.append_choice("p1", "play Mountain")
    engine.append_choice("p2", "pass")

    assert engine.p1_choices_path.read_text(encoding="utf-8").splitlines() == ["play Mountain"]
    assert engine.p2_choices_path.read_text(encoding="utf-8").splitlines() == ["pass"]


def test_cli_argument_parsing_supports_mode_puzzle_goal() -> None:
    parser = build_parser()
    args = parser.parse_args(
        [
            "--seed",
            "7",
            "--mode",
            "agent-vs-agent",
            "--game-dir",
            "foo.game",
            "--puzzle",
            "puzzles/example.pzl",
            "--goal",
            "Find lethal",
            "--scenario",
            "Set up a double block.",
            "--max-turns",
            "12",
            "--no-bug-detection",
            "--",
            "ignored.dck",
        ]
    )

    assert args.seed == 7
    assert args.mode == "agent-vs-agent"
    assert args.game_dir == "foo.game"
    assert args.puzzle == "puzzles/example.pzl"
    assert args.goal == "Find lethal"
    assert args.scenario == "Set up a double block."
    assert args.max_turns == 12
    assert args.bug_detection is False
    assert args.mtg_args == ["--", "ignored.dck"]


def test_query_agent_retries_then_succeeds() -> None:
    responses = [
        SimpleNamespace(returncode=0, stdout="not a number", stderr=""),
        SimpleNamespace(returncode=1, stdout="", stderr="proxy unavailable"),
        SimpleNamespace(returncode=0, stdout="2\n", stderr=""),
    ]

    with patch("agentplay.agent_game.subprocess.run", side_effect=responses) as run_mock:
        decision = _query_agent("prompt", 3, verbose=False)

    assert decision.choice_number == 2
    assert decision.raw_response == "2"
    assert run_mock.call_count == 3


def test_query_agent_accepts_stop_bug_report_without_choice_number() -> None:
    responses = [
        SimpleNamespace(
            returncode=0,
            stdout="STOP\n\nBUG_REPORT: Damage was assigned to a creature that was not in combat.",
            stderr="",
        ),
    ]

    with patch("agentplay.agent_game.subprocess.run", side_effect=responses):
        decision = _query_agent("prompt", 3, verbose=False)

    assert decision.stopped_for_bug is True
    assert decision.choice_number is None
    assert "not in combat" in str(decision.bug_report)


def test_query_agent_fails_after_three_invalid_attempts() -> None:
    responses = [
        SimpleNamespace(returncode=0, stdout="invalid", stderr=""),
        SimpleNamespace(returncode=0, stdout="9", stderr=""),
        SimpleNamespace(returncode=1, stdout="", stderr="boom"),
    ]

    with patch("agentplay.agent_game.subprocess.run", side_effect=responses):
        with pytest.raises(RuntimeError):
            _query_agent("prompt", 3, verbose=False)


def test_mock_mode_selects_randomly_without_subprocess() -> None:
    rng = random.Random(42)
    decision = _choose_for_player(
        mode="agent-vs-heuristic",
        player="p1",
        prompt_text="dummy prompt",
        choice_count=5,
        rng=rng,
        verbose=False,
        mock=True,
    )
    assert decision.choice_number is not None
    assert 0 <= decision.choice_number <= 5
    assert "mock" in decision.raw_response.lower()


def test_mock_mode_is_deterministic_with_same_seed() -> None:
    results = []
    for _ in range(2):
        rng = random.Random(42)
        choices = []
        for _ in range(10):
            decision = _choose_for_player(
                mode="agent-vs-agent",
                player="p1",
                prompt_text="",
                choice_count=3,
                rng=rng,
                verbose=False,
                mock=True,
            )
            assert decision.choice_number is not None
            choices.append(decision.choice_number)
        results.append(choices)
    assert results[0] == results[1]


def test_cli_argument_parsing_supports_mock_flag() -> None:
    parser = build_parser()
    args = parser.parse_args(["--mock", "--seed", "7"])
    assert args.mock is True


def test_cli_argument_parsing_supports_pure_play_alias() -> None:
    parser = build_parser()
    args = parser.parse_args(["--pure-play"])
    assert args.bug_detection is False


def test_cli_argument_parsing_decklists_default_and_opt_out() -> None:
    parser = build_parser()
    default_args = parser.parse_args([])
    assert default_args.decklists is True

    opt_out_args = parser.parse_args(["--no-decklists"])
    assert opt_out_args.decklists is False


def test_format_deck_preamble_skips_metadata_and_labels_players(tmp_path: Path) -> None:
    p1_path = tmp_path / "alice.dck"
    p1_path.write_text(
        "[metadata]\n"
        "Name=Alice's Deck\n"
        "\n"
        "[Main]\n"
        "20 Mountain\n"
        "4 Lightning Bolt\n"
        "\n"
        "[Sideboard]\n"
        "2 Smash to Smithereens\n",
        encoding="utf-8",
    )
    p2_path = tmp_path / "bob.dck"
    p2_path.write_text("[metadata]\nName=Bob\n\n[Main]\n40 Forest\n", encoding="utf-8")

    preamble = format_deck_preamble([("P1", p1_path), ("P2", p2_path)])

    assert "Player decks:" in preamble
    assert "P1 (deck: alice.dck):" in preamble
    assert "  20 Mountain" in preamble
    assert "  4 Lightning Bolt" in preamble
    assert "  [Sideboard]" in preamble
    assert "  2 Smash to Smithereens" in preamble
    assert "P2 (deck: bob.dck):" in preamble
    assert "  40 Forest" in preamble
    # Metadata lines must NOT leak into the preamble.
    assert "Name=" not in preamble
    assert "[metadata]" not in preamble


def test_format_deck_preamble_returns_empty_when_no_decks() -> None:
    assert format_deck_preamble([]) == ""


def test_build_intro_section_includes_role_scenario_and_decks() -> None:
    intro = build_intro_section(
        scenario="Reproduce a counter-then-bolt sequence.",
        goal="Win on turn 4.",
        bug_detection=True,
        deck_preamble="Player decks:\n\nP1 (deck: a.dck):\n  4 Lightning Bolt",
        rules_paths=["/tmp/rules.md"],
    )

    assert "deterministic MTG game used for engine bug finding" in intro
    assert "STOP when the log, state, or menu appears to violate MTG rules" in intro
    assert "engine only presents VALID, LEGAL actions" in intro
    assert "Scenario to reproduce: Reproduce a counter-then-bolt sequence." in intro
    assert "Goal directive: Win on turn 4." in intro
    assert "Player decks:" in intro
    assert "4 Lightning Bolt" in intro
    assert "/tmp/rules.md" in intro


def test_build_intro_section_pure_play_drops_bug_detection_language() -> None:
    intro = build_intro_section(bug_detection=False)
    assert "Pure play mode is enabled" in intro
    assert "BUG DETECTION" not in intro
    assert "STOP when the log" not in intro


def test_build_choice_prompt_threads_deck_preamble_through() -> None:
    prompt = build_choice_prompt(
        {},
        ["pass priority"],
        "",
        deck_preamble="Player decks:\n\nP1 (deck: a.dck):\n  4 Lightning Bolt",
    )
    assert "Player decks:" in prompt
    assert "4 Lightning Bolt" in prompt
    assert "engine only presents VALID, LEGAL actions" in prompt
