---
name: targeted-compatibility
description: Invoke when given a specific card, deck file(s), or set code to drive targeted compatibility testing and fixes, systematically exercising each aspect (passive/triggered/activated abilities, mana and alternative costs, legal zones and timings, interactions) and updating per-card tracking issues with per-aspect WORKING/PARTIAL/BROKEN evidence backed by mtg tui game logs.
---

# Targeted MTG Compatibility Expansion

This skill covers **targeted** compatibility work: the user (or parent
agent) has already named a SPECIFIC focus — a card, one or more deck
files, or a set code — and we drive thorough gameplay coverage of that
focus. It is the directed counterpart to the
`.claude/commands/expand_MTG_compatibility.md` command, which
DISCOVERS broken behavior at random.

## When to use this skill (vs. `/expand_MTG_compatibility`)

| Situation                                          | Use                              |
|----------------------------------------------------|----------------------------------|
| "Pick something broken and fix it" / batch sweep   | `/expand_MTG_compatibility`      |
| "Make `Sengir Vampire` work"                       | **this skill**                   |
| "Get the `old_school/burn_red.dck` deck playable"  | **this skill**                   |
| "Bring set `LEA` to WORKING for every card"        | **this skill**                   |
| "Audit just the kicker cost on `Burning Anger`"    | **this skill** (aspect override) |

If the brief names ANY specific card / deck file / set code as the
subject of the work, this is the right skill. There is **no random
selection step** here.

## Inputs

Exactly one of the following MUST be supplied by the caller:

1. **Card name** — e.g. `"Sengir Vampire"`. Resolve to
   `cardsfolder/<x>/<sanitized_name>.txt`.
2. **Deck file(s)** — one or more `.dck` paths under `decks/` or
   `forge-java/`. Every card in the deck is in scope.
3. **Set code** — e.g. `LEA`, `M11`. Resolve via `cardsfolder/` set
   tags; every printed card in the set is in scope.

Optional refinements:

