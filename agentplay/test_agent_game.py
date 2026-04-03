from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from agentplay.agent_game import _query_agent, build_parser
from agentplay.engine import GameEngine
from agentplay.prompts import build_choice_prompt, parse_agent_response


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
    )

    assert "Goal directive: Win this turn." in prompt
    assert "Turn: 3 | Phase: Pre-combat Main | Step: Main1" in prompt
    assert "[0] pass" in prompt
    assert "[1] play Mountain" in prompt
    assert "[2] cast Lightning Bolt" in prompt
    assert "Recent game log:" in prompt
    assert "Alice drew a card" in prompt
    assert "Bob: life 15" in prompt
    assert "2 hidden card(s)" in prompt
    assert "Output ONLY the choice number on the last line." in prompt


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
            "--max-turns",
            "12",
            "--",
            "ignored.dck",
        ]
    )

    assert args.seed == 7
    assert args.mode == "agent-vs-agent"
    assert args.game_dir == "foo.game"
    assert args.puzzle == "puzzles/example.pzl"
    assert args.goal == "Find lethal"
    assert args.max_turns == 12
    assert args.mtg_args == ["--", "ignored.dck"]


def test_query_agent_retries_then_succeeds() -> None:
    responses = [
        SimpleNamespace(returncode=0, stdout="not a number", stderr=""),
        SimpleNamespace(returncode=1, stdout="", stderr="proxy unavailable"),
        SimpleNamespace(returncode=0, stdout="2\n", stderr=""),
    ]

    with patch("agentplay.agent_game.subprocess.run", side_effect=responses) as run_mock:
        choice, raw = _query_agent("prompt", 3, verbose=False)

    assert choice == 2
    assert raw == "2"
    assert run_mock.call_count == 3


def test_query_agent_fails_after_three_invalid_attempts() -> None:
    responses = [
        SimpleNamespace(returncode=0, stdout="invalid", stderr=""),
        SimpleNamespace(returncode=0, stdout="9", stderr=""),
        SimpleNamespace(returncode=1, stdout="", stderr="boom"),
    ]

    with patch("agentplay.agent_game.subprocess.run", side_effect=responses):
        with pytest.raises(RuntimeError):
            _query_agent("prompt", 3, verbose=False)
