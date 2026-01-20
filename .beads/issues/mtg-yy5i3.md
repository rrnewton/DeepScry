---
title: Creature must be on battlefield to attack - network mode only
status: closed
priority: 3
issue_type: task
labels:
- network
- bug
- fixed
created_at: 2026-01-20T20:03:25.039749274+00:00
updated_at: 2026-01-20T20:35:00.000000000+00:00
---

# Description

## Summary

During network fuzz testing, we observed errors where attack declarations fail
with "Creature must be on battlefield to attack" even though the game completes
successfully. This only happens in NETWORK mode, not in local mode.

## Root Cause (IDENTIFIED)

The bug was caused by **attacker list staleness** during trigger processing:

1. Player chooses attackers: `[Beetle-Headed Merchants (37), Fire Sages (39)]`
2. Loop processes Beetle-Headed Merchants first
3. **Beetle-Headed Merchants has an attack trigger:** "Whenever this creature attacks, you may sacrifice another creature or artifact"
4. Controller opts to sacrifice **Fire Sages (39)** for the trigger cost
5. Fire Sages is removed from battlefield
6. Loop continues to process Fire Sages (39) - but it's no longer on battlefield!
7. `declare_attacker(39)` fails with "Creature must be on battlefield to attack"

**Key insight**: The attacker list becomes stale when a previous attacker's trigger
modifies game state by sacrificing another chosen attacker. The error occurred
because the loop tried to declare a creature that had been sacrificed during
an earlier iteration's trigger resolution.

**Not a network desync**: The issue appears network-specific only because different
RNG states in network mode (heuristic vs random controllers) lead to different
game states where this specific interaction (Beetle-Headed Merchants + Fire Sages
both chosen as attackers) occurs.

## Fix

Added a pre-check in `game_loop/combat.rs` to skip attackers that are no longer
on the battlefield before calling `declare_attacker()`:

\`\`\`rust
// Pre-check: Skip creatures no longer on battlefield
// This can happen when a previous attacker's trigger (e.g., Beetle-Headed Merchants)
// sacrifices another chosen attacker (e.g., Fire Sages) as a cost.
if !self.game.battlefield.contains(*attacker_id) {
    if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
        // Log at verbose level only
    }
    continue;
}
\`\`\`

## Reproducer

**Network mode (showed error before fix):**
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

## Cards Involved

- **Beetle-Headed Merchants**: "Whenever this creature attacks, you may sacrifice another creature or artifact. If you do, draw a card and put a +1/+1 counter on this creature."
- **Fire Sages**: Firebending 1 (adds mana when attacking)

## Related

- Combat trigger cost handling
- Attack declaration validation in `mtg-engine/src/game/game_loop/combat.rs:101-119`
