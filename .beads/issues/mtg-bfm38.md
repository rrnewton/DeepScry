---
title: Networking E2E tests
status: open
priority: 2
issue_type: task
depends_on:
  mtg-to96y: parent-child
created_at: 2025-12-05T17:58:29.730244324+00:00
updated_at: 2025-12-05T17:58:29.730244324+00:00
---

# Description

## Networking E2E Tests

End-to-end tests for networked gameplay.

## Tasks

- [ ] Test: Protocol serialization round-trips for all message types
- [ ] Test: State hash computation excludes hidden info
- [ ] Test: LibraryMode::Remote draw behavior
- [ ] Test: Server accepts two clients, starts game
- [ ] Test: Full game with fixed-script controllers over network
- [ ] Test: Hash verification detects intentional desync
- [ ] Test: Deck visibility flag sends/hides deck lists
- [ ] Test: Graceful handling of client disconnect
- [ ] Test: Concurrent games on same server (if supported)
- [ ] Integration with existing agentplay scripts

## Test Strategy

Use localhost connections with fixed-script or heuristic controllers.
Compare network game results with equivalent local games to verify determinism.

## Validation

\`\`\`bash
## Local game (baseline)
mtg tui deck1.dck deck2.dck --p1=heuristic --p2=heuristic --seed=12345

## Network game (should produce identical result)
mtg server --port=17771 --password=test &
mtg connect deck1.dck --server=localhost:17771 --password=test --controller=heuristic &
mtg connect deck2.dck --server=localhost:17771 --password=test --controller=heuristic
\`\`\`

## Reference

See `ai_docs/NETWORKING_DESIGN_PLAN.md` Part 5
