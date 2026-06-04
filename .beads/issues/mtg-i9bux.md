---
title: 'Battlefield layout engine: review Rust->GUI dataflow + first-principles redesign (justification/whitespace flakiness, aspect-ratio-agnostic, tapped-card distortion, attached-card stacking)'
status: open
priority: 2
issue_type: task
created_at: 2026-06-04T00:54:32.323994956+00:00
updated_at: 2026-06-04T00:54:32.323994956+00:00
---

# Description

USER-REQUESTED 2026-06-03. DESIGN REVIEW + FIRST-PRINCIPLES REDESIGN. Investigate first — the user wants to REVIEW the current logic before any redesign. Near-term deliverable is a review DOC (ai_docs/), NOT immediate code changes.

MOTIVATION: the battlefield layout is "a mess" and "very flaky." Concrete complaints from a web-GUI session:
- Card sections on the battlefield are sometimes CENTERED, sometimes LEFT-JUSTIFIED (inconsistent across sections/states).
- Sections sometimes FILL the battlefield, other times leave a lot of WHITESPACE.
- Attached-card visual STACKING is missing entirely (see the enchant-land render bug [[bug issue]] — auras/equipment not shown on their host).
- ASPECT RATIO is mishandled: the native GUI puts Name and P/T UNDER the card (which may or may not conserve the card's aspect ratio); 90deg-TAPPED cards in the WEB view do NOT retain the aspect ratio of upright cards — they distort roughly into squares (the outer outline becomes square).

INVESTIGATE (read-only; for the user's review):
1. How is the battlefield layout currently PASSED from Rust code to the native/web GUI? Map the full dataflow: the Rust layout/view producer (GameStateView / FancyTuiRenderer / whatever emits zone+section+position info) -> serialization/protocol -> the GUI consumer (native_game.html / tui_game.html JS). Cite files + the exact structs/fields. The user explicitly wants to review THIS logic.
2. Where is justification (center vs left) and fill-vs-whitespace decided, and WHY is it inconsistent across sections/states? Pin the code that makes the choice.
3. How are card dimensions + the Name/PT placement handled? Does the pipeline conserve card aspect ratio anywhere, or assume a fixed box? Where?
4. Why do tapped (90deg-rotated) cards distort to squares in the web view? (Almost certainly a bounding-box-vs-rotated-content sizing bug — rotating a w×h card needs an h×w box, not the same square box. Find the CSS/transform.)
5. Could/should the core layout engine be AGNOSTIC to the actual card aspect ratio (compute positions in abstract card-units, let the renderer apply the real w:h)?

DELIVERABLE: a design-review doc in ai_docs/ = (a) current Rust->GUI layout dataflow MAP (with file:line + struct/field citations), (b) catalog of the inconsistency SOURCES (justification, whitespace/fill, stacking gap, aspect ratio, tapped distortion), each tied to code, (c) a FIRST-PRINCIPLES REDESIGN proposal: aspect-ratio-agnostic positioning, one consistent justification/fill policy, an attached-card stacking model, tapped-card rotation that preserves aspect (w×h -> h×w box). Present trade-offs; this is for the user to review before any implementation is scheduled.

RELATED: [[bug issue]] (the concrete enchant-land render bug that triggered this); the web frontend layout notes in <RepoRoot>/CLAUDE.md (web/ section). NOT yet scheduled for implementation — review gate first.
