---
title: WASM choose_mana_sources_to_pay still carries a vec![len] none-sentinel (single-select decode; 0-source unreachable)
status: open
priority: 3
issue_type: bug
created_at: 2026-06-05T22:34:14.757243806+00:00
updated_at: 2026-06-05T22:53:59.955931656+00:00
---

# Description

After mtg-8ow9h fixed the WASM choose_targets + choose_permanents_to_sacrifice + choose_cards_to_discard '0 chosen' encoding (they now send an EMPTY index list like the native NetworkLocalController, which the server decodes as '0 chosen'), ONE sibling WASM method in mtg-engine/src/wasm/network/local_controller.rs still encodes the empty case as a vec![X.len()] sentinel:
- choose_mana_sources_to_pay: vec![available_sources.len()]

CORRECTION (adversarial desync-review of the seed-19 fix, 2026-06-05): the earlier framing that BOTH mana and discard had 'different decode semantics' was WRONG for discard. The server choose_cards_to_discard decode (controller.rs:1121-1133) is the SAME idx<len index-loop as choose_targets (no trailing 'none' slot), so the WASM hand.len() sentinel was a LIVE latent desync of the exact mtg-8ow9h class (reachable via a 'discard up to N' effect whose controller legally chooses 0; low reachability but real). It was therefore FIXED as part of the seed-19 commit (empty-list parity), NOT deferred.

REMAINING (mana only): choose_mana_sources_to_pay genuinely DIFFERS — the server decodes it SINGLE-select (result.indices.first().unwrap_or(0)), so an empty list would silently pick source index 0 (wrong) rather than mean '0 sources'; the len-sentinel at least errors loudly. The 0-sources case is only reachable for a 0-cost payment, which does not invoke this choice in practice — so it is currently UNREACHED (latent, not live), unlike discard. ACTION: audit choose_mana_sources_to_pay against the server single-select decode; either prove 0-sources is unreachable and delete the dead sentinel branch, or (if a 0-source payment path exists) redesign the wire encoding so native+WASM agree. Add a unit/e2e test that exercises the path if reachable. Low priority (no known repro). Related: mtg-8ow9h.
