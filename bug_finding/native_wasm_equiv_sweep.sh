#!/usr/bin/env bash
# Native-vs-WASM engine-equivalence fuzz sweep wrapper.
#
# Runs `scripts/native_wasm_equiv_sweep.py` over a seed-range x deck-sample
# sweep, asserting the engine produces the SAME random-vs-random game in the
# native binary and the WASM module for every (seed, deck) combo. Exits 1 on
# ANY divergence (a real cross-compile-target determinism bug).
#
# Usage:
#   ./scripts/native_wasm_equiv_sweep.sh                       # default sweep
#   ./scripts/native_wasm_equiv_sweep.sh --seeds 10 --decks 'decks/old_school/*.dck'
#   ./scripts/native_wasm_equiv_sweep.sh --max-turns 40 --seeds 50   # heavy mode
#
# Any extra args are forwarded verbatim to the Python harness (see its --help).
#
# WASM-toolchain gating (mirrors AGENTPLAY_TEST_WASM semantics):
#   * This harness ALWAYS needs the WASM toolchain (it is the whole point).
#   * If the local WASM build (web/pkg + web/data) is missing it tries to build
#     it via `make wasm-dev` unless MTG_EQUIV_NO_BUILD=1.
#   * If Chromium / playwright is genuinely absent AND MTG_EQUIV_REQUIRE_WASM
#     is NOT set, it SKIPS LOUDLY with exit 0 (so a dev box without a browser
#     doesn't block). When MTG_EQUIV_REQUIRE_WASM=1 (set by `make validate`)
#     the missing toolchain is a HARD FAILURE (exit 1) — we never silently
#     green-skip a divergence in CI.
#
# Environment:
#   CARDSFOLDER                -- card definitions path (auto-resolved if unset)
#   MTG_EQUIV_REQUIRE_WASM=1   -- treat absent browser/playwright as a hard fail
#   MTG_EQUIV_NO_BUILD=1       -- do not auto-build the WASM bundle if missing

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# ---------------------------------------------------------------------------
# Resolve CARDSFOLDER (shared logic with test_mode_equivalence.sh).
# ---------------------------------------------------------------------------
if [ -z "${CARDSFOLDER:-}" ]; then
    for candidate in \
        "$REPO_ROOT/cardsfolder" \
        "$REPO_ROOT/forge-java/forge-gui/res/cardsfolder" \
        "/home/newton/work/dev-mtg/mtg-forge-rs/cardsfolder" \
        "/home/newton/working_copies/mtg/mtg-forge-rs/cardsfolder"; do
        if [ -d "$candidate/a" ]; then
            export CARDSFOLDER="$candidate"
            break
        fi
    done
fi
if [ -z "${CARDSFOLDER:-}" ]; then
    echo "Error: no usable CARDSFOLDER found." >&2
    echo "Run 'git submodule update --init forge-java' or set CARDSFOLDER=..." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Toolchain gating: playwright + Chromium present?
# ---------------------------------------------------------------------------
require_wasm="${MTG_EQUIV_REQUIRE_WASM:-0}"

if ! python3 -c "from playwright.sync_api import sync_playwright" >/dev/null 2>&1; then
    msg="playwright python package not importable"
    if [ "$require_wasm" = "1" ]; then
        echo "HARD FAIL: $msg, but MTG_EQUIV_REQUIRE_WASM=1." >&2
        echo "  Install: python3 -m pip install playwright && python3 -m playwright install chromium" >&2
        exit 1
    fi
    echo "SKIP (loud): $msg; MTG_EQUIV_REQUIRE_WASM not set, skipping native-vs-WASM sweep." >&2
    exit 0
fi

# Probe a Chromium launch — cheap and catches a missing browser binary.
if ! python3 - <<'PYPROBE' >/dev/null 2>&1
from playwright.sync_api import sync_playwright
with sync_playwright() as pw:
    b = pw.chromium.launch(headless=True, args=["--no-sandbox", "--enable-unsafe-swiftshader"])
    b.close()
PYPROBE
then
    msg="Chromium failed to launch (browser not installed?)"
    if [ "$require_wasm" = "1" ]; then
        echo "HARD FAIL: $msg, but MTG_EQUIV_REQUIRE_WASM=1." >&2
        echo "  Install: python3 -m playwright install chromium" >&2
        exit 1
    fi
    echo "SKIP (loud): $msg; MTG_EQUIV_REQUIRE_WASM not set, skipping native-vs-WASM sweep." >&2
    exit 0
fi

# ---------------------------------------------------------------------------
# Ensure the WASM bundle exists (build if missing, unless told not to).
# ---------------------------------------------------------------------------
if [ ! -f "$REPO_ROOT/web/pkg/mtg_engine.js" ] || [ ! -f "$REPO_ROOT/web/data/decks.bin" ]; then
    if [ "${MTG_EQUIV_NO_BUILD:-0}" = "1" ]; then
        echo "Error: WASM bundle missing and MTG_EQUIV_NO_BUILD=1 (run 'make wasm-dev')." >&2
        exit 1
    fi
    echo "=== WASM bundle missing — building via 'make wasm-dev' ===" >&2
    make wasm-dev >&2
fi

# Ensure the native release binary exists.
if [ ! -x "$REPO_ROOT/target/release/mtg" ]; then
    echo "=== native release binary missing — building 'cargo build --release' ===" >&2
    cargo build --release --features network >&2
fi

echo "=== native-vs-WASM equivalence sweep ===" >&2
echo "  CARDSFOLDER=$CARDSFOLDER" >&2
echo "  args: $* " >&2

exec python3 "$REPO_ROOT/scripts/native_wasm_equiv_sweep.py" "$@"
