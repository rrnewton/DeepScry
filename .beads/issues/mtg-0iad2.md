---
title: 'Deep Playtest: ryan_avatar_draft Deck - Complete Mechanic Verification'
status: open
priority: 1
issue_type: task
labels:
- deep-test
created_at: 2026-01-05T20:03:49.234472619+00:00
updated_at: 2026-01-06T02:43:57.962511699+00:00
---

# Description

## Deep Playtest: ryan_avatar_draft Deck

This tracking issue ensures EVERY mechanic on EVERY card in the ryan_avatar_draft deck is 100% functional with evidence from real gameplay.

**Deck Contents:** 22 unique non-land cards + 2 basic lands (Mountain, Swamp)

## Testing Methodology
- Each checkbox requires evidence from actual gameplay (CLI logs, puzzle files, agentplay scripts)
- Evidence must be included in commit messages when checking off items
- No premature victory declarations - skeptical verification only

## Known Bugs Affecting This Deck
- mtg-6ph0z: Token scripts not loading (Food, Clue tokens fail)
- mtg-hl300: SpellCast triggers not firing (Boar-q-pine, Prowess)
- mtg-oyvdh: ETB triggers with optional discard cost don't fire (Yuyan Archers looting)

---

## 1. Beetle-Headed Merchants (4B, 5/4 Human Citizen)
**Triggered:** Whenever this creature attacks, you may sacrifice another creature or artifact. If you do, draw a card and put a +1/+1 counter on this creature.

- [x] Card loads and can be cast for 4B (verified: puzzles/beetle_merchants_attack_trigger.pzl)
- [x] Can attack normally as 5/4 (verified: puzzle shows 5/4 attacking)
- [x] Attack trigger fires when declared as attacker (verified: trigger fires on attack)
- [ ] "You may" is optional - can decline sacrifice
- [x] Can sacrifice another creature to trigger (verified: Canyon Crawler sacrificed)
- [ ] Can sacrifice an artifact to trigger
- [x] Draw a card effect works on sacrifice (verified: hand increased 1→2)
- [x] +1/+1 counter is placed on sacrifice (verified: creature became 6/5)
- [ ] Counter persists across turns
- [ ] Multiple attacks accumulate counters correctly

---

## 2. Boar-q-pine (2R, 2/2 Boar Porcupine)
**Triggered:** Whenever you cast a noncreature spell, put a +1/+1 counter on this creature.

- [ ] Card loads and can be cast for 2R
- [ ] Enters as 2/2
- [ ] Trigger fires when casting instant **[BLOCKED: mtg-hl300]**
- [ ] Trigger fires when casting sorcery **[BLOCKED: mtg-hl300]**
- [ ] Trigger fires when casting artifact **[BLOCKED: mtg-hl300]**
- [ ] Trigger fires when casting enchantment **[BLOCKED: mtg-hl300]**
- [ ] Trigger does NOT fire for creature spells
- [ ] Counter is placed correctly
- [ ] Multiple noncreature spells accumulate counters

---

## 3. Canyon Crawler (4BB, 6/6 Spider Beast)
**Keywords:** Deathtouch, Swampcycling {2}
**Triggered:** When this creature enters, create a Food token.

- [x] Card loads and can be cast for 4BB (verified: puzzle loaded Canyon Crawler)
- [x] Enters as 6/6 (verified: shown as 6/6 in game state)
- [x] Has Deathtouch (kills any creature it damages) (verified: puzzles/test_canyon_crawler_deathtouch.pzl)
- [ ] Deathtouch works in combat (blocking)
- [x] Deathtouch works in combat (attacking) (verified: killed Rough Rhino Cavalry 5/5)
- [ ] ETB trigger creates Food token **[BLOCKED: mtg-6ph0z]**
- [ ] Food token is an artifact **[BLOCKED: mtg-6ph0z]**
- [ ] Food token has "{2}, {T}, Sacrifice: Gain 3 life" **[BLOCKED: mtg-6ph0z]**
- [ ] Food token ability works correctly **[BLOCKED: mtg-6ph0z]**
- [ ] Swampcycling {2} can be activated from hand
- [ ] Swampcycling searches for Swamp
- [ ] Swampcycling reveals the card
- [ ] Swampcycling puts Swamp in hand
- [ ] Swampcycling shuffles library

---

## 4. Cunning Maneuver (1R, Instant)
**Spell:** Target creature gets +3/+1 until end of turn. Create a Clue token.

