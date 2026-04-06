"""Replay-from-scratch MTG game engine wrapper for agentplay."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path
from typing import Any, Sequence


class GameEngine:
    """Wrap `mtg tui` for deterministic replay-based choice stepping."""

    def __init__(self, seed: int, game_dir: str | Path | None, verbose: bool) -> None:
        self.seed = seed
        self.verbose = verbose
        self.repo_root = Path(__file__).resolve().parent.parent
        self.agentplay_dir = Path(__file__).resolve().parent
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
        parsed = {
            "game_state": raw_snapshot,
            "choices": list(self._last_choices),
            "active_player": self._extract_active_player(raw_snapshot),
            "log_tail": self._last_log_tail,
            "raw_output": self._last_output,
            "game_over": self._last_game_over,
            "returncode": self._last_returncode,
        }
        self._last_snapshot = parsed
        return parsed

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
        total_choices = len(self._read_lines(self.p1_choices_path)) + len(self._read_lines(self.p2_choices_path))
        return self._run_game(stop_on_choice=total_choices + 1)

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
            "--log-tail=100",
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
        parsed = {
            "game_state": game_state,
            "choices": [],
            "active_player": self._extract_active_player(game_state),
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

    def _extract_log_tail(self, output: str) -> str:
        lines = output.splitlines()
        header_index = next((index for index in range(len(lines) - 1, -1, -1) if "available actions:" in lines[index]), -1)
        relevant = lines[:header_index] if header_index >= 0 else lines
        filtered = [line for line in relevant if not self._is_meta_line(line)]
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
