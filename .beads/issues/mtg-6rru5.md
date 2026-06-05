---
title: ServerConfig start-state/puzzle injection for deterministic network-state e2e tests
status: open
priority: 3
issue_type: task
created_at: 2026-06-05T04:20:16.338241858+00:00
updated_at: 2026-06-05T04:20:16.338241858+00:00
---

# Description

Network e2e tests (network_e2e.rs, netarch_lockstep_oracle_e2e.rs, tests/network_vs_local_equivalence_e2e.sh) can only build games from deck+seed: ServerConfig has only {seed, decks, starting_life, deck_visibility}. There is no way to inject a pre-built GameState / puzzle start-state into the server, so deterministic network regression scenarios that need a specific board/hand (e.g. an empty-handed player facing a targeted discard — mtg-u3dwj/mtg-d62r3 BLOCKER-1) cannot be written as a server/client e2e; they must be Rust integration tests against the GameLoop instead. Add an optional start-state/puzzle field to ServerConfig (mirroring 'mtg tui --start-state PUZZLE') so the in-process loopback harness can start the authoritative game from a fixed GameState. Enables richer deterministic network desync regression tests. Surfaced during the mtg-u3dwj empty-hand-discard fix (the Rust integration test discard_prepare_ordering_tests was used instead).
