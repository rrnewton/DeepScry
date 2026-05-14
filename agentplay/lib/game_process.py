"""Persistent native `mtg tui` subprocess wrappers for agentplay.

This module is the engine-side half of `agentplay/agent_game.py --mode persistent`.
Whereas the legacy stop-and-go mode (`agentplay/lib/engine.py`) re-runs the entire
game from scratch on every decision via `--p1=fixed --p2=fixed` with growing
script files, persistent mode keeps ONE long-running `mtg tui` subprocess alive
and feeds choices to its `InteractiveController` via stdin.

Protocol summary
----------------

A `GameProcess` is a black box that, after `start()`, hands the caller a
`ChoicePoint | GameOver` object describing what the engine wants next. The
caller responds with `send_choice(text)`; the engine processes it and the
process returns the NEXT `ChoicePoint | GameOver`. Repeat until game over.

The native implementation (`NativeTuiProcess`) relies on the engine-side hook
added in `mtg-engine/src/game/interactive_controller.rs`: when
`--tui-snapshot-path=<PATH>` is passed, the controller writes a fresh JSON
`GameSnapshot` of the current game state and prints the literal marker line
`[AGENTPLAY: ready for input]` immediately before each stdin read. The Python
side waits for that marker, parses the snapshot, and pulls the menu/log out of
the accumulated stdout buffer.

This module is intentionally controller-protocol agnostic — it knows nothing
about `claude` or any LLM. The agent half lives in `agent_session.py`.
"""

from __future__ import annotations

import json
import os
import queue
import re
import subprocess
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Sequence


# Marker line printed by the engine's InteractiveController immediately
# before each `read_line` of stdin (gated on `--tui-snapshot-path` being
# set). See `mtg-engine/src/game/interactive_controller.rs`
# `maybe_write_snapshot()`.
READY_MARKER = "[AGENTPLAY: ready for input]"

# Substring present in the per-prompt header line printed by
# `format_choice_menu` (controller.rs:41). We use this to identify menu
# blocks in the captured stdout so we can extract the choice list.
MENU_HEADER_TOKEN = "available actions:"

# Markers that indicate the game has ended. Any of these substrings in
# the engine output means we should stop pumping choices.
GAME_OVER_MARKERS = (
    "=== Game Over ===",
    "Winner:",
    "Game ended in a draw",
    " wins!",
)

# Matches the optional `[Your_Main1]` / `[Their_EndStep | Lightning Bolt
# on stack]` context flavour that the rich/fixed controller prepends to
# its "available actions:" header. Mirrors `_CHOICE_CONTEXT_RE` in
# agentplay/lib/engine.py.
_CHOICE_CONTEXT_RE = re.compile(r"^\s*\[([^\]]+)\]\s+\S+\s+available actions:\s*$")


@dataclass
class ChoicePoint:
    """One decision point surfaced by the engine.

    `choices` are the action descriptions WITHOUT the implicit "pass" entry
    at index 0 — matching the convention used by `agentplay/lib/engine.py`'s
    `_extract_choices` so the prompt-building code can be shared.

    `snapshot` is the parsed JSON `GameSnapshot` written by the engine just
    before the prompt (i.e. the same shape `engine.py:parse_snapshot`
    produces, with the structured `game_state` object inside).

    `log_lines` is the cumulative game log captured to this point — the
    caller is expected to dedup against what they've already shown (same
    pattern as `_new_log_tail_lines`).
    """

    player: str  # "p1" or "p2"
    choices: list[str]
    snapshot: dict[str, Any]
    log_lines: list[str]
    # Raw text of all stdout/stderr lines emitted SINCE the previous
    # `ChoicePoint` (or since startup for the first one). Useful for
    # callers that want to display "what just happened" verbatim.
    fresh_output: str
    choice_context: str | None = None
    turn_number: int | None = None


@dataclass
class GameOver:
    """Terminal state. The engine subprocess has exited or printed a
    Game Over banner."""

    fresh_output: str
    log_lines: list[str]
    return_code: int | None = None
    reason: str | None = None


@dataclass
class _StreamPump:
    """Background thread that drains a subprocess pipe into a queue."""

    pipe: Any
    name: str
    sink: "queue.Queue[tuple[str, str | None]]"
    thread: threading.Thread = field(init=False)

    def __post_init__(self) -> None:
        self.thread = threading.Thread(target=self._run, name=f"pump-{self.name}", daemon=True)
        self.thread.start()

    def _run(self) -> None:
        try:
            for line in iter(self.pipe.readline, ""):
                if line == "":
                    break
                self.sink.put((self.name, line.rstrip("\n")))
        finally:
            # Sentinel: stream closed.
            self.sink.put((self.name, None))


