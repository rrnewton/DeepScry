"""Persistent agent (LLM) session abstractions for `agent_play.py --mode persistent`.

The legacy stop-and-go agentplay flow spawns a fresh `claude -p <prompt>`
subprocess at every decision point. That works but pays the entire prompt cost
on every turn (same intro, same rules references, same growing interleaved
history). The persistent path wants to keep ONE conversation alive so the LLM's
own context is reused — cheaper, faster, and able to carry strategic threads
across turns.

This module defines a small `AgentSession` protocol and a few implementations:

* `ClaudeResumeSession` — `claude --resume <session-id>` after the first call,
  so each turn only sends the delta (the new game log + the new decision menu).
  This is the production path. It depends on the local `claude` CLI exposing
  `--resume` (or `--session-id`); if neither is available the constructor will
  fall back to `ClaudeOneShotSession` and emit a warning.

* `ClaudeOneShotSession` — drop-in compatible behaviour with the legacy
  stop-and-go mode: every `ask()` re-invokes `claude -p <full_prompt>`. Used as
  the safe fallback so the persistent harness still works on machines where
  `claude --resume` is broken.

* `MockSession` — deterministic random/zero-token session for tests.

All three obey the same contract: `ask(prompt_text, valid_choice_count)` returns
an `AgentDecision` (defined in `agentplay/lib/prompts.py`), with the same retry
semantics the legacy `_query_agent` implements.
"""

from __future__ import annotations

import os
import random
import shlex
import subprocess
import sys
import uuid
from dataclasses import dataclass
from typing import Protocol

from .prompts import AgentDecision, parse_agent_decision


@dataclass
class _AskResult:
    """Internal: raw outcome of a single subprocess attempt."""

    returncode: int
    stdout: str
    stderr: str

    @property
    def response_text(self) -> str:
        return self.stdout.strip() or self.stderr.strip()


class AgentSession(Protocol):
    """A single agent's persistent conversation session.

    One session per player keeps each player's reasoning thread independent and
    enforces information-independence in the same way the legacy mode does
    (one `claude` invocation per player decision; controllers never share
    state).
    """

    def ask(
        self,
        prompt_text: str,
        valid_choice_count: int,
        *,
        bug_detection: bool,
    ) -> AgentDecision:
        """Send `prompt_text` and return a parsed decision.

        Implementations are expected to retry up to a small number of times on
        invalid responses (same 3-attempt loop the legacy `_query_agent` uses)
        and finally raise `RuntimeError` if no valid decision can be parsed.
        """
        ...

    def close(self) -> None:
        """Release any underlying resources. Safe to call multiple times."""
        ...


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _retry_warning(prompt: str, last_error: str, choice_count: int, bug_detection: bool) -> str:
    """Build the same retry banner the legacy `_query_agent` uses.

    Mirrors `agent_game.py::_query_agent`'s retry text so persistent and
    stop-and-go modes coach the model with identical instructions.
    """

    valid_response = (
        f"Valid responses are either STOP with a BUG_REPORT, or a choice number from 0 to {choice_count}."
        if bug_detection
        else f"Valid choices are 0 to {choice_count}."
    )
    final_line = (
        "If choosing, the final line MUST be only a single number. "
        "If stopping, write STOP and BUG_REPORT instead."
        if bug_detection
        else f"You MUST respond with ONLY a single number between 0 and {choice_count} on the final line."
    )
    return (
        prompt
        + f"\n\nWARNING: Your previous response was invalid ({last_error}). "
        + valid_response
        + " "
        + final_line
    )


def _parse_with_validation(
    response_text: str,
    *,
    valid_choice_count: int,
    bug_detection: bool,
) -> tuple[AgentDecision | None, str]:
    """Parse a raw response into a decision. Returns (decision, error_msg)."""

    try:
        decision = parse_agent_decision(response_text, bug_detection=bug_detection)
    except ValueError as exc:
        return None, str(exc)
    if decision.stopped_for_bug:
        return decision, ""
    n = decision.choice_number
    if n is None:
        return None, "no choice number parsed"
    if 0 <= n <= valid_choice_count:
        return decision, ""
    return None, f"parsed choice {n} is outside valid range 0..{valid_choice_count}"


# ---------------------------------------------------------------------------
# ClaudeOneShotSession — same shape as legacy stop-and-go
# ---------------------------------------------------------------------------


