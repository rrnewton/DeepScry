---
title: agentplay pytest tests reference removed MockSession.ask()
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-27T13:42:13.947431040+00:00
updated_at: 2026-05-27T13:51:20.878831109+00:00
---

# Description

## Description

Three pytest tests in agentplay/ failed because they called MockSession() and then ask(), but the Python --mock RNG path was removed in commit 61e06688 (2026-05-15). MockSession.ask() now raises NotImplementedError as an intentional deprecation marker.

## Failing tests (now resolved)
- agentplay/test_agent_game.py::test_mock_mode_selects_randomly_without_subprocess (AssertionError: assert 'mock' in 'random choice\n5')
- agentplay/test_persistent_driver.py::test_mock_session_deterministic
- agentplay/test_persistent_driver.py::test_mock_session_returns_in_range

## Root cause
Commit 61e06688 removed the Python-side MockSession.ask() codepath; --mock now uses the engine-side RandomController for byte-identical determinism across stop-and-go / persistent / WASM drivers. The 3 tests above were not updated. Additionally, `_choose_for_player(mock=True)` now routes through `_controller_for_player` which returns "random" instead of "mock", so the raw_response string changed from "mock choice\nN" to "random choice\nN".

## Resolution (2026-05-27)
1. DELETED `test_mock_session_deterministic` and `test_mock_session_returns_in_range` in agentplay/test_persistent_driver.py — they directly call `MockSession.ask()` which is the deprecated stub. Replaced with a comment block explaining the deprecation timeline. Also removed the unused `MockSession` import.
2. FIXED (not deleted) `test_mock_mode_selects_randomly_without_subprocess`: the test still exercises a valid contract — `_choose_for_player(mock=True)` must NOT spawn a subprocess and must return an in-range choice. The mock=True path still works (it routes to controller_kind="random" which falls through to local rng.randint without subprocess), so the test is meaningful. Only the obsolete `"mock" in raw_response` assertion needed updating to `"random" in raw_response`. Added an inline comment explaining the new routing.

The sibling test `test_mock_mode_is_deterministic_with_same_seed` was already passing (only asserts determinism, not the raw_response string).

## Verification
`python3 -m pytest agentplay/ -v` → 75 passed, 7 skipped (skips are unrelated network/WASM gates), 0 failed.

## Related
- 61e06688 (root-cause commit)
- mtg-99og6 (CI status policy)
- mtg-vy7rv (parallel fix on rust-toolchain)
