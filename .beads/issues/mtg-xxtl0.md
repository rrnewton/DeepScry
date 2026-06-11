---
title: 2000 Championship — broken-card root-cause backlog (B1-B11)
status: open
priority: 2
issue_type: task
depends_on:
  mtg-h558o: parent-child
  mtg-684: parent-child
created_at: 2026-06-11T04:22:51.925471860+00:00
updated_at: 2026-06-11T04:22:58.969801961+00:00
---

# Description

2000 World Championship compat sweep — broken/partial-card root-cause backlog. Compiled by agent compat-2000-survey (slot07) from a static script-vs-converter scan of all 72 unique cards + verbosity-3 AI-vs-AI game logs. Rolls up under the 2000 TRACK bead mtg-h558o (umbrella mtg-684).

STAMP: 2026-06-10_#3175(c6dbd34f)

NOTE: SURVEY-ONLY pass — NO engine/web source edited (a separate effects/actions refactor owns those files). All fixes below are DEFERRED until that refactor lands. Reproducer evidence in gitignored debug/compat2000/*.log on branch claude/compat-2000-survey. Several gaps are shared ENGINE features that also block other championship years (1994/2020/2025) — coordinate via mb claims + rebase before editing effect_converter.rs / the static-ability layer.

Decks: 01_finkel_tinker, 02_maher_tinker (mono-U Tinker), 03_vandelogt_replenish (UW Replenish), 04_labarre_chimera (GW combo).

== Prioritized backlog ==

B1 [PARTIAL, HIGHEST VALUE — DO FIRST] Metalworker
  Card: cardsfolder/m/metalworker.txt (4x in BOTH finkel + maher = the 1st & 2nd place decks)
  Script: A:AB$ Reveal | Cost$ T | RevealValid$ Card.Artifact+YouCtrl | AnyNumber$ True | RememberRevealed$ True | SubAbility$ DBMetalWorkerMana ; SVar:DBMetalWorkerMana:DB$ Mana | Produced$ C | Amount$ MetalWorkerX ; SVar:MetalWorkerX:Remembered$Amount/Twice
  Root cause: ApiType::Reveal parses (ability_parser.rs:387 "Reveal"->Self::Reveal) but has NO arm in effect_converter.rs -> the whole ability head becomes Effect::Unimplemented, taking its Mana sub-ability with it. Metalworker is therefore a vanilla 1/2 with a dead {T} ability.
  Empirical: across 4x200 tourney games + 5 verbosity-3 games, Metalworker entered the battlefield (16x) and ATTACKED but its ability was NEVER offered/activated; no Unimplemented warning (it never resolves because it is never a usable action). This is the explosive-mana engine that powers Tinker into Phyrexian Colossus turn 2-3; without it both top decks are crippled vs their real power level.
  Fix shape: add an ApiType::Reveal converter arm that (a) reveals N matching cards from hand, (b) remembers the count, so the chained DB$ Mana | Amount$ Remembered/Twice produces {C}{C} per revealed artifact. Generalizes to other "reveal X, then do Y per X" cards.

B2 [PARTIAL] Tangle Wire
  Card: cardsfolder/t/tangle_wire.txt (4x finkel + maher)
  Script: T:Mode$ Phase | Phase$ Upkeep ... Execute$ TrigChooseToTap ; SVar:TrigChooseToTap:DB$ ChooseCard | Defined$ TriggeredPlayer | Choices$ ... | Amount$ X (=fade counters) | SubAbility$ DBTap ; K:Fading:4
  Root cause: ApiType::ChooseCard has NO converter arm. The upkeep Phase trigger fires but its forced-tap chain (ChooseCard -> Tap) is Unimplemented, so the per-fade-counter "tap an artifact/creature/land you control" never happens. Fading:4 counter tracking + the sac-when-empty parse correctly.
  Impact: Tangle Wire is a tempo/lock piece in the Tinker decks; without the forced-tap it does nothing but sit and fade. PARTIAL.
  Fix shape: ChooseCard converter arm (choose N permanents matching a filter) + ensure the Tap sub-ability runs on the chosen cards.

B3 [PARTIAL] Phyrexian Processor
  Card: cardsfolder/p/phyrexian_processor.txt (4x finkel + maher main; 1x chimera SB)
  Script: R:Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield | ReplaceWith$ PayLife ; SVar:PayLife:AB$ StoreSVar | Cost$ Mandatory PayLife<X> | SVar$ LifePaidOnETB ... ; A:AB$ Token | TokenPower$ LifePaidOnETB | TokenToughness$ LifePaidOnETB ; SVar:LifePaidOnETB:Number$0
  Root cause: ApiType StoreSVar is an explicit Effect::NoOp (effect_converter.rs:1843 — "no runtime SVar store"). The ETB "pay any amount of life" replacement stores nothing, so LifePaidOnETB stays 0 and the {4},{T} token ability always makes a 0/0 Phyrexian Minion (which dies immediately).
  Empirical: Processor cast+resolved in debug/compat2000/v3_finkel_s3.log (line 491) with NO "pay life" prompt and no life change logged at ETB.
  Fix shape: needs (a) an ETB "pay any amount of life" replacement that solicits the amount, and (b) a runtime SVar store so the token reads the paid amount. Bigger feature; lower priority than B1/B5 (Processor is a slower plan).

B4 [BROKEN] Serra Avatar
  Card: cardsfolder/s/serra_avatar.txt (1x chimera main)
  Script: PT:*/* ; S:Mode$ Continuous | CharacteristicDefining$ True | SetPower$ X | SetToughness$ X (X=Count$YourLifeTotal)
  Root cause: continuous CharacteristicDefining SetPower/SetToughness static is NOT in the StaticAbility runtime enum (only ModifyPT/GrantKeyword/ReduceCost/RaiseCost/GrantAbility/GainControl/... exist). SetPower is only handled inside CopyPermanent (effect_converter.rs:1034), not as a standalone continuous CDA.
  Empirical (CONFIRMED): debug/compat2000/v3_chimera.log — "Serra Avatar (41) enters the battlefield as a 0/0 creature" then "Serra Avatar (41) dies from lethal damage" same step. Its shuffle-into-library-on-death trigger (ChangeZone) also did not log (likely fired against an already-gone 0/0).
  Fix shape: implement continuous CharacteristicDefining P/T as a real layer-7b CDA static. Shared with B5.

