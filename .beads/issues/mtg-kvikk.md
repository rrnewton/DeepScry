---
title: agentplay pytest tests reference removed MockSession.ask()
status: open
priority: 2
issue_type: bug
created_at: 2026-05-27T13:42:13.947431040+00:00
updated_at: 2026-05-27T13:42:13.947431040+00:00
---

# Description

Three pytest tests in agentplay/ fail because they call MockSession() and then ask(), but the Python --mock RNG path was removed in commit 61e06688 (2026-05-15). MockSession.ask() now raises NotImplementedError.

## Failing tests
- agentplay/test_agent_game.py::test_mock_mode_selects_randomly_without_subprocess (AssertionError: assert 'mock' in 'random choice\n5')
- agentplay/test_persistent_driver.py::test_mock_session_deterministic
- agentplay/test_persistent_driver.py::test_mock_session_returns_in_range

## Root cause
Commit 61e06688 removed the Python-side MockSession.ask() codepath; --mock now uses the engine-side RandomController for byte-identical determinism across stop-and-go / persistent / WASM drivers. The 3 tests above were not updated.

## Recommended fix
Delete test_mock_session_deterministic and test_mock_session_returns_in_range (they test a removed feature, not a contract worth preserving). Rewrite test_mock_mode_selects_randomly_without_subprocess to assert engine-side RandomController behaviour instead of the old 'mock' string in output.

## Discovery
Found during integration-branch triage 2026-05-27_#2297(b5cbdc85). See ai_docs/integration_triage_20260527.md (F1a/b/c). Same failures reproduce on CI run 26514285939 and locally via 'make validate'.

## Related
- 61e06688 (root-cause commit)
- mtg-99og6 (CI status policy)
