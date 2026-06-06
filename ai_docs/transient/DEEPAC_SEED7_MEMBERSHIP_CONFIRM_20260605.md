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
missing-opponent-delta* family (mtg-799 §1-2 / mtg-752), not the in-stack
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

This is the SAME re-materialization class as mtg-799: an opponent
hidden-zone→hand move on the observer needs the card materialized into the
opponent's hand zone as a counted, identity-hidden reserved card. The dummy
`Searched` reveal deliberately preserves identity-hiding by skipping
instantiation — but in doing so it drops the count/zone-membership that the hand
move requires.

## ✅ FIXED (slot03-lockstep, 2026-06-05) — partial-`valid_cards` index for Remote opponent search

**Seed 7 now PASSES**; all 8 previously-green robots seeds (2,5,6,9,11,18,20,42)
still PASS (no regression); seed 19 still fails at its KNOWN, separate
Mishra's-Factory-animation point. Both binaries mtime-fresh.

The "DEEPER PIN" below (actions/mod.rs:4777) was itself a misread caused by a
MISPLACED probe: my `choose_from_library_with_hook` probe sat AFTER the
`if !self.is_network_mode() { … return }` early-return, so it never fired and I
wrongly concluded the function was unused. A probe at the function ENTRY (which
bypasses that early return) revealed the truth:

```
SEED7_PROBE cflwh ENTRY: player=1 ctrl_type=Network is_network_mode=false valid_cards_len=37 ...   ← SERVER
SEED7_PROBE cflwh ENTRY: player=1 ctrl_type=Remote  is_network_mode=false valid_cards_len=4  ...   ← OBSERVER
```

`choose_from_library_with_hook` IS the resolution path (via its
`!is_network_mode()` branch, since the legacy pre-choice hook is unconfigured —
`is_network_mode()=false`). The real bug: the OBSERVER's `valid_cards` is only
the **materialized subset** of the opponent's library (4 of the server's 37 — the
reserved/unrevealed cards are excluded). The match only fell back to the
authoritative server-fetched CardId when `valid_cards.is_empty()`; with a
**partial, non-empty** `valid_cards` it took `Ok(Some(index)) if index <
valid_cards.len()` — but the server's index addresses ITS 37-card list, so on the
observer it either selected the WRONG card or (index ≥ 4) fell through to
`Ok(None)` → the tutored card 97 was never fetched → P1 hand 5 vs server 6.

