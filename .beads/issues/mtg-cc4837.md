---
title: 'snapshot resume panic: Cache exists after rebuild'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-14T14:16:21.083670959+00:00
updated_at: 2026-05-14T14:16:21.083670959+00:00
---

# Description

FIXED on test-resume-coverage branch.

mtg resume <snapshot> panicked immediately at mtg-engine/src/game/mana_engine.rs:779 'Cache exists after rebuild'.

Root cause: GameState.mana_caches has #[serde(skip)], so after loading a snapshot via GameSnapshot::load_from_file -> snapshot.game_state.clone() the per-player cache slots are missing. ManaEngine::update_mut would then expect a cache slot to exist after rebuild_mana_cache_if_needed, but the latter no-oped because there was nothing to rebuild.

Fix:
- Added GameState::ensure_mana_cache(player_id) and GameState::ensure_mana_caches_for_all_players(), which insert a fresh dirty cache slot for any missing player.
- rebuild_mana_cache_if_needed now calls ensure_mana_cache as a defensive backstop.
- run_resume in main.rs explicitly calls ensure_mana_caches_for_all_players right after restoring from the snapshot, for clarity.

Regression coverage:
- New unit test test_ensure_mana_cache_after_simulated_resume in mtg-engine/src/game/state.rs (simulates the post-deserialize empty state and asserts rebuild does the right thing).
- New e2e test tests/snapshot_resume_e2e.sh wired into validate-parallel-steps in the Makefile. Covers stop-on-choice + resume in both bincode and JSON snapshot formats, deep gamestate comparison via scripts/diff_gamestate.py, and resume-with-controller-override smoke test.

Also added 'mana_state_version' to the strip-list in scripts/diff_gamestate.py: this counter bumps an extra time after restoring because the cache is rebuilt from scratch, but it isn't part of the actual game state.
