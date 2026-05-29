# Equipment Attachment Implementation Plan

Based on study of Java Forge implementation (2025-11-10).

## Architecture Overview

### Java Forge Approach (Verified)

**Card.java**:
- `private GameEntity entityAttachedTo` - single field tracks attachment to Card or Player
- `getEquippedBy()` - returns Equipment attached TO this creature
- `getEquipping()` / `isEquipping()` - checks if THIS card is attached to something
- `attachToEntity(GameEntity e, SpellAbility sa)` - performs attachment with timestamp update
- `unattachFromEntity(GameEntity e)` - handles detachment

**AttachEffect.java**:
- Resolves Equip activated ability
- Validates target can be attached (`canBeAttached` predicate)
- Calls `attachment.attachToEntity(attachTo, sa)`
- Handles optional attachment confirmation

**StaticAbilityContinuous.java**:
- Line 1011-1081: `getAffectedCards()` determines which cards are affected
- "Creature.EquippedBy" selector filters for creatures equipped by this Equipment
- Lines 156-166: `AddPower` / `AddToughness` parameters in MODIFYPT layer
- Lines 688-697: Applies P/T bonuses via `affectedCard.addPTBoost()`

## Rust Implementation Plan

### Phase 1: Card Structure (mtg-engine/src/core/card.rs)

```rust
pub struct Card {
    // ... existing fields ...

    /// Equipment/Aura attachment tracking
    /// - For Equipment/Aura: points to creature/player being enchanted/equipped
    /// - For Creature: empty (use GameState to find attached Equipment)
    pub attached_to: Option<CardId>,
}
```

**Design Decision**: Use `Option<CardId>` instead of `GameEntity` enum for simplicity.
- Equipment only attaches to creatures (not players in our initial impl)
- Bidirectional lookups via GameState helper methods

### Phase 2: Continuous Effects System (mtg-engine/src/game/continuous_effects.rs)

New module for evaluating continuous effects:

```rust
pub struct ContinuousEffect {
    source_id: CardId,
    layer: EffectLayer,
    effect_type: ContinuousEffectType,
    timestamp: u64,
}

pub enum EffectLayer {
    Control,       // Control-changing effects
    Text,          // Text-changing effects
    Type,          // Type-changing effects
    Color,         // Color-changing effects
    Abilities,     // Ability-adding/removing
    PowerToughness // P/T modifications
}

pub enum ContinuousEffectType {
    ModifyPT { power: i32, toughness: i32 },
    AddKeywords(Vec<Keyword>),
    AddTypes(Vec<CardType>, Vec<Subtype>),
    // ... more as needed
}
```

**Key Functions**:
- `apply_continuous_effects(game: &GameState)` - main entry point
- `get_affected_cards(source: &Card, selector: &str) -> Vec<CardId>`
- `apply_equipment_buffs(game: &mut GameState, equip_id: CardId)`

### Phase 3: Equip Ability (mtg-engine/src/game/actions.rs)

```rust
impl GameState {
    /// Attach Equipment to target creature
    pub fn attach_equipment(&mut self, equipment_id: CardId, target_id: CardId) -> Result<()> {
        // Validate Equipment is on battlefield
        // Validate target is creature you control
        // Detach from previous target if needed
        // Update attached_to field
        // Update timestamp for Equipment
        // Trigger ETB-like events if needed
    }

    /// Detach Equipment from its target
    pub fn detach_equipment(&mut self, equipment_id: CardId) -> Result<()> {
        // Clear attached_to
        // No zone change - stays on battlefield
    }
}
```

**Equip Activated Ability**:
```rust
// Parse from card data: K:Equip:3
ActivatedAbility {
    cost: ManaCost::from_string("3"),
    effect: Effect::Attach { target: TargetRef::placeholder() },
    timing: TimingRestriction::Sorcery,
    target_type: TargetType::CreatureYouControl,
}
```

