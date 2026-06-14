# Fixed Input Syntax Reference

This document describes the syntax for fixed input commands used with `--p1-fixed-inputs`
and `--p2-fixed-inputs` flags, as well as the agentplay workflow scripts.

## Overview

Fixed inputs allow you to script player decisions for deterministic game replays.
Commands can be either numeric (menu indices) or rich text (human-readable).

## Command Syntax

### Basic Commands

| Verb | Syntax | Description |
|------|--------|-------------|
| `pass` | `pass` | Pass priority (same as `0` or `p`) |
| `play` | `play <card>` | Play a land card |
| `cast` | `cast <card>` | Cast a spell |
| `activate` | `activate <card>` | Activate an ability |
| `attack` | `attack <creature>` | Declare a creature as attacker |
| `block` | `<blocker> blocks <attacker>` | Declare a blocking assignment |
| `equip` | `equip <equipment>` | Shorthand for equip ability |

### Card Name Matching

Card names are matched using these rules:

1. **Case-insensitive**: `Mountain`, `mountain`, and `MOUNTAIN` all match
2. **Prefix matching**: `light` matches "Lightning Bolt"
3. **Spaces/underscores ignored**: `blackknight`, `black_knight`, and `Black Knight` all match
4. **Trailing punctuation stripped**: `mountain.` and `mountain,` match "Mountain"

### Numeric Commands

| Number | Meaning |
|--------|---------|
| `0` | Pass priority (equivalent to `pass`) |
| `1` to `N` | Select menu item N (1-indexed) |

Menu items are always displayed in this order:
1. `[0] pass`
2. `[1]` - First available action (usually play land)
3. `[2]` - Second available action
4. etc.

### Examples

```bash
# Play a land
play mountain
play "Serra Angel"

# Cast a spell
cast lightning bolt
cast "Black Knight"

# Activate an ability
activate forest
activate forest[2]    # Second ability (1-indexed)

# Declare attackers
attack grizzly
attack serra

# Declare blockers
blackknight blocks whiteknight
serra blocks assassin

# Using quotes (optional)
cast "Black Lotus"
play "Tropical Island"

# Numeric choices
0         # Pass priority
1         # Select first action
```

## Multiple Commands

Commands are separated by semicolons (`;`):

```bash
--p1-fixed-inputs="play mountain;pass;cast bolt;target player2"
```

## PASS_UNTIL: Semantic Anti-Overfitting Command

`PASS_UNTIL` is the primary anti-brittleness primitive. Instead of counting
action indices across turns, you declare the turn+phase you want to act in,
and the controller passes priority through all intervening steps automatically —
regardless of what triggers fire, how many actions are offered, or what state
changes occur in between.

### Syntax

```bash
PASS_UNTIL turn=N,phase=PHASE   # wait for specific turn + phase
PASS_UNTIL phase=PHASE           # wait for next occurrence of PHASE (any turn)
PASS_UNTIL turn=N                # wait for start of turn N (any phase)
```

`phase` (or `step`) accepts case-insensitive names:
`untap`, `upkeep`, `draw`, `main1`, `combat`, `beginCombat`,
`declareAttackers`, `declareBlockers`, `combatDamage`, `endCombat`,
`main2`, `end`, `cleanup`

### Examples

```bash
# Pass all priority windows until Turn 3 post-combat main phase
PASS_UNTIL turn=3,phase=MAIN2

# Pass until combat phase (begin-combat step) of any turn
PASS_UNTIL phase=COMBAT

# Combine: reach a specific state, then act by name
PASS_UNTIL turn=2,phase=MAIN1
cast Grizzly Bears
PASS_UNTIL phase=declareAttackers
attack Grizzly Bears
```

### Why Use PASS_UNTIL vs Wildcard (`*`)

| | Wildcard `*` | `PASS_UNTIL` |
|---|---|---|
| Passes until | Next command matches available action | Turn+phase reached |
| Affected by trigger order? | Yes (command must be available) | No |
| Affected by new actions in menu? | No | No |
| Affected by action count changes? | No | No |
| Expressiveness | "wait until castable" | "wait until this turn+step" |

