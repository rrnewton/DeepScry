"""Prompt helpers for agentplay choice selection."""

from __future__ import annotations

import re
from typing import Any


_STEP_TO_PHASE = {
    "untap": "Beginning",
    "upkeep": "Beginning",
    "draw": "Beginning",
    "main1": "Pre-combat Main",
    "begincombat": "Combat",
    "declareattackers": "Combat",
    "declareblockers": "Combat",
    "combatdamage": "Combat",
    "endcombat": "Combat",
    "main2": "Post-combat Main",
    "end": "Ending",
    "cleanup": "Ending",
}

_MANA_ORDER = ("white", "blue", "black", "red", "green", "colorless")
_MANA_LABELS = {
    "white": "W",
    "blue": "U",
    "black": "B",
    "red": "R",
    "green": "G",
    "colorless": "C",
}


def build_choice_prompt(
    game_state: dict,
    choices: list[str],
    log_tail: str,
    goal: str = None,
) -> str:
    """Build the prompt sent to a headless agent for one MTG choice."""

    root = _snapshot_root(game_state)
    card_map = _build_card_map(root)
    players = _extract_players(root)
    zone_map = _extract_zone_map(root)
    turn = _as_dict(root.get("turn"))
    active_player = _normalize_scalar(turn.get("active_player"))
    priority_player = _normalize_scalar(turn.get("priority_player"))
    battlefield = _zone_cards(root.get("battlefield"))
    stack = _zone_cards(root.get("stack"))

    available_choices = _normalize_choices(choices)
    choice_lines = ["[0] pass"]
    choice_lines.extend(f"[{index}] {choice}" for index, choice in enumerate(available_choices, start=1))

    sections = [
        "You are choosing the next action in a deterministic MTG game.",
        "Pick the single strongest legal choice from the menu based on the current state, tempo, combat, mana, and likely follow-up turns.",
    ]

    if goal:
        sections.append(f"Goal directive: {goal.strip()}")

    sections.extend(
        [
            "",
            "Current game state:",
            _format_state_summary(root, players, zone_map, turn, active_player, priority_player, battlefield, stack, card_map),
            "",
            "Available choices:",
            "\n".join(choice_lines),
            "",
            "Recent game log:",
            log_tail.strip() if log_tail.strip() else "(no recent log lines)",
            "",
            "MTG rules context:",
            "- Turns move through beginning, main, combat, post-combat main, and ending.",
            "- A player normally plays lands only in their own main phase when the stack is empty and they have priority.",
            "- Spells and many abilities use the stack. If the stack is not empty, passing may allow a resolve; acting may add a response.",
            "- Priority passes back and forth. Two consecutive passes on an empty stack advance the step/phase; two passes on a non-empty stack resolve the top object.",
            "- In combat, attackers are declared before blockers, then damage happens. Evaluate lethal attacks, favorable trades, and crack-backs.",
            "",
            "Response format:",
            "Include the chosen choice number clearly in your response so automation can parse it.",
            "If you are not reporting a bug, put the choice number alone on the final line.",
            "If you notice the game engine behaving incorrectly according to the official MTG rules, add a section labeled BUG_REPORT at the end of your response describing: what happened, what should have happened per the rules, and which rule was violated.",
            "If you include a BUG_REPORT section, mention the choice number before that section.",
            "Do not output the choice text.",
        ]
    )

    return "\n".join(sections).strip() + "\n"


def parse_agent_response(response: str) -> int:
    """Extract the chosen menu index from an agent response."""

    if response is None:
        raise ValueError("response is None")

    text = response.strip()
    if not text:
        raise ValueError("response is empty")

    lines = [line.strip() for line in text.splitlines() if line.strip()]
    for line in reversed(lines):
        match = re.search(r"\b(\d+)\b", line)
        if match:
            return int(match.group(1))

    match = re.search(r"\b(\d+)\b", text)
    if match:
        return int(match.group(1))

    raise ValueError(f"could not parse choice number from response: {response!r}")


