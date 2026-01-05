---
title: Refactor CardReveal to include context and reduce redundancy
status: open
priority: 3
issue_type: task
created_at: 2026-01-05T21:00:43.577956413+00:00
updated_at: 2026-01-05T21:00:43.577956413+00:00
---

# Description

## Problem

The current `CardReveal` struct contains redundant information:
- `name`, `mana_cost`, `text`, `power`, `toughness`, etc.
- All parties have the full card database and can look up card info by name
- Only need: `CardId` + card name (for DB lookup)

Additionally, `CardReveal` events lack context about WHERE and WHAT they apply to, making ordering bugs more likely (see mtg-1jtoy for the reveal ordering bug that was fixed).

## Proposed Solution

### Phase 1: Simplify CardReveal struct
- Remove redundant fields (mana_cost, text, power, toughness, etc.)
- Keep: `card_id`, `name`, and maybe `card_types` for quick filtering
- Clients look up full card info from their local card database

### Phase 2: Add context to CardReveal
Include action_count timestamp and position context:
- `action_count: u64` - "at action T, this reveal occurred"
- Reveal type with context:
  - `RevealInHand { player: PlayerId, position: usize, total: usize }` - "card K of N in hand"
  - `RevealFromDraw { player: PlayerId, draw_number: usize }` - "draw M of player P"  
  - `RevealFromPlay { spell_ability: SpellAbility }` - implicit reveal when playing
  - `RevealFromEffect { source_card: CardId }` - effect-caused reveal

### Phase 3: Make played cards implicitly revealed
When a spell is cast, the card reveal should be implicit in the spell cast message itself, not a separate message that can arrive out of order.

## Benefits
- Smaller network messages (bandwidth reduction)
- More robust reveal handling (context prevents misapplication)
- Easier debugging (can trace reveals to specific actions)
- Less fragile ordering (reveals tied to actions, not floating)

## Related
- mtg-1jtoy: Fixed reveal ordering bug (async channels causing out-of-order delivery)
- The fix disabled async reveal sources; this refactor would make the system more robust

## Files to modify
- `mtg-engine/src/network/protocol.rs` - CardReveal struct
- `mtg-engine/src/network/server.rs` - reveal sending
- `mtg-engine/src/network/client.rs` - reveal receiving/processing
- `mtg-engine/src/network/remote_controller.rs` - RemoteMessage::Choice.card_reveal field (currently unused, can remove)
