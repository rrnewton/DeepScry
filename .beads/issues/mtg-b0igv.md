---
title: 'Bug: Prowess PumpCreature trigger fizzles (placeholder not resolved to self)'
status: open
priority: 2
issue_type: task
created_at: 2026-06-06T04:29:45.126103182+00:00
updated_at: 2026-06-06T04:42:51.462527162+00:00
---

# Description

Prowess triggers fire (log: "Trigger: <creature> - [noncreature] Prowess (+1/+1 until end of turn)") but then immediately log "[WARN pump] PumpCreature fizzled: unresolved target 0". The +1/+1 bonus is never applied.

Root cause: In loader/card.rs instantiate(), prowess triggers are created with target: CardId::new(0) (placeholder). The comment in resolve_effect_placeholder() (triggers.rs line 399-402) explicitly says "Note: PumpCreature with CardId::new(0) is NOT handled here because it is ambiguous: - CardDrawn triggers: this creature gets +X/+Y → target is self - ETB triggers: target creature gets +X/+Y → need to find a target. Let context-specific handlers deal with this ambiguity."

However, check_spellcast_triggers() in actions/mod.rs (line 7646+) calls resolve_effect_placeholder() but does NOT have a context-specific resolution for PumpCreature placeholders. The prowess effect reaches execute_effect() with target=0 and fizzles.

Affected cards: Any card with K:Prowess keyword, including:
- Otter Token (created by Stormchaser's Talent and Ral, Crackling Wit)
- Any creature with printed Prowess
- Artist's Talent level-2 and other Class abilities granting triggers

FIXED in branch compat-2025-champ-izzet: Added context-specific PumpCreature placeholder resolution in check_spellcast_triggers() (mtg-engine/src/game/actions/mod.rs). After resolve_effect_placeholder(), if the effect is PumpCreature with a placeholder target, it is now resolved to trigger.source_card_id (the prowess creature itself). This is unambiguous in the SpellCast context.

Also fixed the PutCounter log message in the same function to use the actual counter_type.display_name() instead of hardcoding "+1/+1 counter", which fixes the spurious "+1/+1 counter" log for Ral loyalty counters (mtg-gi104 was a separate tracking issue but shares the fix).

Status: Fixed, pending make validate and merge.
