---
title: 'Bug: Discarded opponent-cause not threaded through triggered-ability discard path (Hypnotic Specter)'
status: open
priority: 3
issue_type: bug
created_at: 2026-06-10T02:45:59.325567218+00:00
updated_at: 2026-06-10T02:45:59.325567218+00:00
---

# Description

Bug/Gap: TriggerEvent::Discarded opponent-cause is plumbed for the SPELL-resolution discard path but NOT the triggered-ability discard path.

Context: implementing Psychic Purge's opponent-forced-discard punisher
(mtg-648, 2026-06-09_#3106) threaded an explicit `cause: Option<PlayerId>`
through `GameState::discard_card`. The cause is correctly supplied from:
- resolve_top_spell_with_discard_hook (spell controller = spell_owner) — the
  Mind Twist / Hymn to Tourach / Mind Rot family.
- priority_round activated-ability discard/loot (current_priority).

NOT yet supplied (defaults to cause=None): forced discards that resolve through
the generic GameState::execute_effect(Effect::DiscardCards { .. }) path invoked
from a TRIGGERED ability — e.g. Hypnotic Specter:
  T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Opponent
    | Execute$ TrigDiscard
  SVar:TrigDiscard:DB$ Discard | Defined$ TriggeredTarget | NumCards$ 1 | Mode$ Random
If Hypnotic Specter's random discard happens to hit Psychic Purge, the
punisher SHOULD fire (an opponent's ABILITY caused the discard) but currently
does not, because the trigger-resolution discard goes through execute_effect
with cause=None.

Why deferred: execute_effect is a shared chokepoint with 52 callers; threading
cause through all of them (or adding a cause to the DiscardCards/Loot Effect
variants, ~90 match sites) is out of proportion to closing this narrow gap for
one card. The DOMINANT 1994 opponent-discard vector (discard SPELLS) is
covered. The current behavior is STRICTER-than-printed only for the
triggered-ability sub-case (punisher silently absent), never wrong-direction.

Proper fix (future): route the trigger-resolution forced-discard through a
cause-aware path. Either (a) extract execute_effect's DiscardCards/Loot arms
into a helper that takes `cause` and have check_triggers_for_controller pass
the trigger's controller, or (b) give the trigger-execution path a cause-aware
discard entry point. Add an e2e (Hypnotic Specter random-discards Psychic Purge
-> caster loses 5 life).

Tracked under Card Compatibility: Psychic Purge (mtg-534).