### Phase 4: State-Based Actions (mtg-engine/src/game/state_based_actions.rs)

New checks to add:
```rust
// Check for Equipment attached to non-existent/invalid creatures
fn check_equipment_attachments(game: &mut GameState) {
    for card_id in game.battlefield.iter() {
        let card = game.cards.get(card_id)?;
        if card.is_equipment() {
            if let Some(attached) = card.attached_to {
                // Detach if target not on battlefield
                if !game.battlefield.contains(attached) {
                    game.detach_equipment(card_id)?;
                }
                // Detach if target not a creature
                let target = game.cards.get(attached)?;
                if !target.is_creature() {
                    game.detach_equipment(card_id)?;
                }
            }
        }
    }
}
```

### Phase 5: Effect Parsing (mtg-engine/src/loader/ability_parser.rs)

Parse Spider-Suit static ability:
```
S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2 | AddType$ Spider & Hero
```

Maps to:
```rust
StaticAbility::Continuous {
    affected: Selector::CreatureEquippedBy(self_id),
    effects: vec![
        ContinuousEffect::ModifyPT { power: 2, toughness: 2 },
        ContinuousEffect::AddTypes {
            card_types: vec![],
            subtypes: vec![Subtype::from("Spider"), Subtype::from("Hero")]
        },
    ],
}
```

## Implementation Order

1. ✅ Study Java implementation (DONE)
2. Add `attached_to` field to Card struct
3. Implement basic attach/detach methods in GameState
4. Add Equip activated ability parsing
5. Implement continuous effects evaluation for Equipment buffs
6. Add state-based actions for Equipment detachment
7. Create comprehensive tests
8. Update mtg-169 issue with progress

## Test Cases

### Unit Tests
- `test_attach_equipment()` - Basic attachment
- `test_detach_when_creature_dies()` - State-based action
- `test_equipment_stat_bonus()` - Continuous effects
- `test_multiple_equipment_stack()` - Multiple Equipment on one creature
- `test_equip_requires_sorcery_timing()` - Timing restrictions

### Integration Tests
- `test_spider_suit_full_workflow()` - Cast, attach, attack, verify +2/+2 damage
- `test_equipment_attachment_snapshot()` - Serialization/deserialization

## Files to Modify

1. **mtg-engine/src/core/card.rs** - Add attached_to field
2. **mtg-engine/src/game/actions.rs** - attach_equipment(), detach_equipment()
3. **mtg-engine/src/game/continuous_effects.rs** - NEW FILE
4. **mtg-engine/src/game/state_based_actions.rs** - Equipment checks
5. **mtg-engine/src/loader/ability_parser.rs** - Parse Continuous effects
6. **mtg-engine/tests/test_spider_suit_equipment.rs** - Integration tests

## Relationship to Java Forge

Java Forge uses a sophisticated layered system with:
- 7 effect layers applied in order (613 rules)
- Timestamp-based ordering within layers
- Full support for Auras, Equipment, and Fortifications
- Complex selector language ("Creature.EquippedBy", "Creature+YouCtrl")

Our initial Rust implementation will:
- Support Equipment → Creature attachments only (no player attachments)
- Use simplified layer system (just PowerToughness for now)
- Implement core mechanics needed for Spider-Suit test case
- Expand incrementally as more cards require advanced features

## Estimated Complexity

- **Phase 1 (Card struct)**: Simple - 30 minutes
- **Phase 2 (Continuous effects)**: Medium - 3 hours
- **Phase 3 (Equip ability)**: Medium - 2 hours
- **Phase 4 (State-based actions)**: Simple - 1 hour
- **Phase 5 (Parsing)**: Medium - 2 hours
- **Testing**: 2 hours
- **Total**: ~10-11 hours of focused development

## Next Steps

1. Start with Phase 1 (Card struct modification)
2. Add helper methods to GameState for finding attached Equipment
3. Implement basic continuous effects for P/T modification
4. Test incrementally with Spider-Suit example
