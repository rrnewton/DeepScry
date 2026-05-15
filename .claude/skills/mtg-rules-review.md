---
name: mtg-rules-review
description: Mandatory MTG Comprehensive Rules compliance review for every bug fix before it merges to integration. Apply regardless of where the bug originated (fuzz testing, user report, tournament play, internal QA). Catches fixes that paper over the wrong layer (e.g., changing game semantics as a workaround) and ensures information-hiding / network-determinism invariants are preserved.
---

# MTG Rules Review

This skill defines the **mandatory rules-compliance review** that every
bug fix must pass before it lands on `integration`. It is *orthogonal*
to `make validate` (which checks that code compiles and tests pass) —
this review checks that the **fix is the correct fix** under the MTG
Comprehensive Rules and the project's network-architecture invariants.

The review is required for **all** bug fixes, no matter where the bug
came from:

- Fuzz testing or QA harnesses
- User-reported bugs
- Tournament-play discoveries
- Differential testing against Forge-Java
- Internal review / refactoring that incidentally fixes a bug

If a change touches game logic and is labeled as a fix, this skill
applies.

---

## When to invoke this skill

- **Before opening / merging a PR** that contains a bug fix into
  `integration` (or any branch that will subsequently be promoted to
  `integration`/`main`).
- **Before committing directly to `integration`** when working on a fix
  there.
- **After landing**, if a regression is discovered that the original
  review missed — re-run the checklist on the fix-of-the-fix.

For pure refactors, doc/process changes, dependency bumps, and
non-gameplay fixes (build, CI, scripts), this skill does not apply.
When in doubt, run the checklist anyway — it is cheap.

---

## Required reading before reviewing

- `rules/02_mtg_rules_condensed_medium_length_gemini.md` — condensed
  rules summary; use this first.
- `rules/01_full_official_MagicCompRules_20250919` — full
  Comprehensive Rules. Cite section numbers (e.g. "CR 701.18" for
  Scry, "CR 701.34" for Surveil, "CR 701.20" for Search).
- `docs/NETWORK_ARCHITECTURE.md` — the deterministic-sequential
  simulation model. Information hiding and controller
  information-independence are inviolable.
- `CLAUDE.md` — coding conventions, branch policy, and the rule that
  desync is **always fatal**.

---

## The Review Checklist

For every bug fix, walk through each item below. Record the answer
and your reasoning in the PR description (or commit message body) under
a heading `## MTG Rules Review`. Each item must be answered explicitly
— do not leave any blank.

### 1. Correct rule implementation

> **Does the fix correctly implement the relevant MTG rule? Cite the CR
> section number(s).**

