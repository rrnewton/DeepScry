---
name: compatibility_tracking
description: Standard workflow for testing, classifying, and tracking individual MTG card compatibility in mtg-forge-rs. Covers per-card beads issue structure, the parallel per-effect/keyword support matrix, the WORKING/PARTIAL/BROKEN classification, the mandatory game-log verification step, regression tests (parser-shape unit + e2e puzzle), and the isolated-worktree commit-and-push flow. Use whenever a card is being evaluated, debugged, or fixed for compatibility — including ad-hoc, single-card work, not just batch testing.
---

# Compatibility Tracking — MTG Forge-rs

This skill codifies how we test, classify, and track individual card
support in mtg-forge-rs. It is invoked by the
`expand_MTG_compatibility` command for random-discovery batch work, by
the `targeted-compatibility` skill
(`.claude/skills/targeted_compatibility/SKILL.md`) for work focused on
a specific card / deck / set, and should also be followed for any
one-off card investigation.

The two artifacts every compatibility pass updates are:

1. **Per-card beads issue** — `Card Compatibility: <NAME>` (one per card)
2. **Per-effect support matrix** — `docs/EFFECT_SUPPORT.md` (one row per
   keyword/effect/script-construct that we evaluate)

Both must be touched whenever the result of a card test changes the
status of a script construct (parser branch, ApiType, trigger pattern,
cost type, replacement effect, …).

---

## 1. Required reading before starting

- `CLAUDE.md` (project conventions, branch policy, beads conventions)
- `docs/HOWTO_AGENTPLAY+REPRODUCERS.md` (puzzle/fixed-input/agentplay
  reproducer infrastructure)
- `ai_docs/CARD_SCRIPT_SPEC.md` (DSL spec — never use substring
  matching on card scripts)
- `mtg-3` tracking issue (top-level MTG feature completeness) and any
  active `Set Compatibility:` / `Deck Compatibility:` parents

Start in a clean worktree (`git status` clean, `make validate` green,
on `integration` or a feature branch off `integration`). For batch
work prefer an isolated git worktree under
`/home/newton/work/mtg/mtg-forge-rs-compat-<topic>` so other agents
are unaffected.

---

## 2. Card test protocol

For each card produce evidence under each of the following headings.
"Evidence" means a quoted log snippet, a puzzle/fixed-input
reproducer command, or a unit test name — never a bare assertion.

### 2.1 Parser-shape verification
- Card loads from `cardsfolder/<x>/<name>.txt` without an
  `Effect::Unimplemented` for any printed ability.
- `mana_cost`, `power`, `toughness`, `types`, `subtypes`, `colors`,
  and `Keywords` match the printed card. Verify with a unit test
  (`test_card_compat_<name>` in
  `mtg-engine/src/game/actions/tests/effects.rs`) that constructs the
  card and asserts the parsed shape.
- Every `K:`, `T:`, `A:`, `S:`, `R:`, `SVar:` line in the script
  produced a non-empty entry in the parsed card. **Silent drops are
  always BROKEN** — see §4 anti-patterns.

### 2.2 Gameplay behavior (the mandatory game-log step)
You **must** drive the card through a real game and inspect
`mtg tui` log output. Static "the parser produced X" checks are
necessary but never sufficient.

Pick the cheapest reproducer that exercises the behavior:

1. `mtg tui --p1-draw '<card>' --p2-draw '<opponent setup>'` for
   simple ETB/cast tests.
2. A puzzle file under `test_puzzles/<card>_<scenario>.pzl` for any
   board-state-dependent behavior (triggers, conditional effects,
   tapped/counter state).
3. `--p1-fixed-inputs '<script>' --p2-fixed-inputs '<script>'` to
   force specific choices. Combine with `--stop-on-choice=N`,
   `--json`, `--seed 42`, `--verbosity 3` for reproducible logs.
4. `agentplay/agent_game.py` only when fixed-inputs cannot drive the
   needed sequence (rare; agents are bad at writing fixed-input
   scripts).

For every printed effect verify the **game log** contains the
expected message:

