#!/usr/bin/env bash
# mtg-767 regression: the engine's auto-saved game-log path
# (/tmp/mtg_game_*.log, announced on stderr as "Log saved to <path>") MUST be
# unique per process so concurrent games never collide. The old per-second
# global path (/tmp/mtg_game_YYYYMMDD_HHMMSS.log) made two games finishing in
# the same wall-clock second truncate each other → a game read back the OTHER
# game's log (the mode-equiv / robots42 validate flakes).
#
# Two checks, both deterministic:
#   (a) the announced path carries a per-process discriminator (pid + seq), i.e.
#       it is NOT the bare per-second format. Pins the path SHAPE.
#   (b) two CONCURRENT games (which routinely finish in the same second) get
#       DISTINCT paths. Pins the actual no-collision property.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary
cd "$WORKSPACE_ROOT"

RED='\033[0;31m'; GREEN='\033[0;32m'; NC='\033[0m'
DECK=decks/grizzly_bears.dck

run_one() { # $1=seed $2=stderr-file
    "$MTG_BIN" tui "$DECK" "$DECK" --p1=zero --p2=zero --seed "$1" --verbosity verbose \
        >/dev/null 2>"$2" || true
}

E1=$(mktemp); E2=$(mktemp)
trap 'rm -f "$E1" "$E2"' EXIT

# (b) two CONCURRENT games → same-second is the common case.
run_one 1 "$E1" &
run_one 2 "$E2" &
wait

P1=$(grep -o 'Log saved to \S*' "$E1" | sed 's/Log saved to //' || true)
P2=$(grep -o 'Log saved to \S*' "$E2" | sed 's/Log saved to //' || true)

if [[ -z "$P1" || -z "$P2" ]]; then
    echo -e "${RED}✗ engine did not announce a log path (P1='$P1' P2='$P2')${NC}"
    exit 1
fi
echo "P1: $P1"
echo "P2: $P2"

# (a) shape: /tmp/mtg_game_<8digits>_<6digits>_<pid>_<seq>.log
shape='^/tmp/mtg_game_[0-9]{8}_[0-9]{6}_[0-9]+_[0-9]+\.log$'
for p in "$P1" "$P2"; do
    if [[ ! "$p" =~ $shape ]]; then
        echo -e "${RED}✗ path '$p' is not the unique-per-process shape (mtg_game_<ts>_<pid>_<seq>.log) — per-second collision regression${NC}"
        exit 1
    fi
done
echo -e "${GREEN}✓ both paths carry pid+seq (unique-per-process shape)${NC}"

# (b) distinct
if [[ "$P1" == "$P2" ]]; then
    echo -e "${RED}✗ two concurrent games COLLIDED on one path → the second truncated the first${NC}"
    exit 1
fi
echo -e "${GREEN}✓ concurrent games got DISTINCT paths (no collision)${NC}"
echo -e "${GREEN}=== PASS: game-log /tmp path is unique per process (mtg-767) ===${NC}"