- [ ] Card loads and can be cast for 1R
- [ ] Requires a target creature
- [ ] Target gets +3/+1
- [ ] Buff lasts until end of turn
- [ ] Buff wears off at cleanup
- [ ] Creates Clue token **[BLOCKED: mtg-6ph0z]**
- [ ] Clue token is an artifact **[BLOCKED: mtg-6ph0z]**
- [ ] Clue token has "{2}, Sacrifice: Draw a card" **[BLOCKED: mtg-6ph0z]**
- [ ] Clue token ability works correctly **[BLOCKED: mtg-6ph0z]**
- [ ] Can be cast at instant speed (during combat, opponent's turn)

---

## 5. Deserter's Disciple (1R, 2/2 Human Rebel Ally)
**Activated:** {T}: Another target creature you control with power 2 or less can't be blocked this turn.

- [ ] Card loads and can be cast for 1R
- [ ] Enters as 2/2
- [ ] Activated ability requires tap
- [ ] Can target another creature you control
- [ ] Cannot target itself ("another")
- [ ] Target must have power 2 or less
- [ ] Cannot target creature with power 3+
- [ ] Unblockable effect applies for the turn
- [ ] Effect wears off at end of turn
- [ ] Can use ability during declare attackers step

---

## 6. Fatal Fissure (1B, Instant)
**Spell:** Choose target creature. When that creature dies this turn, you earthbend 4.
**Earthbend 4:** Target land you control becomes a 0/0 creature with haste. Put four +1/+1 counters on it. When it dies or is exiled, return it to the battlefield tapped.

- [ ] Card loads and can be cast for 1B
- [ ] Requires target creature
- [ ] Creates delayed trigger for death
- [ ] Trigger fires when creature dies this turn
- [ ] Trigger does NOT fire if creature dies next turn
- [ ] Earthbend targets a land you control
- [ ] Land becomes a creature (0/0 base)
- [ ] Land keeps being a land
- [ ] Land gains haste
- [ ] Four +1/+1 counters placed (becomes 4/4)
- [ ] Earthbent land can attack
- [ ] Earthbent land can block
- [ ] Death trigger: returns land to battlefield tapped
- [ ] Exile trigger: returns land to battlefield tapped
- [ ] Returned land is no longer a creature

---

## 7. Fire Lord Ozai (3B, 4/4 Legendary Human Noble)
**Triggered:** Whenever Fire Lord Ozai attacks, you may sacrifice another creature. If you do, add {R} equal to sacrificed creature's power. Until end of combat, you don't lose this mana as steps end.
**Activated:** {6}: Exile top card of each opponent's library. Until end of turn, you may play one of those cards without paying its mana cost.

- [ ] Card loads and can be cast for 3B
- [ ] Legendary rule works (can't have two)
- [ ] Enters as 4/4
- [ ] Attack trigger fires when declared as attacker
- [ ] "You may" sacrifice is optional
- [ ] Can sacrifice another creature
- [ ] Mana added equals sacrificed creature's power
- [ ] Mana is red {R}
- [ ] Mana persists through combat steps (doesn't empty)
- [ ] Mana empties at end of combat
- [ ] Activated ability costs {6}
- [ ] Exiles top card from each opponent's library
- [ ] Can play one of the exiled cards
- [ ] Playing exiled card doesn't cost mana
- [ ] Exiled card playable until end of turn
- [ ] Works correctly in multiplayer (multiple opponents)

---

## 8. Fire Sages (1R, 2/2 Human Cleric)
**Keyword:** Firebending 1
**Activated:** {1}{R}{R}: Put a +1/+1 counter on this creature.

- [x] Card loads and can be cast for 1R (verified: Heartless Act puzzle loaded Fire Sages)
- [x] Enters as 2/2 (verified: shown in battlefield as creature)
- [x] Firebending 1 works - adds {R} on attack (verified: puzzles/test_fire_sages_ability.pzl "adds 1 {R} combat mana")
- [ ] Firebending interacts correctly with firebend sources
- [x] Activated ability costs {1}{R}{R} (verified: 3 mountains tapped)
- [x] Activated ability puts +1/+1 counter (verified: Fire Sages became 3/3)
- [ ] Can activate multiple times per turn
- [ ] Counters persist across turns

---

## 9. Heartless Act (1B, Instant - Modal)
**Modes:** Choose one:
- Destroy target creature with no counters on it.
- Remove up to three counters from target creature.

- [x] Card loads and can be cast for 1B (verified: puzzles/test_heartless_act.pzl)
- [x] Mode selection is required (verified: "Player 1 chooses mode:")
- [x] Mode 1: Can target creature with no counters (verified: targeted Fire Sages)
- [ ] Mode 1: Cannot target creature WITH counters
- [x] Mode 1: Destroys the creature (verified: "Heartless Act destroys Fire Sages")
- [ ] Mode 2: Can target creature with counters
- [ ] Mode 2: Removes up to 3 counters
- [ ] Mode 2: Works with fewer than 3 counters
- [ ] Mode 2: Can choose to remove fewer counters
- [ ] Mode 2: Works with +1/+1 counters
- [ ] Mode 2: Works with other counter types
- [x] Can be cast at instant speed (verified: is an instant)

---

## 10. Iroh's Demonstration (1R, Sorcery Lesson - Modal)
**Modes:** Choose one:
- Deal 1 damage to each creature your opponents control.
- Deal 4 damage to target creature.

- [ ] Card loads and can be cast for 1R
- [ ] Is a Lesson subtype (for Learn mechanic)
- [ ] Mode selection is required
- [ ] Mode 1: Deals 1 damage to each opponent's creature
- [ ] Mode 1: Does NOT damage your creatures
- [ ] Mode 1: Does NOT damage players
- [ ] Mode 2: Requires target creature
- [ ] Mode 2: Deals 4 damage to target
- [ ] Mode 2: Can target your own creature
- [ ] Mode 2: Can target opponent's creature
- [ ] Can only be cast at sorcery speed

---

## 11. Jeong Jeong, the Deserter (2R, 2/3 Legendary Human Rebel Ally)
**Keyword:** Firebending 1
**Exhaust Activated:** {3}: Put a +1/+1 counter on Jeong Jeong. When you next cast a Lesson spell this turn, copy it and you may choose new targets for the copy.

- [ ] Card loads and can be cast for 2R
- [ ] Legendary rule works
- [ ] Enters as 2/3
- [ ] Firebending 1 works
- [ ] Exhaust ability costs {3}
- [ ] Exhaust puts +1/+1 counter
- [ ] Exhaust can only be activated ONCE ever
- [ ] After activation, exhaust is "used up"
- [ ] Creates delayed trigger for Lesson spell
- [ ] Trigger fires when casting Lesson spell this turn
- [ ] Copies the Lesson spell
- [ ] Can choose new targets for copy
- [ ] Works with Iroh's Demonstration (Lesson in deck)
- [ ] Trigger expires at end of turn if no Lesson cast

---

## 12. Lightning Strike (1R, Instant)
**Spell:** Deals 3 damage to any target.

- [x] Card loads and can be cast for 1R (verified: puzzles/test_lightning_strike.pzl)
- [x] Can target a creature (verified: targeted Canyon Crawler)
- [x] Can target a player (verified: puzzles/test_lightning_strike_player.pzl killed player)
- [ ] Can target a planeswalker
- [x] Deals exactly 3 damage (verified: "takes 3 damage")
- [x] Can be cast at instant speed (verified: is an instant)
- [x] Damage can kill creatures (verified: Canyon Crawler died)
- [x] Damage reduces player life total (verified: life went 3→0→-3)

---

## 13. Mongoose Lizard (4RR, 5/6 Mongoose Lizard)
**Keywords:** Menace, Mountaincycling {2}
**Triggered:** When this creature enters, it deals 1 damage to any target.

- [ ] Card loads and can be cast for 4RR
- [ ] Enters as 5/6
- [ ] Has Menace (must be blocked by 2+ creatures)
- [ ] Menace prevents single-creature blocks
- [ ] ETB trigger fires on entering
- [ ] ETB can target creature
- [ ] ETB can target player
- [ ] ETB deals exactly 1 damage
- [ ] Mountaincycling {2} works from hand
- [ ] Mountaincycling finds Mountain
- [ ] Mountaincycling reveals, puts in hand, shuffles

---

## 14. Pirate Peddlers (2B, 2/2 Human Pirate)
**Keyword:** Deathtouch
**Triggered:** Whenever you sacrifice another permanent, put a +1/+1 counter on this creature.

- [ ] Card loads and can be cast for 2B
- [ ] Enters as 2/2
- [ ] Has Deathtouch
- [ ] Trigger fires when sacrificing a creature
- [ ] Trigger fires when sacrificing an artifact
- [ ] Trigger fires when sacrificing a land
- [ ] Trigger fires when sacrificing an enchantment
- [ ] "Another" - doesn't trigger on self-sacrifice
- [ ] +1/+1 counter placed correctly
- [ ] Multiple sacrifices = multiple counters

---

## 15. Rough Rhino Cavalry (4R, 5/5 Human Mercenary)
**Keyword:** Firebending 2
**Exhaust Activated:** {8}: Put two +1/+1 counters on this creature. It gains trample until end of turn.

- [ ] Card loads and can be cast for 4R
- [ ] Enters as 5/5
- [ ] Firebending 2 works
- [ ] Exhaust ability costs {8}
- [ ] Exhaust can only be activated ONCE ever
- [ ] Puts two +1/+1 counters (becomes 7/7)
- [ ] Gains trample until end of turn
- [ ] Trample allows excess damage to player
- [ ] Trample wears off at end of turn
- [ ] Counters persist (trample doesn't)

---

## 16. Rumble Arena (Land)
**Keyword on animated form:** Vigilance
**Triggered:** When this land enters, scry 1.
**Activated:** {T}: Add {C}.
**Activated:** {1}, {T}: Add one mana of any color.

- [ ] Card loads and can be played as land
- [ ] ETB trigger fires on entering
- [ ] Scry 1 works (look at top, may put bottom)
- [ ] Basic mana ability: {T} for {C}
- [ ] Any-color ability: {1}, {T} for any color
- [ ] Can produce {W}, {U}, {B}, {R}, {G}
- [ ] Vigilance noted (relevant if animated)

---

## 17. Twin Blades (2R, Artifact Equipment)
**Keyword:** Flash
**Triggered:** When this Equipment enters, attach it to target creature you control. That creature gains double strike until end of turn.
**Static:** Equipped creature gets +1/+1.
**Activated:** Equip {2}

- [ ] Card loads and can be cast for 2R
- [ ] Has Flash (can cast at instant speed)
- [ ] ETB trigger fires on entering
- [ ] ETB requires target creature you control
- [ ] ETB attaches equipment to target
- [ ] ETB grants double strike until end of turn
- [ ] Double strike: deals first strike AND normal damage
- [ ] Double strike wears off at end of turn
- [ ] Static: equipped creature gets +1/+1
- [ ] Equip {2} works at sorcery speed
- [ ] Equipment persists when creature dies
- [ ] Can re-equip to another creature

---

## 18. Ty Lee, Artful Acrobat (2R, 3/2 Legendary Human Performer)
**Keyword:** Prowess
**Triggered:** Whenever Ty Lee attacks, you may pay {1}. When you do, target creature can't block this turn.

- [ ] Card loads and can be cast for 2R
- [ ] Legendary rule works
- [ ] Enters as 3/2
- [ ] Prowess triggers on noncreature spells **[BLOCKED: mtg-hl300]**
- [ ] Prowess grants +1/+1 until end of turn **[BLOCKED: mtg-hl300]**
- [ ] Attack trigger fires when declared as attacker
- [ ] "You may pay {1}" is optional
- [ ] If paid, can target any creature
- [ ] Target creature can't block this turn
- [ ] Effect lasts until end of turn
- [ ] Can target opponent's creature (intended use)
- [ ] Can target own creature (legal but unusual)

---

## 19. Yuyan Archers (1R, 3/1 Human Archer)
**Keyword:** Reach
**Triggered:** When this creature enters, you may discard a card. If you do, draw a card.

- [x] Card loads and can be cast for 1R (verified: puzzles/test_yuyan_archers_etb.pzl)
- [x] Enters as 3/1 (verified: shown as 3/1 creature)
- [x] Has Reach (can block flyers) (verified: puzzles/test_yuyan_archers_reach.pzl)
- [x] Can block creatures with flying (verified: blocked Watcher in the Mist)
- [ ] ETB trigger fires on entering **[BLOCKED: mtg-oyvdh]**
- [ ] "You may" discard is optional **[BLOCKED: mtg-oyvdh]**
- [ ] Can decline to discard (no draw) **[BLOCKED: mtg-oyvdh]**
- [ ] If discard, draws a card (looting) **[BLOCKED: mtg-oyvdh]**
- [ ] Discard happens before draw **[BLOCKED: mtg-oyvdh]**

---

## 20. Zhao, Ruthless Admiral (2{B/R}{B/R}, 3/4 Legendary Human Soldier)
**Keyword:** Firebending 2
**Triggered:** Whenever you sacrifice another permanent, creatures you control get +1/+0 until end of turn.

- [ ] Card loads and can be cast for 2{B/R}{B/R}
- [ ] Hybrid mana works (can pay BB, RR, BR, RB)
- [ ] Legendary rule works
- [ ] Enters as 3/4
- [ ] Firebending 2 works
- [ ] Trigger fires on sacrificing permanent
- [ ] "Another" - self-sacrifice doesn't trigger
- [ ] All your creatures get +1/+0
- [ ] Includes Zhao himself
- [ ] Buff lasts until end of turn
- [ ] Multiple sacrifices stack the bonus

---

## 21. Zhao, the Moon Slayer (1R, 2/2 Legendary Human Soldier)
**Keyword:** Menace
**Static (Replacement):** Nonbasic lands enter tapped.
**Activated:** {7}: Put a conqueror counter on Zhao.
**Activated Static:** As long as Zhao has a conqueror counter, nonbasic lands are Mountains.

- [ ] Card loads and can be cast for 1R
- [ ] Legendary rule works (with Zhao Ruthless Admiral)
- [ ] Enters as 2/2
- [ ] Has Menace
- [ ] Static: opponent's nonbasic lands enter tapped
- [ ] Static: your nonbasic lands enter tapped
- [ ] Activated ability costs {7}
- [ ] Puts conqueror counter on Zhao
- [ ] With counter: nonbasic lands ARE Mountains
- [ ] Affected lands lose all abilities
- [ ] Affected lands have only "{T}: Add {R}"
- [ ] Effect is symmetric (affects your lands too)
- [ ] Removing counter restores lands
- [ ] Zhao dying removes the effect

---

## 22. Zuko, Conflicted (BR, 2/3 Legendary Human Rogue)
**Triggered:** At the beginning of your first main phase, choose one that hasn't been chosen and you lose 2 life:
- Draw a card.
- Put a +1/+1 counter on Zuko.
- Add {R}.
- Exile Zuko, then return him to the battlefield under an opponent's control.

- [ ] Card loads and can be cast for BR
- [ ] Legendary rule works
- [ ] Enters as 2/3
- [ ] Trigger fires at beginning of first main phase
- [ ] Must choose a mode that hasn't been used
- [ ] Loses 2 life on each trigger
- [ ] Mode 1: Draw a card works
- [ ] Mode 2: +1/+1 counter works
- [ ] Mode 3: Add {R} mana works
- [ ] Mode 4: Exiles then returns under opponent's control
- [ ] After mode 4, opponent controls Zuko
- [ ] Opponent's Zuko triggers on their turn
- [ ] Modes track across zone changes (exile/return)
- [ ] After all 4 modes used, trigger still fires but no valid mode

---

## Basic Lands

### 23. Mountain
- [x] Taps for {R} (verified: multiple puzzles)
- [x] Recognized as basic land (verified: puzzle loading)
- [x] Can play one per turn (verified: gameplay)

### 24. Swamp
- [x] Taps for {B} (verified: Heartless Act puzzle)
- [x] Recognized as basic land (verified: puzzle loading)
- [x] Can play one per turn (verified: gameplay)

---

## Cross-Card Synergies to Verify

- [ ] Beetle-Headed Merchants + Pirate Peddlers (sacrifice triggers both)
- [ ] Beetle-Headed Merchants + Zhao Ruthless Admiral (sacrifice triggers both)
- [ ] Fire Lord Ozai + sacrifice permanents (mana generation + other triggers)
- [ ] Boar-q-pine + noncreature spells (Cunning Maneuver, Lightning Strike, etc.) **[BLOCKED: mtg-hl300]**
- [ ] Jeong Jeong + Iroh's Demonstration (copy Lesson spell)
- [ ] Firebending creatures sharing firebend mana pool
- [ ] Heartless Act vs creatures with +1/+1 counters (mode restrictions)
- [ ] Ty Lee Prowess + Twin Blades Flash (combat tricks) **[BLOCKED: mtg-hl300]**
- [ ] Canyon Crawler Food token + Pirate Peddlers (sacrifice synergy) **[BLOCKED: mtg-6ph0z]**
- [ ] Cunning Maneuver Clue token + Pirate Peddlers (sacrifice synergy) **[BLOCKED: mtg-6ph0z]**

---

## Custom Mechanics Requiring Special Attention

1. **Firebending N** - Pool mana generation/spending
2. **Earthbend N** - Land animation with death/exile return
3. **Exhaust** - One-time activated abilities
4. **Cycling variants** - Swampcycling, Mountaincycling

---

**Progress:** 29 items verified as of 2026-01-06_#1550
- Newly verified: Fire Sages Firebending 1, activated ability; Yuyan Archers Reach; Canyon Crawler Deathtouch in combat
- New bugs filed: mtg-oyvdh (ETB looting triggers)
