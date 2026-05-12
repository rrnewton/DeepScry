# HOWTO: Play MTG Games and Build Reproducers (For AI Agents)

It's important that you have the experience of playing the MTG game we're developing so you can see actual issues with gameplay and compare to expected behavior given MTG rules. Testing in code is insufficient.

You cannot interact with the interactive TUI via stdin, but the `agentplay/`
toolchain lets you drive deterministic games one choice at a time, either
manually or via Claude subagents. Every session is replayable from a single
reproducer script.

## Tool Overview

Three Python entry points under `agentplay/`:

| Script | Purpose |
| --- | --- |
| `agent_game.py` | **Recommended.** End-to-end agent-driven game: a Claude subagent (or `--mock` random selector) makes each choice, with optional scenario / bug-detection prompting. |
| `start_game.py` | Manually start a session: fork off the first choice prompt, write the initial files, print the menu. |
| `continue_game.py` | Manually append one player's next choice and replay the whole game so far. |

All three produce the same on-disk session layout (see **Game Directory
Structure** below) and the same reproducer script, so you can switch between
agent-driven and manual modes against the same session.

## Quick Start: agent_game.py (Recommended)

```bash
# Bug-detection is on by default. The agent picks a choice number on each
# turn, or writes STOP with a BUG_REPORT section if it sees a rules/engine bug.
./agentplay/agent_game.py -- decks/old_school/01_rogue_rogerbrand.dck decks/old_school/02_thedeck_peterschnidrig.dck

# Keep a reproduction target in the prompt at every decision:
./agentplay/agent_game.py --seed 42 \
    --scenario "Play until P2 attacks with a flying creature, then try to double-block and cast an instant combat trick" \
    -- decks/booster_draft/avatar/eric_avatar_draft.dck decks/booster_draft/avatar/gabriel_avatar_draft.dck

# Pure-play mode disables STOP / BUG_REPORT prompting (just play the game):
./agentplay/agent_game.py --pure-play -- decks/old_school/01_rogue_rogerbrand.dck decks/old_school/05_mono_black_rogerbrand.dck

# Mock mode: local random choice selection, no API tokens burned. Useful for
# smoke-testing the harness or generating long deterministic logs.
./agentplay/agent_game.py --mock --seed 42 -- decks/old_school/01_rogue_rogerbrand.dck decks/old_school/02_thedeck_peterschnidrig.dck

# Start from a puzzle/start-state file instead of a normal `mtg tui` run:
./agentplay/agent_game.py --puzzle puzzles/bolt_test.pzl
```

### Key `agent_game.py` flags

- `--mode {agent-vs-heuristic, agent-vs-random, agent-vs-agent, random-vs-random}`
  — controls who drives each seat. `agent-vs-agent` is the default.
- `--scenario "<english text>"` — keeps a reproduction target in the agent prompt every turn.
- `--goal "<english text>"` — like `--scenario`, but framed as a directed-play goal.
- `--puzzle <file>` — run `mtg puzzle <file>` instead of `mtg tui`.
- `--bug-detection` / `--no-bug-detection` / `--pure-play` — toggle STOP/BUG_REPORT prompting.
- `--stop-on-bug` — exit the loop as soon as any BUG_REPORT is emitted (default behavior in bug-detection mode).
- `--max-turns N` — safety limit on game turn number.
- `--p1-draw "Mountain;Lightning Bolt;Mountain"` / `--p2-draw "..."` — override starting hands.
- `--decklists` / `--no-decklists` — include or omit full deck lists in the agent's preamble (default: enabled).
- `--model {haiku, sonnet, opus, <claude --model value>}` — pick the LLM (default: `haiku`).
- `--claude-args '<extra args>'` — pass-through extras to the underlying `claude` CLI.
- `--game-dir <name>` — write to an explicit directory under `agentplay/` instead of the next `NNN.game/`.
- `--seed N` — deterministic seed (default 42).
- `--verbose` — print replay and agent diagnostic details.

Each agent prompt includes the current game state, the full log interleaved
with prior choices and rationale, the log since the last decision, a recap of
the previous decision, the current menu of choices, and (if provided) the
`--scenario` / `--goal` text.