- **Aspect filter** — a specific dimension to focus on (e.g. "only
  verify kicker cost", "only activated abilities", "only triggered
  abilities on death"). Skips the full per-aspect sweep below.
- **Existing puzzle** — pointer to a `.pzl` reproducer to use as the
  starting state instead of a synthesized one.

## Delegated workflow: compatibility_tracking

This skill delegates the **mechanical ceremony** of per-card
classification, beads issue structure, support-matrix updates, the
WORKING/PARTIAL/BROKEN vocabulary, and game-log evidence requirements
to `.claude/skills/compatibility_tracking/SKILL.md`. Read that skill
in full before starting — it owns:

- §2.2 mandatory game-log verification table
- §3 status classification
- §5 per-card beads issue template
- §6 `docs/EFFECT_SUPPORT.md` row format
- §8 quick-reference checklist

This skill adds the **targeting and per-aspect sweep** on top of that
ceremony. Do not duplicate the ceremony rules here; cite the section
numbers when you apply them.

## Per-aspect checklist (the core of targeted work)

For each card in scope, drive a SEPARATE gameplay verification for
every aspect below that the card exhibits. Each verified aspect
produces its own bullet in the dated `Findings (...)` block of the
per-card beads issue (see compatibility_tracking §5), tagged
WORKING / PARTIAL / BROKEN with a quoted log snippet.

### Static / passive characteristics

- [ ] Parses with the printed mana cost, P/T (if creature/vehicle),
      colors, types, subtypes, supertypes (legendary, snow, …).
- [ ] Every printed keyword (`Flying`, `First Strike`, `Protection
      from X`, `Regenerate`, `Vanishing N`, `Living Weapon`, …)
      appears in the parsed `Keywords` list AND is honored by the
      engine in a real game.
- [ ] Static abilities (`S:`) — each conditional clause
      (`IsPresent$`, `Threshold$`, controller/zone filters) verified
      in BOTH the satisfied and unsatisfied state.

### Casting

- [ ] Castable from the **hand** at expected timing (sorcery vs.
      instant speed; see CR 307/304/302).
- [ ] **Alternative costs** — flashback, madness, overload, kicker,
      multikicker, escalate, evoke, dash, prowl, surge, awaken,
      bestow, morph/manifest (face-down cast cost). Each alternative
      cost mode tested separately.
- [ ] **Additional costs** — sacrifice, discard, pay life, tap
      another permanent, exile from graveyard, reveal.
- [ ] **Mana cost composition** — hybrid, Phyrexian, snow {S},
      generic-X with X-derived effects, colorless {C}.
- [ ] **Targeting at cast time** — legal target enumeration,
      illegal-target rejection, "if able" / "you may" optionality.
- [ ] Castable from **non-hand zones** when the card permits
      (flashback / graveyard, suspend / exile, foretell, adventure,
      cycling, etc.). Each legal-cast zone produces a separate
      WORKING/PARTIAL/BROKEN bullet.

### Triggered abilities (`T:`)

For each `T:` line:

- [ ] Trigger source enumeration is correct (self-trigger vs.
      `ValidCard$` predicate).
- [ ] Trigger fires at the **right event** (ETB / LTB / dies / attacks
      / blocks / deals damage / cast / draw / upkeep / end step / …).
- [ ] Trigger fires for the **right party** (controller vs. any
      player, "you" vs. "each player").
- [ ] Trigger respects **conditional restrictions** (`Condition$`,
      "if you control", once-per-turn, intervening "if").
- [ ] Trigger interaction with **leaves-the-battlefield** timing
      (LKI, last-known-information for "this creature's controller").
- [ ] Trigger fires **on the stack as a separate object** — modal
      choices, targeting, and counterability are exercised.

### Activated abilities (`A:`)

For each `A:` line:

- [ ] **Cost shape** parsed correctly (mana, tap, untap, sacrifice,
      counter removal `SubCounter<...>`, discard, exile, pay life,
      additional costs).
- [ ] **Activation timing** — sorcery-speed vs. instant-speed; restricted
      to specific phases / step; restricted to controller's turn.
- [ ] **Activation zone** — battlefield (default), graveyard
      (flashback-like / unearth), hand (cycling, channel), exile
      (suspend, foretell), library (rarely).
- [ ] **Activation limit** — "Activate only once each turn", "Activate
      only if …".
- [ ] **Mana abilities** (CR 605) — produce mana of the right color,
      do not use the stack, no targeting.
- [ ] **Loyalty abilities** for planeswalkers — costs paid as loyalty
      counters; once-per-turn restriction.

### Replacement / prevention effects (`R:`)

- [ ] Triggers on the right `Event$`.
- [ ] Modifies the event correctly (replace damage, replace zone
      change, replace draw, replace ETB with counters).
- [ ] Self-replacement vs. global replacement scoping.
- [ ] Multiple replacement effects: controller of affected object
      chooses order (CR 616).

### Interactions to probe

A card is not "WORKING" until it survives at least the relevant
subset of these probes:

- [ ] **Counterspell** at cast (`Counterspell`, `Force Spike`).
- [ ] **Targeted removal** (`Lightning Bolt`, `Doom Blade`,
      `Swords to Plowshares` — exile vs. destroy vs. damage).
- [ ] **Bounce** (`Unsummon`, `Boomerang`) — returns to hand, ETB
      triggers re-fire on recast.
- [ ] **Sacrifice** — triggers vs. dies-replacement order.
- [ ] **Zone migration** — graveyard ↔ exile ↔ hand ↔ library; verify
      LTB triggers fire from each origin.
- [ ] **Copying** — `Clone`, `Fork`; the copy retains relevant
      attributes.
- [ ] **Static-effect interaction** with anthems (`Glorious Anthem`),
      cost reducers (`Helm of Awakening`), color hosers (`CoP: Red`),
      and protection.

### Card-shape edge cases

- [ ] **Split cards / fuse** — each half cast independently, fused
      cast pays both costs and resolves both halves (CR 708.2).
- [ ] **DFCs / MDFCs / adventures** — both faces parse and are
      castable per the printed rules.
- [ ] **Modal spells** — each mode chosen separately produces correct
      log evidence.
- [ ] **X-cost** — X=0, X=1, X=large all behave correctly; X
      announced on cast and locked.
- [ ] **Tokens created by the card** — token name, type line, P/T,
      keywords, colors match the printed instruction.

When an aspect does not apply to a particular card, write
`[N/A] <aspect>: <one-line reason>` in the findings block rather than
silently omitting it. The N/A entries are the audit trail proving you
considered the aspect.

## Gameplay methodology

Reuse the reproducer ladder from
`docs/HOWTO_AGENTPLAY+REPRODUCERS.md`, simplest first. The
TARGETING twist: you must guarantee the card actually appears in the
relevant zone for the aspect under test.

1. **Curated draw**: `mtg tui --p1-draw '<card>' --p2-draw '<setup>'`
   for ETB / cast-from-hand verification.
2. **Puzzle file** (`test_puzzles/<name>_<aspect>.pzl`): pre-place
   the card in the zone the aspect requires (battlefield with
   counters, graveyard for flashback, exile with time counter for
   suspend, attached for Auras/Equipment, etc.). One puzzle per
   non-trivial aspect — do NOT cram multiple aspects into one puzzle,
   that destroys the per-aspect evidence trail.
3. **Fixed-input scripts**: `--p1-fixed-inputs '<seq>' --seed 42
   --verbosity 3 --json` to drive precisely-controlled choice
   sequences. Combine with puzzle files for activated-ability and
   interaction probes.
4. **agentplay/agent_game.py**: last resort for sequences that fixed
   inputs cannot reach (true free-form play, multi-turn setups). Slow
   and token-heavy; use only when (1) and (3) cannot cover the
   scenario.

For a **deck-level** target, curate the deck list (or use the deck
file as-is) and play several full games with `mtg tourney` style
runs, but **focus the per-aspect work on cards that actually
triggered or were cast** during those games. Cards in the deck that
never made it to a relevant zone need a synthesized puzzle reproducer
— do not declare them "WORKING" based on the parser alone.

For a **set-level** target, iterate cards in `bd list` order of the
parent `Set Compatibility:` issue, marking each as you finish.

## Evidence and tracking-issue updates

Every aspect-verification produces:

1. A **dated `Findings (...)` block bullet** in the per-card beads
   issue per compatibility_tracking §5, formatted as one of:
   - `[x] <aspect>: <one-line summary>` for WORKING
   - `[PARTIAL] <aspect>: <what works> / <what's missing>` —
     references a `Bug:` issue
   - `[BROKEN] <aspect>: <root cause>` — references a `Bug:` issue
   - `[N/A] <aspect>: <reason>`
2. A **quoted `mtg tui --verbosity 3` log snippet** under the
   `Expected log evidence` section that proves the aspect. Sentinel
   strings (`Fixed1`, `Unknown(*)`, raw IDs) make the evidence count
   as BROKEN even if mechanics look right (compatibility_tracking
   §2.2).
3. A **reproducer command** that the next agent can copy-paste to
   re-verify. Prefer puzzle + fixed-inputs over agentplay for
   reproducibility.
4. A **`docs/EFFECT_SUPPORT.md` row update** for every construct
   whose status changed in this aspect's verification
   (compatibility_tracking §6).
5. A **pointer to the agentplay capture** if `agentplay/agent_game.py`
   was used — drop the captured `.log` / `.json` under
   `agentplay/<card>_<aspect>_<date>/` and cite the relative path in
   the issue.

Update the trailing `CARD STATUS:` line to reflect the **worst** of
the per-aspect statuses: any BROKEN aspect ⇒ BROKEN; any PARTIAL ⇒
PARTIAL; else WORKING.

For **deck-level** and **set-level** targets, ALSO update the parent
`Deck Compatibility:` / `Set Compatibility:` issue to reflect the
roll-up — but do NOT close the parent until every constituent card is
WORKING (or explicitly waived).

## Bug discovery workflow

When an aspect resolves to BROKEN or PARTIAL:

1. **Identify the root cause class** (compatibility_tracking §4):
   silent parser drop, effect-converter hardcoding, missing engine
   feature, log gap, hacky string-op on structured data.
2. **File a separate `Bug:` beads issue** describing the
   parser/engine gap. One bug typically affects many cards — do NOT
   bury it inside the per-card issue. Reference the bug from BOTH the
   per-card issue and the matrix row.
3. **Write a minimal reproducer** under `test_puzzles/` (preferred)
   or `tests/` that fails today. The reproducer goes in the same
   commit as the bug filing.
4. **Prefer fixing the root cause** over papering over symptoms. Any
   fix to game logic triggers the **mandatory MTG rules review** —
   follow `.claude/skills/mtg-rules-review/SKILL.md` end-to-end and
   include the verdict block in the commit message.
5. **Generalize** — find sibling cards affected by the same bug
   class. Either fix them in the same change or list them in the bug
   issue.

## Stop conditions

The targeted task is "done" when:

- **Card target**: the per-card issue has every relevant aspect from
  the checklist either checked WORKING with log evidence or marked
  N/A, the CARD STATUS line says WORKING, parser unit test + e2e
  puzzle test exist (compatibility_tracking §2.3), and the issue is
  closed.
- **Deck target**: every constituent card has its own per-card issue
  at WORKING (or PARTIAL with an accepted bug-issue followup), AND
  the deck has been played end-to-end at least once via `mtg tourney`
  / `mtg tui` with no illegal-action / desync / crash errors. Update
  the `Deck Compatibility:` parent and close it only when all
  constituents are green.
- **Set target**: same as deck, but iterated across every printed
  card in the set. Close the `Set Compatibility:` parent only when
  every constituent is WORKING.
- **Aspect-filtered target** (single aspect override): only that
  aspect's bullet needs to be resolved. CARD STATUS may remain
  PARTIAL with the other aspects still unverified, but the
  Findings block must show `[unverified] <aspect>: deferred — out of
  scope of this task` for everything we skipped, so the next agent
  knows what's left.

## Pre-commit / push

Follow the project rules in `CLAUDE.md` and compatibility_tracking
§7 verbatim:

1. `cargo fmt --all` then `cargo fmt --all -- --check`.
2. `make validate` green.
3. Commit message includes Test Results Summary, Relationship to Java
   Forge, and gameplay-log evidence (a runnable reproducer command).
4. For any game-logic bug fix: include the MTG Rules Review verdict
   block (`mtg-rules-review` skill §Verdict).
5. Push to the feature branch. Do not merge to `main` directly —
   `ci-integration-monitor` promotes `integration` → `main`.

## Quick-reference checklist

Before declaring a targeted task done:

- [ ] Target (card / deck / set) explicitly named in the brief and
      in the beads issue title hierarchy.
- [ ] Per-card beads issue exists for every card in scope, populated
      with the §5 template.
- [ ] Per-aspect checklist walked for every card; every aspect is
      WORKING / PARTIAL / BROKEN / N/A with reasoning.
- [ ] Every WORKING aspect has quoted log evidence (no sentinels).
- [ ] Every PARTIAL / BROKEN aspect has a linked `Bug:` issue and a
      failing-today reproducer.
- [ ] `docs/EFFECT_SUPPORT.md` has rows for every construct touched,
      with current status and timestamp.
- [ ] Parent `Deck Compatibility:` / `Set Compatibility:` issue
      updated; closed only if every constituent is WORKING.
- [ ] `make validate` green, fmt clean, MTG Rules Review verdict
      block present for any game-logic fix.
- [ ] Branch pushed; no direct push to `main`.