class NativeTuiProcess:
    """A persistent `mtg tui --p1=tui --p2=<X>` subprocess.

    Lifecycle:

        proc = NativeTuiProcess(
            binary=Path("target/release/mtg"),
            mtg_args=["decks/simple_bolt.dck", "decks/simple_bolt.dck"],
            game_dir=Path("agentplay/001.game"),
            seed=42,
            p1_controller="tui",
            p2_controller="heuristic",
        )
        first = proc.start()                # ChoicePoint | GameOver
        if isinstance(first, ChoicePoint):
            nxt = proc.send_choice(first.player, "play Mountain")
            ...
        proc.close()

    The class spawns the subprocess with text-mode pipes and reads stdout +
    stderr off background threads so we never deadlock on a full pipe.
    """

    def __init__(
        self,
        *,
        binary: Path,
        mtg_args: Sequence[str],
        game_dir: Path,
        seed: int,
        p1_controller: str,
        p2_controller: str,
        log_tail: int = 1000,
        cardsfolder: Path | None = None,
        cwd: Path | None = None,
        verbose: bool = False,
    ) -> None:
        self.binary = binary
        self.mtg_args = list(mtg_args)
        self.game_dir = game_dir
        self.seed = seed
        self.p1_controller = p1_controller
        self.p2_controller = p2_controller
        self.log_tail = log_tail
        self.cardsfolder = cardsfolder
        self.cwd = cwd
        self.verbose = verbose

        # Path the engine writes its per-choice JSON snapshot to. Lives
        # inside the game_dir so that all per-game artefacts stay grouped.
        self.snapshot_path = self.game_dir / "snapshot.json"
        # Persistent transcript of everything the engine printed (mirror of
        # what stop-and-go mode writes to `game.log`).
        self.transcript_path = self.game_dir / "engine_stdout.log"

        self.proc: subprocess.Popen[str] | None = None
        self._stdout_pump: _StreamPump | None = None
        self._stderr_pump: _StreamPump | None = None
        self._sink: "queue.Queue[tuple[str, str | None]]" = queue.Queue()
        # All stdout lines we've ever pulled off the queue, in order.
        self._all_lines: list[str] = []
        # Index into self._all_lines marking the start of the "fresh"
        # window since the last ChoicePoint we emitted.
        self._fresh_window_start: int = 0
        # Subset of self._all_lines that look like real game-log content
        # (i.e. not menu/prompt scaffolding). This is what the AI prompt
        # builder consumes as the cumulative log.
        self._log_lines: list[str] = []
        # Index into self._log_lines marking the start of the new log
        # window since the last ChoicePoint we emitted.
        self._log_window_start: int = 0
        self._closed_streams: set[str] = set()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def start(self) -> ChoicePoint | GameOver:
        """Spawn the subprocess and read up to the first decision (or game
        over)."""

        if self.proc is not None:
            raise RuntimeError("NativeTuiProcess.start() called twice")

        self.game_dir.mkdir(parents=True, exist_ok=True)
        # Wipe any stale snapshot from a prior run so we never accidentally
        # consume someone else's state.
        if self.snapshot_path.exists():
            self.snapshot_path.unlink()

        cmd: list[str] = [
            str(self.binary),
            "tui",
            *self.mtg_args,
            f"--p1={self.p1_controller}",
            f"--p2={self.p2_controller}",
            f"--seed={self.seed}",
            f"--log-tail={self.log_tail}",
            f"--tui-snapshot-path={self.snapshot_path}",
            "--verbosity=verbose",
        ]
        if self.verbose:
            print(f"[persistent] $ {' '.join(cmd)}")

        env = dict(os.environ)
        if self.cardsfolder is not None:
            env["CARDSFOLDER"] = str(self.cardsfolder)
        # Suppress INFO-level Rust log lines; keep WARN and ERROR visible.
        # Same default the legacy engine uses.
        env.setdefault("RUST_LOG", "warn")

        self.proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,  # line-buffered
            cwd=str(self.cwd) if self.cwd else None,
            env=env,
        )
        # Start background stream pumps. Both share the same sink queue.
        assert self.proc.stdout is not None
        assert self.proc.stderr is not None
        self._stdout_pump = _StreamPump(self.proc.stdout, "stdout", self._sink)
        self._stderr_pump = _StreamPump(self.proc.stderr, "stderr", self._sink)

        return self._wait_for_next_event()

    def send_choice(self, expected_player: str, choice_text: str) -> ChoicePoint | GameOver:
        """Send `choice_text` over stdin and read up to the next event.

        `expected_player` is informational — it's what the caller thinks the
        engine is asking. We don't enforce it because the engine's own
        sequencing is authoritative; the caller can compare against the
        previously-returned `ChoicePoint.player` if it wants to assert.
        """

        if self.proc is None or self.proc.stdin is None:
            raise RuntimeError("send_choice() called before start() (or after close())")

        # Reset the fresh-window cursors so the next ChoicePoint we emit
        # only includes what happened AFTER this choice.
        self._fresh_window_start = len(self._all_lines)
        self._log_window_start = len(self._log_lines)

        # Engine reads via `read_line`; an unterminated line would block.
        payload = choice_text.strip() + "\n"
        try:
            self.proc.stdin.write(payload)
            self.proc.stdin.flush()
        except (BrokenPipeError, OSError) as exc:
            # The subprocess probably exited. Drain remaining output and
            # surface a GameOver.
            return self._drain_to_game_over(reason=f"stdin write failed: {exc}")

        return self._wait_for_next_event()

    def close(self) -> None:
        """Best-effort shutdown of the subprocess and pumps."""

        if self.proc is None:
            return
        # Persist the transcript we accumulated.
        try:
            self.transcript_path.write_text("\n".join(self._all_lines) + "\n", encoding="utf-8")
        except OSError:
            pass

        try:
            if self.proc.stdin is not None:
                self.proc.stdin.close()
        except Exception:
            pass
        try:
            self.proc.terminate()
            self.proc.wait(timeout=5)
        except Exception:
            try:
                self.proc.kill()
            except Exception:
                pass
        self.proc = None

    # ------------------------------------------------------------------
    # Internals
    # ------------------------------------------------------------------

    def _wait_for_next_event(self, timeout: float = 600.0) -> ChoicePoint | GameOver:
        """Pump stdout/stderr until we see the READY_MARKER or game-over.

        Returns either a `ChoicePoint` (engine is blocked on stdin) or a
        `GameOver` (engine has exited or printed a terminal banner).

        `timeout` caps how long we'll wait without receiving any output —
        a safety belt against hangs. The engine should always either prompt
        again or exit.
        """

        deadline = time.monotonic() + timeout
        marker_seen = False
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RuntimeError(
                    f"NativeTuiProcess: timed out waiting {timeout:.0f}s for next event "
                    f"(last lines: {self._all_lines[-5:]!r})"
                )

            try:
                stream, line = self._sink.get(timeout=min(remaining, 1.0))
            except queue.Empty:
                # No new output. Check if the process exited.
                if self.proc is not None and self.proc.poll() is not None:
                    return self._drain_to_game_over(
                        reason=f"process exited (rc={self.proc.returncode}) without ready marker"
                    )
                continue

            if line is None:
                # Stream closed.
                self._closed_streams.add(stream)
                if (
                    "stdout" in self._closed_streams
                    and "stderr" in self._closed_streams
                ):
                    # Both streams closed — game has fully exited.
                    return self._drain_to_game_over(reason="all streams closed")
                continue

            self._all_lines.append(line)
            self._maybe_record_log_line(line)

            # Detect game-over markers in either stream.
            if any(marker in line for marker in GAME_OVER_MARKERS):
                # Drain a bit more so we capture e.g. "Winner: Player 1"
                # and any closing banner lines, then return.
                return self._drain_to_game_over(reason=f"game-over banner: {line!r}")

            if line.strip() == READY_MARKER:
                marker_seen = True
                # The engine has flushed snapshot.json and is now blocked
                # on stdin. Build the ChoicePoint and return.
                return self._build_choice_point()

            _ = marker_seen  # quiet unused-var lint if loop falls through

    def _build_choice_point(self) -> ChoicePoint:
        """Assemble a ChoicePoint from the accumulated stdout buffer + the
        snapshot the engine just wrote to disk."""

        snapshot = self._read_snapshot()
        # Identify the most-recent "X available actions:" header line, then
        # collect the [N] choice items that follow. Mirrors the parser in
        # `agentplay/lib/engine.py:_extract_choices`.
        choices, choosing_player, choice_context = self._extract_menu()
        fresh_output = "\n".join(self._all_lines[self._fresh_window_start :])
        log_lines = self._log_lines[self._log_window_start :]
        # Pull turn number out of the snapshot if available.
        turn_number = self._extract_turn_number(snapshot)
        return ChoicePoint(
            player=choosing_player or "?",
            choices=choices,
            snapshot=snapshot,
            log_lines=log_lines,
            fresh_output=fresh_output,
            choice_context=choice_context,
            turn_number=turn_number,
        )

    def _extract_menu(self) -> tuple[list[str], str | None, str | None]:
        """Locate the LAST 'available actions:' block in self._all_lines and
        return (choices_excluding_pass, choosing_player, choice_context)."""

        header_index = -1
        header_line = ""
        for index in range(len(self._all_lines) - 1, -1, -1):
            line = self._all_lines[index]
            if MENU_HEADER_TOKEN in line:
                header_index = index
                header_line = line
                break
        if header_index < 0:
            return ([], None, None)

        choices_by_index: dict[int, str] = {}
        for line in self._all_lines[header_index + 1 :]:
            stripped = line.strip()
            if not stripped:
                if choices_by_index:
                    break
                continue
            if not stripped.startswith("["):
                if choices_by_index:
                    break
                continue
            prefix, _, remainder = stripped.partition("]")
            if not remainder:
                continue
            index_text = prefix.lstrip("[")
            if not index_text.isdigit():
                continue
            choices_by_index[int(index_text)] = remainder.strip()

        # Drop the pass choice at index 0 — the prompt builder re-injects
        # it via the same convention `engine.py` uses.
        choices = [text for idx, text in sorted(choices_by_index.items()) if idx > 0]

        # "FixedN/Human1/AI-Heuristic2 available actions:" — the controller
        # name historically ends with "1" (p1) or "2" (p2). This matches
        # `engine.py:_extract_choosing_player`.
        choosing_player = self._extract_choosing_player(header_line)
        choice_context = self._extract_choice_context(header_line)
        return (choices, choosing_player, choice_context)

    @staticmethod
    def _extract_choosing_player(header_line: str) -> str | None:
        # Header looks like:  "[Your_Main1] Human1 available actions:"
        # Strip optional bracketed context first.
        stripped = header_line.strip()
        if stripped.startswith("["):
            close = stripped.find("]")
            if close >= 0:
                stripped = stripped[close + 1 :].strip()
        name_part = stripped.split("available")[0].strip()
        if name_part.endswith("1"):
            return "p1"
        if name_part.endswith("2"):
            return "p2"
        return None

    @staticmethod
    def _extract_choice_context(header_line: str) -> str | None:
        m = _CHOICE_CONTEXT_RE.match(header_line)
        return m.group(1).strip() if m else None

    def _maybe_record_log_line(self, line: str) -> None:
        """Add to `self._log_lines` if `line` looks like real game-log
        content (rather than menu scaffolding or interactive prompt text).

        This is the persistent-mode analogue of `engine.py:_extract_log_tail`,
        which filters out "available actions:" blocks and meta lines on a
        post-hoc basis. Here we apply the same filter incrementally so the
        prompt builder receives exactly the same kind of log content the
        legacy mode produces.
        """

        stripped = line.strip()
        if not stripped:
            self._log_lines.append(line)
            return
        if stripped == READY_MARKER:
            return
        if MENU_HEADER_TOKEN in line:
            return
        # Drop the "  [N] choice text" entries that follow a menu header.
        # We can't easily know we're inside a menu block here (we'd need to
        # carry state), so use the same heuristic as `_extract_log_tail`:
        # any line of the form "  [<digit>] ..." is menu scaffolding.
        if stripped.startswith("[") and "]" in stripped and stripped[1:2].isdigit():
            return
        # Interactive prompt scaffolding.
        if stripped.startswith("Choose action ("):
            return
        if stripped.startswith("Enter choice ("):
            return
        # The "==> [Phase] Priority Player: life X, Step" header is useful
        # context — keep it.
        self._log_lines.append(line)

    def _read_snapshot(self) -> dict[str, Any]:
        if not self.snapshot_path.exists():
            return {}
        try:
            with self.snapshot_path.open("r", encoding="utf-8") as handle:
                return json.load(handle)
        except (OSError, json.JSONDecodeError):
            return {}

    @staticmethod
    def _extract_turn_number(snapshot: dict[str, Any]) -> int | None:
        # GameSnapshot has top-level `turn_number`; `game_state.turn.turn_number`
        # is the authoritative one inside the live state. Prefer the inner
        # value to match what the prompt builder uses.
        gs = snapshot.get("game_state")
        if isinstance(gs, dict):
            turn = gs.get("turn")
            if isinstance(turn, dict):
                value = turn.get("turn_number")
                if isinstance(value, int):
                    return value
        value = snapshot.get("turn_number")
        return value if isinstance(value, int) else None

    def _drain_to_game_over(self, *, reason: str) -> GameOver:
        """Pull any remaining queued lines (briefly) and return GameOver."""

        # Give a short window for closing banner lines to flush.
        deadline = time.monotonic() + 1.0
        while time.monotonic() < deadline:
            try:
                stream, line = self._sink.get(timeout=0.1)
            except queue.Empty:
                if self.proc is None or self.proc.poll() is not None:
                    break
                continue
            if line is None:
                self._closed_streams.add(stream)
                continue
            self._all_lines.append(line)
            self._maybe_record_log_line(line)

        rc = self.proc.poll() if self.proc is not None else None
        fresh_output = "\n".join(self._all_lines[self._fresh_window_start :])
        log_lines = self._log_lines[self._log_window_start :]
        return GameOver(
            fresh_output=fresh_output,
            log_lines=log_lines,
            return_code=rc,
            reason=reason,
        )
