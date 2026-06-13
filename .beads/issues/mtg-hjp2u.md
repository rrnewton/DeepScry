---
title: 'Bug: Return<N/Type> cost unsupported in AlternativeCost + unless-cost paths'
status: open
priority: 3
issue_type: bug
created_at: 2026-06-13T19:45:23.502909108+00:00
updated_at: 2026-06-13T19:45:23.502909108+00:00
---

# Description

STAMP: 2026-06-13_#3389(1ae0e772f)

The Return<N/Type> cost concept (return permanent(s) to owner's hand as a cost)
was added for ACTIVATED abilities in compat-2000-wave10 (Cost::ReturnToHand;
Attunement, mtg-mbcm0). TWO other cost paths still mis-handle Return<...>:

1. AlternativeCost path (loader/card.rs ~line 5971): an AlternativeCost block
   like Daze's 'Cost$ Return<1/Island>' (return an Island instead of paying
   {1}{U}) is passed to ManaCost::from_string('Return<1/Island>') -> garbage,
   no Island-return alternative is offered. Affects Daze and similar 'free' /
   alt-return counterspells (Foil, Misdirection-family return costs, etc.).

2. unless-cost path (loader/effect_converter.rs parse_unless_cost else branch):
   'sacrifice ~ unless you return a land' (Coral Atoll / Dormant Volcano /
   Everglades karoo lands' ETB-return) also routes Return<...> through the
   mana-cost fallback.

Fix shape: reuse Cost::ReturnToHand. (a) In the AlternativeCost handler, try
Cost::parse() first and represent a Return alt-cost as an alternative payable
cost. (b) Add an UnlessCostType::ReturnToHand arm in parse_unless_cost.

~44 cards total match 'Cost$ Return<' in cardsfolder/; the activated-ability
subset is fixed (mtg-mbcm0). This issue tracks the remaining alt-cost +
unless-cost subset.

Parent: mtg-912 (2000 WC tracker), mtg-913 (backlog).
