---
title: Spider-Man deck gameplay issues
status: open
priority: 2
issue_type: bug
created_at: 2025-11-21T01:03:59.768389132+00:00
updated_at: 2025-11-21T15:21:19.861549816+00:00
---

# Description

Spider-Man deck gameplay issues - ETB token creation

## Status

PARTIALLY FIXED - ETB triggers now create tokens from actual tokenscripts definitions

## Critical Issues Found

1. **ETB triggers not creating tokens** - FIXED ✅
   - Spider-Ham should create a Food token on ETB, but didn't
   - Root cause: Trigger parser didn't handle Execute$ parameter that references SVars with DB$ Token effects
   - **Fix**: Added parsing logic for Execute$ → SVar lookup → DB$ Token parameter extraction
   - **Enhancement**: Refactored token creation with centralized helper function
   - **Token Loading**: Implemented proper token definition loading from tokenscripts/ directory
   - **Verification**: Spider-Ham now successfully creates Food tokens when cast

2. **Friendly Neighborhood: Unknown Affected$ selector Land.AttachedBy** - NOT FIXED ❌
   - The selector `Land.AttachedBy` is not recognized
   - Need to implement support for `AttachedBy` selector in static abilities
   - This affects Aura abilities that grant abilities to the enchanted permanent

3. **Spider-Ham: Unknown Affected$ selector for comma-separated types** - NOT FIXED ❌
   - Multi-type selector like `Spider.Other+YouCtrl,Boar.Other+YouCtrl,...` not recognized
   - Need to parse comma-separated type lists in Affected$ selectors
   - This prevents Spider-Ham's "Animal May-Ham" ability from working

4. **Spider-Punk: Unknown Affected$ selector Spider.Other+YouCtrl** - NOT FIXED ❌
   - Selector with "Other" qualifier not recognized
   - Need to implement support for "Other" in type selectors (means "other than this permanent")
   - This prevents Spider-Punk's "other Spiders you control have riot" ability

## Implementation Details

### Commit 3: feat(tokens) - Token Loading System

Replaced hardcoded token stubs with dynamic loading from tokenscripts/:

1. **Token preloading during game initialization** (`game_init.rs:58-79`)
   - Scans all deck cards for TokenScript$ references in SVars
   - Loads referenced token definitions from tokenscripts/
   - Caches definitions in GameState.token_definitions

2. **Token definition extraction** (`card.rs:161-192`)
   - New extract_token_scripts() method on CardDefinition
   - Parses SVar lines containing "DB$ Token | TokenScript$ ..."
   - Returns unique token script names for preloading

3. **Token loading from tokenscripts** (`database_async.rs:244-282`)
   - New get_token() method on AsyncCardDatabase
   - Canonicalizes cardsfolder path (handles symlinks)
   - Navigates to sibling tokenscripts/ directory

4. **Token creation from cached definitions** (`actions.rs:1280-1324`)
   - CreateToken effect now looks up cached token definition
   - Instantiates tokens using CardDefinition.instantiate()
   - Returns error if token not preloaded (fail fast)

5. **GameState changes** (`state.rs:57-61, 120`)
   - Added token_definitions: HashMap<String, Arc<CardDefinition>>
   - Marked with #[serde(skip)] to exclude from snapshots
   - Initialized empty in constructor

### Commit 1: feat(triggers) - ETB Token Creation

1. **mtg-engine/src/core/effects.rs** (lines 115-126)
   - Added `Effect::CreateToken` variant with:
     - `controller: PlayerId` - player who will control the tokens
     - `token_script: String` - token script name (e.g., "c_a_food_sac" for Food token)
     - `amount: u8` - number of tokens to create

2. **mtg-engine/src/loader/card.rs** (lines 1175-1223)
   - Added parsing logic in `parse_triggers` for `Execute$` parameter
   - Looks up referenced SVar (e.g., `Execute$ TrigToken` → `SVar:TrigToken:...`)
   - Parses `DB$ Token` effects from SVar body
   - Extracts `TokenScript$` and `TokenAmount$` parameters
   - Creates `Effect::CreateToken` with parsed values

3. **mtg-engine/src/game/actions.rs** (lines 1280-1348)
   - Added `Effect::CreateToken` case to `execute_effect`
   - Creates token cards and adds to battlefield
   - Logs token creation
   - Added placeholder controller ID replacement (lines 1487-1499)

### Commit 2: refactor(tokens) - Common Token Types

4. **mtg-engine/src/game/actions.rs** (lines 2486-2550)
   - Added `parse_token_from_script` helper method (now removed in Commit 3)
   - Centralized token type parsing from script names
   - Stub implementation for common token types (now replaced by actual loading)

## Test Results

Created test deck `decks/spider_ham_test.dck` with Spider-Ham and verified:
- Spider-Ham successfully creates Food Token on ETB
- Token definitions properly loaded from tokenscripts/
- All 492 unit tests pass
- New E2E test: tests/spider_ham_tokens_e2e.sh

## Remaining Work

The following issues still need to be addressed:

1. **Friendly Neighborhood selector** (`Land.AttachedBy`)
   - Need to implement AttachedBy qualifier for Aura abilities
   
2. **Comma-separated type lists** (Spider-Ham's multi-animal ability)
   - Need to parse comma-separated selectors in Affected$
   
3. **"Other" qualifier** (Spider-Punk's ability)
   - Need to implement "Other" in type selectors