## Manual Sessions: start_game.py + continue_game.py

For tight scripted reproducers or when you don't want to spend agent tokens,
drive the game directly:

```bash
# Start a session. Archives any prior current.game/ if one exists, creates a
# fresh numbered directory like agentplay/040.game/, runs up to the first
# choice, and prints the available actions.
./agentplay/start_game.py decks/old_school/01_rogue_rogerbrand.dck decks/old_school/02_thedeck_peterschnidrig.dck

# With controlled initial hands:
./agentplay/start_game.py decks/grizzly_bears.dck decks/royal_assassin.dck \
    --p1-draw="Forest;Grizzly Bears;Forest"

# From a puzzle / start-state file:
./agentplay/start_game.py --start-state="puzzles/bolt_test.pzl"

# Use an explicit game directory instead of the next numbered one:
./agentplay/start_game.py --game-dir=my_test.game decks/a.dck decks/b.dck
```

Add choices one at a time. `continue_game.py` requires the player (`p1` or
`p2`) and the choice as a number or rich-text command:

```bash
./agentplay/continue_game.py p1 "0"
./agentplay/continue_game.py p1 "play mountain"
./agentplay/continue_game.py p1 "cast lightning bolt"
./agentplay/continue_game.py p1 "target bob"
./agentplay/continue_game.py p2 "pass"

# Target a specific past session:
./agentplay/continue_game.py --game-dir=my_test.game p1 "1"
```

Each `continue_game.py` call:

1. Appends the choice to that player's `pN_choices.txt`.
2. Replays the full game from scratch with every accumulated choice.
3. Stops once the next choice is needed, prints the new menu.
4. Rewrites `reproduce_game.sh` with the up-to-date reproducer.

### Rich-text vs numeric input

Both forms work, in any mix:

- **Numeric** — simple but fragile to menu reordering: `"0"`, `"3"`.
- **Rich text** — robust to option ordering: `"play mountain"`, `"cast lightning bolt"`, `"target bob"`, `"pass"`.

Full syntax (card-name matching, wildcards, special cases) lives in
[FIXED_INPUT_SYNTAX.md](./FIXED_INPUT_SYNTAX.md).

## Game Directory Structure

Sessions live under `agentplay/`. By default each new session goes to the next
unused `NNN.game/` directory; the most recent one is also reachable as
`agentplay/current.game/` (typically a symlink or alias maintained by the
scripts). Use `--game-dir <name>` to opt out of the numbering scheme.

A session directory contains:

| File | Contents |
| --- | --- |
| `p1_choices.txt` | Player 1's choices, one per line, in order. |
| `p2_choices.txt` | Player 2's choices, one per line, in order. |
| `initial_args.txt` | The original `mtg tui` (or `mtg puzzle`) argv. |
| `snapshot.json` | Latest replayed game state (JSON). |
| `game.snapshot` | Binary snapshot (when produced by the engine). |
| `game.log` | Engine log from the last replay. |
| `reproduce_game.sh` | Executable shell script that replays the whole session deterministically. |
| `enriched_log.md` | (agent_game.py only) Interleaved log + agent reasoning per choice. |
| `bug_reports.log` | (bug-detection mode) STOP / BUG_REPORT entries, if any. |

Use these for inspection, attaching to bug reports, and re-running.

## Reproducers

`reproduce_game.sh` is regenerated after every choice and embeds a `cargo run`
invocation of the form:

```bash
cargo run --release --bin mtg -- tui decks/old_school/01_rogue_rogerbrand.dck decks/old_school/02_thedeck_peterschnidrig.dck \
    --p1=fixed --p2=fixed \
    --p1-fixed-inputs="0;1;pass;play swamp" \
    --p2-fixed-inputs="0;1;pass;play swamp" \
    --stop-on-choice=5 \
    --seed=42 --json --log-tail=100
```

Run it directly (`./agentplay/NNN.game/reproduce_game.sh`) to replay the
session from scratch. The same script is what you should paste into bug
reports — it's self-contained, deterministic, and survives session cleanup.

### Session management

