---
title: 'Polish: unify XPaid lowering (executor vs logger) + actions/mod.rs module split'
status: open
priority: 4
issue_type: task
created_at: 2026-05-30T07:38:25.722266219+00:00
updated_at: 2026-05-30T07:38:25.722266219+00:00
---

# Description

Low-priority code-elegance follow-ups from the 2026-05-30 elegance review of card-compat waves 8-10 (review verdict: OVERALL CLEAN, no hacks, no blockers). Tracked here so they aren't lost; NOT urgent.

1. **XPaid lowering duplicated (executor vs logger)** — refs mtg-521 (Mind Twist). XPaid→concrete value lowering happens twice: at execution in `actions/mod.rs::resolve_x_paid_effect`, and at logging time in `priority.rs` (post-resolution rewrite arms in resolve_top_spell_from_stack). This is documented intentional pragmatism (logger sees original stored `card.effects` in XPaid form; executor sees a lowered copy), NOT a hack — but a future refactor could unify by having the logger consume a per-resolution lowered effect copy rather than re-deriving. Polish only; current path is correct + tested.

2. **`actions/mod.rs` is ~8549 lines** — far past the project's 2000-line module-split guideline (pre-dates the recent fixes; recent additions were well-scoped ~100 lines). Candidate for a broader actions-layer module split in a future architecture pass.

3. **Doc nits (trivial):** add a short grammar reference comment to `count_cards_matching_filter` (now handles card-types/subtypes/colors/ownership/'+'-joined qualifiers — mtg-517), and a docstring on `Card::is_color` clarifying basic lands are colorless regardless of subtype (CR 105.2a).

Source: elegance-review subagent, integration @30dd3c20.
