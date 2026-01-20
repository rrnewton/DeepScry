---
title: Creature must be on battlefield to attack - network mode only
status: open
priority: 3
issue_type: task
labels:
- network
- bug
created_at: 2026-01-20T20:03:25.039749274+00:00
updated_at: 2026-01-20T20:20:22.019335815+00:00
---

# Description

## Summary

During network fuzz testing, we observed errors where attack declarations fail
with "Creature must be on battlefield to attack" even though the game completes
successfully. This only happens in NETWORK mode, not in local mode.

## Root Cause Analysis (Partial)

The error occurs during combat when an attack trigger with sacrifice cost is involved:

1. Player chooses 2 attackers: Fire Sages(39) and Beetle-Headed Merchants(37)
2. First declare_attacker processes Fire Sages
3. Fire Sages has an attack trigger with sacrifice cost - it sacrifices itself
4. Second declare_attacker attempts to process another creature
5. Error: "Creature must be on battlefield to attack"
6. Yet Beetle-Headed Merchants still deals 7 damage successfully

**Key observations:**
- The error appears in ALL logs (server + both clients) - it's server-side
- Only 1 "declares" message prints despite 2 attackers chosen
- The creature that deals damage (Beetle-Headed Merchants) is different from the one sacrificed
- Game completes successfully despite the error

**Network-specific**: Same seed in local mode does NOT produce this error.

## Reproducer

**Network mode (shows error):**
\`\`\`bash
PORT=19200
./target/release/mtg server --port $PORT --seed 5 > server.log 2>&1 &
sleep 1
./target/release/mtg connect decks/booster_draft/avatar/ryan_avatar_draft.dck \
  --server "localhost:$PORT" --controller heuristic --name P1 &
sleep 0.3
./target/release/mtg connect decks/booster_draft/avatar/gabriel_avatar_draft.dck \
  --server "localhost:$PORT" --controller random --name P2 &
wait
grep "Creature must" server.log
\`\`\`

**Local mode (NO error):**
\`\`\`bash
./target/release/mtg tui decks/booster_draft/avatar/ryan_avatar_draft.dck \
  decks/booster_draft/avatar/gabriel_avatar_draft.dck \
  --p1 heuristic --p2 random --seed 5
\`\`\`

## Log Evidence

From server.log at seed=5:
\`\`\`
[INFO  mtg_forge_rs::game::actions] Sacrificing Fire Sages (39) for trigger cost
  P1 declares Beetle-Headed Merchants (37) (7/6) as attacker
  Error declaring attacker: Invalid game action: Creature must be on battlefield to attack
  Beetle-Headed Merchants (37) deals 7 damage to P2 (life: 7)
\`\`\`

From client1.log:
\`\`\`
<Choice> chose 2 attackers from 2 available creatures (aggression=3, opponent blockers=0)
[INFO  mtg_forge_rs::game::actions] Sacrificing Fire Sages (39) for trigger cost
  P1 declares Beetle-Headed Merchants (37) (7/6) as attacker
  Error declaring attacker: Invalid game action: Creature must be on battlefield to attack
\`\`\`

## Hypothesis

There may be a state inconsistency in how attack declarations with sacrifice costs
are handled differently in network vs local mode. The error might be:
1. A card ID mismatch between what the controller chose and what the server processes
2. A timing issue with when sacrifice removes creatures from the battlefield
3. A missing "declares" message for Fire Sages because cards.get() fails after sacrifice

## Priority

Low - game completes successfully, this is a non-fatal warning. But it indicates
a potential state consistency issue that should be investigated.

## Related

- Combat trigger cost handling
- Attack declaration validation
- Network vs local mode differences