B5 [BROKEN, HIGH VALUE] Opalescence
  Card: cardsfolder/o/opalescence.txt (4x vandelogt_replenish main; 1x chimera SB)
  Script: S:Mode$ Continuous | Affected$ Enchantment.nonAura+Other | SetPower$ AffectedX | SetToughness$ AffectedX | AddType$ Creature (AffectedX=Count$CardManaCost)
  Root cause: same continuous SetPower/SetToughness gap as B4, PLUS continuous AddType$ Creature unsupported. Other non-Aura enchantments never become creatures with P/T = mana value.
  Impact: Opalescence is the WIN CONDITION of the Replenish deck (Replenish returns a board of enchantments; Opalescence turns them into a lethal creature team). Without it the deck assembles its combo and deals no damage. Empirical: Replenish itself WORKS (moves all enchantments Gy->Battlefield, debug/compat2000/v3_replenish.log) but Opalescence never reached the battlefield in the heuristic games; the static gap is confirmed by enum inspection.
  Fix shape: continuous CDA P/T (shared B4) + continuous AddType layer-4 type-adding static.

B6 [PARTIAL/inert] Cursed Totem
  Card: cardsfolder/c/cursed_totem.txt (1x replenish SB)
  Script: S:Mode$ CantBeActivated | ValidCard$ Creature | ValidSA$ Activated
  Root cause: CantBeActivated static mode unsupported (not in StaticAbility runtime enum). Creatures' activated abilities are NOT blocked. Inert hate-piece. Fix: add a CantBeActivated activation-legality static check.

B7 [PARTIAL/inert] Light of Day
  Card: cardsfolder/l/light_of_day.txt (1x chimera SB)
  Script: S:Mode$ CantAttack,CantBlock | ValidCard$ Creature.Black
  Root cause: CantAttack/CantBlock parse into StaticAbilityMode (svar_parser.rs) but are NOT in the StaticAbility runtime enum -> black creatures still attack/block. Inert. Fix: wire CantAttack/CantBlock into the declare-attackers/blockers legality checks.

B8 [PARTIAL/inert] Energy Flux
  Card: cardsfolder/e/energy_flux.txt (1x chimera SB)
  Script: S:Mode$ Continuous | Affected$ Artifact | AddTrigger$ UpkeepCostTrigger ("At the beginning of your upkeep, sacrifice this artifact unless you pay {2}")
  Root cause: continuous AddTrigger (grant a trigger to a set of permanents) unsupported. Artifacts get no upkeep tax. Inert vs an artifact deck. Fix: continuous trigger-granting static.

