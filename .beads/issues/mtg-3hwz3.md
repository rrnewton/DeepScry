---
title: 'Bug: setARN set-origin matching + Mode$ Always state-trigger + CantBeCast/CantPlayLand statics unimplemented'
status: open
priority: 2
issue_type: task
created_at: 2026-05-31T01:49:00.357649793+00:00
updated_at: 2026-05-31T09:34:16.292142317+00:00
---

# Description

City in a Bottle (Arabian Nights, ARN). Implemented as THREE general, reusable constructs (not a City hack). FIXED @2026-05-31_#2540(ad30b333), fix-mtg-3hwz3-city-in-a-bottle.

Card: cardsfolder/c/city_in_a_bottle.txt — {2} Artifact.
Script:
  T:Mode$ Always | IsPresent$ Permanent.!token+setARN+Other | Execute$ TrigSac
  SVar:TrigSac:DB$ SacrificeAll | ValidCards$ Permanent.!token+setARN+Other
  S:Mode$ CantPlayLand | ValidCard$ Card.setARN
  S:Mode$ CantBeCast | ValidCard$ Card.setARN
Current (errata) Oracle: "Whenever one or more other nontoken permanents [originally printed in ARN] are on the battlefield, their controllers sacrifice them. Players can't cast spells or play lands [printed in ARN]." (Note: errata is a state trigger covering lands too, not the old "nonland permanents" ETB wording.)

CONSTRUCT 1 — set-origin filter (general):
- New strong type core::SetCode (normalized uppercase, serde, interned Arc<str>).
- CardDefinition gains serialized `origin_set: Option<SetCode>` = card's EARLIEST printing.
  Stamped post-parse: native via CardDatabase (loads CardEditionIndex from the
  editions/ dir resolved as a sibling of cardsfolder); WASM at per-set bin export
  time from PrimarySetAssignment. Both derive from the same edition data ->
  deterministic & identical. CardEditionIndex::get_origin_set (earliest year,
  lex tie-break) mirrors PrimarySetAssignment.
- Card::origin_set()/is_from_set() accessors.
- TargetRestriction (the valid/filter matcher) gains required_set + requires_other.
  parse() recognizes `set<CODE>` (e.g. setARN) and `Other`. matches() enforces
  set; new matches_excluding(card, source) enforces `Other` self-exclusion.
  Lifts EVERY set-referencing card, not just City.

CONSTRUCT 2 — continuous "destroy any that's present/enters" + ETB sweep (general):
- T:Mode$ Always (with IsPresent$ + Execute$ SacrificeAll/DestroyAll) parses into
  new StaticAbility::SacrificeMatchingPresent { restriction }. Filter taken from
  the Execute$ SVar's ValidCards$ (authoritative), fallback IsPresent$.
- New SBA-like sweep GameState::check_set_origin_sacrifice(): for each sweeper on
  the battlefield, every OTHER battlefield permanent matching its filter is moved
  to its owner's graveyard. One rule covers both the on-enter sweep AND
  destroy-any-that-enters-afterward (re-run every SBA pass / before priority is
  granted, CR 603.8 / 704.3). Wired at the 3 post-resolution SBA sites + once
  before each priority grant (so a quiescent/puzzle board is covered).
- NO new game state: derived from already-serialized static abilities + origin_set.

CONSTRUCT 3 — cast/play-restriction statics (general):
- New StaticAbility::CantBeCast { valid_card } and CantPlayLand { valid_card },
  parsed from S:Mode$ CantBeCast / CantPlayLand (tokenized, no substring match).
- GameState::is_cast_prohibited / is_land_play_prohibited / is_play_prohibited.
- get_available_spell_abilities skips prohibited spells AND prohibited lands, so a
  prohibited card is never offered as a legal play (CR 605). General color/set/
  type-hoser machinery.

EVIDENCE (real game log, mtg tui --start-state test_puzzles/city_in_a_bottle_arn_hoser.pzl --p1 zero --p2 zero --seed 42 --verbosity 3):
  Camel (5) goes to graveyard
  Camel (5) is sacrificed (originally-printed-set hoser)
  Player 1 declares Grizzly Bears (6) (2/2) as attacker     <- non-ARN survives & attacks
  (City in a Bottle (4) stays on battlefield; the held Camel in hand is never played)

Tests:
- Parser-shape unit: test_card_compat_city_in_a_bottle (mtg-engine/src/game/actions/tests/effects.rs) — asserts all 3 statics parse with setARN/!token/Other and origin_set stamping (ARN vs non-ARN).
- E2E: test_city_in_a_bottle_arn_hoser (mtg-engine/tests/puzzle_e2e.rs) + test_puzzles/city_in_a_bottle_arn_hoser.pzl — ETB sweep, destroy-on-enter afterward, unplayable ARN card, non-ARN survival, Other self-exclusion.

Determinism: origin_set serialized in CardDefinition (native CardDatabase + WASM export both stamp from edition data); SacrificeMatchingPresent derived from serialized statics; sweep iterates battlefield in stable order; no un-serialized side state.

Relationship to Java Forge: Forge models the Mode$ Always state trigger as a continuously-checked state trigger; this Rust port applies it as a deterministic SBA-like sweep at the same checkpoints, which is equivalent for this card and keeps the engine controller-agnostic.

CARD STATUS: WORKING — all three constructs verified with game-log + unit + e2e evidence.