- Identify which rule(s) the buggy behavior violated.
- Identify which rule(s) the fix now satisfies.
- Cite specific Comprehensive Rules sections (e.g. "CR 603.2 trigger
  events", "CR 117.5 priority").
- If the fix touches more than one rule (typical for replacement /
  trigger interactions), cite each.
- If the bug is about a card-specific interaction, also cite the
  card's Oracle text and the rule that governs the interaction.

### 2. Reveal ordering (controller sees information *before* deciding)

> **Are card reveals sent to the controlling player BEFORE they make
> the decision derived from those cards?**

This applies to (non-exhaustive):

- **Scry** (CR 701.18) — controller must see the top N before
  choosing to keep / put on bottom.
- **Surveil** (CR 701.34) — controller must see the top N before
  choosing keep / mill.
- **Search** (CR 701.20) — controller must see the searched zone's
  contents (subject to hidden-information rules) before choosing
  what to fetch.
- **Look at the top N**, **reveal until**, **choose one from
  among**, mode/target choices that depend on hidden info.

For each such effect touched by the fix:

- Confirm the engine emits the reveal/snapshot **into the controller's
  view** *before* requesting the decision.
- Confirm the opponent does **not** receive the reveal unless the
  effect text says so ("reveal them" vs. "look at them").
- Confirm the gamelog records the reveal in an order that a replayer
  can reconstruct.

### 3. Information hiding

> **Is hidden information actually hidden from players who shouldn't
> see it?**

- Opponent's hand contents (CR 402): hidden unless revealed.
- Library order (CR 401.2): hidden from everyone except via specific
  effects.
- Face-down cards (CR 708): hidden from opponents; the controller
  knows what they are.
- Sideboard / command zone hidden cards: handled per format rules.

The fix must not:

- Leak hidden state into messages sent to other players.
- Leak hidden state into controller decisions made by a player who
  shouldn't have that information (a heuristic AI playing for player
  A must not see player B's hand).
- Bypass the shadow-state mechanism described in
  `docs/NETWORK_ARCHITECTURE.md`.

### 4. Server / client state sync

> **Does game state stay synchronized between server and client,
> modulo information hiding?**

- After the fix, run (or describe how you would run) a network mode
  game and confirm no desync warnings.
- Confirm any new fields / events are part of the deterministic
  delta stream and are reproducible from the same RNG seed.
- Confirm controllers remain **information-independent**: the same
  controller fed the same visible-state stream must produce the same
  decision whether running on the server or a client.
- **Reminder: desync is ALWAYS fatal.** A fix is unacceptable if it
  introduces silent recovery from inconsistent state.

### 5. Semantic workaround vs. real fix

> **Does the fix change game semantics as a workaround, rather than
> fixing the actual bug?**

Red flags:

- The fix narrows or alters what an effect does so the buggy code
  path is no longer reached, but the effect now violates Oracle text
  / CR.
- The fix silently swallows or degrades an event (e.g. "if the
  trigger would fire here, just skip it") instead of handling the
  trigger correctly.
- The fix adds a special case keyed on a specific card name when the
  underlying mechanic is general.
- The fix adds a `// TODO: real fix` while shipping the workaround.

If a true fix is out of scope, the workaround MUST:

1. Be clearly labeled as such in code comments.
2. File a beads issue describing the real fix (and reference it via
   `// TODO(mtg-NNN):`).
3. Be flagged as `CONCERN` in the verdict (see below), not `PASS`.

### 6. Generalization / bug-class search

> **Are there other cards or mechanics that could have the same bug
> class?**

- Identify the *class* of bug, not just the instance (e.g. "all
  triggered abilities that look at zones the trigger source has
  already left", "all reveal-then-choose effects", "all replacement
  effects that fire during state-based actions").
- Search the codebase for sibling sites:
  - `meta_codesearch:code_search` for the relevant ApiType, trigger
    pattern, parser branch, or keyword.
  - Grep `rules/cards/` (if present) for cards that share the
    keyword / template.
- Either fix the siblings in the same change, or file beads issues
  for each remaining instance and link them in the verdict.

---

## Verdict format

Conclude the review with a single verdict block at the end of the PR
description / commit message:

```
## MTG Rules Review — Verdict: <PASS | FAIL | CONCERN>

1. Correct rule implementation: <answer + CR cite>
2. Reveal ordering:             <answer or N/A>
3. Information hiding:          <answer or N/A>
4. Server/client sync:          <answer or N/A>
5. Workaround vs. real fix:     <answer>
6. Bug-class generalization:    <answer + linked beads issues>

Reasoning: <2–6 sentences explaining the verdict, calling out any
follow-up issues filed and any items marked CONCERN.>
```

### Verdict meanings

- **PASS** — All six items are answered satisfactorily; the fix is
  correct under the CR and respects information-hiding /
  determinism invariants. Safe to merge to `integration`.
- **CONCERN** — The fix is acceptable to land, but at least one item
  exposes a known limitation (e.g. acknowledged workaround, partial
  bug-class coverage). A beads issue MUST be filed and referenced
  for each concern. Reviewer must explicitly accept the concerns
  before merge.
- **FAIL** — At least one item identifies a correctness, information-
  hiding, or determinism violation that the fix does not address.
  Do **not** merge. Iterate on the fix and re-review.

A FAIL verdict blocks the merge. A CONCERN verdict requires a
follow-up beads issue (referenced in the verdict block) before merge.
Only PASS / accepted-CONCERN may land on `integration`.

---

## How this skill interacts with other workflows

- **`compatibility_tracking`** — if the bug fix changes the support
  matrix for a card or effect, also run that skill and update the
  per-card beads issue + `docs/EFFECT_SUPPORT.md`.
- **`ci-integration-monitor` agent** — when promoting feature branches
  to `integration`, the agent must confirm that each merged commit
  has an MTG Rules Review verdict in its message (or in the PR body
  for squash-merged PRs). Commits without a verdict are rejected.
- **Pre-Commit checklist in `CLAUDE.md`** — `make validate` and the
  fmt check still apply unconditionally. The rules review is in
  *addition* to those, not a replacement.
- **Real-game justification** — the existing CLAUDE.md requirement to
  cite a gamelog snippet from `agentplay/*.sh` / `mtg tui` games
  remains in force, and the snippet should demonstrate the corrected
  behavior cited in item 1 of the checklist.
