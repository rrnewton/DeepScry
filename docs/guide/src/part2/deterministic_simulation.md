# Deterministic Simulation

Determinism is the foundation everything else is built on. If a game is not
reproducible, then reproducers do not reproduce, snapshots do not resume
faithfully, and a networked copy cannot be trusted to match the server. This
chapter explains where determinism comes from in DeepScry and what the codebase
does to protect it.

## What "deterministic" means here

A DeepScry game is a pure function of four things:

- the two decks,
- the starting hands,
- the random seed,
- and the ordered sequence of choices made by the controllers.

Given those, the engine always produces the same sequence of game actions and
the same final state — down to the bytes. There is deliberately **no** other
source of variation. In particular:

- **No wall-clock time.** Nothing in the game loop branches on the current time.
- **No ambient randomness.** All randomness flows from the seed through an
  explicit, serialised RNG. There are no calls to a global random source.
- **No parallelism inside a game.** A single game is simulated strictly
  sequentially. (Tournaments run *many independent games* in parallel, but each
  individual game is sequential.)

Because of this, the reproducer scripts described in
[Agent Play and Reproducers](../part1/agentplay.md) can replay a game **from
scratch** — same seed plus same choices yields the same game — rather than
having to ship a saved state.

## Rewind and bit-identical replay

The engine can rewind a game to an earlier point (for example, the start of a
turn) and replay forward to reconstruct a later state. The requirement is
strict: replay must reproduce a **bit-identical** state, including internal
bookkeeping such as the undo log and the view hashes used for network and
in-browser rewind. This is the mechanism the snapshot/resume feature and the
networked shadow state both lean on; the mechanics are detailed in
[Snapshot and Replay](./snapshot_architecture.md).

The single largest threat to bit-identical replay is *transient mutable state* —
a value that one step writes onto the game and a later step reads back. If such a
value is not faithfully reconstructed when the game rewinds and replays, the
replayed state diverges from the original, and divergence is fatal.

## The project conventions that protect determinism

The codebase encodes several rules to keep replay honest. They are stated in the
project's `CLAUDE.md`; the short version:

- **Prefer functional/immutable style for game state.** Mutating shared state is
  the most common source of replay divergence, so the engine prefers
  constructions that don't depend on a value having been mutated earlier in a
  specific order.

- **Thread explicit parameters instead of stashing transient state.** If a value
  is only needed across a few call sites for one resolution (say, "who caused
  this discard"), it is passed as a parameter rather than written to a field and
  read back later. An explicit parameter carries no reconstruct-on-rewind
  obligation; a mutable field does.

- **If state must live on the game, it must be serialised.** Anything that
  genuinely has to persist on `GameState` is serialised (with `#[serde(default)]`)
  so that snapshot/resume, the network shadow, and in-browser rewind all
  reconstruct it identically. A field that is skipped from serialisation but
  still holds real game-loop state across a choice point is "a desync waiting to
  happen."

- **The litmus test:** *"if the game rewinds to turn start and replays, does this
  value come back bit-identical?"* If correctness depends on a mutation happening
  in a particular order, the value must be threaded as a parameter or serialised,
  not left as ad-hoc mutable state.

## Controllers must not smuggle in hidden information

A subtle determinism requirement applies to the decision-makers, not just the
engine. Every controller — heuristic, random, zero, fixed — **must make the same
decision whether it is running on the server (with full information) or on a
client (with only the information that client is allowed to see).** A controller
that peeks at an opponent's hand, the library order, or raw RNG state would make
different decisions in local versus networked play, which is an
information-leakage bug and a source of desync.

This is why the scripting primitives are careful to read only public state — for
example, `PASS_UNTIL` (see [Scripted Play](../part1/scripted_play.md)) keys off
the turn number and current step and nothing else. The information-independence
rule is treated as part of the network contract; see
[Network Architecture](./network_architecture.md) for the authoritative
statement.

## Verifying determinism in practice

Two mechanisms let you confirm the property holds:

- `--network-debug` attaches a state hash to network traffic and checks, after
  every choice, that the client's state hash matches the server's. A mismatch is
  reported immediately rather than allowed to fester.
- `--tag-gamelogs` prefixes each game-action log line with `[GAMELOG TurnN STEP]`
  so a local game's log and a networked game's log can be diffed line-for-line —
  an exact match is the visible proof that both ran the same deterministic
  simulation.
