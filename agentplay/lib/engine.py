"""Replay-from-scratch MTG game engine wrapper for agentplay."""

from __future__ import annotations

import difflib
import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any, Sequence


DEFAULT_LOG_TAIL_LINES = 1000

# Matches the optional `[Your_Main1]` / `[Their_EndStep | Lightning Bolt on stack]`
# context flavour that the rich/fixed controller prepends to its
# "available actions:" header. See mtg-engine/src/game/controller.rs:
# `format_choice_menu` (`wants_context=true` branch).
_CHOICE_CONTEXT_RE = re.compile(r"^\s*\[([^\]]+)\]\s+\S+\s+available actions:\s*$")


def new_log_tail_lines(current_tail: str, printed_lines: Sequence[str]) -> str:
    """Return lines from `current_tail` that have not been shown yet.

    The engine replays the entire game from scratch on every step, so
    consecutive `log_tail` snapshots overlap heavily. Three patterns occur:

    1. Simple growth: the new tail extends the previous tail with a
       few appended lines (the just-resolved action plus phase
       transitions leading to the next choice point).
    2. Bounded-tail roll-off: the engine's `--log-tail=N` flag drops
       old lines off the front, so the previous tail's suffix aligns
       with the new tail's prefix.
    3. Replay divergence: a newly-appended choice re-runs an action
       at an EARLIER choice point (the wildcard-driven `--p?-fixed-inputs`
       script can match it sooner), so the same end-of-log state is
       reached but with extra lines INSERTED mid-log.

    We use `difflib.SequenceMatcher` against the **cumulative** record
    of every line we have previously shown. Only the `insert`/`replace`
    spans -- content in `current_tail` that has no counterpart in the
    cumulative log -- are returned, in `current_tail` order. Lines that
    rolled off, or that the replay reorders without producing genuinely
    new content, are correctly suppressed.
    """
    if not current_tail:
        return ""
    curr = current_tail.splitlines()
    if not printed_lines:
        return current_tail

    # Normalise leading whitespace before diffing: the engine's verbose
    # logger sometimes re-emits the same action with different indentation
    # depending on how deep into the casting/resolution sequence the replay
    # stops (a "Fixed1 plays Mountain" line printed after the Mountain
    # land-play is flush left, but the same line printed from inside the
    # spell-playing block on a later replay has a 2-space indent). Comparing
    # the lstripped form keeps those lines deduped while preserving the
    # original indentation in whatever we ultimately emit to the user.
    def _key(line: str) -> str:
        return line.lstrip()

    matcher = difflib.SequenceMatcher(
        a=[_key(line) for line in printed_lines],
        b=[_key(line) for line in curr],
        autojunk=False,
    )
    new_segments: list[str] = []
    for tag, _i1, _i2, j1, j2 in matcher.get_opcodes():
        if tag in ("insert", "replace"):
            new_segments.extend(curr[j1:j2])
    return "\n".join(new_segments) if new_segments else ""


