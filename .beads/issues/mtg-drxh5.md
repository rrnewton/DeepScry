---
title: Remove dead launcher-only JS from game pages + migrate non-validate game-page tests
status: open
priority: 4
issue_type: task
created_at: 2026-06-01T19:04:02.800820860+00:00
updated_at: 2026-06-01T19:04:02.800820860+00:00
---

# Description

Follow-up to mtg-35z3s page 3 (lobby redo). When native_game.html and tui_game.html became PURE renderers (built-in launcher deleted), the launcher-only JS functions were left in place but are now UNCALLED (no DOM to drive them): in tui_game.html — saveSettings/restoreSettings/setupSettingsPersistence/restoreDeckSelections/updateGameModeUI/updateNetworkUI/loadAllCards/setupControllerHandlers/initDeckCollections/initUploadModal/setupDeckBuilderButtons + the whole deck-builder + .dck-upload code (showUploadModal/parseDckFormat/etc.); in native_game.html — applyLocalImageGate's deleted-checkbox refs are null-guarded but dead. These are not invoked (verified: page boots via bootFromParams + the renderer/network/WASM wiring only), and a JS syntax check passes, but the dead code violates the no-dead-code/short-files rule. getCustomDecks/CUSTOM_DECKS_KEY MUST stay (loadCardsForDecks uses them for the custom-deck boot path). Also: several NON-validate web tests still drive the old launcher and would fail if run manually (not in make validate/CI): test_game_gui.js, test_game_gui_bugfixes.js, test_game_gui_deep.js, test_game_gui_playtest.js, test_game_gui_rebuild.js, test_network_e2e.js, test_network_human_input.js, test_network_random_e2e.js, test_bug_report.js. Migrate them to the param boot (web/game_boot_params.js helpers: firstBuiltinDeck/pickBuiltinDeck/localGameUrl/parseDckIntoCustomDeck) or to the network param contract (?mode=network&controller=...).
