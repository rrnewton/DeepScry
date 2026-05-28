#!/usr/bin/env bash
# oldschool_progress.sh — quantify 1994 Old School playtest progress (mtg-pph0s).
#
# For each deck-tracking issue ("TRACK: Old School 1994 deck ...") it parses the
# per-card 'Card Compatibility:' issue IDs it references and reports how many are
# CLOSED (= card reached WORKING per the compatibility_tracking convention).
# Also prints the overall Card-Compatibility closed/total.
#
# Usage: scripts/oldschool_progress.sh            # run from the repo root
# Requires: bd (minibeads) on PATH.
set -euo pipefail
cd "$(dirname "$0")/../.."   # scripts/temp/ -> repo root

command -v bd >/dev/null || { echo "error: bd (minibeads) not on PATH" >&2; exit 1; }

# Build an id->status map from one pass over open+closed issues.
# bd list line format: "mtg-396: <title> [open] (priority: 3)"
tmp_status="$(mktemp)"; trap 'rm -f "$tmp_status"' EXIT
{ bd list --status open 2>/dev/null; bd list --status closed 2>/dev/null; } \
  | sed -nE 's/^(mtg-[0-9a-z]+):.*\[(open|closed|in_progress)\].*/\1 \2/p' \
  | sort -u > "$tmp_status"

status_of() { awk -v id="$1" '$1==id{print $2; found=1} END{if(!found)print "unknown"}' "$tmp_status"; }

# Discover deck-tracking issues. Two title conventions exist:
#   "TRACK: Old School 1994 deck '<name>' — full compatibility"  (skeleton-filed)
#   "Deck Compatibility: <name> (<file>.dck)"                     (pre-existing)
deck_ids=$(bd list 2>/dev/null \
  | sed -nE 's/^(mtg-[0-9a-z]+): (TRACK: Old School 1994 deck|Deck Compatibility:).*/\1/p' \
  | sort -u)
[ -n "$deck_ids" ] || { echo "No deck-tracking issues found." >&2; exit 1; }

echo "=== 1994 Old School playtest progress (umbrella: 1994 Old School goal) ==="
echo "$(date '+%Y-%m-%d %H:%M')  |  integration $(git rev-parse --short HEAD 2>/dev/null)"
echo
union="$(mktemp)"; trap 'rm -f "$tmp_status" "$union"' EXIT   # unique card IDs across all decks
for deck in $deck_ids; do
  title=$(bd show "$deck" 2>/dev/null | sed -nE 's/^Title: (TRACK: Old School 1994 deck |Deck Compatibility: )?//p' \
            | head -1 | sed -E "s/ — full compatibility$//; s/^'//; s/' *$//")
  # per-card refs: mtg-NNN tokens in the description, excluding the deck's own id
  cards=$(bd show "$deck" 2>/dev/null | grep -oE 'mtg-[0-9a-z]+' | grep -v "^$deck$" | sort -u)
  done=0 total=0
  for c in $cards; do
    st=$(status_of "$c")
    [ "$st" = "unknown" ] && continue   # not a tracked issue ref
    total=$((total+1)); [ "$st" = "closed" ] && done=$((done+1))
    echo "$c $st" >> "$union"
  done
  pct=0; [ "$total" -gt 0 ] && pct=$(( done * 100 / total ))
  printf "  %-42s %2d/%-3d (%3d%%)  [%s]\n" "${title:-$deck}" "$done" "$total" "$pct" "$deck"
done

# True union across all decks (a card in multiple decks is counted once).
u_total=$(sort -u "$union" | awk '{print $1}' | sort -u | wc -l)
u_done=$(sort -u "$union" | awk '$2=="closed"{print $1}' | sort -u | wc -l)
# Overall 'Card Compatibility:' issues across the whole tracker (not just deck-linked).
cc_open=$(bd list --status open 2>/dev/null | grep -c 'Card Compatibility:' || true)
cc_closed=$(bd list --status closed 2>/dev/null | grep -c 'Card Compatibility:' || true)
cc_all=$((cc_open + cc_closed))
echo
upct=0; [ "$u_total" -gt 0 ] && upct=$(( u_done * 100 / u_total ))
printf "  %-42s %2d/%-3d (%3d%%)\n" "UNIQUE deck-linked cards (union)" "$u_done" "$u_total" "$upct"
acpct=0; [ "$cc_all" -gt 0 ] && acpct=$(( cc_closed * 100 / cc_all ))
printf "  %-42s %2d/%-3d (%3d%%)\n" "ALL 'Card Compatibility:' issues" "$cc_closed" "$cc_all" "$acpct"
