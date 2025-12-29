---
title: Client-server reveal timing race condition
status: closed
priority: 2
issue_type: task
created_at: 2025-12-29T03:23:21.515918539+00:00
updated_at: 2025-12-29T03:39:34.226584377+00:00
---

# Description

## Description

## Summary

Network games deadlock due to a race condition between reveal message processing and the game loop's draw step.

## Root Cause

When client A simulates client B's turn, client A needs to know the card IDs of cards drawn by client B. These reveals are sent bundled with ChoiceRequest/OpponentChoice messages, but the game loop runs ahead:

1. Server runs draw step for Player 1 (draw from library to hand)
2. Server continues until Player 1 needs to make a choice
3. Server sends ChoiceRequest to Player 1 with reveals bundled
4. Server continues until Player 0 needs a choice
5. Server sends reveals to Player 0 with OpponentChoice

But Player 0's client is simulating Player 1's draw in its own game loop. If Player 0's game loop reaches the draw step BEFORE receiving the reveals (e.g., while waiting for Player 0's OWN choice), the draw fails because \`pending_reveals=0\`.

## Fix Implemented (2025-12-29)

Added immediate reveal broadcasting via \`opponent_reveal_tx\`/\`reveal_rx\` channels:

1. When the server sends a ChoiceRequest to player A with reveals, it now ALSO broadcasts those reveals to player B immediately
2. Player B receives the broadcasts via \`reveal_rx\` and sends CardRevealed messages to their client
3. This ensures both clients have reveals BEFORE their game loops need them

Key changes to \`mtg-engine/src/network/server.rs\`:
- Added \`RevealBroadcast\` struct to carry reveal info between handlers
- Added \`reveal_rx\` and \`opponent_reveal_tx\` channels to \`PlayerConnection\`
- When processing ChoiceRequest with reveals, broadcast to opponent via \`opponent_reveal_tx\`
- Added handler for \`reveal_rx\` in the select loop to send CardRevealed messages

## Verification

Tested with:
\`\`\`bash
./target/release/mtg server --port 55000 --password testpwd --cardsfolder mtg-engine/cardsfolder --seed 42
./target/release/mtg connect --server localhost:55000 --password testpwd --name Player1 --controller heuristic decks/julian_spiderman_draft.dck
./target/release/mtg connect --server localhost:55000 --password testpwd --name Player2 --controller heuristic decks/ryan_spiderman_draft.dck
\`\`\`

- Game now progresses past Turn 5/6 without "Invalid ability index" errors
- No sync warnings or errors in logs

## Related Issues

- mtg-037fw (4-way gamelog equivalence test) - can proceed with client state verification now
