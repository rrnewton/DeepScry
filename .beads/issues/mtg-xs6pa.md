---
title: 'Bug: DB$ Sacrifice UnlessPayer/UnlessCost self-sacrifice not implemented'
status: open
priority: 3
issue_type: task
depends_on:
  mtg-4zlpr: parent-child
created_at: 2026-06-03T03:44:23.965377144+00:00
updated_at: 2026-06-03T03:44:23.965377144+00:00
---

# Description

Bug: DB$ Sacrifice with UnlessPayer$/UnlessCost$ (self-sacrifice-unless-pay) is
converted to a ForceSacrifice (opponent sacrifices) instead of a
pay-or-self-sacrifice. No mana prompt is presented and the host never
self-sacrifices.

Discovered (2026-06-02_#2674(51c28554)) during the 1994 World Championship
compat sweep (mtg-4zlpr).

Affected cards (at least):
- Stasis (mtg-f3qdj): "At the beginning of your upkeep, sacrifice Stasis unless
  you pay {U}." -> Stasis never self-sacrifices; drawback missing.
- Any other "sacrifice CARDNAME unless you pay X" upkeep card uses the same
  SVar:...:DB$ Sacrifice | UnlessPayer$ You | UnlessCost$ <cost> shape.

Root cause: effect_converter.rs ApiType::Sacrifice builds a ForceSacrifice
effect (chooses an opponent's permanent) and ignores UnlessPayer$/UnlessCost$.
The intended semantics: the named payer may pay UnlessCost; if they do NOT, the
DEFINED permanent (default Self) is sacrificed by its controller. Needs a
self-sacrifice + UnlessCost wrapper (cf. the existing unless-pay prompt used by
counter-unless-pay).

Fix plan: route DB$ Sacrifice with Defined$ Self (or no ValidCard targeting) +
UnlessPayer$/UnlessCost$ into a SacrificeSelfUnlessPay effect that prompts the
payer for the cost and sacrifices the host on decline.

Severity: gameplay correctness (host becomes strictly better than printed). Game
remains legal/complete (no crash).