class ClaudeOneShotSession:
    """Re-invokes `claude -p <prompt>` per turn (no state reuse).

    This matches the legacy `_query_agent` behaviour from `agent_game.py`. It's
    the safe fallback for environments where `claude --resume` doesn't work,
    and it's also the ground-truth oracle the persistent path can be compared
    against on identical seeds.
    """

    def __init__(
        self,
        *,
        claude_args: list[str] | None = None,
        verbose: bool = False,
        attempts: int = 3,
        label: str = "",
    ) -> None:
        self.claude_args = list(claude_args or [])
        self.verbose = verbose
        self.attempts = attempts
        self.label = label

    def ask(
        self,
        prompt_text: str,
        valid_choice_count: int,
        *,
        bug_detection: bool,
    ) -> AgentDecision:
        last_error = "no agent attempts made"
        for attempt in range(1, self.attempts + 1):
            retry_prompt = (
                prompt_text
                if attempt == 1
                else _retry_warning(prompt_text, last_error, valid_choice_count, bug_detection)
            )
            result = self._invoke_claude(retry_prompt, attempt)
            if result.returncode != 0:
                last_error = f"claude exited with code {result.returncode}: {result.response_text}"
                if self.verbose:
                    print(f"[retry {attempt}/{self.attempts}] {last_error}", file=sys.stderr)
                continue
            decision, err = _parse_with_validation(
                result.response_text,
                valid_choice_count=valid_choice_count,
                bug_detection=bug_detection,
            )
            if decision is not None:
                return decision
            last_error = err
            if self.verbose:
                print(f"[retry {attempt}/{self.attempts}] {last_error}", file=sys.stderr)
        raise RuntimeError(
            f"failed to get a valid Claude choice after {self.attempts} attempts: {last_error}"
        )

    def _invoke_claude(self, prompt: str, attempt: int) -> _AskResult:
        cmd = ["claude"] + self.claude_args + ["-p", prompt]
        if self.verbose:
            display_cmd = ["claude"] + self.claude_args + ["-p", f"<prompt {len(prompt)} chars>"]
            tag = f"[{self.label}] " if self.label else ""
            print(
                f"{tag}[verbose] attempt {attempt}/{self.attempts}: $ {shlex.join(display_cmd)}",
                file=sys.stderr,
                flush=True,
            )
        completed = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if self.verbose:
            print(
                f"[verbose] claude exit={completed.returncode} "
                f"stdout={len(completed.stdout)}B stderr={len(completed.stderr)}B",
                file=sys.stderr,
                flush=True,
            )
        return _AskResult(
            returncode=completed.returncode,
            stdout=completed.stdout,
            stderr=completed.stderr,
        )

    def close(self) -> None:
        # Stateless wrapper — nothing to release.
        pass


# ---------------------------------------------------------------------------
# ClaudeResumeSession — persistent conversation via `claude --resume`
# ---------------------------------------------------------------------------


