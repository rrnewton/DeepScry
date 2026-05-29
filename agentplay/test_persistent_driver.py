"""Unit tests for the persistent agentplay driver pieces.

Covers `agentplay/lib/game_process.py` (parsing logic only — we don't spawn
a real subprocess here; that's covered by the smoke scripts in `debug/`)
and `agentplay/lib/agent_session.py`.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from agentplay.lib.agent_session import (
    ClaudeOneShotSession,
    ClaudeResumeSession,
    _parse_with_validation,
    _retry_warning,
)
from agentplay.lib.game_process import (
    READY_MARKER,
    ChoicePoint,
    GameOver,
    NativeTuiProcess,
)
from agentplay.lib.prompts import AgentDecision


# ---------------------------------------------------------------------------
# game_process.py: menu / log / snapshot helpers
# ---------------------------------------------------------------------------


def _make_proc(tmp_path: Path) -> NativeTuiProcess:
    """Build a NativeTuiProcess WITHOUT calling start() so we can poke at
    its internal parsing helpers in isolation."""

    return NativeTuiProcess(
        binary=Path("/bin/true"),
        mtg_args=[],
        game_dir=tmp_path,
        seed=1,
        p1_controller="tui",
        p2_controller="heuristic",
    )


def test_extract_menu_basic(tmp_path: Path) -> None:
    proc = _make_proc(tmp_path)
    proc._all_lines = [
        "Some game log line",
        "[Your_Main1] Human1 available actions:",
        "  [0] pass",
        "  [1] play Mountain",
        "  [2] cast Lightning Bolt",
        READY_MARKER,
    ]
    choices, player, context = proc._extract_menu()
    # Pass is dropped; we only return the actionable choices.
    assert choices == ["play Mountain", "cast Lightning Bolt"]
    assert player == "p1"
    assert context == "Your_Main1"


def test_extract_menu_p2_no_context(tmp_path: Path) -> None:
    proc = _make_proc(tmp_path)
    proc._all_lines = [
        "Human2 available actions:",
        "  [0] pass",
        "  [1] play Forest",
    ]
    choices, player, context = proc._extract_menu()
    assert choices == ["play Forest"]
    assert player == "p2"
    assert context is None


def test_extract_menu_picks_LAST_block(tmp_path: Path) -> None:
    """If the engine has emitted multiple `available actions:` headers (e.g.
    after a re-prompt loop), we should always parse the most recent one."""

    proc = _make_proc(tmp_path)
    proc._all_lines = [
        "Human1 available actions:",
        "  [0] pass",
        "  [1] play Mountain",  # OLD menu
        "Some intermediate game-log noise",
        "[Your_Combat_Begin] Human1 available actions:",
        "  [0] pass",
        "  [1] cast Giant Growth",  # NEW menu (the one we want)
    ]
    choices, _player, context = proc._extract_menu()
    assert choices == ["cast Giant Growth"]
    assert context == "Your_Combat_Begin"


def test_log_filter_drops_menu_scaffolding(tmp_path: Path) -> None:
    """`_maybe_record_log_line` should keep real game-log content and drop
    menu scaffolding / interactive prompts."""

    proc = _make_proc(tmp_path)
    inputs = [
        "Human1 plays Mountain",
        "Human1 available actions:",  # dropped (menu header)
        "  [0] pass",                   # dropped (menu item)
        "  [1] cast Lightning Bolt",    # dropped (menu item)
        READY_MARKER,                   # dropped (control marker)
        "Choose action (0-1, or ? for help): ",  # dropped (interactive prompt)
        "  ==> [Your_Main1] Priority Human1: life 20, Main1",  # KEPT
        "Human1 casts Lightning Bolt",
    ]
    for line in inputs:
        proc._all_lines.append(line)
        proc._maybe_record_log_line(line)
    assert "Human1 plays Mountain" in proc._log_lines
    assert "Human1 casts Lightning Bolt" in proc._log_lines
    assert any("Priority Human1" in line for line in proc._log_lines)
    # Menu scaffolding should NOT have leaked in.
    assert not any("available actions" in line for line in proc._log_lines)
    assert not any(line.strip() == READY_MARKER for line in proc._log_lines)
    assert not any(line.strip().startswith("Choose action") for line in proc._log_lines)


def test_extract_turn_number_prefers_inner(tmp_path: Path) -> None:
    proc = _make_proc(tmp_path)
    snap = {
        "turn_number": 5,  # outer (set at snapshot time)
        "game_state": {"turn": {"turn_number": 7}},  # inner (live state)
    }
    assert proc._extract_turn_number(snap) == 7
    # Falls back to outer when inner is missing.
    assert proc._extract_turn_number({"turn_number": 4}) == 4
    # None when neither.
    assert proc._extract_turn_number({}) is None


# ---------------------------------------------------------------------------
# agent_session.py: parsing/retry helpers
# ---------------------------------------------------------------------------


def test_retry_warning_includes_constraints() -> None:
    base = "Pick a choice."
    warn = _retry_warning(base, "bad parse", choice_count=4, bug_detection=True)
    assert "Pick a choice." in warn
    assert "BUG_REPORT" in warn
    assert "0 to 4" in warn

    warn = _retry_warning(base, "bad parse", choice_count=4, bug_detection=False)
    assert "BUG_REPORT" not in warn
    assert "0 to 4" in warn or "0 and 4" in warn


def test_parse_with_validation_in_range() -> None:
    decision, err = _parse_with_validation("2", valid_choice_count=3, bug_detection=False)
    assert err == ""
    assert decision is not None
    assert decision.choice_number == 2


def test_parse_with_validation_out_of_range() -> None:
    decision, err = _parse_with_validation("99", valid_choice_count=3, bug_detection=False)
    assert decision is None
    assert "99" in err and "0..3" in err


def test_parse_with_validation_bug_report() -> None:
    response = "STOP\nBUG_REPORT: engine offered an illegal play"
    decision, err = _parse_with_validation(response, valid_choice_count=3, bug_detection=True)
    assert err == ""
    assert decision is not None
    assert decision.stopped_for_bug


# ---------------------------------------------------------------------------
# MockSession-based tests removed in commit fixing mtg-460.
#
# Historically these tests exercised `MockSession.ask()` (a Python-side
# `random.Random(seed)` path). That code path was removed in 61e06688
# (2026-05-15) in favor of the engine-side `RandomController` so all three
# agentplay drivers (stop-and-go / persistent / WASM) produce byte-identical
# games for the same seed. `MockSession.ask()` now raises NotImplementedError
# as a deprecation marker, so there is nothing left to unit-test here — the
# equivalent coverage now lives in the Rust `RandomController` tests and the
# driver-equivalence integration tests.
# ---------------------------------------------------------------------------


# ---------------------------------------------------------------------------
# ClaudeResumeSession: --resume probe
# ---------------------------------------------------------------------------


def test_claude_resume_session_force_oneshot_env(monkeypatch: pytest.MonkeyPatch) -> None:
    """The AGENTPLAY_FORCE_ONESHOT env var should cause ClaudeResumeSession to
    fall back to one-shot mode regardless of what `claude --help` says."""

    monkeypatch.setenv("AGENTPLAY_FORCE_ONESHOT", "1")
    sess = ClaudeResumeSession(intro_text="Hello", verbose=False)
    assert sess._fallback is not None
    assert isinstance(sess._fallback, ClaudeOneShotSession)


def test_claude_oneshot_session_close_is_safe() -> None:
    sess = ClaudeOneShotSession()
    sess.close()  # should not raise
    sess.close()  # idempotent


# ---------------------------------------------------------------------------
# Integration smoke: GameOver dataclass
# ---------------------------------------------------------------------------


def test_game_over_dataclass_fields() -> None:
    over = GameOver(fresh_output="goodbye", log_lines=["a", "b"], return_code=0, reason="exit")
    assert over.fresh_output == "goodbye"
    assert over.log_lines == ["a", "b"]
    assert over.return_code == 0


def test_choice_point_dataclass_fields() -> None:
    cp = ChoicePoint(
        player="p1",
        choices=["play Mountain"],
        snapshot={"turn_number": 1},
        log_lines=["foo"],
        fresh_output="foo\n",
        choice_context="Your_Main1",
        turn_number=1,
    )
    assert cp.player == "p1"
    assert cp.choices == ["play Mountain"]
    assert cp.choice_context == "Your_Main1"
    assert cp.turn_number == 1