| Effect printed on the card                | Required log evidence                        |
|-------------------------------------------|----------------------------------------------|
| Draw N cards                              | `<player> draws <card>` × N                  |
| Discard N cards                           | `<player> discards <card>` × N               |
| Deal N damage to target X                 | `<source> deals N damage to <target>`        |
| Counter placed/removed                    | `<card> gets/loses <kind> counter`           |
| Life change                               | `<player> {gains,loses} N life`              |
| Zone change                               | `<card> moves from <zone> to <zone>`         |
| Trigger fires                             | `Trigger: <ability> on <source>`             |
| Mana produced                             | `<source> produces {<mana>}` (correct color) |

**Sentinel values, placeholder identifiers, or `Unknown(*)` strings
appearing in the log count as a BROKEN log even if the mechanical
state is right.** They mislead the next agent that reads the log.

### 2.3 Regression tests
Every fix or status change ships with two tests:

1. **Parser-shape unit test** — `test_card_compat_<name>` in
   `mtg-engine/src/game/actions/tests/effects.rs`. Asserts the
   structural pieces (cost, P/T, types, keywords, parsed
   triggers/abilities). Cheap, fast, runs in `make validate`.
2. **End-to-end puzzle/shell test** — either a `.pzl` driven by a
   `tests/test_card_<name>.sh` shell harness, or an inline Rust
   integration test that uses the `tui` binary. Asserts the **log**
   contents (use `grep` / `assert_log_contains!`). E2E test names go
   into `Cargo.toml` and are picked up by `make validate`.

Both tests must reference the beads issue ID in a comment so future
greppers can find them.

---

## 3. Status classification

Apply exactly one of these tags in the issue title/footer, and in
`docs/EFFECT_SUPPORT.md`:

- **WORKING** — every printed ability resolves correctly *and*
  produces correct log output. Includes correct interaction with at
  least one other card that triggers off it (where applicable).
- **PARTIAL** — vanilla / static side works, but at least one printed
  ability is silently dropped, mis-parsed, or requires an engine
  feature that doesn't exist. Card is functional but strictly weaker
  (or stronger) than printed. Must reference a separate `Bug:` beads
  issue for the missing piece.
- **BROKEN** — card cannot be cast, crashes, or produces incorrect
  state for its primary printed ability.

A card is never "WORKING" until §2.2 game-log evidence exists in the
issue. A passing parser unit test alone is **PARTIAL** at best.

---

## 4. Common bug patterns to look for

These have all bitten us; look for them explicitly:

- **Silent parser drops** — a `T:`, `S:`, `K:`, or cost line is
  ignored because the parser only matches a narrow shape. Examples:
  `ChangesZone` parser only matching `ValidCard$ Card.Self`, missing
  `Creature.DamagedBy`; `SubCounter` cost type missing from cost
  parser; `StaticAbility` `IsPresent$`/`Threshold` conditions
  dropped; `Enchant` description text discarded.
- **Effect converter hardcoding** — e.g. `Produced$ Any` collapsed to
  colorless instead of an actual color choice. Search `params_to_effect`
  for `match` arms with TODO-shaped fallbacks.
- **Missing engine features** — trigger parses but no engine state
  feeds it (e.g. `damaged_by` set never populated, `DamagedCreatureDies`
  trigger has no firing site).
- **Game-log gaps** — effect resolves but emits no log line, or emits
  a sentinel like `Fixed1` / `Unknown(0)` / placeholder card name.
- **`contains()` on script bodies** — see CLAUDE.md "NO HACKY STRING
  OPERATIONS ON STRUCTURED DATA". Always use
  `AbilityParams::parse()` then map lookup.

When you find one of these, file a separate `Bug:` beads issue
describing the parser/engine gap, and reference it from the per-card
issue and from `docs/EFFECT_SUPPORT.md`. Do not bury the bug
description inside the card's issue alone — a single bug typically
affects many cards.

---

## 5. Per-card beads issue structure

Create with `bd create` (never duplicate; search first with
`bd list --status open` filtered by `Card Compatibility:`).

Title: `Card Compatibility: <Card Name>`

Description template:

