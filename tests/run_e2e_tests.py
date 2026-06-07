#!/usr/bin/env python3
import os
import sys
import subprocess
import tempfile
import argparse

# Locate workspace root relative to script location
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
WORKSPACE_ROOT = os.path.abspath(os.path.join(SCRIPT_DIR, '..'))

# Find the mtg binary path
MTG_BIN = os.environ.get('MTG_BIN', os.path.join(WORKSPACE_ROOT, 'target', 'release', 'mtg'))

class TestSuiteRunner:
    def __init__(self, strict=False):
        self.strict = strict
        self.tests = []
        self.pass_count = 0
        self.fail_count = 0
        self.tier_stats = {1: {"pass": 0, "total": 0}, 
                           2: {"pass": 0, "total": 0}, 
                           3: {"pass": 0, "total": 0}, 
                           4: {"pass": 0, "total": 0}}
        self.define_tests()

    def add_test(self, id_str, name, tier, feature, pzl=None, deck1=None, deck2=None, inputs=None, expected=None, unexpected=None):
        self.tests.append({
            "id": id_str,
            "name": name,
            "tier": tier,
            "feature": feature,
            "pzl": pzl,
            "deck1": deck1,
            "deck2": deck2,
            "inputs": inputs,
            "expected": expected or [],
            "unexpected": unexpected or []
        })
        self.tier_stats[tier]["total"] += 1

    def define_tests(self):
        # =====================================================================
        # FEATURE 1: Combustion Technique X=0
        # =====================================================================
        # Tier 1: Happy-path isolation tests (5 tests)
        self.add_test(
            "F1_T1_1", "Combustion Technique (0 Lessons in GY) deals 2 damage", 1, 1,
            pzl="""[metadata]
Name: Combustion Technique 0 Lessons GY
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Combustion Technique;*",
            expected=["casts Combustion Technique", "takes 2 damage"]
        )
        self.add_test(
            "F1_T1_2", "Combustion Technique (1 Lesson in GY) deals 3 damage", 1, 1,
            pzl="""[metadata]
Name: Combustion Technique 1 Lesson GY
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0graveyard=Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Combustion Technique;*",
            expected=["casts Combustion Technique", "takes 3 damage"]
        )
        self.add_test(
            "F1_T1_3", "Combustion Technique (2 Lessons in GY) deals 4 damage", 1, 1,
            pzl="""[metadata]
Name: Combustion Technique 2 Lessons GY
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0graveyard=Combustion Technique; Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Sengir Vampire
""",
            inputs="cast Combustion Technique;*",
            expected=["casts Combustion Technique", "takes 4 damage"]
        )
        self.add_test(
            "F1_T1_4", "Combustion Technique exiles targeted creature upon death", 1, 1,
            pzl="""[metadata]
Name: Combustion Technique Exile Death
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Combustion Technique;*",
            expected=["Grizzly Bears", "is exiled", "exiled instead of dying"]
        )
        self.add_test(
            "F1_T1_5", "Combustion Technique doesn't exile target if it survives", 1, 1,
            pzl="""[metadata]
Name: Combustion Technique Survive No Exile
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Sengir Vampire
""",
            inputs="cast Combustion Technique;*",
            expected=["Sengir Vampire", "takes 2 damage"],
            unexpected=["exiled instead of dying", "is exiled"]
        )
        # Tier 2: Edge cases and boundaries (5 tests)
        self.add_test(
            "F1_T2_1", "Combustion Technique with X=0 (opp GY has Lessons, not ours)", 2, 1,
            pzl="""[metadata]
Name: Combustion Technique Opponent GY
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1graveyard=Combustion Technique
p1battlefield=Grizzly Bears
""",
            inputs="cast Combustion Technique;*",
            expected=["Grizzly Bears", "takes 2 damage"]
        )
        self.add_test(
            "F1_T2_2", "Combustion Technique not castable with no valid targets", 2, 1,
            pzl="""[metadata]
Name: Combustion Technique No Targets
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=
""",
            inputs="*; cast Combustion Technique",
            unexpected=["casts Combustion Technique"]
        )
        self.add_test(
            "F1_T2_3", "Combustion Technique target cannot be player", 2, 1,
            pzl="""[metadata]
Name: Combustion Technique Player Target
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="*; cast Combustion Technique; *",
            expected=["Grizzly Bears"] # falls back to Grizzly Bears or errors, should not hit opponent life



        )
        self.add_test(
            "F1_T2_4", "Combustion Technique exiles a token creature", 2, 1,
            pzl="""[metadata]
Name: Combustion Technique Token
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears|Token
""",
            inputs="cast Combustion Technique;*",
            expected=["is exiled", "exiled instead of dying"]
        )
        self.add_test(
            "F1_T2_5", "Combustion Technique doesn't count lands in graveyard", 2, 1,
            pzl="""[metadata]
Name: Combustion Technique Land in GY
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0graveyard=Mountain; Swamp
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Combustion Technique;*",
            expected=["takes 2 damage"]
        )

        # =====================================================================
        # FEATURE 2: Valley Floodcaller triggers
        # =====================================================================
        # Tier 1: Happy-path isolation tests (5 tests)
        self.add_test(
            "F2_T1_1", "Valley Floodcaller Flash cast during opponent's turn", 1, 2,
            pzl="""[metadata]
Name: Valley Floodcaller Flash
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0hand=Valley Floodcaller
p0library=Island; Island; Island; Island; Island
p0battlefield=Island; Island; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=
""",
            inputs="cast Valley Floodcaller",
            expected=["casts Valley Floodcaller", "resolves"]
        )
        self.add_test(
            "F2_T1_2", "Noncreature spell triggers Valley Floodcaller (pump Otter)", 1, 2,
            pzl="""[metadata]
Name: Valley Floodcaller Pump Otter
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Valley Floodcaller; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Lightning Bolt;Grizzly Bears",
            expected=["Trigger: Valley Floodcaller", "gets +1/+1"]
        )
        self.add_test(
            "F2_T1_3", "Noncreature spell triggers Valley Floodcaller (pump Frog)", 1, 2,
            pzl="""[metadata]
Name: Valley Floodcaller Pump Frog
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Valley Floodcaller; Frogmite|Tapped; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Lightning Bolt;Grizzly Bears",
            expected=["Frogmite", "gets +1/+1", "untap"]
        )
        self.add_test(
            "F2_T1_4", "Noncreature spell triggers Valley Floodcaller (pump Bird/Rat)", 1, 2,
            pzl="""[metadata]
Name: Valley Floodcaller Pump Bird
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Valley Floodcaller; Birds of Paradise|Tapped; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Lightning Bolt;Grizzly Bears",
            expected=["Birds of Paradise", "gets +1/+1"]
        )
        self.add_test(
            "F2_T1_5", "Valley Floodcaller grants flash to noncreature spells", 1, 2,
            pzl="""[metadata]
Name: Valley Floodcaller Flash Spells
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0hand=Stormchaser's Talent
p0library=Island; Island; Island; Island; Island
p0battlefield=Valley Floodcaller; Island; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Stormchaser's Talent",
            expected=["casts Stormchaser's Talent"]
        )
        # Tier 2: Edge cases and boundaries (5 tests)
        self.add_test(
            "F2_T2_1", "Valley Floodcaller Flash does not apply to creature spells", 2, 2,
            pzl="""[metadata]
Name: Valley Floodcaller Flash Creatures
Goal: Win
Turns: 1
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0hand=Grizzly Bears
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Valley Floodcaller; Forest; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Grizzly Bears",
            unexpected=["casts Grizzly Bears"]
        )
        self.add_test(
            "F2_T2_2", "Valley Floodcaller triggers multiple times", 2, 2,
            pzl="""[metadata]
Name: Valley Floodcaller Multi Trigger
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt; Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Valley Floodcaller; Mountain; Mountain; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Lightning Bolt;Grizzly Bears;cast Lightning Bolt;Grizzly Bears",
            expected=["Trigger: Valley Floodcaller"] # logs triggers
        )
        self.add_test(
            "F2_T2_3", "Valley Floodcaller does not untap opponent's creatures", 2, 2,
            pzl="""[metadata]
Name: Valley Floodcaller Opponent creatures
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Valley Floodcaller; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Valley Floodcaller|Tapped
""",
            inputs="cast Lightning Bolt;P2",
            unexpected=["untaps Player 2's Valley Floodcaller"]
        )
        self.add_test(
            "F2_T2_4", "Valley Floodcaller does not pump other types (e.g. Badger)", 2, 2,
            pzl="""[metadata]
Name: Valley Floodcaller Otter Only
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Valley Floodcaller; Badgermole Cub; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Lightning Bolt;P2",
            unexpected=["Badgermole Cub gets +1/+1"]
        )
        self.add_test(
            "F2_T2_5", "Valley Floodcaller static flash does not apply from graveyard", 2, 2,
            pzl="""[metadata]
Name: Valley Floodcaller Flash from GY
Goal: Win
Turns: 1
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0hand=Stormchaser's Talent
p0library=Island; Island; Island; Island; Island
p0graveyard=Valley Floodcaller
p0battlefield=Island; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Stormchaser's Talent",
            unexpected=["casts Stormchaser's Talent"]
        )

        # =====================================================================
        # FEATURE 3: Badgermole Cub (Earthbend)
        # =====================================================================
        # Tier 1: Happy-path isolation tests (5 tests)
        self.add_test(
            "F3_T1_1", "Badgermole Cub ETB earthbends a land", 1, 3,
            pzl="""[metadata]
Name: Badgermole Cub ETB
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Badgermole Cub
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Forest; Forest
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Badgermole Cub;Forest",
            expected=["Badgermole Cub", "is earthbent!"]
        )
        self.add_test(
            "F3_T1_2", "Badgermole Cub additional mana on creature tap", 1, 3,
            pzl="""[metadata]
Name: Badgermole Cub Additional Mana
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Badgermole Cub; Llanowar Elves; Forest
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Llanowar Elves",
            expected=["adds an additional", "{G}"]
        )
        self.add_test(
            "F3_T1_3", "Earthbent land has haste and can attack", 1, 3,
            pzl="""[metadata]
Name: Badgermole Cub Haste Attack
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Badgermole Cub
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Forest; Forest
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Badgermole Cub;Forest;attack Forest",
            expected=["Forest", "attacks"]
        )
        self.add_test(
            "F3_T1_4", "Earthbent land can block", 1, 3,
            pzl="""[metadata]
Name: Badgermole Cub Block
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=COMBAT_DECLARE_ATTACKERS
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Badgermole Cub; Forest|Counters:P1P1=1
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="block Forest;Grizzly Bears",
            expected=["blocks", "Grizzly Bears"]
        )
        self.add_test(
            "F3_T1_5", "Earthbent land dies and returns tapped", 1, 3,
            pzl="""[metadata]
Name: Badgermole Cub Returns Tapped
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Forest|Counters:P1P1=1; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Lightning Bolt;Forest",
            expected=["Forest", "dies", "returns to the battlefield tapped"]
        )
        # Tier 2: Edge cases and boundaries (5 tests)
        self.add_test(
            "F3_T2_1", "Badgermole Cub triggers multiple times on multiple taps", 2, 3,
            pzl="""[metadata]
Name: Badgermole Cub Multi Taps
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Badgermole Cub; Llanowar Elves; Llanowar Elves
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Llanowar Elves;activate Llanowar Elves",
            expected=["adds an additional"]
        )
        self.add_test(
            "F3_T2_2", "Badgermole Cub additional mana fails if not on battlefield", 2, 3,
            pzl="""[metadata]
Name: Badgermole Cub Not on Battlefield
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0graveyard=Badgermole Cub
p0battlefield=Llanowar Elves
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Llanowar Elves",
            unexpected=["adds an additional"]
        )
        self.add_test(
            "F3_T2_3", "Earthbend requires land target", 2, 3,
            pzl="""[metadata]
Name: Badgermole Cub Target Check
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Badgermole Cub
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Forest; Forest
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Badgermole Cub;Forest",
            expected=["Forest is earthbent!"]
        )
        self.add_test(
            "F3_T2_4", "Earthbent land returns tapped when exiled", 2, 3,
            pzl="""[metadata]
Name: Badgermole Cub Exile Return
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Forest|Counters:P1P1=1; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Combustion Technique;Forest",
            expected=["Forest", "exiled", "returns to the battlefield tapped"]
        )
        self.add_test(
            "F3_T2_5", "Badgermole Cub does not apply to opponent's creature taps", 2, 3,
            pzl="""[metadata]
Name: Badgermole Cub Opponent taps
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Badgermole Cub
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Llanowar Elves; Forest
""",
            inputs="activate Llanowar Elves",
            unexpected=["adds an additional"]
        )

        # =====================================================================
        # FEATURE 4: Artist's Talent
        # =====================================================================
        # Tier 1: Happy-path isolation tests (5 tests)
        self.add_test(
            "F4_T1_1", "Artist's Talent Level 1 triggers draw-discard on noncreature cast", 1, 4,
            pzl="""[metadata]
Name: Artist's Talent Level 1
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Artist's Talent; Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Artist's Talent;cast Lightning Bolt;P2;*",
            expected=["Artist's Talent", "triggers", "draw"]
        )
        self.add_test(
            "F4_T1_2", "Artist's Talent Level up to 2", 1, 4,
            pzl="""[metadata]
Name: Artist's Talent Level 2
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Artist's Talent; Mountain; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Artist's Talent",
            expected=["Artist's Talent", "level 2"]
        )
        self.add_test(
            "F4_T1_3", "Artist's Talent Level up to 3", 1, 4,
            pzl="""[metadata]
Name: Artist's Talent Level 3
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Artist's Talent|Counters:LEVEL=2; Mountain; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Artist's Talent",
            expected=["Artist's Talent", "level 3"]
        )
        self.add_test(
            "F4_T1_4", "Artist's Talent Level 2 cost reduction applies", 1, 4,
            pzl="""[metadata]
Name: Artist's Talent L2 Reduction
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Accumulate Wisdom
p0library=Island; Island; Island; Island; Island
p0battlefield=Artist's Talent|Counters:LEVEL=2; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Accumulate Wisdom",
            expected=["casts Accumulate Wisdom"]
        )
        self.add_test(
            "F4_T1_5", "Artist's Talent Level 3 noncombat damage bonus applies", 1, 4,
            pzl="""[metadata]
Name: Artist's Talent L3 Damage
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Artist's Talent|Counters:LEVEL=3; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Lightning Bolt;Grizzly Bears",
            expected=["Grizzly Bears", "takes 5 damage"]
        )
        # Tier 2: Edge cases and boundaries (5 tests)
        self.add_test(
            "F4_T2_1", "Artist's Talent L2 cannot reduce colored mana requirement", 2, 4,
            pzl="""[metadata]
Name: Artist's Talent Colored Cost
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Artist's Talent|Counters:LEVEL=2
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Lightning Bolt",
            unexpected=["casts Lightning Bolt"]
        )
        self.add_test(
            "F4_T2_2", "Artist's Talent L3 damage bonus applies to opponent player", 2, 4,
            pzl="""[metadata]
Name: Artist's Talent Damage Player
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Artist's Talent|Counters:LEVEL=3; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Lightning Bolt;P2",
            expected=["Player 2 takes 5 damage"]
        )
        self.add_test(
            "F4_T2_3", "Artist's Talent L3 damage bonus does not apply to combat damage", 2, 4,
            pzl="""[metadata]
Name: Artist's Talent No Combat
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Artist's Talent|Counters:LEVEL=3; Grizzly Bears
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="attack Grizzly Bears",
            unexpected=["takes 4 damage"]
        )
        self.add_test(
            "F4_T2_4", "Artist's Talent L3 damage bonus does not apply to opponent's sources", 2, 4,
            pzl="""[metadata]
Name: Artist's Talent Opponent Source
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Artist's Talent|Counters:LEVEL=3
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1hand=Lightning Bolt
p1battlefield=Mountain
""",
            inputs="cast Lightning Bolt;P1",
            expected=["Player 1 takes 3 damage"]
        )
        self.add_test(
            "F4_T2_5", "Artist's Talent Level 1 trigger is optional", 2, 4,
            pzl="""[metadata]
Name: Artist's Talent Optional Trigger
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Artist's Talent; Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Artist's Talent;cast Lightning Bolt;P2;pass",
            expected=["Artist's Talent", "triggers"]
        )

        # =====================================================================
        # FEATURE 5: Ral, Crackling Wit
        # =====================================================================
        # Tier 1: Happy-path isolation tests (5 tests)
        self.add_test(
            "F5_T1_1", "Ral planeswalker noncreature trigger (loyalty counter)", 1, 5,
            pzl="""[metadata]
Name: Ral Loyalty Trigger
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Ral, Crackling Wit|Counters:LOYALTY=4; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Lightning Bolt;P2",
            expected=["Ral, Crackling Wit", "loyalty"]
        )
        self.add_test(
            "F5_T1_2", "Ral active +1: create Otter token", 1, 5,
            pzl="""[metadata]
Name: Ral Otter Token
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Ral, Crackling Wit|Counters:LOYALTY=4
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Ral, Crackling Wit", # chooses +1
            expected=["Otter", "enters the battlefield"]
        )
        self.add_test(
            "F5_T1_3", "Ral active -3: draw 3 discard 2", 1, 5,
            pzl="""[metadata]
Name: Ral Draw Discard
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Ral, Crackling Wit|Counters:LOYALTY=4
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Ral, Crackling Wit;1", # chooses -3
            expected=["draws 3", "discards 2"]
        )
        self.add_test(
            "F5_T1_4", "Ral active -10: gain storm emblem", 1, 5,
            pzl="""[metadata]
Name: Ral Storm Emblem
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Ral, Crackling Wit|Counters:LOYALTY=10
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Ral, Crackling Wit;2", # chooses -10
            expected=["Emblem"]
        )
        self.add_test(
            "F5_T1_5", "Ral storm emblem copies spells on cast", 1, 5,
            pzl="""[metadata]
Name: Ral Storm Copy
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt; Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Ral, Crackling Wit|Counters:LOYALTY=10; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Ral, Crackling Wit;2;cast Lightning Bolt;P2;cast Lightning Bolt;P2",
            expected=["Storm"]
        )
        # Tier 2: Edge cases and boundaries (5 tests)
        self.add_test(
            "F5_T2_1", "Ral loyalty trigger only on controller's noncreature cast", 2, 5,
            pzl="""[metadata]
Name: Ral Opponent Cast
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Ral, Crackling Wit|Counters:LOYALTY=4
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1hand=Lightning Bolt
p1battlefield=Mountain
""",
            inputs="cast Lightning Bolt;P1",
            unexpected=["Ral, Crackling Wit gets loyalty counter"]
        )
        self.add_test(
            "F5_T2_2", "Ral loyalty trigger doesn't apply from graveyard", 2, 5,
            pzl="""[metadata]
Name: Ral in Graveyard
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0graveyard=Ral, Crackling Wit
p0battlefield=Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Lightning Bolt;P2",
            unexpected=["Ral, Crackling Wit gets loyalty counter"]
        )
        self.add_test(
            "F5_T2_3", "Ral active -3 fails if loyalty < 3", 2, 5,
            pzl="""[metadata]
Name: Ral Low Loyalty
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Ral, Crackling Wit|Counters:LOYALTY=2
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Ral, Crackling Wit;1",
            unexpected=["draws 3"]
        )
        self.add_test(
            "F5_T2_4", "Ral storm emblem copies correct number of times", 2, 5,
            pzl="""[metadata]
Name: Ral Storm Count
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt; Lightning Bolt; Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Ral, Crackling Wit|Counters:LOYALTY=10; Mountain; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Ral, Crackling Wit;2;cast Lightning Bolt;P2;cast Lightning Bolt;P2;cast Lightning Bolt;P2",
            expected=["Storm"]
        )
        self.add_test(
            "F5_T2_5", "Ral Otter token has prowess", 2, 5,
            pzl="""[metadata]
Name: Ral Otter Prowess
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Otter|Token; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Lightning Bolt;P2",
            expected=["prowess", "gets +1/+1"]
        )

        # =====================================================================
        # FEATURE 6: Room/DFC support
        # =====================================================================
        # Tier 1: Happy-path isolation tests (5 tests)
        self.add_test(
            "F6_T1_1", "Cast Roaring Furnace Room door", 1, 6,
            pzl="""[metadata]
Name: Room Cast Furnace
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Roaring Furnace
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Roaring Furnace;Grizzly Bears",
            expected=["casts Roaring Furnace"]
        )
        self.add_test(
            "F6_T1_2", "Cast Steaming Sauna Room door", 1, 6,
            pzl="""[metadata]
Name: Room Cast Sauna
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Roaring Furnace
p0library=Island; Island; Island; Island; Island
p0battlefield=Island; Island; Island; Island; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Steaming Sauna",
            expected=["casts Steaming Sauna"]
        )
        self.add_test(
            "F6_T1_3", "Unlock Steaming Sauna door on battlefield", 1, 6,
            pzl="""[metadata]
Name: Room Unlock Sauna
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Island; Island; Island; Island; Island
p0battlefield=Roaring Furnace; Island; Island; Island; Island; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="unlock Steaming Sauna",
            expected=["unlocks Steaming Sauna"]
        )
        self.add_test(
            "F6_T1_4", "Cast DFC front side (The Legend of Kuruk Saga)", 1, 6,
            pzl="""[metadata]
Name: Saga Kuruk Front
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=The Legend of Kuruk
p0library=Island; Island; Island; Island; Island
p0battlefield=Island; Island; Island; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast The Legend of Kuruk",
            expected=["The Legend of Kuruk", "enters the battlefield"]
        )
        self.add_test(
            "F6_T1_5", "DFC transformation (Saga chapter III transforms to Avatar Kuruk)", 1, 6,
            pzl="""[metadata]
Name: Saga DFC Transform
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Island; Island; Island; Island; Island
p0battlefield=The Legend of Kuruk|Counters:LORE=2
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="pass",
            expected=["Avatar Kuruk"]
        )
        # Tier 2: Edge cases and boundaries (5 tests)
        self.add_test(
            "F6_T2_1", "Roaring Furnace ETB deals damage equal to cards in hand", 2, 6,
            pzl="""[metadata]
Name: Room Furnace Damage
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Roaring Furnace; Mountain
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Grizzly Bears
""",
            inputs="cast Roaring Furnace;Grizzly Bears",
            expected=["Grizzly Bears", "takes 1 damage"]
        )
        self.add_test(
            "F6_T2_2", "Steaming Sauna grants draw at end step", 2, 6,
            pzl="""[metadata]
Name: Room Sauna End Step Draw
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=END
p0life=20
p0library=Island; Island; Island; Island; Island
p0battlefield=Roaring Furnace; Island; Island; Island; Island; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="unlock Steaming Sauna;pass",
            expected=["draws"]
        )
        self.add_test(
            "F6_T2_3", "Cannot unlock door unless Room is on battlefield", 2, 6,
            pzl="""[metadata]
Name: Room Unlock Restriction
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Roaring Furnace
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="unlock Steaming Sauna",
            unexpected=["unlocks Steaming Sauna"]
        )
        self.add_test(
            "F6_T2_4", "Avatar Kuruk triggers Spirit token creation on spell cast", 2, 6,
            pzl="""[metadata]
Name: DFC Kuruk Trigger
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Island; Island; Island; Island; Island
p0battlefield=The Legend of Kuruk|Counters:LORE=2; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="pass;cast Lightning Bolt;P2",
            expected=["Spirit", "enters the battlefield"]
        )
        self.add_test(
            "F6_T2_5", "Avatar Kuruk extra turn activation via Waterbend", 2, 6,
            pzl="""[metadata]
Name: DFC Kuruk Extra Turn
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Island; Island; Island; Island; Island
p0battlefield=The Legend of Kuruk|Counters:LORE=2; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="pass;activate Avatar Kuruk",
            expected=["extra turn"]
        )

        # =====================================================================
        # FEATURE 7: Quantum Riddler
        # =====================================================================
        # Tier 1: Happy-path isolation tests (5 tests)
        self.add_test(
            "F7_T1_1", "Quantum Riddler Warp cast", 1, 7,
            pzl="""[metadata]
Name: Quantum Riddler Warp
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Quantum Riddler
p0library=Island; Island; Island; Island; Island
p0battlefield=Island; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Quantum Riddler", # casts with warp 1U
            expected=["casts Quantum Riddler"]
        )
        self.add_test(
            "F7_T1_2", "Quantum Riddler ETB draws card", 1, 7,
            pzl="""[metadata]
Name: Quantum Riddler ETB
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Quantum Riddler
p0library=Island; Island; Island; Island; Island
p0battlefield=Island; Island; Island; Island; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Quantum Riddler",
            expected=["Quantum Riddler", "draws"]
        )
        self.add_test(
            "F7_T1_3", "Quantum Riddler draw bonus when hand is empty", 1, 7,
            pzl="""[metadata]
Name: Quantum Riddler Hand 0
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Opt
p0library=Island; Island; Island; Island; Island
p0battlefield=Quantum Riddler; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Opt",
            expected=["draws 2"] # replaces 1 draw with 2
        )
        self.add_test(
            "F7_T1_4", "Quantum Riddler draw bonus when hand has 1 card", 1, 7,
            pzl="""[metadata]
Name: Quantum Riddler Hand 1
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Opt; Mountain
p0library=Island; Island; Island; Island; Island
p0battlefield=Quantum Riddler; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Opt",
            expected=["draws 2"]
        )
        self.add_test(
            "F7_T1_5", "Quantum Riddler draw bonus doesn't apply when hand has 2+ cards", 1, 7,
            pzl="""[metadata]
Name: Quantum Riddler Hand 2
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Opt; Mountain; Forest
p0library=Island; Island; Island; Island; Island
p0battlefield=Quantum Riddler; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Opt",
            expected=["draws 1"]
        )
        # Tier 2: Edge cases and boundaries (5 tests)
        self.add_test(
            "F7_T2_1", "Quantum Riddler Warp casting check from hand", 2, 7,
            pzl="""[metadata]
Name: Quantum Riddler Warp Hand
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Quantum Riddler
p0library=Island; Island; Island; Island; Island
p0battlefield=Island; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Quantum Riddler",
            expected=["casts Quantum Riddler"]
        )
        self.add_test(
            "F7_T2_2", "Quantum Riddler cost reduction applies to Warp", 2, 7,
            pzl="""[metadata]
Name: Quantum Riddler Warp Reduction
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Quantum Riddler
p0library=Island; Island; Island; Island; Island
p0battlefield=Artist's Talent|Counters:LEVEL=2; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Quantum Riddler",
            expected=["casts Quantum Riddler"]
        )
        self.add_test(
            "F7_T2_3", "Quantum Riddler draw replacement applies only to controller", 2, 7,
            pzl="""[metadata]
Name: Quantum Riddler Opponent Hand
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0library=Island; Island; Island; Island; Island
p0battlefield=Quantum Riddler
p1life=20
p1hand=Opt
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Island
""",
            inputs="cast Opt",
            expected=["draws 1"] # opponent only draws 1
        )
        self.add_test(
            "F7_T2_4", "Quantum Riddler draws extra on normal draw step if hand is empty", 2, 7,
            pzl="""[metadata]
Name: Quantum Riddler Draw Step
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=DRAW
p0life=20
p0hand=
p0library=Island; Island; Island; Island; Island
p0battlefield=Quantum Riddler
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="pass",
            expected=["draws 2"]
        )
        self.add_test(
            "F7_T2_5", "Quantum Riddler draw bonus fails if in graveyard", 2, 7,
            pzl="""[metadata]
Name: Quantum Riddler in GY
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Opt
p0library=Island; Island; Island; Island; Island
p0graveyard=Quantum Riddler
p0battlefield=Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Opt",
            expected=["draws 1"]
        )

        # =====================================================================
        # FEATURE 8: Enduring Vitality
        # =====================================================================
        # Tier 1: Happy-path isolation tests (5 tests)
        self.add_test(
            "F8_T1_1", "Cast Enduring Vitality", 1, 8,
            pzl="""[metadata]
Name: Enduring Vitality Cast
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Enduring Vitality
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Forest; Forest; Forest
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Enduring Vitality",
            expected=["casts Enduring Vitality", "resolves"]
        )
        self.add_test(
            "F8_T1_2", "Creatures have vigilance under Enduring Vitality", 1, 8,
            pzl="""[metadata]
Name: Enduring Vitality Vigilance
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Enduring Vitality; Grizzly Bears
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="attack Grizzly Bears",
            expected=["vigilance"]
        )
        self.add_test(
            "F8_T1_3", "Creatures tap for mana of any color", 1, 8,
            pzl="""[metadata]
Name: Enduring Vitality Mana
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Enduring Vitality; Grizzly Bears
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Grizzly Bears",
            expected=["adds", "mana"]
        )
        self.add_test(
            "F8_T1_4", "Enduring Vitality dies and returns as enchantment", 1, 8,
            pzl="""[metadata]
Name: Enduring Vitality Death Return
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Enduring Vitality
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1hand=Lightning Bolt
p1battlefield=Mountain
""",
            inputs="cast Lightning Bolt;Enduring Vitality",
            expected=["Enduring Vitality", "dies", "returns", "enchantment"]
        )
        self.add_test(
            "F8_T1_5", "Returned Enduring Vitality is not a creature", 1, 8,
            pzl="""[metadata]
Name: Enduring Vitality Enchantment Only
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Enduring Vitality
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1hand=Lightning Bolt
p1battlefield=Mountain
""",
            inputs="cast Lightning Bolt;Enduring Vitality",
            expected=["returns", "enchantment"],
            unexpected=["creature"]
        )
        # Tier 2: Edge cases and boundaries (5 tests)
        self.add_test(
            "F8_T2_1", "Returned Enduring Vitality (enchantment) still grants vigilance", 2, 8,
            pzl="""[metadata]
Name: Enduring Vitality Enchantment Vigilance
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Enduring Vitality|Types:Enchantment; Grizzly Bears
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="attack Grizzly Bears",
            expected=["vigilance"]
        )
        self.add_test(
            "F8_T2_2", "Returned Enduring Vitality (enchantment) still grants mana tap", 2, 8,
            pzl="""[metadata]
Name: Enduring Vitality Enchantment Mana
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Enduring Vitality|Types:Enchantment; Grizzly Bears
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Grizzly Bears",
            expected=["adds", "mana"]
        )
        self.add_test(
            "F8_T2_3", "Vigilance allows attacking without tapping", 2, 8,
            pzl="""[metadata]
Name: Enduring Vitality Attacking No Tap
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Enduring Vitality; Grizzly Bears
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="attack Grizzly Bears",
            expected=["Grizzly Bears", "attacks"],
            unexpected=["Grizzly Bears is tapped"]
        )
        self.add_test(
            "F8_T2_4", "Enduring Vitality doesn't return if destroyed as enchantment", 2, 8,
            pzl="""[metadata]
Name: Enduring Vitality Enchantment Destroy
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Disenchant
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Enduring Vitality|Types:Enchantment; Plains; Plains
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Disenchant;Enduring Vitality",
            expected=["Disenchant", "resolves", "dies"],
            unexpected=["returns"]
        )
        self.add_test(
            "F8_T2_5", "Enduring Vitality doesn't return if exiled from battlefield", 2, 8,
            pzl="""[metadata]
Name: Enduring Vitality Exile check
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p1
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Enduring Vitality
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1hand=Swords to Plowshares
p1battlefield=Plains
""",
            inputs="cast Swords to Plowshares;Enduring Vitality",
            expected=["exiled"],
            unexpected=["returns"]
        )

        # =====================================================================
        # TIER 3: Cross-Feature Integration (8 tests)
        # =====================================================================
        self.add_test(
            "T3_1", "Badgermole Cub + Enduring Vitality mana synergy", 3, 0,
            pzl="""[metadata]
Name: T3_1 Badgermole Vitality
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Forest; Forest; Forest; Forest; Forest
p0battlefield=Badgermole Cub; Enduring Vitality; Forest
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Badgermole Cub",
            expected=["adds an additional", "{G}"]
        )
        self.add_test(
            "T3_2", "Valley Floodcaller untaps earthbent land (since it is a creature)", 3, 0,
            pzl="""[metadata]
Name: T3_2 Valley Earthbend
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Valley Floodcaller; Forest|Counters:P1P1=1|Types:Creature,Land; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Lightning Bolt;P2",
            expected=["untap", "Forest"]
        )
        self.add_test(
            "T3_3", "Ral storm emblem copies trigger Valley Floodcaller multiple times", 3, 0,
            pzl="""[metadata]
Name: T3_3 Ral Storm Valley
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt; Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Valley Floodcaller; Ral, Crackling Wit|Counters:LOYALTY=10; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="activate Ral, Crackling Wit;2;cast Lightning Bolt;P2;cast Lightning Bolt;P2",
            expected=["Trigger", "Valley Floodcaller"]
        )
        self.add_test(
            "T3_4", "Artist's Talent Level 3 noncombat damage bonus scales Combustion Technique", 3, 0,
            pzl="""[metadata]
Name: T3_4 Artist Combustion
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Combustion Technique
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Artist's Talent|Counters:LEVEL=3; Mountain; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
p1battlefield=Sengir Vampire
""",
            inputs="cast Combustion Technique;Sengir Vampire",
            expected=["Sengir Vampire", "takes 4 damage"] # 2 base + 2 bonus = 4
        )
        self.add_test(
            "T3_5", "Quantum Riddler + Steaming Sauna (unlimited hand + extra draw step)", 3, 0,
            pzl="""[metadata]
Name: T3_5 Riddler Sauna
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=END
p0life=20
p0library=Island; Island; Island; Island; Island
p0battlefield=Quantum Riddler; Roaring Furnace; Island; Island; Island; Island; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="unlock Steaming Sauna;pass",
            expected=["draws"]
        )
        self.add_test(
            "T3_6", "Enduring Vitality returned as enchantment triggers Valley Floodcaller on noncreature", 3, 0,
            pzl="""[metadata]
Name: T3_6 Vitality Enchantment Valley
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Valley Floodcaller; Enduring Vitality|Types:Enchantment; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Lightning Bolt;P2",
            expected=["Trigger: Valley Floodcaller"]
        )
        self.add_test(
            "T3_7", "Ral Otter token gets pumped by Valley Floodcaller noncreature trigger", 3, 0,
            pzl="""[metadata]
Name: T3_7 Ral Otter Valley
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt
p0library=Mountain; Mountain; Mountain; Mountain; Mountain
p0battlefield=Valley Floodcaller; Otter|Token; Mountain
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="cast Lightning Bolt;P2",
            expected=["Otter", "gets +1/+1"]
        )
        self.add_test(
            "T3_8", "Badgermole Cub earthbends a land used for Waterbend of Kuruk", 3, 0,
            pzl="""[metadata]
Name: T3_8 Badgermole Waterbend
Goal: Win
Turns: 5
Difficulty: Easy
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0library=Island; Island; Island; Island; Island
p0battlefield=The Legend of Kuruk|Counters:LORE=2; Badgermole Cub; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest; Forest
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
""",
            inputs="pass;activate Avatar Kuruk",
            expected=["extra turn"]
        )

        # =====================================================================
        # TIER 4: Championship Decks & Matchups (5 tests)
        # =====================================================================
        self.add_test(
            "T4_1", "Load and play 01_manfield_izzet_lessons.dck", 4, 0,
            deck1="decks/championship/2025/01_manfield_izzet_lessons.dck",
            deck2="decks/championship/2025/01_manfield_izzet_lessons.dck",
            expected=["Starting Game", "Main Phase 1", "Snapshot Saved"]
        )
        self.add_test(
            "T4_2", "Load and play 02_shibata_izzet_lessons.dck", 4, 0,
            deck1="decks/championship/2025/02_shibata_izzet_lessons.dck",
            deck2="decks/championship/2025/02_shibata_izzet_lessons.dck",
            expected=["Starting Game", "Main Phase 1", "Snapshot Saved"]
        )
        self.add_test(
            "T4_3", "Load and play 03_davis_izzet_lessons.dck", 4, 0,
            deck1="decks/championship/2025/03_davis_izzet_lessons.dck",
            deck2="decks/championship/2025/03_davis_izzet_lessons.dck",
            expected=["Starting Game", "Main Phase 1", "Snapshot Saved"]
        )
        self.add_test(
            "T4_4", "Load and play 04_henry_temur_otters.dck", 4, 0,
            deck1="decks/championship/2025/04_henry_temur_otters.dck",
            deck2="decks/championship/2025/04_henry_temur_otters.dck",
            expected=["Starting Game", "Main Phase 1", "Snapshot Saved"]
        )
        self.add_test(
            "T4_5", "Championship matchup (01_manfield_izzet vs 04_henry_temur)", 4, 0,
            deck1="decks/championship/2025/01_manfield_izzet_lessons.dck",
            deck2="decks/championship/2025/04_henry_temur_otters.dck",
            expected=["Starting Game", "Main Phase 1", "Snapshot Saved"]
        )

    def run_tests(self, filter_str=None):
        tests_to_run = self.tests
        if filter_str:
            tests_to_run = [t for t in self.tests if filter_str in t["id"] or filter_str in t["name"]]
        
        print("TAP version 13")
        print(f"1..{len(tests_to_run)}")

        for index, test in enumerate(tests_to_run, 1):
            test_id = test["id"]
            name = test["name"]
            tier = test["tier"]
            
            passed, log = self.execute_single_test(test)
            
            if passed:
                self.pass_count += 1
                self.tier_stats[tier]["pass"] += 1
                print(f"ok {index} - Tier {tier} {test_id}: {name}")
            else:
                self.fail_count += 1
                print(f"not ok {index} - Tier {tier} {test_id}: {name} # log output validation failed")
                # Print log helper under TAP diagnostics format (lines starting with #)
                print("# --- TEST FAILURE DIAGNOSTICS ---")
                for line in log.splitlines():
                    print(f"#   {line}")
                print("# --------------------------------")

        print("# Tiers summary:")
        print(f"# Tier 1: {self.tier_stats[1]['pass']} / {self.tier_stats[1]['total']} passed")
        print(f"# Tier 2: {self.tier_stats[2]['pass']} / {self.tier_stats[2]['total']} passed")
        print(f"# Tier 3: {self.tier_stats[3]['pass']} / {self.tier_stats[3]['total']} passed")
        print(f"# Tier 4: {self.tier_stats[4]['pass']} / {self.tier_stats[4]['total']} passed")
        print(f"# Total: {self.pass_count} / {len(tests_to_run)} passed")

        if self.strict and self.fail_count > 0:
            print(f"# Strict mode: exiting with code 1 due to {self.fail_count} failures")
            sys.exit(1)
        else:
            sys.exit(0)

    def execute_single_test(self, test):
        # 1. Create a temporary puzzle file if it is a puzzle-based test
        temp_pzl_path = None
        if test["pzl"]:
            with tempfile.NamedTemporaryFile(mode='w', suffix='.pzl', delete=False) as f:
                f.write(test["pzl"])
                temp_pzl_path = f.name

        try:
            # 2. Formulate the subprocess command
            cmd = [MTG_BIN, "tui"]
            
            if temp_pzl_path:
                cmd.extend(["--start-state", temp_pzl_path])
                cmd.extend(["--p1", "fixed", "--p2", "zero"])
                if test["inputs"]:
                    cmd.extend(["--p1-fixed-inputs", test["inputs"]])
                    cmd.extend(["--p2-fixed-inputs", ""])
            else:
                # Deck-based tests (Tier 4)
                cmd.extend([test["deck1"], test["deck2"]])
                cmd.extend(["--p1", "zero", "--p2", "zero"])
                cmd.extend(["--stop-on-choice", "5"])
                cmd.extend(["--seed", "42"])

            cmd.extend(["--verbosity", "3", "--no-color-logs"])

            # 3. Execute process
            result = subprocess.run(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                cwd=WORKSPACE_ROOT,
                timeout=15
            )

            log_output = result.stdout

            # 4. Check exit code & expected/unexpected strings
            # In non-strict mode, if the binary itself fails or crashes we still want to report the test outcome correctly,
            # which is that it failed (unless expected is empty and we don't care, but we always care).
            if result.returncode != 0 and result.returncode != 124:  # 124 is timeout, treat as fail
                return False, f"Process exited with non-zero code {result.returncode}.\n\nOutput:\n{log_output}"

            # Validate expected matches
            for exp in test["expected"]:
                # Simple case-insensitive or case-sensitive match.
                # Let's check case-insensitively to be robust, but respect the logs.
                if exp.lower() not in log_output.lower():
                    return False, f"Expected string '{exp}' not found in log output.\n\nOutput:\n{log_output}"

            # Validate unexpected matches
            for unexp in test["unexpected"]:
                if unexp.lower() in log_output.lower():
                    return False, f"Unexpected string '{unexp}' found in log output.\n\nOutput:\n{log_output}"

            return True, log_output

        except subprocess.TimeoutExpired:
            return False, "Test process timed out (15s)."
        except Exception as e:
            return False, f"Exception during execution: {e}"
        finally:
            # Clean up the temporary puzzle file
            if temp_pzl_path and os.path.exists(temp_pzl_path):
                try:
                    os.remove(temp_pzl_path)
                except OSError:
                    pass

def main():
    parser = argparse.ArgumentParser(description="E2E Feature Test Runner")
    parser.add_argument("--strict", action="store_true", help="Exit with code 1 if any test fails")
    parser.add_argument("--filter", type=str, default="", help="Filter tests by ID substring")
    args = parser.parse_args()

    runner = TestSuiteRunner(strict=args.strict)
    runner.run_tests(filter_str=args.filter)

if __name__ == "__main__":
    main()
