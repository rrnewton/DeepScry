# Architecture Overview

Part II explains the engineering heart of DeepScry: how a game can run
identically on a server and on every client, how the engine reproduces a game
bit-for-bit on demand, and how the two ideas combine into a replicated
state-machine model for networked multiplayer.

Three principles tie the whole engine together. They are worth stating up front
because every later chapter is an elaboration of one of them.

1. **Deterministic sequential simulation.** Given the same inputs — decks,
   starting hands, seed, and the ordered sequence of player choices — the engine
   always produces exactly the same game. There is no wall-clock dependence, no
   parallel race, no hidden randomness. This is what makes reproducers,
   snapshots, and replay possible at all.

2. **The replicated state machine.** A networked game is one *server* running
   the authoritative game and two *clients* each running an identical copy. The
   server does not stream pixels or diffs to the clients; it streams the same
   *inputs* into the same deterministic engine, and every copy stays in step on
   its own. A client only ever knows what it is *allowed* to know — hidden
   information (an opponent's hand, the library order) stays hidden — yet its
   copy still computes the same public state.

3. **Desync is always fatal.** Because the copies are supposed to be identical,
   *any* divergence between them is a bug, full stop. The engine never tries to
   "recover" from a desync by patching state; it treats divergence as an
   immediate fatal error. Extra validation data carried in network messages
   exists only to *detect* divergence early, never to repair it.

## How the chapters fit together

- **[Network Architecture](./network_architecture.md)** is the north-star
  document for the networked model: the replicated golden/shadow state, linear
  control transfer, the list of forbidden patterns, and the
  controller-information-independence rule. *(This chapter is the project's
  canonical `docs/NETWORK_ARCHITECTURE.md`, included verbatim.)*

- **[Deterministic Simulation](./deterministic_simulation.md)** drills into
  principle 1: where determinism comes from, what threatens it, and the project
  conventions that protect it.

- **[The Replicated State Machine](./replicated_state_machine.md)** drills into
  principle 2: how identical engine copies are kept in sync by replicating
  inputs, and how hidden information is reconciled with deterministic shared
  identifiers for cards.

- **[Snapshot and Replay](./snapshot_architecture.md)** covers the rewind /
  replay mechanism that the single-player snapshot feature and the networked
  shadow state both rely on. *(Included from the project's
  `ai_docs/reference/snapshot_architecture.md`.)*

> **A word on freshness.** Two of these chapters are *included directly* from
> existing project documents that were judged current and clean. The other two
> were **written fresh for this guide** because their source documents had
> drifted from the code — most importantly, the card-identity model was
> reworked (the "late-binding card identifier" change) and older docs still
> described the superseded design. Where this guide states how something works
> today, it follows the code; the
> [Replicated State Machine](./replicated_state_machine.md) chapter calls out
> exactly which older design it supersedes.