class ClaudeResumeSession:
    """Persistent Claude conversation pinned to a stable session id.

    The first `ask()` call invokes `claude --session-id <uuid> -p <prompt>` to
    establish the session. Subsequent calls invoke
    `claude --resume <uuid> -p <delta_prompt>`.

    The CALLER is responsible for sending only the DELTA on follow-up turns
    (not the full intro / interleaved history) because the model already has
    the prior context. To make this drop-in for an existing prompt builder, the
    constructor takes an optional `intro_text` that's sent only on the first
    turn; subsequent turns get the prompt verbatim.

    If the local `claude` CLI doesn't support `--session-id` / `--resume` we
    fall back to one-shot mode and emit a warning. Detection is best-effort
    via `claude --help` parsing.
    """

    def __init__(
        self,
        *,
        intro_text: str,
        claude_args: list[str] | None = None,
        verbose: bool = False,
        attempts: int = 3,
        label: str = "",
    ) -> None:
        self.intro_text = intro_text
        self.claude_args = list(claude_args or [])
        self.verbose = verbose
        self.attempts = attempts
        self.label = label
        self.session_id: str | None = None
        self.first_call: bool = True
        self._fallback: ClaudeOneShotSession | None = None

        if not self._supports_resume():
            if verbose:
                print(
                    "[agent_session] `claude --resume` not detected on this CLI; "
                    "falling back to one-shot mode.",
                    file=sys.stderr,
                )
            self._fallback = ClaudeOneShotSession(
                claude_args=claude_args,
                verbose=verbose,
                attempts=attempts,
                label=label,
            )

    def ask(
        self,
        prompt_text: str,
        valid_choice_count: int,
        *,
        bug_detection: bool,
    ) -> AgentDecision:
        if self._fallback is not None:
            # Compose the same prompt the one-shot path would have built: the
            # full prompt text already contains the intro, so just forward it.
            return self._fallback.ask(
                prompt_text,
                valid_choice_count,
                bug_detection=bug_detection,
            )

        last_error = "no agent attempts made"
        for attempt in range(1, self.attempts + 1):
            retry_prompt = (
                prompt_text
                if attempt == 1
                else _retry_warning(prompt_text, last_error, valid_choice_count, bug_detection)
            )

            if self.first_call:
                self.session_id = str(uuid.uuid4())
                # The very first message includes the full intro + decision
                # prompt so Claude has the system context.
                full = self._compose_first_prompt(retry_prompt)
                cmd = ["claude", "--session-id", self.session_id] + self.claude_args + ["-p", full]
            else:
                # The intro is already in the conversation; just send the delta.
                cmd = ["claude", "--resume", self.session_id] + self.claude_args + ["-p", retry_prompt]

            result = self._run(cmd, prompt=retry_prompt, attempt=attempt)
            if result.returncode != 0:
                last_error = f"claude exited with code {result.returncode}: {result.response_text}"
                if self.verbose:
                    print(f"[retry {attempt}/{self.attempts}] {last_error}", file=sys.stderr)
                continue
            decision, err = _parse_with_validation(
                result.response_text,
                valid_choice_count=valid_choice_count,
                bug_detection=bug_detection,
            )
            if decision is not None:
                # Mark the session as "live" so subsequent asks use --resume.
                self.first_call = False
                return decision
            last_error = err
            if self.verbose:
                print(f"[retry {attempt}/{self.attempts}] {last_error}", file=sys.stderr)
        raise RuntimeError(
            f"failed to get a valid Claude choice after {self.attempts} attempts: {last_error}"
        )

    def _compose_first_prompt(self, prompt_text: str) -> str:
        # The persistent prompt builder is expected to include the intro every
        # turn for stop-and-go parity. The intro is therefore already in
        # `prompt_text`; nothing extra to prepend on the first call.
        return prompt_text

    def _run(self, cmd: list[str], *, prompt: str, attempt: int) -> _AskResult:
        if self.verbose:
            display = list(cmd)
            for i, tok in enumerate(display):
                if tok == "-p" and i + 1 < len(display):
                    display[i + 1] = f"<prompt {len(prompt)} chars>"
            tag = f"[{self.label}] " if self.label else ""
            print(
                f"{tag}[verbose] attempt {attempt}/{self.attempts}: $ {shlex.join(display)}",
                file=sys.stderr,
                flush=True,
            )
        completed = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if self.verbose:
            print(
                f"[verbose] claude exit={completed.returncode} "
                f"stdout={len(completed.stdout)}B stderr={len(completed.stderr)}B",
                file=sys.stderr,
                flush=True,
            )
        return _AskResult(
            returncode=completed.returncode,
            stdout=completed.stdout,
            stderr=completed.stderr,
        )

    def close(self) -> None:
        # The session id lives only in claude's internal store; nothing to do.
        if self._fallback is not None:
            self._fallback.close()

    @staticmethod
    def _supports_resume() -> bool:
        """Cheap probe: run `claude --help` and look for `--resume`/`--session-id`.

        Falls back to True if the probe itself succeeds but yields nothing
        recognizable, on the assumption that real environments will have it.
        Returns False on probe failure (e.g. claude not installed) — the
        caller will then surface a clear error when it actually invokes claude.
        """

        # Allow override for tests / forced fallback.
        env = os.environ.get("AGENTPLAY_FORCE_ONESHOT", "").strip()
        if env in ("1", "true", "yes"):
            return False

        try:
            completed = subprocess.run(
                ["claude", "--help"],
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )
        except (FileNotFoundError, subprocess.TimeoutExpired):
            return False
        text = (completed.stdout or "") + "\n" + (completed.stderr or "")
        return ("--resume" in text) and ("--session-id" in text or "--session" in text)


# ---------------------------------------------------------------------------
# MockSession — deterministic random for tests
# ---------------------------------------------------------------------------


class MockSession:
    """DEPRECATED stub kept only for backwards-compatible imports.

    Historically `agent_game.py --mock` plumbed a Python-side
    `random.Random(seed)` through this class to make zero-token "fake agent"
    decisions. That added a third independent RNG to the system (alongside
    the engine RNG and the per-controller RNG), which silently caused the
    same `--seed --mock` invocation to produce three DIFFERENT games across
    the stop-and-go / persistent / WASM drivers.

    The Python RNG path has been removed: `--mock` now collapses to the
    engine-side `RandomController` (seeded via the centralized
    `derive_player_seed`), and the three drivers are byte-identical for the
    same seed. This stub remains so older imports don't break, but `ask()`
    refuses to run — any caller that gets here is on a code path that needs
    to be migrated to the engine-side controller.
    """

    def __init__(self, *, seed: int = 42, label: str = "mock") -> None:
        del seed  # intentionally unused — see class docstring
        self.label = label

    def ask(
        self,
        prompt_text: str,
        valid_choice_count: int,
        *,
        bug_detection: bool,
    ) -> AgentDecision:
        del prompt_text, valid_choice_count, bug_detection
        raise NotImplementedError(
            "MockSession.ask() was removed: --mock now uses engine-side "
            "RandomController instead of a Python random.Random. If you "
            "reach this method, your driver is still trying to feed mock "
            "decisions through Python — switch to the engine-side path so "
            "all three drivers (stop-and-go / persistent / WASM) stay "
            "byte-identical for the same seed."
        )

    def close(self) -> None:
        pass