class GameEngine:
    """Wrap `mtg tui` for deterministic replay-based choice stepping."""

    def __init__(self, seed: int, game_dir: str | Path | None, verbose: bool) -> None:
        self.seed = seed
        self.verbose = verbose
        self.repo_root = Path(__file__).resolve().parent.parent.parent
        self.agentplay_dir = Path(__file__).resolve().parent.parent
        self.binary_path = self.repo_root / "target" / "release" / "mtg"
        self.cardsfolder_path = self.repo_root / "cardsfolder"
        self.forge_cardsfolder_path = self.repo_root / "forge-java" / "forge-gui" / "res" / "cardsfolder"
        self.game_dir = self._resolve_game_dir(game_dir)
        self.snapshot_path = self.game_dir / "snapshot.json"
        self.enriched_log_path = self.game_dir / "enriched_log.md"
        self.p1_choices_path = self.game_dir / "p1_choices.txt"
        self.p2_choices_path = self.game_dir / "p2_choices.txt"
        self.initial_args_path = self.game_dir / "initial_args.txt"
        self.initial_args: list[str] = self._read_lines(self.initial_args_path) if self.initial_args_path.exists() else []
        self.command_name = "tui"
        self._last_choices: list[str] = []
        self._last_choosing_player: str | None = None
        self._last_choice_context: str | None = None
        self._last_log_tail = ""
        self._last_output = ""
        self._last_returncode = 0
        self._last_game_over = False
        self._last_snapshot: dict[str, Any] | None = None

    def set_initial_args(self, mtg_args: Sequence[str]) -> None:
        self.initial_args = [str(arg) for arg in mtg_args]

    def set_command(self, command_name: str) -> None:
        self.command_name = command_name

    def start_game(self) -> dict[str, Any]:
        if not self.initial_args:
            raise ValueError("start_game requires initial mtg tui arguments")
        self._prepare_game_dir()
        self._write_lines(self.initial_args_path, self.initial_args)
        self.p1_choices_path.touch()
        self.p2_choices_path.touch()
        return self._run_game(stop_on_choice=1)

    def parse_snapshot(self, path: str | Path) -> dict[str, Any]:
        snapshot_path = Path(path)
        if not snapshot_path.exists():
            return self._build_terminal_snapshot({})
        with snapshot_path.open("r", encoding="utf-8") as handle:
            raw_snapshot = json.load(handle)
        turn_number, current_step = self._extract_turn_info(raw_snapshot)
        parsed = {
            "game_state": raw_snapshot,
            "choices": list(self._last_choices),
            "active_player": self._last_choosing_player or self._extract_active_player(raw_snapshot),
            "choice_context": self._last_choice_context,
            "turn_number": turn_number,
            "current_step": current_step,
            "log_tail": self._last_log_tail,
            "raw_output": self._last_output,
            "game_over": self._last_game_over,
            "returncode": self._last_returncode,
        }
        self._last_snapshot = parsed
        return parsed

    def total_choices_made(self) -> int:
        """Count of player choices already recorded in the choice files."""
        return len(self._read_lines(self.p1_choices_path)) + len(self._read_lines(self.p2_choices_path))

    def append_choice(self, player: str, choice_text: str) -> None:
        target_path = self._choices_path(player)
        with target_path.open("a", encoding="utf-8") as handle:
            handle.write(choice_text.strip())
            handle.write("\n")

    def continue_game(self) -> dict[str, Any]:
        if not self.initial_args:
            self.initial_args = self._read_lines(self.initial_args_path)
        if not self.initial_args:
            raise ValueError("continue_game requires initial_args.txt or set_initial_args()")
        return self._run_game(stop_on_choice=self.total_choices_made() + 1)

    def is_game_over(self, snapshot: dict[str, Any]) -> bool:
        return bool(snapshot.get("game_over"))

    def append_enriched_log(
        self,
        before_snapshot: dict[str, Any],
        game_state_summary: str,
        available_choices: Sequence[str],
        agent_response: str,
        chosen_action: str,
        after_snapshot: dict[str, Any],
    ) -> None:
        with self.enriched_log_path.open("a", encoding="utf-8") as handle:
            if handle.tell() == 0:
                handle.write("# Enriched Agent Game Log\n\n")
            handle.write(
                self._format_enriched_entry(
                    before_snapshot,
                    game_state_summary,
                    available_choices,
                    agent_response,
                    chosen_action,
                    after_snapshot,
                )
            )
            handle.write("\n")

    def _resolve_game_dir(self, game_dir: str | Path | None) -> Path:
        if game_dir is None:
            game_num = 1
            while (self.agentplay_dir / f"{game_num:03d}.game").exists():
                game_num += 1
            return self.agentplay_dir / f"{game_num:03d}.game"
        game_path = Path(game_dir)
        if not game_path.is_absolute():
            game_path = self.agentplay_dir / game_path
        return game_path

    def _prepare_game_dir(self) -> None:
        if self.game_dir.exists():
            if any(self.game_dir.iterdir()):
                raise FileExistsError(f"game directory already exists and is not empty: {self.game_dir}")
        else:
            self.game_dir.mkdir(parents=True, exist_ok=True)

    def _choices_path(self, player: str) -> Path:
        if player == "p1":
            return self.p1_choices_path
        if player == "p2":
            return self.p2_choices_path
        raise ValueError(f"unknown player {player!r}, expected 'p1' or 'p2'")

    def _run_game(self, stop_on_choice: int) -> dict[str, Any]:
        cardsfolder = self._resolve_cardsfolder()
        if cardsfolder is None:
            raise RuntimeError(
                "cardsfolder is unavailable. The repository symlink is broken and forge-java is not initialized. "
                "Run `git submodule update --init forge-java` or provide a valid CARDSFOLDER path."
            )
        if not self.binary_path.exists():
            raise RuntimeError(
                f"Error: MTG engine binary not found at {self.binary_path}\n"
                "Build it with: cargo build --release"
            )

        # Each choice is preceded by a wildcard (*) so the controller waits
        # until the choice point where the command matches, rather than consuming
        # commands strictly in sequence (which breaks when priority auto-passes
        # create extra choice points on replay).
        p1_choices = self._read_lines(self.p1_choices_path)
        p2_choices = self._read_lines(self.p2_choices_path)
        p1_script = ";".join(f"*;{c}" for c in p1_choices) if p1_choices else ""
        p2_script = ";".join(f"*;{c}" for c in p2_choices) if p2_choices else ""
        if self.snapshot_path.exists():
            self.snapshot_path.unlink()

        cmd = [
            str(self.binary_path),
            self.command_name,
            *self.initial_args,
            "--p1=fixed",
            "--p2=fixed",
            f"--p1-fixed-inputs={p1_script}",
            f"--p2-fixed-inputs={p2_script}",
            f"--stop-on-choice={stop_on_choice}",
            f"--snapshot-output={self.snapshot_path}",
            "--json",
            f"--log-tail={DEFAULT_LOG_TAIL_LINES}",
            f"--seed={self.seed}",
            "--verbosity=verbose",  # Show all step headers so agent sees phase transitions
        ]
        if self.verbose:
            print(f"$ {' '.join(cmd)}")

        env = dict(os.environ)
        env["CARDSFOLDER"] = str(cardsfolder)
        # Suppress INFO-level Rust log lines; keep WARN and ERROR visible
        env.setdefault("RUST_LOG", "warn")
        completed = subprocess.run(
            cmd,
            cwd=self.repo_root,
            capture_output=True,
            text=True,
            check=False,
            env=env,
        )
        output = self._combine_output(completed.stdout, completed.stderr)
        self._last_output = output
        self._last_choices = self._extract_choices(output)
        self._last_choosing_player = self._extract_choosing_player(output)
        self._last_choice_context = self._extract_choice_context(output)
        self._last_log_tail = self._extract_log_tail(output)
        self._last_returncode = completed.returncode
        self._last_game_over = self._detect_game_over(output)

        if completed.returncode != 0 and not self._last_game_over:
            raise RuntimeError(
                f"mtg tui failed with exit code {completed.returncode}\n{output.strip() or '(no output)'}"
            )

        if self.snapshot_path.exists():
            return self.parse_snapshot(self.snapshot_path)
        if self._last_game_over:
            return self._build_terminal_snapshot(
                self._last_snapshot["game_state"] if self._last_snapshot else {}
            )
        raise RuntimeError(f"mtg tui exited without snapshot or game-over marker\n{output.strip() or '(no output)'}")

    def _build_terminal_snapshot(self, game_state: dict[str, Any]) -> dict[str, Any]:
        turn_number, current_step = self._extract_turn_info(game_state)
        parsed = {
            "game_state": game_state,
            "choices": [],
            "active_player": self._extract_active_player(game_state),
            "choice_context": self._last_choice_context,
            "turn_number": turn_number,
            "current_step": current_step,
            "log_tail": self._last_log_tail,
            "raw_output": self._last_output,
            "game_over": self._last_game_over,
            "returncode": self._last_returncode,
        }
        self._last_snapshot = parsed
        return parsed

    def _format_enriched_entry(
        self,
        before_snapshot: dict[str, Any],
        game_state_summary: str,
        available_choices: Sequence[str],
        agent_response: str,
        chosen_action: str,
        after_snapshot: dict[str, Any],
    ) -> str:
        after_log = str(after_snapshot.get("log_tail", "")).strip() or "(no game log captured)"
        lines = [
            f"## Choice {_entry_turn_label(before_snapshot)}",
            "",
            "### Game State Summary",
            "```text",
            game_state_summary.strip() or "(no summary available)",
            "```",
            "",
            "### Available Choices",
        ]
        if available_choices:
            lines.extend(f"- [{index}] {choice}" for index, choice in enumerate(available_choices, start=1))
        else:
            lines.append("- (none)")
        lines.extend(
            [
                "",
                "### Agent Reasoning",
                "```text",
                agent_response.strip() or "(empty response)",
                "```",
                "",
                "### Chosen Action",
                f"- {chosen_action}",
                "",
                "### Game Log After Action",
                "```text",
                after_log,
                "```",
            ]
        )
        return "\n".join(lines)

    @staticmethod
    def _read_lines(path: Path) -> list[str]:
        if not path.exists():
            return []
        with path.open("r", encoding="utf-8") as handle:
            return [line.strip() for line in handle if line.strip()]

    @staticmethod
    def _write_lines(path: Path, lines: Sequence[str]) -> None:
        with path.open("w", encoding="utf-8") as handle:
            for line in lines:
                handle.write(str(line))
                handle.write("\n")

    @staticmethod
    def _combine_output(stdout: str, stderr: str) -> str:
        pieces = []
        if stdout.strip():
            pieces.append(stdout.strip("\n"))
        if stderr.strip():
            pieces.append(stderr.strip("\n"))
        return "\n".join(pieces)

    @staticmethod
    def _extract_active_player(snapshot: dict[str, Any]) -> str | None:
        if not isinstance(snapshot, dict):
            return None
        game_state = snapshot.get("game_state")
        root = game_state if isinstance(game_state, dict) else snapshot
        turn = root.get("turn")
        if not isinstance(turn, dict):
            return None
        active_player = turn.get("active_player")
        if active_player is None:
            return None
        return str(active_player)

    def _extract_choices(self, output: str) -> list[str]:
        lines = output.splitlines()
        header_index = -1
        for index, line in enumerate(lines):
            if "available actions:" in line:
                header_index = index
        if header_index < 0:
            return []

        choices_by_index: dict[int, str] = {}
        for line in lines[header_index + 1 :]:
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

        return [text for index, text in sorted(choices_by_index.items()) if index > 0]

    @staticmethod
    def _extract_choice_context(output: str) -> str | None:
        """Pull the bracketed `[Your_Main1 | ...]` context off the last action header.

        The fixed/rich controller prepends this when `wants_context()` is
        true (see mtg-engine/src/game/controller.rs::format_choice_menu).
        We return the raw inner string (without surrounding brackets) so
        the caller can render it however it likes.
        """
        last_match: str | None = None
        for line in output.splitlines():
            m = _CHOICE_CONTEXT_RE.match(line)
            if m:
                last_match = m.group(1).strip()
        return last_match

    @staticmethod
    def _extract_turn_info(snapshot: dict[str, Any] | None) -> tuple[int | None, str | None]:
        """Pull (turn_number, current_step) out of either a wrapped or raw snapshot."""
        if not isinstance(snapshot, dict):
            return None, None
        root = snapshot
        wrapped = snapshot.get("game_state")
        if isinstance(wrapped, dict):
            root = wrapped
        turn = root.get("turn") if isinstance(root, dict) else None
        if not isinstance(turn, dict):
            return None, None
        turn_number = turn.get("turn_number")
        if not isinstance(turn_number, int):
            turn_number = None
        step = turn.get("current_step")
        if not isinstance(step, str):
            step = None
        return turn_number, step

    @staticmethod
    def _extract_choosing_player(output: str) -> str | None:
        """Parse the choosing player from the last 'FixedN available actions:' header.

        Controller names end with 1 (player 0 / p1) or 2 (player 1 / p2).
        This is more reliable than turn.active_player from the snapshot,
        which reflects whose *turn* it is rather than who has *priority*.
        """
        last_header = None
        for line in output.splitlines():
            if "available actions:" in line:
                last_header = line
        if last_header is None:
            return None
        name_part = last_header.strip().split("available")[0].strip()
        if name_part.endswith("1"):
            return "0"
        if name_part.endswith("2"):
            return "1"
        return None

    def _extract_log_tail(self, output: str) -> str:
        lines = output.splitlines()
        header_index = next((index for index in range(len(lines) - 1, -1, -1) if "available actions:" in lines[index]), -1)
        relevant = lines[:header_index] if header_index >= 0 else lines
        # Filter out replayed "available actions:" blocks and meta lines
        filtered: list[str] = []
        skip_action_items = False
        for line in relevant:
            if self._is_meta_line(line):
                continue
            if "available actions:" in line:
                skip_action_items = True
                continue
            if skip_action_items:
                stripped = line.strip()
                if stripped.startswith("[") and "]" in stripped:
                    continue
                skip_action_items = False
            filtered.append(line)
        return "\n".join(line.rstrip() for line in filtered).strip()

    @staticmethod
    def _is_meta_line(line: str) -> bool:
        stripped = line.strip()
        if not stripped:
            return False
        meta_prefixes = (
            "=== Starting Game ===",
            "=== Continuing Game ===",
            "=== Snapshot Saved ===",
            "Snapshot output:",
            "Choice limit reached:",
            "Snapshot saved to:",
            "Intra-turn choices:",
            "Actions rewound:",
            "Turns played:",
            "Reason:",
            "=== Final State ===",
        )
        return stripped.startswith(meta_prefixes)

    @staticmethod
    def _detect_game_over(output: str) -> bool:
        markers = (
            "=== Game Over ===",
            "Winner:",
            "Game ended in a draw",
            " wins!",
        )
        return any(marker in output for marker in markers)

    def _resolve_cardsfolder(self) -> Path | None:
        env_path = os.environ.get("CARDSFOLDER")
        if env_path:
            candidate = Path(env_path)
            if self._looks_like_cardsfolder(candidate):
                return candidate
        for candidate in (self.cardsfolder_path, self.forge_cardsfolder_path):
            if self._looks_like_cardsfolder(candidate):
                return candidate
        return None

    @staticmethod
    def _looks_like_cardsfolder(path: Path) -> bool:
        return path.exists() and all((path / letter).is_dir() for letter in ("a", "b", "c"))


def _entry_turn_label(snapshot: dict[str, Any]) -> str:
    game_state = snapshot.get("game_state")
    if not isinstance(game_state, dict):
        return "Unknown"
    root = game_state.get("game_state")
    if isinstance(root, dict):
        game_state = root
    turn = game_state.get("turn")
    if not isinstance(turn, dict):
        return "Unknown"
    turn_number = turn.get("turn_number", "?")
    step = turn.get("current_step", "?")
    return f"Turn {turn_number} {step}"