def _snapshot_root(game_state: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(game_state, dict):
        return {}
    nested = game_state.get("game_state")
    if isinstance(nested, dict):
        return nested
    return game_state


def _as_dict(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def _as_list(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def _normalize_scalar(value: Any) -> str | None:
    if value is None:
        return None
    if isinstance(value, (str, int, float, bool)):
        return str(value)
    if isinstance(value, dict):
        for key in ("id", "value", "index"):
            if key in value:
                return _normalize_scalar(value.get(key))
    return None


def _player_key(value: Any) -> str:
    return _normalize_scalar(value) or "?"


def _build_card_map(root: dict[str, Any]) -> dict[int, dict[str, Any]]:
    cards = root.get("cards")
    if not isinstance(cards, list):
        return {}
    result: dict[int, dict[str, Any]] = {}
    for index, entry in enumerate(cards):
        if isinstance(entry, dict):
            result[index] = entry
    return result


def _extract_players(root: dict[str, Any]) -> list[dict[str, Any]]:
    players = []
    for index, player in enumerate(_as_list(root.get("players"))):
        if not isinstance(player, dict):
            continue
        player_id = _normalize_scalar(player.get("id"))
        players.append(
            {
                "index": index,
                "id": player_id if player_id is not None else str(index),
                "name": str(player.get("name", f"Player {index + 1}")),
                "life": player.get("life", "?"),
                "mana_pool": _as_dict(player.get("mana_pool")),
                "combat_mana_pool": _as_dict(player.get("combat_mana_pool")),
            }
        )
    return players


def _extract_zone_map(root: dict[str, Any]) -> dict[str, dict[str, list[Any]]]:
    zone_map: dict[str, dict[str, list[Any]]] = {}
    for item in _as_list(root.get("player_zones")):
        if not isinstance(item, list) or len(item) != 2:
            continue
        player_ref, zones = item
        zone_map[_player_key(player_ref)] = _extract_named_zones(_as_dict(zones))
    return zone_map


def _extract_named_zones(zones: dict[str, Any]) -> dict[str, list[Any]]:
    return {
        "hand": _zone_cards(zones.get("hand")),
        "graveyard": _zone_cards(zones.get("graveyard")),
        "library": _zone_cards(zones.get("library")),
        "exile": _zone_cards(zones.get("exile")),
    }


def _zone_cards(zone: Any) -> list[Any]:
    if isinstance(zone, dict):
        cards = zone.get("cards")
        if isinstance(cards, list):
            return cards
    if isinstance(zone, list):
        return zone
    return []


def _format_state_summary(
    root: dict[str, Any],
    players: list[dict[str, Any]],
    zone_map: dict[str, dict[str, list[Any]]],
    turn: dict[str, Any],
    active_player: str | None,
    priority_player: str | None,
    battlefield: list[Any],
    stack: list[Any],
    card_map: dict[int, dict[str, Any]],
) -> str:
    lines = []
    decision_player = priority_player or active_player
    lines.extend(_format_turn_line(turn, players, active_player, priority_player))
    lines.append("Players:")
    lines.extend(_format_player_lines(players, zone_map, card_map, decision_player))
    lines.append("Battlefield:")
    battlefield_lines = _format_battlefield(players, battlefield, card_map)
    lines.extend(battlefield_lines if battlefield_lines else ["- (empty)"])
    lines.append("Stack:")
    stack_text = _format_card_list(stack, card_map, limit=8)
    lines.append(f"- {stack_text}")
    if not players and not root:
        lines.append("- Snapshot data missing or unrecognized.")
    return "\n".join(lines)


def _format_turn_line(
    turn: dict[str, Any],
    players: list[dict[str, Any]],
    active_player: str | None,
    priority_player: str | None,
) -> list[str]:
    turn_number = turn.get("turn_number", "?")
    step = str(turn.get("current_step", "?"))
    phase = _phase_name(step)
    active_name = _player_name_by_id(players, active_player)
    priority_name = _player_name_by_id(players, priority_player) if priority_player is not None else "None"
    return [
        (
            f"Turn: {turn_number} | Phase: {phase} | Step: {step} | "
            f"Active player: {active_name} | Priority: {priority_name}"
        )
    ]


def _format_player_lines(
    players: list[dict[str, Any]],
    zone_map: dict[str, dict[str, list[Any]]],
    card_map: dict[int, dict[str, Any]],
    decision_player: str | None,
) -> list[str]:
    lines = []
    for player in players:
        player_id = player["id"]
        zones = zone_map.get(player_id, {})
        hand_cards = zones.get("hand", [])
        if player_id == decision_player:
            hand = _format_card_list(hand_cards, card_map, limit=10)
        else:
            hand = f"{len(hand_cards)} hidden card(s)"
        graveyard = _format_card_list(zones.get("graveyard", []), card_map, limit=8)
        library_size = len(zones.get("library", []))
        exile_size = len(zones.get("exile", []))
        mana = _format_mana_pool(player.get("mana_pool", {}), player.get("combat_mana_pool", {}))
        lines.append(
            f"- {player['name']}: life {player['life']}, mana {mana}, hand {hand}, "
            f"graveyard {graveyard}, library {library_size} cards, exile {exile_size} cards"
        )
    if not lines:
        return ["- (no player data)"]
    return lines


def _format_battlefield(
    players: list[dict[str, Any]],
    battlefield: list[Any],
    card_map: dict[int, dict[str, Any]],
) -> list[str]:
    if not battlefield:
        return []
    grouped: dict[str, list[str]] = {}
    for card_ref in battlefield:
        card = _resolve_card(card_ref, card_map)
        controller = _player_key(card.get("controller")) if isinstance(card, dict) else "?"
        owner_name = _player_name_by_id(players, controller)
        grouped.setdefault(owner_name, []).append(_describe_card(card_ref, card_map))
    return [f"- {player}: {', '.join(cards[:12])}{' ...' if len(cards) > 12 else ''}" for player, cards in grouped.items()]


def _player_name_by_id(players: list[dict[str, Any]], player_id: str | None) -> str:
    if player_id is None:
        return "Unknown"
    for player in players:
        if player["id"] == player_id:
            return str(player["name"])
    if player_id.isdigit():
        return f"Player {int(player_id) + 1}"
    return f"Player {player_id}"


def _phase_name(step: str) -> str:
    return _STEP_TO_PHASE.get(step.replace("_", "").replace(" ", "").lower(), "Unknown")


def _format_mana_pool(mana_pool: dict[str, Any], combat_mana_pool: dict[str, Any]) -> str:
    values = []
    for color in _MANA_ORDER:
        amount = _intish(mana_pool.get(color))
        combat_amount = _intish(combat_mana_pool.get(color))
        total = amount + combat_amount
        if total:
            values.append(f"{_MANA_LABELS[color]}={total}")
    return "empty" if not values else " ".join(values)


def _format_card_list(cards: list[Any], card_map: dict[int, dict[str, Any]], limit: int) -> str:
    if not cards:
        return "(empty)"
    names = [_describe_card(card_ref, card_map) for card_ref in cards[:limit]]
    if len(cards) > limit:
        names.append(f"... +{len(cards) - limit} more")
    return ", ".join(names)


def _describe_card(card_ref: Any, card_map: dict[int, dict[str, Any]]) -> str:
    card = _resolve_card(card_ref, card_map)
    if isinstance(card, dict):
        name = str(card.get("name", card_ref))
        extras = []
        if card.get("tapped") is True:
            extras.append("tapped")
        if _intish(card.get("damage")):
            extras.append(f"damage={_intish(card.get('damage'))}")
        power = card.get("base_power")
        toughness = card.get("base_toughness")
        if power is not None and toughness is not None:
            extras.append(f"{power}/{toughness}")
        counters = card.get("counters")
        if isinstance(counters, list) and counters:
            extras.append(f"counters={len(counters)}")
        return f"{name} ({', '.join(extras)})" if extras else name
    return str(card_ref)


def _resolve_card(card_ref: Any, card_map: dict[int, dict[str, Any]]) -> Any:
    if isinstance(card_ref, int):
        return card_map.get(card_ref, card_ref)
    if isinstance(card_ref, str) and card_ref.isdigit():
        return card_map.get(int(card_ref), card_ref)
    if isinstance(card_ref, dict):
        return card_ref
    return card_ref


def _normalize_choices(choices: list[str]) -> list[str]:
    normalized = []
    for choice in choices:
        text = str(choice).strip()
        if not text:
            continue
        if text.lower() == "pass":
            continue
        normalized.append(text)
    return normalized


def _intish(value: Any) -> int:
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return int(value)
    if isinstance(value, str) and value.isdigit():
        return int(value)
    return 0
