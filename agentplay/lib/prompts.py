"""Prompt helpers for agentplay choice selection."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Sequence


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


@dataclass(frozen=True)
class AgentDecision:
    """Parsed action request from an agent response."""

    choice_number: int | None
    raw_response: str
    bug_report: str | None = None

    @property
    def stopped_for_bug(self) -> bool:
        return self.choice_number is None


def format_deck_preamble(deck_entries: Sequence[tuple[str, Path]]) -> str:
    """Format deck lists as a preamble section.

    Each entry is ``(player_label, deck_path)``. Returns the empty string if no
    decks could be read. The preamble lists the raw deck file contents (counts
    plus card names) so the agent knows the full pool of cards each player can
    draw from.
    """

    sections: list[str] = []
    for label, path in deck_entries:
        try:
            text = path.read_text(encoding="utf-8")
        except OSError:
            continue
        # Track which section ([metadata], [main], [sideboard], ...) we're in.
        # We only want card lines from [main] and [sideboard]; metadata lines
        # like "Name=Foo" must be filtered out so they don't leak into the
        # preamble shown to the agent.
        cards: list[str] = []
        sideboard: list[str] = []
        section = ""
        for raw in text.splitlines():
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("[") and line.endswith("]"):
                section = line[1:-1].strip().lower()
                continue
            if section == "main":
                cards.append(line)
            elif section == "sideboard":
                sideboard.append(line)
            # Other sections (metadata, etc.) are ignored.
        if not cards and not sideboard:
            continue
        sections.append(f"{label} (deck: {path.name}):")
        sections.extend(f"  {c}" for c in cards)
        if sideboard:
            sections.append("  [Sideboard]")
            sections.extend(f"  {c}" for c in sideboard)
        sections.append("")
    if not sections:
        return ""
    return "\n".join(["Player decks:", "", *sections]).rstrip()


def build_intro_section(
    scenario: str | None = None,
    goal: str | None = None,
    bug_detection: bool = True,
    deck_preamble: str | None = None,
    rules_paths: list[str] | None = None,
) -> str:
    """Build the static intro/system-prompt portion of the agent prompt.

    This is the role/agenthood setup that is identical at every decision point:
    the role description, scenario, goal, deck list preamble, and rules
    references. It is suitable for echoing once at startup so a human can see
    exactly what the agent has been told.
    """

    sections = [
        "You are choosing the next action in a deterministic MTG game used for engine bug finding.",
    ]

    if bug_detection:
        sections.append(
            "At each decision point, either choose the strongest legal action or STOP when the log, state, or menu appears to violate MTG rules."
        )
    else:
        sections.append(
            "Pure play mode is enabled: choose the strongest legal action from the menu based on state, tempo, combat, mana, and likely follow-up turns."
        )

    sections.append(
        "IMPORTANT: The game engine only presents VALID, LEGAL actions. Every choice in the menu has already been validated by the engine: the mana cost is payable, targeting requirements are met, timing restrictions are satisfied, and any other gating conditions are already true. You do NOT need to re-derive legality. Focus your reasoning on STRATEGY (which valid action is best given the state, tempo, combat, mana, and likely follow-up turns?)"
        + (" and BUG DETECTION (is the engine wrongly offering an action, missing one it should offer, or describing the state/log in a way that contradicts MTG rules?)." if bug_detection else ".")
    )

    if scenario:
        sections.append(f"Scenario to reproduce: {scenario.strip()}")
        sections.append("Keep this scenario in mind and prefer legal choices that move the game toward reproducing it.")

    if goal:
        sections.append(f"Goal directive: {goal.strip()}")

    if deck_preamble and deck_preamble.strip():
        sections.extend(["", deck_preamble.strip()])

    if rules_paths:
        sections.extend(["", "MTG rules references (read for detailed rules):"])
        for rp in rules_paths:
            sections.append(f"  {rp}")

    return "\n".join(sections)


def build_choice_prompt(
    game_state: dict[str, Any],
    choices: list[str],
    log_since_last_decision: str,
    goal: str | None = None,
    scenario: str | None = None,
    interleaved_history: str | None = None,
    previous_decision: str | None = None,
    card_definitions: str | None = None,
    rules_paths: list[str] | None = None,
    bug_detection: bool = True,
    deck_preamble: str | None = None,
) -> str:
    """Build the prompt sent to a headless agent for one MTG choice.

    `game_state` is the structured `GameSnapshot` JSON shape produced by
    `mtg tui --snapshot-output` / `--tui-snapshot-path` (i.e. an outer
    `GameSnapshot` dict with a nested `game_state` field). For drivers that
    don't have that JSON shape (e.g. the WASM bridge, which has a `GuiViewModel`
    instead), use `build_choice_prompt_with_summary` directly with a
    pre-formatted state summary string.
    """

    root = _snapshot_root(game_state)
    card_map = _build_card_map(root)
    players = _extract_players(root)
    zone_map = _extract_zone_map(root)
    turn = _as_dict(root.get("turn"))
    active_player = _normalize_scalar(turn.get("active_player"))
    priority_player = _normalize_scalar(turn.get("priority_player"))
    battlefield = _zone_cards(root.get("battlefield"))
    stack = _zone_cards(root.get("stack"))

    state_summary = _format_state_summary(
        root, players, zone_map, turn, active_player, priority_player, battlefield, stack, card_map
    )
    return build_choice_prompt_with_summary(
        state_summary=state_summary,
        choices=choices,
        log_since_last_decision=log_since_last_decision,
        goal=goal,
        scenario=scenario,
        interleaved_history=interleaved_history,
        previous_decision=previous_decision,
        card_definitions=card_definitions,
        rules_paths=rules_paths,
        bug_detection=bug_detection,
        deck_preamble=deck_preamble,
    )


def build_choice_prompt_with_summary(
    state_summary: str,
    choices: list[str],
    log_since_last_decision: str,
    goal: str | None = None,
    scenario: str | None = None,
    interleaved_history: str | None = None,
    previous_decision: str | None = None,
    card_definitions: str | None = None,
    rules_paths: list[str] | None = None,
    bug_detection: bool = True,
    deck_preamble: str | None = None,
) -> str:
    """Build the prompt with a precomputed state-summary text block.

    This is the variant the WASM driver uses: the state summary is
    derived from `GuiViewModel` JSON (via `agentplay/lib/text_formatter.py`)
    rather than the `GameSnapshot` shape `_format_state_summary` consumes.
    Every other section (intro, history, log, choices, rules context,
    response format) is identical to what `build_choice_prompt` produces,
    so the LLM sees structurally-identical prompts across every driver.
    """

    available_choices = _normalize_choices(choices)
    choice_lines = ["[0] pass"]
    choice_lines.extend(f"[{index}] {choice}" for index, choice in enumerate(available_choices, start=1))

    intro = build_intro_section(
        scenario=scenario,
        goal=goal,
        bug_detection=bug_detection,
        deck_preamble=deck_preamble,
        rules_paths=rules_paths,
    )
    sections = [intro]

    # Card definitions (grow over time as new cards are seen, so kept in the
    # per-decision body rather than the static intro).
    if card_definitions:
        sections.extend(["", "Card definitions (cards seen in this game):", card_definitions])

    sections.extend(
        [
            "",
            "Current game state:",
            state_summary.strip() if state_summary.strip() else "(no game state available)",
            "",
            "Interleaved history so far:",
            interleaved_history.strip() if interleaved_history and interleaved_history.strip() else "(no prior decisions)",
            "",
            "Previous decision:",
            previous_decision.strip() if previous_decision and previous_decision.strip() else "(no previous decision)",
            "",
            "Game log since last decision:",
            log_since_last_decision.strip() if log_since_last_decision.strip() else "(no new log lines since the last decision)",
            "",
            "Available choices:",
            "\n".join(choice_lines),
            "",
            "MTG rules context:",
            "- Turns move through beginning, main, combat, post-combat main, and ending.",
            "- A player normally plays lands only in their own main phase when the stack is empty and they have priority.",
            "- Spells and many abilities use the stack. If the stack is not empty, passing may allow a resolve; acting may add a response.",
            "- Priority passes back and forth. Two consecutive passes on an empty stack advance the step/phase; two passes on a non-empty stack resolve the top object.",
            "- In combat, attackers are declared before blockers, then damage happens. Evaluate lethal attacks, favorable trades, and crack-backs.",
            "",
        ]
    )

    sections.extend(_response_format_lines(bug_detection))

    return "\n".join(sections).strip() + "\n"


def _response_format_lines(bug_detection: bool) -> list[str]:
    lines = [
        "",
        "Response format:",
    ]
    if bug_detection:
        lines.extend(
            [
                "To continue playing, explain the reason briefly and put the choice number alone on the final line.",
                "To report a gameplay bug, write STOP on its own line, then add a BUG_REPORT section.",
                "A BUG_REPORT must explain: observed behavior, expected behavior under MTG rules, relevant rule basis if known, and the log/state/menu evidence.",
                "Stop for illegal choices being offered, missing legal choices, state inconsistencies, wrong damage, impossible timing, or other engine-rule mismatches.",
                "Do not output a choice number when stopping for a bug.",
                "Do not output the choice text as the final line.",
            ]
        )
    else:
        lines.extend(
            [
                "Include the chosen choice number clearly in your response so automation can parse it.",
                "Put the choice number alone on the final line.",
                "Do not output the choice text as the final line.",
            ]
        )
    return lines


def parse_agent_decision(response: str, *, bug_detection: bool) -> AgentDecision:
    """Parse an agent response as either a choice number or a bug-stop."""

    bug_report = extract_bug_report(response)
    if bug_detection and _is_stop_response(response, bug_report):
        return AgentDecision(
            choice_number=None,
            raw_response=response.strip(),
            bug_report=bug_report or "(agent stopped for a suspected gameplay bug, but no BUG_REPORT details were provided)",
        )
    return AgentDecision(choice_number=parse_agent_response(response), raw_response=response.strip(), bug_report=bug_report)


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


def extract_bug_report(response: str) -> str | None:
    marker = "BUG_REPORT"
    if response is None or marker not in response:
        return None
    _, bug_report = response.split(marker, 1)
    return bug_report.lstrip(" :\n\t").strip() or "(BUG_REPORT marker present, but no details were provided)"


def _is_stop_response(response: str, bug_report: str | None) -> bool:
    if bug_report is not None:
        return True
    if response is None:
        return False
    lines = [line.strip().upper() for line in response.splitlines() if line.strip()]
    return any(line == "STOP" or line.startswith("STOP:") for line in lines)


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
