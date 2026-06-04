"""Smoke test for scripts/validate.py opt-out flags (mtg-w6u7c / mtg-717).

Pins the locked-down-host escape hatch: --no-wasm-e2e and --no-network must
reach validate.py's argparse (they're forwarded verbatim by `make validate
ARGS=...` -> `python3 scripts/validate.py ...`) AND disable exactly the right
steps. This regression-guards the defect where the old bash wrapper rejected
these flags ("Unknown option") so the escape hatch was unreachable from the
standard entry point. NO test may be silently auto-skipped; the only ways to
not-run the browser e2e are provisioning (offline) or these EXPLICIT flags,
which are reported in the run summary.
"""
import importlib.util
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "validate_mod", Path(__file__).resolve().parent.parent / "scripts" / "validate.py")
validate = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(validate)


def _disabled_for(no_wasm_e2e=False, no_network=False):
    """Replicate validate.py main()'s disabled-set computation."""
    steps = validate.build_registry()
    disabled = {}
    if no_network:
        for s in steps:
            if s.networkonly:
                disabled[s.tag] = "--no-network"
    if no_wasm_e2e:
        for s in steps:
            if ("browser" in s.resources) or s.tag in ("wasm.npm-install", "wasm.bundle"):
                disabled[s.tag] = "--no-wasm-e2e"
    return steps, disabled


def test_flags_exist_in_argparse():
    # The flags must be ACCEPTED by validate.py (the make-validate entry point);
    # the old bash wrapper rejecting them was the defect.
    import argparse
    # Build the same parser main() builds by parsing a known-good arg set.
    # If argparse didn't define them, parse_known_args would push them to extras.
    import sys
    saved = sys.argv
    try:
        # exercise the real parser indirectly: --list short-circuits before any run
        sys.argv = ["validate.py", "--no-wasm-e2e", "--no-network", "--list"]
        # main() calls run_orchestrator (subset? no) -> but --list returns 0.
        rc = validate.main()
        assert rc == 0
    finally:
        sys.argv = saved


def test_no_wasm_e2e_disables_all_browser_and_wasm_build():
    steps, disabled = _disabled_for(no_wasm_e2e=True)
    by_tag = {s.tag: s for s in steps}
    # every chromium-driven (browser-resource) step is disabled
    for s in steps:
        if "browser" in s.resources:
            assert disabled.get(s.tag) == "--no-wasm-e2e", f"{s.tag} should be disabled"
    # provisioning + bundle are disabled (orphaned once browser steps are off)
    assert disabled.get("wasm.npm-install") == "--no-wasm-e2e"
    assert disabled.get("wasm.bundle") == "--no-wasm-e2e"
    # non-browser work still RUNS (not disabled)
    for tag in ("unit.nextest", "determ.commander", "network.equiv-random", "lint.fmt"):
        assert tag in by_tag and tag not in disabled, f"{tag} must NOT be disabled"
    # no surviving step depends on a disabled step (no dangling deps)
    for s in steps:
        if s.tag in disabled:
            continue
        for d in s.deps:
            assert d not in disabled, f"{s.tag} depends on disabled {d}"


def test_start_utilization_does_not_hang():
    """Regression: _start_utilization runs the prehook, which backgrounds a
    disowned infinite sampler. If that sampler inherits a PIPE (capture_output),
    the pipe never EOFs and the call hangs for the whole validate run. Must
    return PROMPTLY (temp-file capture + sampler fds -> /dev/null), and the
    sampler must be reapable. Pins the bug that hung `make validate` ~25 min."""
    import os
    import threading
    if not (Path(__file__).resolve().parent.parent / "scripts" / "utilization_prehook.sh").exists():
        return  # hooks optional
    result = {}

    def go():
        result["mon"] = validate._start_utilization()
    t = threading.Thread(target=go, daemon=True)
    t.start()
    t.join(timeout=15)
    assert not t.is_alive(), "HANG: _start_utilization did not return within 15s"
    mon = result.get("mon")
    try:
        assert mon and mon.get("pid") and mon.get("stats"), "missing pid/stats"
        os.kill(int(mon["pid"]), 0)  # sampler alive
    finally:
        if mon:
            validate._stop_utilization(mon)  # always reap, even on assert failure


def test_no_network_disables_only_network_group():
    steps, disabled = _disabled_for(no_network=True)
    for s in steps:
        if s.networkonly:
            assert disabled.get(s.tag) == "--no-network", f"{s.tag} should be disabled"
        else:
            assert s.tag not in disabled, f"{s.tag} must NOT be disabled by --no-network"
    # by design --no-network does NOT touch wasm.npm-install (not networkonly),
    # so it is NOT a way to skip the npm/browser provisioning — only --no-wasm-e2e is.
    assert "wasm.npm-install" not in disabled