**FIX (two parts):**
1. `game_loop/network_choice.rs` (`choose_from_library_with_hook`): for a
   **Remote** controller (replaying an opponent's hidden search), IGNORE the
   partial-`valid_cards` index and use the authoritative server-fetched CardId
   (`searched_card_lookup` → `take_library_search_result`). Generalises the
   existing empty-list arm to the partial case.
2. `network/client.rs`: add the native `SharedNetworkState::searched_card_for`
   (ac-keyed dummy-`Searched` reveal lookup) and wire it into the native
   `GameLoop` via `with_searched_card_lookup` — previously WASM-only. This makes
   the fetch **rewind/replay-surviving** (the raced `take_library_search_result`
   is empty on replay).

The `actions/mod.rs:4777` `execute_effect` SearchLibrary arm (the
"try_get(None)" path) is the NON-interactive fallback used only when
`needs_interactive` is false; it is NOT on the network spell-resolution path
(`needs_interactive` includes `SearchLibrary`, routing to the discard-hook +
`choose_from_library_with_hook`). It remains a latent mtg-725 concern for any
non-interactive caller but is NOT the seed-7 cause.

## DEEPER PIN (slot03-lockstep, 2026-06-05, post fix-attempt) — exact lost-move site [SUPERSEDED — see FIXED section above]

A first fix attempt (wire the native `searched_card_lookup` + fall back to it in
the `UseChoice` library-search branch) had **ZERO effect** — byte-identical hash
`9455` across three runs. Instrumentation (`SEED7_DBG`) proved why:
**`choose_from_library_with_hook` is called ZERO times all game** (server, native,
browser). The opponent's tutor does NOT go through the search-pick / `UseChoice` /
`searched_card_lookup` machinery at all. That fix was reverted (unverified +
wrong path = would have been a band-aid).

Actual resolution path, pinned:
- Demonic Tutor is a **Sorcery**: `A:SP$ ChangeZone | Origin$ Library |
  Destination$ Hand | ChangeType$ Card` (`cardsfolder/d/demonic_tutor.txt`).
- `ApiType::ChangeZone` with `Origin=Library` converts to **`Effect::SearchLibrary`**
  (`effect_converter.rs:404→464`).
- A Sorcery's `Effect::SearchLibrary` resolves from the stack via
  **`GameState::apply_effect` → actions/mod.rs:4777**, NOT the GameLoop's
  `choose_from_library_with_hook` (that arm, priority.rs:3859, is for
  activated-ability/in-stack resolution; it was never hit here).
- That arm finds the card with:
  ```rust
  for &card_id in &library_cards {
      if let Some(card) = self.cards.try_get(card_id) {        // ← reserved ⇒ None
          if Self::card_matches_search_filter(card, filter) { found_card = Some(card_id); break; }
      }
  }
  if let Some(card_id) = found_card { self.move_card(card_id, Library, dest, player)?; }
  if shuffle { self.shuffle_library(player); }
  ```
- On the SERVER the opponent's library cards are real instances → `found_card =
  first "Card"-matching card` (= 97) → moved to hand. On the OBSERVER the
  opponent's library cards are **reserved (instance-less)** → `try_get` returns
  None → the filter is never checked → **`found_card = None` → no move** → then
  `shuffle_library` reorders, and the subsequent server `LibraryReordered`
  overwrites to the post-fetch order → **97 is silently dropped**. Observer P1
  hand stays 5; server 6.

**This is the mtg-725 `try_get(None)` nondeterminism class**: a control-flow
branch on whether a reserved opponent card is materialized. The non-interactive
`Effect::SearchLibrary` resolution is information-DEPENDENT (it reads opponent
library card identities that the observer's shadow does not have), violating the
controller-information-independence invariant — but here it is the ENGINE
resolution, not a controller.

### Correct fix direction (durable — confirmed rearchitecture, NOT session-surgical)
The network resolution of a Sorcery's `Effect::SearchLibrary` (and any
hidden-library ChangeZone→Hand) must use the **authoritative fetched CardId**
(the rewind-surviving `searched_card_lookup` / dummy `Searched` reveal) instead
of re-deriving `found_card` by scanning materialized library instances — OR route
the resolution through `choose_from_library_with_hook` like the activated-ability
path does. Either way it must materialize the reserved opponent card into the
hand as a counted, identity-hidden slot and survive rewind/replay. Touching the
`GameState::apply_effect` SearchLibrary arm to be network-shadow-aware is the
core of the mtg-799 / mtg-725 work and was correctly NOT band-aided this
session.

## Fix direction (durable, mtg-799 §1-2 — NOT a session-surgical patch)

Carry the opponent search-to-hand as a counted hand-membership delta on the
observer: when the observer applies an opponent's `Searched`-into-hand, deposit
an identity-hidden reserved placeholder into the opponent's hand zone (so
`hand.len()` increments) keyed by the search-resolution action_count, surviving
rewind/replay. This is the missing-opponent-delta half the prior checkpoint
explicitly deferred. It must be verified across ALL 10 robots seeds in
maximally-strict mode (action_count re-included) before the action_count
exclusion prize (`eb8f938e`) is re-applied. Related: mtg-752 (reveal-as-choice
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

---

# Seed-19 (mtg-796) — mtg-789 #2 done + divergence pinned to the Fireball target choice

PLAIN-LANGUAGE: seed 19 crashes on turn 24 because the browser player's
Fireball offers ONE MORE valid target than the server allows. Both sides agree
on every *hashed* fact, but the browser thinks an extra permanent is a legal
target. The most likely culprit is a Man-land (Mishra's Factory) whose
"is currently an animated creature" status — a temporary continuous effect that
is NOT part of the network sync hash — has drifted between the two sides.

## mtg-789 #2 DONE (committed): WASM client now populates SubmitChoice.spell_ability

Threaded the chosen `SpellAbility` through the WASM priority path
(`choose_spell_ability_to_play` → new `submit_choice_to_server_with_ability` →
new `WasmNetworkClient::submit_choice_with_ability` → `submit_choice_full`),
mirroring native `local_controller.rs:461-484`. The server's always-on CardId
cross-check (controller.rs:663-685) now protects the DEPLOYED WEB path, not just
native-vs-native. NO REGRESSION: robots seeds 2 and 42 still PASS with the
cross-check active (it does not false-positive — the WASM and server enumerate
identical abilities on converged seeds).

## Seed-19 divergence pinned (NOT closed — non-hashed continuous-effect class)

The spell_ability cross-check does NOT crash seed 19 earlier: the existing
index-out-of-bounds guard (controller.rs:650-656) fires FIRST on the same
divergence. The divergence is genuinely AT turn 24, not an upstream silent
ability swap.

Crash: `DESYNC: NetworkController 1 received invalid choice index 2 (only 2
options available). Client sent indices [2]` — the Fireball TARGET choice.
WASM undo sequence (seed 19, turn 24):
  [1761] Choice(P1 #32 = CastSpell{card_id:118})   (Fireball)
  [1762] SetXPaid(card=118, prev=0)
  [1763] Choice(P1 #33 = XValue(0))
  [1764] <Fireball target choice> — WASM enumerates 3 valid targets, server 2;
         WASM picks index 2 (3rd) → server rejects (only 2).

All state hashes MATCH up to the crash (no mismatch box) → the diverging field
is NOT in the view hash but DOES affect target legality. Both battlefields are
full of Mishra's Factory man-lands (NativeAI: 49,51,27; WebRandom: 119,110) that
were actively animated via `ActivateAbility{ability_index:1}` ("becomes a
creature") in the turns leading up (WASM undo [1734]/[1739]/[1744]/[1746]).
LEADING HYPOTHESIS: a Mishra's Factory is an animated CREATURE on the WASM
shadow but a plain land on the server (or its animation expiry diverged), so the
WASM offers it as an extra Fireball target. This is the mtg-784 controller
option-set family: a continuous-effect / type-changing state (creature
animation) that `compute_view_hash` does not capture (it hashes card_id + tapped
+ controller, NOT P/T/types/animation) diverges and surfaces only through option
generation.

NEXT STEP to close: instrument the Fireball target enumeration to dump the 3 WASM
vs 2 server target CardIds at the crash (name the extra), then bisect where that
permanent's animated-creature status diverged (turn-end "until end of turn"
cleanup vs the animation ActivateAbility application on the shadow). Likely fix
family: ensure continuous-effect / type-change state is reconstructed in lockstep
on the shadow (or include the relevant derived targetability in the cross-check).
Repro: `cd web && node test_network_gui_e2e.js --deck
decks/old_school/03_robots_jesseisbak.dck --seed 19 --undo-dump`.
