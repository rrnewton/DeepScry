#!/usr/bin/env bash
# E2E test: Paralyze's "pay {4} to untap" upkeep escape (mtg-646 / mtg-529).
#
# Paralyze's third ability:
#   T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player.EnchantedController
#     | TriggerZones$ Battlefield | Execute$ TrigUntap
#   SVar:TrigUntap:DB$ Untap | Defined$ Enchanted | UnlessCost$ 4
#     | UnlessPayer$ EnchantedController | UnlessSwitched$ True
# "At the beginning of the upkeep of enchanted creature's controller, that
#  player MAY pay {4}. If the player does, untap the creature."
#
# Before the fix the phase-trigger parser only recognised ValidPlayer$ You
# (controller-only) and could not resolve Defined$ Enchanted, so this optional
# untap trigger was silently dropped — the doesn't-untap lock was permanent
# with NO escape (strictly stricter than printed).
#
# After the fix the loader emits a BeginningOfUpkeep trigger flagged
# enchanted_controller_turn_only (fires on the HOST creature's controller's
# upkeep, NOT the Aura controller's), whose effect is an
# UnlessCostWrapper { UntapPermanent(Enchanted) } with a {4} mana cost and
# UnlessSwitched$ True (the untap runs ONLY when the {4} is paid). The
# pay/don't-pay decision uses the shared, determinism-safe in-engine
# UnlessCost executor (tracked under mtg-rpmpg). A naive unconditional untap
# would make Paralyze FREE to escape every upkeep — explicitly avoided.
#
# Two scenarios (both: P0 casts Paralyze on P1's Grizzly Bears, seed 42):
#   PAY   (test_puzzles/paralyze_pay4_untap_e2e.pzl):
#         P1 has 4 Forests -> can pay {4}. Each of P1's upkeeps the lock holds
#         in the untap step ("doesn't untap (locked tapped)"), then the trigger
#         pays {4} and untaps the bears so they ATTACK on turn 4 (Player 1 life
#         falls below 18).
#   NOPAY (test_puzzles/paralyze_cannot_pay4_stays_tapped_e2e.pzl):
#         P1 has only 1 Forest -> cannot pay {4}. After the first attack the
#         bears stay "locked tapped" on turns 4 and 6 and NEVER attack again
#         (Player 1 life stays at 18).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Paralyze pay-{4}-to-untap E2E ==="
echo

cd "$WORKSPACE_ROOT"

# P1 casts Paralyze, then wildcard-passes/auto-plays through the rest of the
# game (it has no further meaningful decisions). P2 is heuristic so it pays the
# optional {4} when able and attacks with the untapped creature.
P1_INPUTS="cast Paralyze"
for _ in $(seq 1 60); do P1_INPUTS="${P1_INPUTS};*"; done

run_paralyze_puzzle() {
    local puzzle="$1" log="$2"
    if run_mtg_with_timeout 90 tui \
        --start-state "$WORKSPACE_ROOT/test_puzzles/$puzzle" \
        --p1=fixed --p2=heuristic \
        --p1-fixed-inputs="$P1_INPUTS" \
        --json --seed 42 --verbosity 3 \
        > "$log" 2>&1; then
        echo -e "${GREEN}✓ Game completed ($puzzle)${NC}"
    else
        local status=$?
        echo -e "${RED}✗ Game failed (exit $status) for $puzzle${NC}"
        head -80 "$log"
        exit 1
    fi
}

# ---------------------------------------------------------------------------
# Scenario 1: controller CAN pay {4} -> untaps and attacks past the lock.
# ---------------------------------------------------------------------------
PAY_LOG=/tmp/paralyze_pay4_untap_e2e.txt
run_paralyze_puzzle paralyze_pay4_untap_e2e.pzl "$PAY_LOG"

# The Aura must attach (the cast scenario must actually enchant the creature).
if ! grep -qE "Paralyze enchants Grizzly Bears" "$PAY_LOG"; then
    echo -e "${RED}✗ Paralyze did not enchant Grizzly Bears${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Paralyze enchanted Grizzly Bears${NC}"

# The pay-{4} trigger must fire on the ENCHANTED creature's controller's upkeep.
if ! grep -qE "Trigger: Paralyze - At the beginning of the upkeep of enchanted creature's controller" "$PAY_LOG"; then
    echo -e "${RED}✗ Paralyze pay-{4} upkeep trigger never fired (silent drop?)${NC}"
    grep -E "Trigger:|Paralyze" "$PAY_LOG" || true
    exit 1
fi
echo -e "${GREEN}✓ Pay-{4} upkeep trigger fired on host controller's upkeep${NC}"

# The doesn't-untap lock must still hold in the untap step (the trigger is the
# only escape) ...
if ! grep -qE "Grizzly Bears doesn't untap \(locked tapped\)" "$PAY_LOG"; then
    echo -e "${RED}✗ doesn't-untap lock never engaged${NC}"
    exit 1
fi
echo -e "${GREEN}✓ doesn't-untap lock held in the untap step${NC}"

# ... but having PAID {4}, the bears untap and attack again on a LATER turn,
# driving Player 1 below the single-attack total of 18 (to 16). This is the
# decisive proof the optional untap actually fired.
if ! grep -qE "Grizzly Bears \(15\) deals 2 damage to Player 1 \(life: 16\)" "$PAY_LOG"; then
    echo -e "${RED}✗ Paid {4} but creature did NOT untap+attack on a later turn${NC}"
    grep -E "deals 2 damage to Player 1" "$PAY_LOG" || echo "(no further attacks)"
    exit 1
fi
echo -e "${GREEN}✓ After paying {4}, the creature untapped and attacked again (life 18 -> 16)${NC}"

# ---------------------------------------------------------------------------
# Scenario 2: controller CANNOT pay {4} -> stays locked, no further attacks.
# ---------------------------------------------------------------------------
NOPAY_LOG=/tmp/paralyze_cannot_pay4_stays_tapped_e2e.txt
run_paralyze_puzzle paralyze_cannot_pay4_stays_tapped_e2e.pzl "$NOPAY_LOG"

# The lock must engage on the later upkeeps ...
if ! grep -qE "Grizzly Bears doesn't untap \(locked tapped\)" "$NOPAY_LOG"; then
    echo -e "${RED}✗ doesn't-untap lock never engaged in no-pay scenario${NC}"
    exit 1
fi
echo -e "${GREEN}✓ doesn't-untap lock held (no-pay scenario)${NC}"

# ... and with the {4} unpayable the creature must NEVER untap+attack again:
# Player 1's life must stay at 18 (it took exactly one early hit, never 16).
if grep -qE "deals 2 damage to Player 1 \(life: 16\)" "$NOPAY_LOG"; then
    echo -e "${RED}✗ Creature attacked again despite being unable to pay {4} (lock leaked)${NC}"
    grep -E "deals 2 damage to Player 1" "$NOPAY_LOG"
    exit 1
fi
echo -e "${GREEN}✓ Unable to pay {4}: creature stayed locked, no further attacks${NC}"

echo
echo -e "${GREEN}=== Paralyze pay-{4}-to-untap E2E PASSED ===${NC}"
