---
title: Game-log deck-name header lines before Turn 1
status: open
priority: 4
issue_type: task
created_at: 2026-06-14T07:06:26.078091290+00:00
updated_at: 2026-06-14T07:06:26.078091290+00:00
---

# Description

Emit two deck-name header lines at the very start of a game's gamelog, immediately before the '>>> Turn 1 - ...' header, e.g.:

  P1: Red Burn Fuzz deck
  P2: Blue Control Fuzz deck

so the log is self-identifying about which decks are playing (task #9, user request).

DESIGN:
- New strong type core::DeckName (Arc<str>, serde), mirroring PlayerName.
- DeckList gains 'name: Option<DeckName>', captured from the .dck 'Name=' metadata during parse; load_from_file falls back to the file stem.
- GameState gains 'deck_names: [Option<DeckName>; 2]' (#[serde(default)]), populated at game-init from the two DeckLists. Public info, serialized -> replicates to network shadow + survives WASM rewind/replay, so the deterministic header is identical across native/WASM and local/network.
- GameLoop::emit_turn_one_header emits the per-seat 'P<n>: <deck> deck' lines (new helper emit_deck_name_headers) right before the Turn 1 separator, only when a deck name is present.

DETERMINISM / GOLDEN SAFETY:
- Puzzle / --start-state games load no decks -> deck_names = [None, None] -> ZERO header lines -> puzzle golden logs UNCHANGED (verified: make puzzle-golden-check green WITHOUT re-bless).
- Network deck submissions carry no name metadata -> both server and client derive None -> network gamelog identity preserved.

TESTS:
- loader::deck: parse captures Name=; absent Name leaves None.
- game_loop: deck-name lines precede Turn 1 (order P1<P2<Turn1, exactly one each); NO lines when decks absent (puzzle parity).

STATUS: DONE, validated.