```bash
# List all sessions, oldest to newest.
ls -d agentplay/*.game/

# Replay an archived session deterministically.
./agentplay/017.game/reproduce_game.sh

# Wipe everything (only do this if you really mean it).
rm -rf agentplay/*.game/ agentplay/current.game/
```

## Advanced: Driving `mtg tui` Directly

When the agentplay scripts don't fit (custom controller mixes, snapshotting at
specific points), use `mtg tui` / `mtg resume` directly.

### Fixed-input loop (manual reproducer construction)

```bash
# Start with empty inputs; engine stops when the script runs out.
cargo run --release --bin mtg -- tui decks/grizzly_bears.dck decks/royal_assassin.dck \
    --seed=100 \
    --stop-when-fixed-exhausted \
    --p1=fixed --p1-fixed-inputs="" \
    --p2=fixed --p2-fixed-inputs="" \
    --log-tail=100

# Append the next choice(s) and rerun:
cargo run --release --bin mtg -- tui decks/grizzly_bears.dck decks/royal_assassin.dck \
    --seed=100 \
    --stop-when-fixed-exhausted \
    --p1=fixed --p1-fixed-inputs="0" \
    --p2=fixed --p2-fixed-inputs="0" \
    --log-tail=100
```

### Snapshot / resume

```bash
# Capture a snapshot after N choices played by heuristic controllers.
cargo run --release --bin mtg -- tui DECK1.dck DECK2.dck \
    --seed=100 \
    --stop-on-choice=10 \
    --snapshot-output=game.snapshot \
    --p1=heuristic --p2=heuristic

# Resume with different controllers (e.g. drop into fixed-input mode for a hand-crafted finish).
cargo run --release --bin mtg -- resume game.snapshot \
    --override-p1=fixed --p1-fixed-inputs="0;1;2" \
    --override-p2=fixed --p2-fixed-inputs="0;1;2"
```

### Useful flags

- `--stop-on-choice=N` — stop after N total choices.
- `--stop-on-choice=N:p1` / `:p2` — stop after N choices by a specific seat.
- `--p1-draw="Mountain;Lightning Bolt;Mountain"` / `--p2-draw="..."` — pin the
  starting hand (1–7 cards, semicolon-separated).
- `--start-state="puzzles/combat_test.pzl"` — load a pre-built board state.

## Key Concepts for AI Agents

- **You can't type at stdin.** Use fixed-input controllers, snapshots, or the
  agentplay scripts — they replay determistically.
- **Fixed inputs are semicolon-separated** (not commas, not spaces):
  `--p1-fixed-inputs="0;1;2;0"` or
  `--p1-fixed-inputs="play mountain;cast bolt;target bob"`.
- **Per-player choice files**: agentplay separates `p1_choices.txt` and
  `p2_choices.txt`. `continue_game.py` decides which file to extend from its
  first positional argument (`p1` or `p2`).
- **Snapshots preserve everything**: game state, RNG state, controller state,
  turn / choice counters. Resume produces byte-identical continuations.
- **Reproducers replay from scratch** via `mtg tui`, not by loading a
  snapshot — same seed plus same choices implies the same outcome.

## Tips for Building Good Reproducers

1. **Start minimal** — use the smallest decks that reproduce the issue.
2. **Pin the opening hand** with `--p1-draw` / `--p2-draw` (or pass them
   through `agent_game.py`) so the bug doesn't depend on top-deck luck.
3. **Stop as soon as the bug fires.** Trim trailing choices; a shorter
   reproducer is easier to debug and faster to run in CI.
4. **Re-run the reproducer** at least once to confirm determinism before
   filing it.
5. **Attach `reproduce_game.sh`** (or its inlined command) to the bug
   report along with the relevant log excerpt.

## Reference

```bash
cargo run --bin mtg -- tui --help
cargo run --bin mtg -- resume --help
./agentplay/agent_game.py --help
./agentplay/start_game.py --help
./agentplay/continue_game.py --help
```

Related docs:

- [FIXED_INPUT_SYNTAX.md](./FIXED_INPUT_SYNTAX.md) — full input-command syntax.
- [agentplay/README.md](../agentplay/README.md) — short tour of the harness.
- `src/main.rs` — authoritative CLI argument definitions.
