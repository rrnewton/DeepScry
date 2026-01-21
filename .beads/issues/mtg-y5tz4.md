---
title: Extract execute_effect() into dispatch table pattern
status: open
priority: 3
issue_type: task
created_at: 2026-01-21T00:24:43.289156091+00:00
updated_at: 2026-01-21T00:24:43.289156091+00:00
---

# Description

## Refactor Giant execute_effect() Function

## Problem

`execute_effect()` in `mtg-engine/src/game/actions/mod.rs` (lines 1145-2300+) is over **1000 lines** with 41+ match arms. This violates the principle of keeping functions under 200 lines.

The function is a giant match statement that handles every Effect variant inline.

## Current Structure
```rust
pub fn execute_effect(&mut self, effect: &Effect) -> Result<()> {
    match effect {
        Effect::DealDamage { .. } => { /* 50+ lines */ }
        Effect::DrawCards { .. } => { /* 20+ lines */ }
        Effect::CreateToken { .. } => { /* 100+ lines */ }
        // ... 38 more arms
    }
}
```

## Proposed Solution

Extract each match arm into a dedicated helper method:

```rust
pub fn execute_effect(&mut self, effect: &Effect) -> Result<()> {
    match effect {
        Effect::DealDamage { target, amount } => 
            self.execute_deal_damage(*target, *amount),
        Effect::DrawCards { player, count } => 
            self.execute_draw_cards(*player, *count),
        Effect::CreateToken { controller, token_script, amount, for_each_player } => 
            self.execute_create_token(*controller, token_script, *amount, *for_each_player),
        // ... etc
    }
}

fn execute_deal_damage(&mut self, target: TargetRef, amount: u8) -> Result<()> { ... }
fn execute_draw_cards(&mut self, player: PlayerId, count: u8) -> Result<()> { ... }
fn execute_create_token(&mut self, ...) -> Result<()> { ... }
```

## Benefits
- Each effect handler is testable in isolation
- execute_effect() becomes a thin dispatcher (~100 lines)
- Effect handlers can be organized into logical groups (damage, cards, tokens, etc.)
- Easier to add new effects without bloating one giant function

## Acceptance Criteria
- [ ] execute_effect() is under 150 lines
- [ ] Each helper method is under 100 lines
- [ ] All tests pass
- [ ] No functionality changes

## Performance Requirements

**IMPORTANT**: Follow OPTIMIZATION.md guidelines when implementing this refactoring:
- No performance regressions allowed - run benchmarks before/after
- Helper functions must be `#[inline]` if called frequently in hot paths
- Avoid adding indirection that prevents compiler optimization
- Prefer passing references (`&Effect`) over cloning values
- The dispatch pattern should compile to equivalent machine code as the original match