B9 [PARTIAL] Crumbling Sanctuary
  Card: cardsfolder/c/crumbling_sanctuary.txt (1x finkel + maher main)
  Script: R:Event$ DamageDone | ValidTarget$ Player | ReplaceWith$ ExileTop (DB$ Dig | DigNum$ X=ReplaceCount$DamageAmount | DestinationZone$ Exile)
  Root cause: the loader's structural R: classifier (loader/card.rs) recognizes only narrow shapes (ETB-tapped, CantHappen-untap, RPrevent damage-prevention). A DamageDone->exile-from-library replacement is not among them, so the replacement is dropped — players take normal damage instead of milling-to-exile. PARTIAL. Fix: general damage-replacement-redirect category (overlaps the 2020 Torbran damage-modification gap, mtg-902 B1).

B10 [PARTIAL] Worship
  Card: cardsfolder/w/worship.txt (1x chimera SB)
  Script: R:Event$ LifeReduced | ValidPlayer$ You.lifeGE1 | Result$ LT1 | IsDamage$ True | IsPresent$ Creature.YouCtrl | ReplaceWith$ ReduceLoss
  Root cause: LifeReduced "can't go below 1 while you control a creature" replacement shape unrecognized by the R: classifier. You can be reduced below 1 normally. PARTIAL. Fix: life-loss-floor replacement (similar shape to B9's damage replacement layer).

B11 [VERIFY] Meekstone
  Card: cardsfolder/m/meekstone.txt (1x chimera SB)
  Script: R:Event$ Untap | ValidCard$ Creature.powerGE3 | Layer$ CantHappen
  Status: CantHappen-untap is one of the recognized R: shapes in loader/card.rs (used by tap-down lock cards) — Meekstone MAY already work, but it was never exercised in the heuristic games. Needs a targeted puzzle: put a 3+ power creature + Meekstone on the battlefield, tap the creature, pass to untap step, assert it stays tapped. Classify after confirmation.

== Cards that are WORKING (evidence-backed this pass — NOT in backlog) ==
Tinker, Masticore, Grim Monolith, Thran Dynamo, Replenish, Frantic Search, Enlightened Tutor, Mystical Tutor, Counterspell, Birds of Paradise, Llanowar Elves, Priest of Titania, Metalworker-BODY (1/2 attacks; ability is B1), basic lands. Logs: debug/compat2000/v3_{finkel,replenish,chimera}.log.

== Cards LIKELY-FINE but UNVALIDATED (constructs IMPL; no per-card game-log this pass — next agent should puzzle-verify) ==
Most mana rocks/lands (Crystal Vein, Heart of Ramos, Sky Diamond, Saprazzan Skerry, Phyrexian Tower, Thran Quarry, High Market, Brushland, City of Brass, Adarkar Wastes, Rishadan Port pacifism-tap), Voltaic Key (Untap), Phyrexian Colossus (Untap upkeep), Brainstorm (Draw+ChangeZone), Armageddon/Wrath of God (DestroyAll), Yawgmoth's Will & Yawgmoth's Bargain (Effect/Draw), Annul/Daze/Miscalculation (Counter), Blaze (X DealDamage), Aura Fracture/Seal of Cleansing/Seal of Removal/Erase (Destroy/ChangeZone), Fecundity (death-draw trigger), Attunement (Draw+Discard), Confiscate (GainControl continuous — supported), Absolute Law (AddKeyword->GrantKeyword — supported), Pattern of Rebirth dies-search TRIGGER (AddSVar rider is B8-class but the search itself is ChangeZone), Academy Rector (death-tutor ChangeZone), Saproling Cluster/Snake Basket (Token), Whetstone (Mill), Ashnod's Altar (sac-for-mana), Rising Waters (Untap-lock), Parallax Wave/Tide (Fading + ChangeZone removal — counter-removal exile/return; VERIFY the per-counter ChangeZone actually fires, related to B2 ChooseCard class), Chill/Defense Grid (RaiseCost — supported), Mishra's Helix (Tap), Lilting Refrain (Counter+PutCounter), Circle of Protection: Black (ChooseSource+Effect prevention), Energy Field (DamageDone prevent + gy-sac trigger — partial, prevention may work), Cursed... etc. Headline: evidence-backed WORKING 15/72 (21%); estimated functional (WORKING+likely-fine) ~59/72 (~82%); KNOWN broken/partial 13/72 (~18%, B1-B11 + the two cards each bug hits).

Driven by agent compat-2000-survey (slot07), 2026-06-10 overnight survey pass.
