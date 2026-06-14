# The Replicated State Machine

A networked DeepScry game is **one authoritative game running in three identical
copies**: the server's copy (the "golden" state) and one "shadow" copy inside
each of the two connected clients. The clients are not thin terminals receiving a
rendered view — each one runs the *same engine* over the *same inputs* and
computes the same public game state locally.

This chapter explains how those copies are kept identical, and how that is
possible even though each player is forbidden from seeing the other's hidden
information.

> **Freshness note.** This chapter was written fresh for the guide. It
> supersedes the older `ai_docs/reference/SHADOW_STATE_AND_CARDID_ALLOCATION.md`
> and the proposal `ai_docs/reference/DESIGN_late_binding_cardid.md`, whose
> concrete mechanisms (`LibraryMode::Remote`, `pending_reveals`,
> `hidden_card_count`) **no longer exist in the code**. The current design — the
> "late-binding card identifier" model — is described below and verified against
> the source.

## Replicate the inputs, not the state

The core move is this: rather than send game *state* across the network, the
server sends the same *inputs* into every copy of the engine, and determinism
(see [Deterministic Simulation](./deterministic_simulation.md)) guarantees each
copy ends up in the same place on its own.

Concretely, when it is a player's turn to make a choice, exactly one copy is
"driving"; the resulting choice is distributed and every copy applies it. Because
the engine is a deterministic function of its inputs, applying the same choice to
the same prior state yields the same next state everywhere. Control passes from
copy to copy linearly — there is never more than one decision in flight, and
there is no parallel processing to create a race.

## Desync is a fatal error, never something to patch

Since the copies are *supposed* to be identical, any divergence between them is a
bug by definition. The engine therefore treats desync as an **immediate fatal
error**. It does not attempt to reconcile or repair diverged state. Validation
data carried alongside network messages — most visibly the per-choice state
hashes enabled by `--network-debug` — exists purely to *catch* divergence as
early as possible, not to recover from it. This "desync is always fatal" stance
is a hard project invariant; see [Network Architecture](./network_architecture.md)
for the full discussion and the list of recovery patterns that are explicitly
forbidden.

## The hidden-information problem

Replicating inputs runs into an obvious tension: a client must compute the same
public state as the server, but it is not allowed to *know* the opponent's hand
or the order of either library. If a client doesn't know what a face-down card
is, how can its copy of the game agree with the server about that card?

DeepScry resolves this with **shared card identifiers whose contents are bound
late**.

### Shared identifiers, deterministic allocation

Every card slot in the game has a stable, public identifier (a `CardId`) that is
the *same number* on the server and on both clients. These identifiers are
allocated deterministically: at game start, each side runs the same
initialisation over the same decklists, so every side independently assigns
identical identifiers to identical slots. No identifier has to be negotiated over
the wire — determinism makes them agree for free.

This is why a player's whole decklist is shared at game start (in the
`GameStarted` message): not so the opponent can read your cards, but so both
sides can run the same deterministic allocation and arrive at the same set of
identifiers.

### Late binding: the identifier is public, the card is not

The key idea is that a `CardId` being public does **not** mean the card's
*identity* (its name) is public. A card has a per-viewer reveal state: it is
known to some players and unknown to others. In the code this is a bitmask on the
card — `revealed_to_mask: u8` in `mtg-engine/src/core/card.rs` — recording which
players are currently allowed to see that card's face.

So all three copies agree on "there is a card with identifier 57 in the
opponent's hand", and they agree on its public properties, but only the copies
permitted to see card 57's face know that it is, say, Lightning Bolt. The
opponent's client tracks the slot and its identifier without ever learning the
name.

### Revealing is a first-class, undoable action

When hidden information legitimately becomes visible — a card is drawn into a
hand the viewer can see, revealed by an effect, or moved to a public zone — the
engine performs an explicit `RevealCard` action (`mtg-engine/src/undo.rs`). That
action records the new reveal mask (and the previous mask, so it can be undone
during rewind). Reveals therefore flow through the same deterministic,
rewind-safe action machinery as everything else: they replay bit-identically and
can be rolled back cleanly.

This "late-binding card identifier" design replaced an earlier model that tracked
hidden cards by *counting* them and only handed out identifiers at reveal time.
The current model is simpler: identifiers exist up front and are shared; only the
*binding* of identifier to face is deferred. The header comment in
`mtg-engine/src/zones.rs` records this explicitly ("Late-Binding CardID
Architecture") and notes that it "eliminates the old `LibraryMode::Remote` and
`pending_reveals` complexity."

## Putting it together

A networked game thus stays in lock-step like this:

1. At game start, every copy deterministically allocates the same shared
   `CardId`s from the shared decklists, and sets reveal masks so each player can
   see only what they should.
2. Play proceeds by replicating one choice at a time into every copy; each copy
   advances its own deterministic engine.
3. When information legitimately changes hands, an explicit, undoable
   `RevealCard` updates the per-viewer reveal mask — identically on every copy
   permitted to see it.
4. Optional per-choice state hashes confirm the copies still match; if they ever
   don't, the game fails fast rather than papering over the divergence.

The result is a replicated state machine in which every participant computes the
same public truth, no participant learns more than the rules allow, and any
deviation is caught immediately.