```
Test all behavioral aspects of <NAME> in MTG Forge-rs.

Card: cardsfolder/<x>/<name>.txt
Set: <CODE> (<set tracking issue>)
Deck: <deck name> (<deck tracking issue>)             # if applicable
Test puzzle: test_puzzles/<name>_<scenario>.pzl       # if applicable

Card text:
  <printed mana cost> <P/T> <types - subtypes>
  <oracle text, line-by-line>

Findings (YYYY-MM-DD_#<gitdepth>(<short>), <author>):

1. [x] Parses as ... cost {...}
2. [x] Has Keyword::<X>
3. [BROKEN] <ability> silently dropped:
   - The script line is '<exact line>'
   - <root cause: which parser branch / which engine gap>
   - Affects all <pattern> cards.
   Filed as: <bug-issue-id>
4. [unverified] <thing we couldn't test>

Reproducer:

```sh
./target/release/mtg tui --start-state ... --p1=... \
  --p1-fixed-inputs='...' --seed 42 --verbosity 3 --json
```

**FORMATTING RULE (mandatory): put every reproducer in a fenced
` ```sh ` code block, flush-left (no leading indentation).** Indented
reproducers are not copy-pasteable, and — critically — if the
reproducer contains a `cat <<EOF` heredoc that writes a `.pzl`/`.dck`,
indenting the body injects leading spaces into the generated file and
breaks the parser. Use a quoted, non-indented heredoc delimiter
(`<<'P'` … `P`) so the emitted file is byte-correct when pasted.

**MECHANICAL-VERIFIABILITY RULE (mandatory): every reproducer command
MUST be paired with a 1–3 line snippet of the expected stdout** — the
specific log lines that prove the behavior. This turns each reproducer
into a runnable check: *run the command, grep for these lines.* Put the
expected lines in the adjacent fenced block below; keep them to the few
exact lines a verifier (human or agent) should match, not a full dump.

Expected log evidence (mandatory for WORKING):

```
<quoted log snippet showing draw/damage/counter/etc.>
```

