---
title: 'Shadow state desync: reveal architecture violation'
status: blocked
priority: 2
issue_type: task
created_at: 2026-01-08T15:59:50.778794868+00:00
updated_at: 2026-01-08T22:30:31.481711838+00:00
blocked_by: mtg-secqu
---

# Description

## Summary

Network games experience intermittent desync with action_count mismatches or timeouts
during complex gameplay. Root cause: **reveal architecture violates network principles**.

## Root Cause (2026-01-08)

The reveal infrastructure violates `docs/NETWORK_ARCHITECTURE.md`:

1. **Reveals computed in server handlers, not core engine**
   - `collect_reveals_since_last_choice()` in NetworkController infers reveals from MoveCard
   - Should be `GameAction::RevealCard` logged by GameLoop before moves

2. **Reveals bundled with ChoiceRequest, not in action log**
   - Causes timing issues: client validates before reveals arrive
   - Should be: reveals in log, server forwards them, client processes in order

3. **Deduplication in wrong place**
   - `revealed_cards: HashSet` in PlayerConnection (server handler)
   - Should be: deduplication at log time in core engine

4. **No `revealed_to` field**
   - Current RevealCard doesn't track WHO should see the reveal
   - Need: P1, P2, or BOTH as target audience

## Fix Required

Implement proper reveal architecture per `docs/NETWORK_ARCHITECTURE.md`:

```
1. GameLoop about to move card from hidden zone
2. GameLoop logs: RevealCard { card_id, name, revealed_to }
3. GameLoop logs: MoveCard { ... }
4. Server reads RevealCard from log, sends CardRevealed to target clients
5. Client receives CardRevealed, instantiates card
6. Client receives ChoiceRequest (reveals already processed)
```

This is tracked in mtg-secqu "Reveal Architecture" section.

## Reproducer

```bash
# Run multiple times - flaky test
cargo test --features network --test network_e2e test_run_game_with_random_controllers -- --ignored
```

## Related

- mtg-secqu: Network architecture compliance (blocks this)
- mtg-to96y: Main networking tracking issue
- mtg-qtqcr: Hidden information architecture
- docs/NETWORK_ARCHITECTURE.md: Architecture principles
