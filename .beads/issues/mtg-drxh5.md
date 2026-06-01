---
title: Remove dead launcher-only JS from game pages + migrate non-validate game-page tests
status: in_progress
priority: 4
issue_type: task
created_at: 2026-06-01T19:04:02.800820860+00:00
updated_at: 2026-06-01T22:18:02.903771283+00:00
---

# Description

Remove dead launcher-only JS from game pages + migrate non-validate game-page tests.

STATUS 2026-06-01 (branch lobby-redo-verify-cleanup): DONE.

== Dead-code removal (web/JS only, no engine change) ==
tui_game.html (3527 -> 2863 lines, ~664 net removed): deleted the now-UNCALLED launcher-only JS left resident after the page became a PURE renderer (mtg-682 page 3): loadAllCards, updateNetworkUI, saveSettings, restoreSettings, restoreDeckSelections, setupSettingsPersistence, the SETTINGS_KEY const, and the ENTIRE deck-builder + .dck-upload block (DECK_COLLECTIONS, saveCustomDecks, initDeckCollections, updateDeckDropdown, updateDeckButtonStates, parseDckFormat, showUploadModal, hideUploadModal, importDeckFromModal, initUploadModal, setupDeckBuilderButtons, setupControllerHandlers, loadAllCardsForDeckBuilder, launchDeckBuilderForPlayer, exitDeckBuilder, onDeckBuilderUpdate, the __mtg_test_import_and_select_deck hook). Also trimmed the deck-builder WASM import block to ONLY cleanup_deck_builder_state (still called defensively in launchGame); dropped launch_deck_builder/deck_builder_set_*/deckBuilderEnabled imports.
KEPT (verified live): getCustomDecks + CUSTOM_DECKS_KEY (loadCardsForDecks custom-deck boot path), loadCardsForDecks, getControllerType, resolveLocalImageAllowed, applyLocalImageGate (now publishes window.__allowLocalImgLoad only).
native_game.html: applyLocalImageGate stripped of its dead #img-src-local checkbox/label/gate-note refs (deleted DOM); now just sets window.__allowLocalImgLoad.
Verified each removed fn truly uncalled (grep) + JS module parse OK for both files. test_redo_lobby_e2e.js (pure-renderer asserts incl native-8-card render + lobby->launcher->Play) ALL PASS against the cleaned pages.

== Test migration off the deleted launcher form (9 files) ==
Pointed at param-boot helpers in game_boot_params.js (added listBuiltinDecks; firstBuiltinDeck/pickBuiltinDeck now share it, DRY) / the ?mode=network&controller= contract:
 - test_game_gui.js: localGameUrl native heuristic-vs-heuristic. PASS.
 - test_game_gui_bugfixes.js: localGameUrl human-vs-heuristic; relaxed the launcher-era 'Enter <= +1 turn' assertion to monotonic + bounded (<= +2; a human priority-pass can auto-resolve the opponent turn). PASS.
 - test_game_gui_playtest.js: localGameUrl p1Deck/p2Deck; exit-to-launcher assertion -> exit-to-lobby (index.html); filtered benign post-exit /lobby WS 404. PASS.
 - test_game_gui_rebuild.js: localGameUrl; merged page_load/configure/launch; exit-to-lobby. 19/19 PASS.
 - test_game_gui_deep.js: 5 launcher blocks -> param boot; COLLECTION_DECK_RE picks representative built-in decks per collection; land-P/T assertion fixed to exclude ANIMATED lands (a land that is also a creature legitimately shows P/T, CR 208.3 - this was surfacing eric/gabriel avatar animated Forest 2/2); exit-to-lobby. 32 OK 0 FAIL.
 - test_network_e2e.js: ?mode=network&controller=fixed param boot + custom-deck localStorage seed. PASS.
 - test_network_random_e2e.js: also replaced the DELETED scripts/launch_network_game.sh dependency with direct server+native-random spawn; ?mode=network&controller=random. PASS (full game, 141 choices, game_over).
 - test_network_human_input.js: ?mode=network&controller=human param boot + deck seed. Migrated + RUNS, but KNOWN-FAILING on the pre-existing human-controller desync mtg-679 (identical hash f7ec406da80a882e/3b7dd10b9f66711d at choice_seq=1 action_count=45, deck-independent). Per 'desync is ALWAYS fatal' the test correctly REFUSES to pass; engine fix is out of scope (web/JS-only gate). Header documents this. Will pass once mtg-679 is fixed.

8/9 migrated tests pass; #9 (human_input) correctly blocked by mtg-679 (out of scope).
Result-JSON artifacts gitignored (web/*_test_results.json). No images. No engine/wasm change.