Unit test:    test_card_compat_<name> in mtg-engine/src/game/actions/tests/effects.rs
E2E test:     tests/test_card_<name>.sh   (or named #[test] fn)

CARD STATUS: WORKING | PARTIAL | BROKEN — <one-line summary>
```

Use `YYYY-MM-DD_#<depth>(<commit>)` per CLAUDE.md beads conventions
for the dated header. All content goes in the description field
(never `--notes`).

When status changes, **append** a new dated `Findings (...)` block;
don't rewrite history. Update the trailing `CARD STATUS:` line.

---

## 6. Per-effect support matrix

File: `docs/EFFECT_SUPPORT.md` (create if absent — see §6.1).

This is the second tracking artifact and the one most often forgotten.
A per-card issue tells you "does Sengir Vampire work"; the support
matrix tells you "does the `ChangesZone | Creature.DamagedBy`
trigger pattern work, and which other cards are blocked on it".

### 6.1 Required sections

- **Keywords** — one row per keyword (Flying, First Strike,
  Protection from X, Regenerate, Vanishing N, Living Weapon, …).
- **Triggers (T:)** — rows keyed by `Mode$ <mode>` plus the
  discriminating `Valid*` qualifier (e.g.
  `ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Creature.DamagedBy`).
- **Activated abilities (A:)** — rows keyed by `AB$ <ApiType>` and
  any unusual cost shape (e.g. `Cost$ SubCounter<ChargeCounter>`).
- **Static abilities (S:)** — rows keyed by `Mode$` plus
  conditional qualifiers (`IsPresent$`, `Threshold$`, …).
- **Replacement effects (R:)** — keyed by `Event$`.
- **Mana production** — `Produced$` color/any/conditional.
- **SVar / cost / selector primitives** — one row per
  `$`-parameter we have to interpret (`Affected$`, `ValidTgts$`,
  `Defined$`, …).

### 6.2 Row format

```
| Construct                                          | Status   | Last verified            | Bug issue        | Sample cards |
| ChangesZone B→Gy ValidCard$ Creature.DamagedBy     | BROKEN   | 2026-05-12_#NNNN(d1c581)  | mtg-f0bfb8       | Sengir Vampire |
| Cost$ SubCounter<ChargeCounter>                    | WORKING  | 2026-05-12_#NNNN(901d85)  | (fixed)          | Triskelion     |
| Produced$ Any                                      | WORKING  | 2026-05-12_#NNNN(c808ce)  | (fixed)          | City of Brass  |
| StaticAbility IsPresent$ <selector>                | BROKEN   | 2026-05-12_#NNNN(...)     | mtg-XXXXXX       | (multiple)     |
| StaticAbility Threshold$                           | BROKEN   | 2026-05-12_#NNNN(...)     | mtg-XXXXXX       | (multiple)     |
| K:Protection from <color>                          | WORKING  | 2026-05-12_#NNNN(...)     | (none)           | Black Knight   |
| K:First Strike                                     | WORKING  | 2026-05-12_#NNNN(...)     | (none)           | Black Knight   |
| K:Regenerate                                       | WORKING  | 2026-05-12_#NNNN(...)     | (none)           | Sedge Troll    |
```

Use the same `WORKING / PARTIAL / BROKEN` vocabulary as cards.
"Last verified" uses the CLAUDE.md timestamp format. "Sample cards"
points back to representative `Card Compatibility:` issues so the
matrix stays cross-linked.

When a card test discovers a new construct, **append a row** in the
same commit as the per-card issue update. When a fix lands, flip the
status, bump the timestamp, and clear the bug-issue reference. Never
delete rows — historical "BROKEN → WORKING" transitions are valuable
context for future agents.

### 6.3 Anti-duplication rule

Before adding a new row, grep `docs/EFFECT_SUPPORT.md` and
`bd list --json --status open` for existing entries. The matrix is
small and append-only; duplicate rows for the same construct cause
silent drift between the matrix and beads.

---

## 7. Workflow summary (one card, end to end)

1. **Clean start.** `git status` clean, `make validate` green, on a
   feature branch off `integration` (or in an isolated worktree).
2. **Look up card** in `cardsfolder/`. Read the script line by line.
3. **Search beads** for an existing `Card Compatibility: <name>`
   issue. If absent, create it with the §5 template and the parser
   shape filled in.
4. **Write the parser-shape unit test** (§2.3.1). Run it.
5. **Build a reproducer** (puzzle / fixed-inputs) and play it
   through `mtg tui --verbosity 3`. Capture the log.
6. **Compare log to the §2.2 expected-evidence table.** Note every
   discrepancy.
7. **For each discrepancy:** identify root cause (parser drop /
   engine gap / log gap / converter hardcoding). File or update a
   `Bug:` beads issue.
8. **Fix what you can in scope.** Update the per-card issue with
   evidence and the new status; update `docs/EFFECT_SUPPORT.md` row
   for every construct whose status changed.
9. **Add an e2e test** that asserts the log contains the expected
   evidence (§2.3.2).
10. **Pre-commit:** `cargo fmt --all`, `cargo fmt --all -- --check`,
    `make validate`. Commit message follows CLAUDE.md template
    (Test Results Summary, Relationship to Java Forge, gameplay-log
    evidence section with the runnable reproducer command).
11. **Push** to the feature branch. Do not merge to `main` — let
    `ci-integration-monitor` promote `integration` → `main`.
12. **Close the per-card issue only when status is WORKING.**
    PARTIAL/BROKEN issues stay open referencing their bug issues.

---

## 8. Quick-reference checklist

Use this when you think you're done with a card:

- [ ] `Card Compatibility: <name>` beads issue exists with §5 template
- [ ] Parser-shape unit test exists and passes
- [ ] Reproducer command in the issue, copy-pasteable
- [ ] Game-log evidence quoted in the issue (no sentinels, no
      `Fixed1`/`Unknown(*)` placeholders)
- [ ] E2E test asserting the log evidence exists and runs in
      `make validate`
- [ ] Every script-line construct has a row in
      `docs/EFFECT_SUPPORT.md` with current status
- [ ] Every BROKEN/PARTIAL discrepancy has a separate `Bug:` beads
      issue, referenced from both the per-card issue and the support
      matrix
- [ ] CARD STATUS line in the per-card issue is one of
      WORKING/PARTIAL/BROKEN
- [ ] Commit message has Test Results Summary, Relationship to Java
      Forge, and gameplay-log evidence
- [ ] `make validate` green; pushed to feature branch (not `main`)
