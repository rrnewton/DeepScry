# Seed-19 (mtg-796) — RESOLVED: the bug was the WASM "0 targets" index encoding, NOT animation

**Stamp:** 2026-06-05 (branch `fix-seed19-mtg-796`, slot03-seed19)
**Status:** FIXED. seed 19 FAIL→PASS + all 9 other broad-robots seeds PASS, mtime-fresh.

PLAIN-LANGUAGE: seed 19 crashed on turn 24 when the browser player (WebRandom)
cast Fireball. The earlier diagnosis blamed a "man-land" (Mishra's Factory) that
was supposedly still counted as a creature on the browser after its temporary
animation should have worn off. **That diagnosis was wrong.** When I instrumented
the browser directly, every Factory was correctly a plain land, and the browser
computed the *same two* legal Fireball targets as the server. The real cause was
much simpler and lived in the network message format: Fireball lets you hit "any
number" of targets (including zero), and the browser's random AI legitimately
chose **zero** targets. The browser encoded "zero targets" as a made-up extra
menu index (one past the end of the real list); the server's menu had no such
entry and rejected it as an out-of-range choice → fatal desync. The fix makes the
browser send an **empty** choice for "zero targets", exactly like the existing
native client already does, which the server already understands as "chose none".

## Why the earlier "animated man-land" hypothesis was wrong (empirical refutation)

Two DEEPAC_-prefixed `log::warn` probes (the prefix is what the e2e captures into
`debug/netarch-undo-dumps/*_deepac_wasm.log`; the WASM logger is level-filtered,
not module-filtered, so even `game::*` warns surface as long as they carry that
prefix):

1. In `unwind_state_sync_to` (the per-turn rewind apply site,
   wasm/network/client.rs): at R=1675 every Mishra's Factory on the browser
   shadow was `is_creature=false, types=[Land]` — including card 51. The shadow
   does NOT retain a stale animation across the rewind. → **no animation
   reconstruction is needed.**
2. In `get_valid_targets_for_spell` (gated to `is_shadow_game`): at the turn-24
   Fireball (118) cast the browser computed `valid=[119,110]` — exactly the two
   creatures the server saw (WebRandom's own two factories, animated *this* turn).
   Card 51 was correctly excluded. → **the target SET matches; the predecessor's
   "browser sees 3 targets" was an unverified inference.**

Animation is only a PRECONDITION: animating 110/119 this turn created the two
creature targets that made a "choose 0 of 2" decision possible. Creature-ness is
not in `compute_view_hash`, so the hashes matched right up to the fatal — which is
consistent with, but NOT caused by, an animation divergence.

## Root cause (protocol parity)

Fireball is a DivideEvenly X-spell: `target_count_bounds_for_spell` returns
`(0, num_valid)`. WebRandom's RNG legally chose count=0. The WASM local
controller (`wasm/network/local_controller.rs::choose_targets`) encoded "no
targets" as a sentinel index `vec![valid_targets.len()]` = `vec![2]`. But the
server (`network/controller.rs::choose_targets`) builds exactly
`valid_targets.len()` options with **no** trailing "none" entry, and validates
`idx < options.len()` → index 2 is out of range → `DESYNC DETECTED: invalid
choice index 2 (only 2 options available)`.

The native `NetworkLocalController::choose_targets` never had this bug: it maps
chosen targets to `filter_map(position).collect()`, which is an EMPTY list for 0
targets, and the server's decode loop treats an empty index list as "0 targets
chosen". The WASM path was the sole divergent encoder.

## Fix

`mtg-engine/src/wasm/network/local_controller.rs`: removed the
`if chosen.is_empty() { vec![X.len()] }` sentinel branch in BOTH `choose_targets`
and `choose_permanents_to_sacrifice`, replacing it with the same
`filter_map(position).collect()` the native client uses (empty list for "none").
This restores WASM↔native parity. Two sibling methods (`choose_mana_sources_to_pay`,
`choose_cards_to_discard`) keep a similar sentinel but have DIFFERENT server
decode semantics and are effectively unreached in robots play — left as-is and
tracked in follow-up **mtg-809**.

## Evidence (mtime-fresh: native + wasm both built 15:21)

- seed 19: FAIL (turn 24) → **PASS** (now runs 29 turns / 422 choices).
- broad-robots no-regression, ALL PASS: 2(40t), 5(17t), 6(14t), 7(21t), 9(41t),
  11(27t), 18(47t), 20(28t), 42(25t). → **10/10 broad robots seeds green.**
- `cargo fmt --all -- --check`, `cargo clippy -p mtg-engine --all-targets
  --all-features --features network -- -D warnings`, and `make clippy-wasm`: all
  green.

Repro:
```
cd web && node test_network_gui_e2e.js \
  --deck decks/old_school/03_robots_jesseisbak.dck --seed 19
```

## Scope / relationships
seed-7 fix (@6a708dda) does not regress here. This is the LAST broad-robots
blocker before all-10-strict → the action_count exclusion prize (eb8f938e) → 100%.
Related: mtg-784, mtg-797, mtg-789, mtg-610; follow-up mtg-809.
