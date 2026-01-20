---
title: Creature must be on battlefield to attack - network mode only
status: open
priority: 3
issue_type: task
labels:
- network
- bug
created_at: 2026-01-20T20:03:25.039749274+00:00
updated_at: 2026-01-20T20:03:25.039749274+00:00
---

# Description

## Summary

During network fuzz testing, we observed errors where attack declarations fail
with "Creature must be on battlefield to attack" even though the creature appears
to be on the battlefield. The game completes successfully despite these warnings.

## Symptoms

The error occurs during attack declaration when a creature (Fire Sages) is
sacrificed as a trigger cost during the same attack declaration phase, and
then a subsequent attacker declaration (Beetle-Headed Merchants) fails.

## Observations

- **Network mode**: Reproducible with seed=5, heuristic vs random
- **Local mode**: Does NOT reproduce with the same seed
- Game completes successfully - this is a warning, not a fatal error
- Timing/trigger handling difference between network and local modes suspected

## Reproducer

Network mode (shows the error):
\`\`\`bash
PORT=18885
./target/release/mtg server --port $PORT --seed 5 &
sleep 1
./target/release/mtg connect localhost:$PORT --deck decks/booster_draft/avatar/ryan_avatar_draft.dck --name Ryan --controller heuristic &
sleep 0.2
./target/release/mtg connect localhost:$PORT --deck decks/booster_draft/avatar/gabriel_avatar_draft.dck --name Gabriel --controller random &
wait
\`\`\`

Local mode (does NOT show the error):
\`\`\`bash
./target/release/mtg tui decks/booster_draft/avatar/ryan_avatar_draft.dck \
  decks/booster_draft/avatar/gabriel_avatar_draft.dck --p1 heuristic --p2 random --seed 5
\`\`\`

## Error context from logs

\`\`\`
WARN: Creature must be on battlefield to attack: CardRef { id: CardID(76), ... }
  - Context: Beetle-Headed Merchants attack declaration
  - During same combat: Fire Sages was sacrificed as trigger cost
\`\`\`

## Priority

Low - game completes successfully, this is a non-fatal warning that only
appears in network mode. May be a timing issue with how creature sacrifices
during trigger costs interact with attack declaration validation.

## Related

- Network protocol implementation
- Trigger cost handling
- Attack declaration validation
