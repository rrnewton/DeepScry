"""Unit tests for scripts/check_clean_environment.py cargo cwd-scoping.

WHY HERE: `pytest agentplay/` is the project's single auto-discovered Python test
root — run IDENTICALLY by `make validate` (validate-agentplay-step) and GitHub CI
(.github/workflows/ci.yml). Placing this here wires the test into both with no
Makefile/CI edits. The subject under test is the harness pre-flight checker
(scripts/check_clean_environment.py), not agentplay gameplay.

WHAT IT GUARDS (the fix): the validation pre-check used to flag ANY
`cargo … test/build` whose cmdline contained `-p mtg` — a GLOBAL match that
false-collided across worktrees, so two agents' concurrent validates serialized
each other. The fix scopes the cargo check by the process's REAL working
directory (`/proc/<pid>/cwd`), only flagging cargo running WITHIN this worktree,
with a fail-safe to the original scoped `current_dir in cmd` check when the cwd
is unavailable (process exited mid-scan, or no /proc on macOS). These tests pin
that behavior AND prove the OTHER protections (MTG binary, validate.py,
chromium/playwright, python-mtg) are unchanged.
"""

import importlib.util
import os
from pathlib import Path

# Load scripts/check_clean_environment.py by path (it is a standalone script,
# not an importable package module).
_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "check_clean_environment.py"
_spec = importlib.util.spec_from_file_location("check_clean_environment", _SCRIPT)
cce = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cce)

CWD = "/home/newton/work/dev-mtg/worktrees/logo-assets"
OTHER_WT = "/home/newton/work/dev-mtg/worktrees/netarch-undo-holes"


def ps_line(pid, cmd, user="newton"):
    """Synthesize a `ps aux` row. is_conflicting_process splits on the first 10
    runs of whitespace, so columns 0..9 are header fields and column 10 is the
    full command. Values other than pid/cmd are irrelevant to the matcher."""
    return f"{user} {pid} 0.0 0.0 1000 1000 ? Rl 20:00 0:30 {cmd}"


def with_cwd(monkeypatch, mapping):
    monkeypatch.setattr(cce, "proc_cwd", lambda pid: mapping.get(str(pid)))


# ── The fix: cargo scoped by real cwd ───────────────────────────────────────

def test_cargo_in_this_worktree_is_flagged(monkeypatch):
    with_cwd(monkeypatch, {"1001": CWD})
    ok, desc = cce.is_conflicting_process(
        ps_line(1001, "cargo test -p mtg-engine --features network"), CWD)
    assert ok, desc


def test_cargo_in_subdir_of_this_worktree_is_flagged(monkeypatch):
    # cwd is a descendant (e.g. running from mtg-engine/).
    with_cwd(monkeypatch, {"1003": CWD + "/mtg-engine"})
    ok, _ = cce.is_conflicting_process(
        ps_line(1003, "cargo nextest run -p mtg-engine"), CWD)
    assert ok


def test_same_cargo_cmdline_in_OTHER_worktree_is_NOT_flagged(monkeypatch):
    # THE regression: identical `-p mtg-engine` cmdline, different cwd → ignored.
    with_cwd(monkeypatch, {"1002": OTHER_WT})
    ok, _ = cce.is_conflicting_process(
        ps_line(1002, "cargo test -p mtg-engine --features network"), CWD)
    assert not ok


def test_sibling_prefix_worktree_is_NOT_flagged(monkeypatch):
    # current_dir + "-other" must NOT match current_dir (trailing-sep guard).
    with_cwd(monkeypatch, {"1007": CWD + "-other"})
    ok, _ = cce.is_conflicting_process(
        ps_line(1007, "cargo build --release -p mtg-engine"), CWD)
    assert not ok


# ── Fail-safe when cwd is unavailable (no /proc / exited process) ────────────

def test_fallback_flags_when_cmdline_carries_current_dir(monkeypatch):
    with_cwd(monkeypatch, {})  # proc_cwd → None for all pids
    ok, _ = cce.is_conflicting_process(
        ps_line(1005, f"cargo build --manifest-path {CWD}/Cargo.toml"), CWD)
    assert ok


def test_fallback_is_scoped_not_global(monkeypatch):
    # cwd unavailable AND current_dir not in cmd → NOT flagged. (The OLD code
    # would have flagged this via the global `-p mtg` substring.)
    with_cwd(monkeypatch, {})
    ok, _ = cce.is_conflicting_process(
        ps_line(1006, "cargo test -p mtg-engine --features network"), CWD)
    assert not ok


def test_proc_cwd_never_raises_on_bogus_pid():
    # Real /proc call on a pid that cannot exist → None, no exception.
    assert cce.proc_cwd("0") is None
    assert cce.proc_cwd("not-a-pid") is None


# ── Other protections must remain intact (unchanged by the fix) ─────────────

def test_dirty_mtg_binary_in_worktree_still_flagged(monkeypatch):
    with_cwd(monkeypatch, {})  # MTG-binary check does not consult cwd
    ok, _ = cce.is_conflicting_process(
        ps_line(2001, f"{CWD}/target/release/mtg server --port 24205"), CWD)
    assert ok


def test_validate_py_for_this_worktree_still_flagged(monkeypatch):
    with_cwd(monkeypatch, {})
    ok, _ = cce.is_conflicting_process(
        ps_line(2002, f"python3 {CWD}/scripts/validate.py"), CWD)
    assert ok


def test_chromium_localhost_still_flagged(monkeypatch):
    with_cwd(monkeypatch, {})
    ok, _ = cce.is_conflicting_process(
        ps_line(2003, "chromium --headless --remote-debugging-port=0 http://localhost:8767"), CWD)
    assert ok


def test_python_mtg_for_this_worktree_still_flagged(monkeypatch):
    with_cwd(monkeypatch, {})
    ok, _ = cce.is_conflicting_process(
        ps_line(2004, f"python3 {CWD}/scripts/mtg_wasm_game.py"), CWD)
    assert ok


def test_unrelated_process_is_not_flagged(monkeypatch):
    with_cwd(monkeypatch, {"3001": OTHER_WT})
    ok, _ = cce.is_conflicting_process(ps_line(3001, "vim notes.md"), CWD)
    assert not ok
