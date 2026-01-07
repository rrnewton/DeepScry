---
title: Enable missing_panics_doc and missing_errors_doc clippy lints
status: open
priority: 3
issue_type: task
created_at: 2026-01-07T13:58:17.352320041+00:00
updated_at: 2026-01-07T14:36:08.548051106+00:00
---

# Description

## Description

Enable documentation lints for functions that can panic or return errors.

## Progress

### missing_panics_doc (COMPLETED)
- [x] Enabled `missing_panics_doc` = "warn" (16 issues → 0)
- Fixed by adding `# Panics` sections or eliminating panics:
  - costs.rs: Eliminated panic by returning Option directly
  - entity.rs: Documented intentional panic in remove()
  - player.rs: Used let-else pattern to eliminate unwrap
  - download.rs: Documented expect() panics
  - controller.rs: Documented unwrap() panic
  - fancy_tui_controller.rs: Documented SystemTime panic
  - mana_engine.rs: Documented expect() panic
  - cardsfolder.rs: Documented require_cardsfolder panic
  - tournament.rs: Documented mutex lock panic
  - network/client.rs: Documented 4 functions with potential panics
  - wasm/fancy_tui.rs: Documented launch_fancy_tui panic
  - wasm/deck_builder.rs: Documented launch_deck_builder panic

### missing_errors_doc (TODO)
- [ ] `missing_errors_doc` = "warn" (117 issues remaining)

Files affected by remaining work:
- game/state.rs (16 functions)
- game/actions/mod.rs (20 functions)
- network/client.rs (8 functions)
- game/snapshot.rs (6 functions)
- And many others...

This is a significant documentation effort that should be done incrementally.

## Reference

Issue counts from clippy analysis on 2026-01-07.