Use `*` when you care about availability; use `PASS_UNTIL` when you care
about timing. Scripts that mix both are fine.

### Information Independence

`PASS_UNTIL` only uses the public game state (turn number and current step).
It does not inspect the stack, hand, opponent's choices, or any hidden
information. This makes it safe for both local and network-mode play.

## Wildcard Mode

Use `*` to skip commands until the next one matches:

```bash
# Play mountain, then pass until we can cast fireball
--p1-fixed-inputs="play mountain;*;cast fireball"

# Equip, then wait until attack phase
--p1-fixed-inputs="equip accorder;*;attack grizzly"
```

In wildcard mode:
- Non-matching commands pass priority without error
- The controller waits until the specified command becomes available
- Useful for scripts that need to span multiple turns

## Error Handling

### Normal Mode (no wildcard)

Commands MUST match an available action. If they don't, the controller returns an error:

```
Error: InvalidAction("Controller error: Command 'cast fireball' did not match
any available action. Available actions: [\"play Mountain\", \"play Swamp\"]")
```

### Wildcard Mode (after `*`)

Non-matching commands silently pass priority. No error is raised until the next
non-wildcard command that doesn't match.

## Menu Display Format

The game displays available actions in this format to match the input syntax:

```
Player1 available actions:
  [0] pass
  [1] play Mountain
  [2] play Mountain
  [3] play Swamp
```

You can copy the text directly (without the `[N]` prefix) as input.

## Special Cases

### Multiple Abilities on Same Permanent

Use indexed activation for permanents with multiple abilities:

```bash
activate forest      # First ability
activate forest[1]   # Also first ability (1-indexed)
activate forest[2]   # Second ability
```

### Targeting

Targeting commands use the same card matching rules:

```bash
target grizzly       # Target creature
target player1       # Target player
target "Black Knight"
```

#### Inline `targeting` clause (robust to forced targets)

A standalone `target ...` command only works when the engine actually *asks*
the controller to choose a target. When a spell has exactly one legal target
(e.g. Lightning Bolt vs. the only creature on board), the engine auto-selects
it WITHOUT a target prompt (CR 601.2c forced choice), and a following
`target ...` line would strand and error.

To make targeted plays robust either way, append an inline `targeting <selector>`
clause to the `cast` / `activate` command:

```bash
cast Lightning Bolt targeting Grizzly Bears
cast Shock targeting p2          # p2 = the second player (also accepts p1/p0)
activate Prodigal Sorcerer targeting Grizzly Bears
```

The selector after `targeting` is matched against the engine's offered valid
targets using the same anti-overfitting card matcher (prefix / case- /
space-insensitive), or a `pN` player sentinel. If the engine prompts for a
target, the named one is chosen; if the target was forced, the clause is a
harmless no-op. This is the preferred form for scripted puzzle actions (see
the `[p0_script]` / `[p1_script]` puzzle sections).

### Blocking Multiple Attackers

Use semicolon-separated clauses:

```bash
--p2-fixed-inputs="serra blocks grizzly;knight blocks elf"
```

## Best Practices

1. **Use rich text over numeric**: Menu indices can change as cards enter/leave
2. **Use wildcards for timing**: `*;cast fireball` waits until fireball is castable
3. **Quote card names with spaces**: `"Black Knight"` (optional but clearer)
4. **Test reproducibility**: Run the same command twice to verify determinism

## Implementation Details

The parsing logic is implemented in:
- `mtg-engine/src/game/command_parsing.rs` - Core parsing functions
- `mtg-engine/src/game/rich_input_controller.rs` - Rich input controller
- `mtg-engine/src/game/controller.rs` - Menu formatting

## See Also

- [HOWTO: Play MTG Games and Build Reproducers](./HOWTO_AGENTPLAY+REPRODUCERS.md)
- `mtg tui --help` - CLI options
- `agentplay/*.sh` - Wrapper scripts for interactive play
