---
title: 'Unlogged regular mana_pool payment in pay_from_total_mana/pay_cost — per-action undo gap (separate from mtg-ba6uq #7)'
status: open
priority: 3
issue_type: bug
created_at: 2026-06-03T06:56:26.333251918+00:00
updated_at: 2026-06-03T06:56:26.333251918+00:00
---

# Description

Found while fixing mtg-ba6uq #7 (combat_mana_pool). DISTINCT from the 8-hole audit: that #7 covered the per-player combat_mana_pool (Avatar/Firebend) field; THIS is the REGULAR mana_pool.

GAP: Player::pay_from_total_mana (core/player.rs) and ManaPool::pay_cost mutate the regular mana_pool with NO covering undo GameAction. Call sites (all unlogged): actions/mod.rs:353, 2136 (pay_from_total_mana), and 9047/9073/9281 (pay_cost). A payment reduces mana_pool but logs nothing.

mana_pool IS hashed: it is a plain serialized field (core/player.rs:19, 'empties at end of each step'), NOT in any EXCLUDED_FIELDS list in state_hash.rs (only mana_state_version is excluded). So a divergence in mana_pool WOULD show in compute_undo_test_hash / network hash.

RISK READ (honest, for the zero-undo-log-incompleteness bar):
- NETWORK / turn-start rewind path: LIKELY BENIGN. mana_pool empties at the end of EVERY step (CR 500.4), so at any turn boundary it is already 0. rewind_to_turn_start lands on a boundary where mana_pool==0 regardless, and the logged AddMana undos net to 0 on a FULL rewind (add R then pay R → undo pay is a no-op-by-omission, undo AddMana does saturating_sub(R) on 0 → 0 == correct start). So full-rewind endpoints match; this is why the existing oracles stayed green and the 8-hole audit didn't flag it.
- PER-ACTION undo (MCTS / human mid-step / UndoTest PARTIAL rewind): POTENTIAL real hole. A partial undo that stops BETWEEN an AddMana and its consuming payment would observe mana_pool=0 where it should be R (the payment was never logged, so it isn't restored; only undoing the earlier AddMana would touch the pool, and saturating_sub already clamped). test_aggressive_undo_snapshots does partial rewinds to snapshots but passed — either no payment fell between its snapshot points in the sampled random games, or the saturating-sub masked it. NOT proven safe for the partial-undo case.

RECOMMENDATION: log an EmptyManaPool-style snapshot (or a SetManaPool{prev}) before each pay_cost/pay_from_total_mana spend, mirroring mtg-ba6uq #4's SetCardCounters snapshot pattern, so the regular mana_pool round-trips on the per-action path too. Then add a per-action negative test that does a PARTIAL undo stopping between a land-tap (AddMana) and a spell payment, asserting mana_pool restores to the added amount. Until then the 'zero undo-log incompleteness' bar is met for the NETWORK/turn-start path but has this caveat on the per-action path.

Relates mtg-ba6uq (#7), mtg-610 (netarch). Filed by netarch-dev6 on netarch-undo-holes.
