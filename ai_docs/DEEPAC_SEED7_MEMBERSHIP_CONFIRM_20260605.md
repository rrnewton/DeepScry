# Seed-7 deep-AC blocker — DECISIVE membership confirm (corrects the "stamping skew" reframe)

**Stamp:** 2026-06-05 (branch `fix-deep-ac`, slot03-lockstep)
**Supersedes** the seed-7 conclusion in `DEEPAC_BLOCKERS_CHECKPOINT_20260605.md`
(the "choice_seq↔action_count STAMPING SKEW, NOT a state divergence" reframe).

PLAIN-LANGUAGE: the browser-vs-desktop network test crashes on seed 7 because the
**desktop (observer) client genuinely loses a card**. When the browser player
casts Demonic Tutor and fetches a card (Fireball, internal id 97) from its
library into its own hand, the desktop client — which is only *watching* that
happen — takes the card OUT of the browser player's library but never puts it
INTO that player's hand. So the desktop thinks the browser has 5 cards in hand
while the server (and the browser itself) correctly say 6. That one-card
disagreement is hashed into the sync check and is fatal. This is a real
lost-card bug, NOT a timing/labelling artifact, and it is the *observer-side
missing-opponent-delta* family (mtg-ho2r8 §1-2 / mtg-o99ow), not the in-stack
"stamping alignment" the previous checkpoint hypothesised.

## How this was proven (fresh, mtime-fresh, maximally-strict instrumented repro)

Built server (`make build-network`, 12:16) + WASM (`make wasm-network`, 12:15)
fresh, then:

```
cd web && node test_network_gui_e2e.js \
  --deck decks/old_school/03_robots_jesseisbak.dck --seed 7 --undo-dump
```

Added two diagnostic dumps (committed, see "Instrumentation" below):
- server `DebugSyncInfo.hand_ids[2]` — BOTH players' known hand CardIds, printed
  in the mismatch box + a per-player set-difference line.
- WASM `WASM_CARD_DETAIL hand0=/hand1=` — the shadow's known hand CardIds.

### The mismatch box (fresh run), `player=0` (NATIVE desktop client) `choice_seq=230 action_count=1341`:

```
SERVER : Hands [7,6]  Hand CardIds(known): P0=[12,24,42,43,50,57,58] P1=[79,86,90,91,96,97]
CLIENT : Hands [7,5]  Hand CardIds(known): P0=[12,24,42,43,50,57,58] P1=[79,86,90,91,96]
DIFFERENCES:
  - Hand sizes DIFFER
  - P1 hand CardIds DIFFER: on_server_only=[97] on_client_only=[]
```

Every OTHER zone is byte-identical: P0 hand (7, identical), both graveyards,
both libraries (sorted-identical, **97 absent from both** → it already left the
library on both sides), battlefield (16, identical incl. the played land 82).
The ONLY divergence is **P1 hand membership: card 97 present on server, absent
on the client** — a single lost card.

### Two facts that overturn the prior reframe

1. **It is a STATE divergence, not a stamping skew.** Direct membership shows
   `on_server_only=[97]`. The browser's OWN forward shadow is fine
   (`WASM_CARD_DETAIL seq=242 ac=1340 hand1=[79,86,90,91,96,97]`, hash
   `aed45eeb8ebd1756` == server hash). The card is lost only on the OBSERVER.

2. **The "+12 choice_seq drift" / "two acs (1230 vs 1341) straddling the land
   play" in the prior checkpoint was an artifact of `choice_seq` being counted
   PER-PLAYER**, not a bug. Server dumps:
   `player=1 choice_seq=230 action_count=1230` (browser's 230th choice, turn 17)
   and `player=0 choice_seq=230 action_count=1341` (desktop's 230th choice, turn
   19). The fatal is the desktop's (`player=0`); the box label "P1" is the
   1-indexed display of player **index 0** = the native desktop client (it sends
   rich `debug_info`; the WASM browser sends `debug_info:None`, so every
   full-detail CLIENT box is necessarily the NATIVE client's shadow).

## Root cause (pinned to the line)

The observer of an OPPONENT's library search-to-hand:
- The server sends the observer a **dummy `Searched` reveal** (empty name,
  carrying card_id 97, owner = searcher, stamped at the search ac). Dummy
  reveals are **skipped wholesale** — `reveal_processor::process_card_reveal`
  returns at the `is_dummy_reveal` guard (reveal_processor.rs:77-86), so 97 is
  never instantiated on the observer.
- The observer's GameLoop then replays the opponent's
  `LibrarySearch(Some(97))` (via `searched_card_for` / OpponentChoice
  `library_search_result`) and calls `move_card(97, Library → Hand, owner=P1)`.
- `player_hand_size` is literally `zones.hand.len()` (controller.rs:646-651).
  The observer's P1 hand zone ends at **5** — 97 is never added as a counted
  hand card. 97 leaves the library accounting (lib=36, matches) but is deposited
  nowhere countable → **lost card**.

This is the SAME re-materialization class as mtg-ho2r8: an opponent
hidden-zone→hand move on the observer needs the card materialized into the
opponent's hand zone as a counted, identity-hidden reserved card. The dummy
`Searched` reveal deliberately preserves identity-hiding by skipping
instantiation — but in doing so it drops the count/zone-membership that the hand
move requires.

## Fix direction (durable, mtg-ho2r8 §1-2 — NOT a session-surgical patch)

Carry the opponent search-to-hand as a counted hand-membership delta on the
observer: when the observer applies an opponent's `Searched`-into-hand, deposit
an identity-hidden reserved placeholder into the opponent's hand zone (so
`hand.len()` increments) keyed by the search-resolution action_count, surviving
rewind/replay. This is the missing-opponent-delta half the prior checkpoint
explicitly deferred. It must be verified across ALL 10 robots seeds in
maximally-strict mode (action_count re-included) before the action_count
exclusion prize (`eb8f938e`) is re-applied. Related: mtg-o99ow (reveal-as-choice
unification keyed by ac), mtg-677 (rewind-faithful reveals).

## Why NOT the stamping-alignment fix the brief requested

The brief (inherited from the prior checkpoint) asked to align the client's
submit-hash action_count with the server's validation action_count. The fresh
data shows the action_counts already AGREE (`SRV_P1_RECV client_ac=1341
expected_ac=1341`) and the browser's own forward hash already matches the
server. There is no ac-stamp to align; the desktop observer is simply missing a
card. Pursuing stamping alignment would be fixing a non-bug.

## Instrumentation committed with this checkpoint (kept — it is what settled it)

- `protocol.rs`: `DebugSyncInfo.hand_ids: [Vec<u32>; 2]` (serde default).
- `state_hash.rs::build_debug_sync_info`: populate both players' known hand ids.
- `server.rs`: print `Hand CardIds(known)` + per-player set-difference
  (`on_server_only` / `on_client_only`) in the mismatch box.
- `wasm/network/local_controller.rs`: `WASM_CARD_DETAIL hand0=/hand1=` dump.

All four are diagnostics-only (network-debug-gated paths / debug box), no
behavior change. Repro artifacts: `debug/netarch-undo-dumps/gui_random_03_robots_jesseisbak_seed7_*`.
