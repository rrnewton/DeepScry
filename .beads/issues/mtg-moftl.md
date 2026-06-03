---
title: 'Card: Old Man of the Sea — exact tapped+powerLEX control duration (1994 B1 follow-up)'
status: open
priority: 3
issue_type: bug
created_at: 2026-06-03T21:10:13.711570875+00:00
updated_at: 2026-06-03T21:10:13.711570875+00:00
---

# Description

Follow-up to mtg-713 B1 / mtg-egzyt (Aladdin). The B1 GainControl infrastructure (ControlDuration + TargetRestriction + recompute_source_control SBA pass + targeting/activation arms) now makes Old Man of the Sea castable and correctly TARGETED:
- ValidTgts$ Creature.powerLEX is parsed (TargetRestriction.power_le_source) and enforced via matches_with_source_power (X = Old Man's current power), so it only targets creatures with power <= Old Man's power.

REMAINING (PARTIAL): Old Man's exact DURATION is approximated. Its script is:
  LoseControl$ Untap,LeavesPlay,LoseControl,StaticCommandCheck | StaticCommandCheckSVar$ Y | StaticCommandSVarCompare$ GTX
i.e. you lose control when Old Man UNTAPS, leaves play, you lose control of it, OR the stolen creature's power becomes > Old Man's power. The current converter maps this to ControlDuration::WhileControlSource (control reverts only when you stop controlling Old Man) — so it does NOT yet revert when Old Man untaps or when the creature's power rises above Old Man's. (Old Man has 'you may choose not to untap', so a player keeping it tapped approximates the intended lock, but the power-comparison + untap revert are missing.)

TODO: add ControlDuration variant(s) for tapped-source + dynamic power-comparison (StaticCommandCheck GTX), and have recompute_source_control evaluate them. Then write the Old Man e2e (steal a small creature; it returns when Old Man untaps or the creature outgrows it). Until then Old Man is PARTIAL (targeting correct, duration approximate).
