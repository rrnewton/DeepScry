---
title: Spell-path draw-then-discard network-equivalence e2e (Blast of Genius / Ancient Excavation / Artificer's Epiphany)
status: open
priority: 3
issue_type: task
created_at: 2026-06-05T04:20:16.341418206+00:00
updated_at: 2026-06-05T04:20:16.341418206+00:00
---

# Description

Follow-up to mtg-u3dwj/mtg-d62r3. The fix added prepare->sync->decide to the SPELL-resolution discard path (resolve_top_spell_with_discard_hook) for draw-then-discard SPELLS. That path is directly covered by the Rust unit test game_loop::priority::discard_prepare_ordering_tests (empty-hand + non-empty, prove-it-bites). A full NETWORK-equivalence e2e for a real draw-then-discard spell would add end-to-end coverage. Candidate cards that EXIST in cardsfolder: b/blast_of_genius (Draw 3 then Discard 1), a/ancient_excavation, a/artificers_epiphany. Blocked on a deterministic deck+seed that routes the spell-resolution discard in a network-vs-local equivalence game (cf. the rogerbrand canary which covers the ACTIVATED-ability draw-then-discard path). Deferred as best-effort per team-lead; the Rust unit test is the mandatory gate.
