"""Card definition loader for agent prompts.

Parses card text files from the forge-java card database to provide
card definitions to the AI agent.
"""

from __future__ import annotations

from pathlib import Path
from typing import Optional


class CardDefinition:
    """A card's key properties for agent context."""

    __slots__ = ("name", "mana_cost", "types", "oracle", "pt")

    def __init__(
        self,
        name: str,
        mana_cost: str,
        types: str,
        oracle: str,
        pt: Optional[str] = None,
    ) -> None:
        self.name = name
        self.mana_cost = mana_cost
        self.types = types
        self.oracle = oracle
        self.pt = pt

    def format(self) -> str:
        """Format card definition for inclusion in agent prompt."""
        parts = [f"  {self.name}"]
        if self.mana_cost:
            parts[0] += f" {{{self.mana_cost}}}"
        parts.append(f"    {self.types}")
        if self.pt:
            parts.append(f"    {self.pt}")
        if self.oracle:
            parts.append(f"    {self.oracle}")
        return "\n".join(parts)


class CardDatabase:
    """Loads card definitions from forge-java card files."""

    def __init__(self, cardsfolder: Path) -> None:
        self._cards: dict[str, CardDefinition] = {}
        self._cardsfolder = cardsfolder

    def load_deck(self, deck_path: Path) -> None:
        """Load card definitions for all cards in a deck file."""
        if not deck_path.exists():
            return
        for line in deck_path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line or line.startswith("[") or line.startswith("#"):
                continue
            # Format: "N CardName" or "N CardName|SET"
            parts = line.split(None, 1)
            if len(parts) < 2:
                continue
            card_name = parts[1].split("|")[0].strip()
            if card_name not in self._cards:
                defn = self._load_card(card_name)
                if defn is not None:
                    self._cards[card_name] = defn

    def get(self, name: str) -> Optional[CardDefinition]:
        """Look up a card definition by name."""
        return self._cards.get(name)

    def all_names(self) -> set[str]:
        """Return all loaded card names."""
        return set(self._cards.keys())

    def format_definitions(self, names: set[str]) -> str:
        """Format card definitions for a set of card names."""
        defs = []
        for name in sorted(names):
            defn = self._cards.get(name)
            if defn is not None:
                defs.append(defn.format())
        return "\n".join(defs) if defs else ""

    def _load_card(self, card_name: str) -> Optional[CardDefinition]:
        """Load a single card definition from the cardsfolder."""
        # Card files are stored as: cardsfolder/first_letter/card_name.txt
        # with underscores replacing spaces
        filename = card_name.lower().replace(" ", "_").replace("'", "").replace(",", "")
        first_letter = filename[0] if filename else "a"
        card_path = self._cardsfolder / first_letter / f"{filename}.txt"
        if not card_path.exists():
            # Try with apostrophe variants
            for variant in [
                card_name.lower().replace(" ", "_"),
                card_name.lower().replace(" ", "_").replace("'", "_"),
            ]:
                alt = self._cardsfolder / variant[0] / f"{variant}.txt"
                if alt.exists():
                    card_path = alt
                    break
            else:
                return None
        return self._parse_card_file(card_path)

    @staticmethod
    def _parse_card_file(path: Path) -> Optional[CardDefinition]:
        """Parse a forge-java card text file."""
        name = ""
        mana_cost = ""
        types = ""
        oracle = ""
        pt = ""
        try:
            for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
                if line.startswith("Name:"):
                    name = line[5:].strip()
                elif line.startswith("ManaCost:"):
                    mana_cost = line[9:].strip()
                elif line.startswith("Types:"):
                    types = line[6:].strip()
                elif line.startswith("Oracle:"):
                    oracle = line[7:].strip().replace("\\n", " / ")
                elif line.startswith("PT:"):
                    pt = line[3:].strip()
        except OSError:
            return None
        if not name:
            return None
        return CardDefinition(name=name, mana_cost=mana_cost, types=types, oracle=oracle, pt=pt or None)


def find_mentioned_cards(text: str, all_names: set[str]) -> set[str]:
    """Find card names mentioned in a text string."""
    found: set[str] = set()
    for name in all_names:
        if name in text:
            found.add(name)
    return found
