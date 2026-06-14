# Scripted Play (Fixed Inputs)

The `fixed` controller replays a predetermined script of choices. This is the
backbone of reproducers and regression tests: a fixed script plus a seed fully
determines a game.

You pass a script with `--p1-fixed-inputs` / `--p2-fixed-inputs` (and select the
`fixed` controller with `--p1 fixed` / `--p2 fixed`):

```bash
mtg tui decks/a.dck decks/b.dck \
    --p1 fixed --p1-fixed-inputs="play mountain;cast bolt;target bob" \
    --p2 fixed --p2-fixed-inputs="0;0;pass"
```

Commands are **semicolon-separated** (not commas, not spaces).

## Two ways to name a choice

- **Numeric** — the menu index, e.g. `0`, `1`, `3`. `0` always means "pass
  priority". Simple, but fragile: indices shift as cards enter and leave.
- **Rich text** — a human-readable command like `play mountain`, `cast
  lightning bolt`, `attack grizzly`. Robust to menu reordering, so it is
  preferred for durable scripts.

Card names in rich text match case-insensitively, by prefix, ignoring spaces and
underscores, and with trailing punctuation stripped — so `light`, `Lightning
Bolt`, and `lightning_bolt` all resolve to "Lightning Bolt".

## Robustness primitives

Two features keep scripts from breaking when unrelated things change between
turns:

- **Wildcard `*`** — skip (pass priority on) commands until the *next* command
  matches an available action. Use it when you care about *availability*:
  "pass until I can cast Fireball".

  ```text
  --p1-fixed-inputs="play mountain;*;cast fireball"
  ```

- **`PASS_UNTIL`** — pass priority through every intervening step until a named
  turn and/or phase is reached. Use it when you care about *timing*:

  ```text
  PASS_UNTIL turn=3,phase=MAIN2     # wait for turn 3, post-combat main
  PASS_UNTIL phase=COMBAT           # next combat, any turn
  PASS_UNTIL turn=2                 # start of turn 2, any phase
  ```

  Phase names are case-insensitive: `untap`, `upkeep`, `draw`, `main1`,
  `combat`, `beginCombat`, `declareAttackers`, `declareBlockers`,
  `combatDamage`, `endCombat`, `main2`, `end`, `cleanup`.

`PASS_UNTIL` only ever reads the *public* game state (the turn number and the
current step). It never inspects the stack, a hand, or any hidden information,
which is exactly why it behaves identically in local and networked play — see
the controller information-independence rule in
[Network Architecture](../part2/network_architecture.md).

## Targeting, abilities, and blocking

```text
target grizzly            # target a creature
target player1            # target a player
activate forest           # first ability
activate forest[2]        # second ability (1-indexed)
serra blocks assassin     # declare a block
serra blocks grizzly;knight blocks elf   # multiple blocks, semicolon-separated
```

## Error behaviour

In normal mode, a command that matches no available action is a hard error that
lists what *was* available:

```text
Error: Command 'cast fireball' did not match any available action.
Available actions: ["play Mountain", "play Swamp"]
```

After a wildcard `*`, non-matching commands silently pass priority until the
next non-wildcard command that fails to match.

The complete grammar — every verb, every special case — is in the
[Fixed-Input reference](../part3/fixed_input_syntax.md) in Part III. The parsing
implementation lives in `mtg-engine/src/game/command_parsing.rs` and
`mtg-engine/src/game/rich_input_controller.rs`.
