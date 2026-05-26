"""Convert WASM `GuiViewModel` JSON to text matching the basic text interface.

The native `mtg tui` text interface and the legacy stop-and-go agentplay mode
both produce a "Current game state:" section in the agent prompt by formatting
a structured `GameSnapshot` JSON via `agentplay/lib/prompts.py::_format_state_summary`.

The WASM bridge (`agentplay/lib/wasm_process.py`) doesn't have a `GameSnapshot`
— it has a different, schema-versioned `GuiViewModel` (see
`mtg-engine/src/wasm/gui_view_model.rs`). This module bridges the two so the
LLM sees structurally-identical "Current game state:" content across all
drivers.

The WASM `GuiViewModel` is the authoritative source for everything game.html
and tui_game.html render — using it here means the agent's textual view is
guaranteed to stay in sync with the visual GUI a human would see.

This module also exposes helpers for converting log entries (`LogEntryView`)
and choice menus (`ChoiceView`) so the WASM driver can recover game-log lines
in the same format the native driver collects from stdout.
"""

from __future__ import annotations

from typing import Any, Iterable

# Mirrors `_STEP_TO_PHASE` in `agentplay/lib/prompts.py` so the phase label we
# print for WASM games matches what the native driver prints.
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


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def view_model_to_state_summary(view_model: dict[str, Any]) -> str:
    """Render a `GuiViewModel` JSON dict as a "Current game state:" text block.

    The output shape mirrors `agentplay/lib/prompts.py::_format_state_summary`
    so that `build_choice_prompt_with_summary(view_model_to_state_summary(vm),
    ...)` produces a prompt that's structurally identical to what
    `build_choice_prompt(snapshot, ...)` produces in stop-and-go / persistent
    modes.

    Lines emitted (in order):

    - "Turn: N | Phase: P | Step: S | Active player: X | Priority: Y"
    - "Players:"
    - per-player: "- Name: life L, mana M, hand …, graveyard …, library N
      cards, exile N cards"  (opponent hand is rendered as "K hidden card(s)"
      to preserve the same information-hiding behaviour as the native path)
    - "Battlefield:"
    - per-controller groups: "- Player: card1, card2 (tapped), …"
    - "Stack:"
    - "- card1, card2, …" (or "(empty)")
    """

    if not isinstance(view_model, dict):
        return "(no game state available)"

    turn_number = view_model.get("turn_number", "?")
    current_step = str(view_model.get("current_step", "?"))
    phase = _STEP_TO_PHASE.get(_normalise_step(current_step), "Unknown")
    players = view_model.get("players") or []
    if not isinstance(players, list):
        players = []

    active_idx = view_model.get("active_player_idx")
    our_idx = view_model.get("our_player_idx")
    # The view model exposes `active_player_idx` (whose turn) but does NOT
    # carry a separate `priority_player_idx` field — the WASM TUI shows the
    # current pending choice from whoever has priority via `current_prompt`.
    # For the prompt-builder text we use `our_player_idx` (the perspective
    # the WASM session is rendering for) as the priority holder, which is
    # also the player the agent is making a decision for. This matches the
    # native driver's convention (priority_player == decision-maker).
    priority_idx = our_idx if our_idx is not None else active_idx

    active_name = _player_label(players, active_idx)
    priority_name = _player_label(players, priority_idx) if priority_idx is not None else "None"

    lines: list[str] = []
    lines.append(
        f"Turn: {turn_number} | Phase: {phase} | Step: {current_step} | "
        f"Active player: {active_name} | Priority: {priority_name}"
    )
    lines.append("Players:")
    lines.extend(_format_players(players, decision_idx=priority_idx))
    lines.append("Battlefield:")
    lines.extend(_format_battlefield(players))
    lines.append("Stack:")
    lines.append(f"- {_format_stack(view_model.get('stack'))}")
    return "\n".join(lines)


def view_model_log_lines(view_model: dict[str, Any]) -> list[str]:
    """Pull the (filtered, plain-text) game-log lines out of a view model.

    The view model carries `logs: [LogEntryView, ...]` — same content the GUI
    renders in its log pane. We drop `<Choice>` tracker entries (matching
    game.html's `renderLog` filter) so the agent sees the same log content a
    human watching tui_game.html / game.html would see.
    """

    logs = view_model.get("logs") if isinstance(view_model, dict) else None
    if not isinstance(logs, list):
        return []
    out: list[str] = []
    for entry in logs:
        if not isinstance(entry, dict):
            continue
        if entry.get("is_choice"):
            continue
        text = entry.get("text")
        if isinstance(text, str):
            out.append(text)
    return out


def strip_menu_prefix(text: str) -> str:
    """Strip the leading `[N] ` (or `[N]: `) menu prefix from a WASM choice
    text. The WASM `ChoiceView.text` field includes the menu number prefix
    (e.g. "[0] pass", "[1] play Maze of Ith") because the TUI renders it
    that way; the native CLI driver's `_extract_choices` strips that prefix
    before handing the choice list to the prompt builder, so we do the same
    here for parity.
    """

    s = text.lstrip()
    if not s.startswith("["):
        return text.strip()
    close = s.find("]")
    if close < 0:
        return text.strip()
    inside = s[1:close]
    if not inside.isdigit():
        return text.strip()
    rest = s[close + 1 :].lstrip(": ")
    return rest.strip()


def view_model_choices(view_model: dict[str, Any]) -> list[str]:
    """Return the actionable choices text in the order the prompt builder
    expects (excluding the implicit "pass" at index 0).

    The WASM `ChoiceView` already includes a `display_number` (1-based after
    pass), so we just take everything in `choices[].text` order. The WASM
    TUI emits the pass option as one of the choice entries (with text
    `"[0] pass"`), so we filter that out and strip the menu-number prefix
    from the others to match the native parser convention in
    `agentplay/lib/engine.py::_extract_choices`.
    """

    if not isinstance(view_model, dict):
        return []
    choices = view_model.get("choices")
    if not isinstance(choices, list):
        return []
    out: list[str] = []
    for c in choices:
        if not isinstance(c, dict):
            continue
        text = c.get("text")
        if not isinstance(text, str):
            continue
        cleaned = strip_menu_prefix(text)
        if cleaned.lower() == "pass":
            continue
        out.append(cleaned)
    return out


def view_model_priority_player(view_model: dict[str, Any]) -> str | None:
    """Return "p1"/"p2" for whichever player the WASM session is currently
    awaiting input from, or None if the session is mid-resolution / game over.

    The WASM bridge typically launches with `our_player_idx == 0` (the
    Python-driven side) and lets the engine handle the opponent
    automatically, so this normally returns "p1" whenever there is a
    pending choice. It still inspects `our_player_idx` so a future
    bidirectional WASM bridge (both seats human-driven) gets the right
    answer.
    """

    if not isinstance(view_model, dict):
        return None
    if not view_model.get("choices"):
        return None
    idx = view_model.get("our_player_idx")
    if not isinstance(idx, int):
        idx = view_model.get("active_player_idx")
    if not isinstance(idx, int):
        return None
    return f"p{idx + 1}"


def view_model_turn_number(view_model: dict[str, Any]) -> int | None:
    if not isinstance(view_model, dict):
        return None
    value = view_model.get("turn_number")
    return value if isinstance(value, int) else None


def view_model_choice_context(view_model: dict[str, Any]) -> str | None:
    if not isinstance(view_model, dict):
        return None
    ctx = view_model.get("choice_context")
    if isinstance(ctx, str) and ctx and ctx.lower() != "none":
        return ctx
    return None


def view_model_is_game_over(view_model: dict[str, Any]) -> bool:
    if not isinstance(view_model, dict):
        return False
    return bool(view_model.get("game_over"))


# ---------------------------------------------------------------------------
# Internals
# ---------------------------------------------------------------------------


def _normalise_step(step: str) -> str:
    return step.replace("_", "").replace(" ", "").lower()


def _player_label(players: list[Any], idx: Any) -> str:
    if not isinstance(idx, int):
        return "Unknown"
    if 0 <= idx < len(players) and isinstance(players[idx], dict):
        name = players[idx].get("name")
        if isinstance(name, str) and name:
            return name
    return f"Player {idx + 1}"


def _format_players(players: list[Any], *, decision_idx: Any) -> list[str]:
    if not players:
        return ["- (no player data)"]
    out: list[str] = []
    for idx, player in enumerate(players):
        if not isinstance(player, dict):
            continue
        name = player.get("name", f"Player {idx + 1}")
        life = player.get("life", "?")
        mana = _format_mana_pool(player.get("mana_pool") or {})
        hand_size = _intish(player.get("hand_size"))
        hand_cards = player.get("hand") or []
        is_us = bool(player.get("is_us"))
        # The WASM view model only fills `hand[]` for the local player.
        if isinstance(decision_idx, int) and idx == decision_idx and isinstance(hand_cards, list) and hand_cards:
            hand_text = _format_card_list(_card_descriptions(hand_cards), limit=10)
        elif is_us and isinstance(hand_cards, list) and hand_cards:
            hand_text = _format_card_list(_card_descriptions(hand_cards), limit=10)
        else:
            hand_text = f"{hand_size} hidden card(s)"
        graveyard_size = _intish(player.get("graveyard_size"))
        graveyard_cards = player.get("graveyard") or []
        if isinstance(graveyard_cards, list) and graveyard_cards:
            graveyard_text = _format_card_list(_card_descriptions(graveyard_cards), limit=8)
        else:
            graveyard_text = "(empty)" if graveyard_size == 0 else f"{graveyard_size} card(s)"
        library_size = _intish(player.get("library_size"))
        # The view model doesn't track exile size separately; default 0 for parity.
        exile_size = _intish(player.get("exile_size"))
        out.append(
            f"- {name}: life {life}, mana {mana}, hand {hand_text}, "
            f"graveyard {graveyard_text}, library {library_size} cards, exile {exile_size} cards"
        )
    return out or ["- (no player data)"]


def _format_battlefield(players: list[Any]) -> list[str]:
    grouped: dict[str, list[str]] = {}
    for player in players:
        if not isinstance(player, dict):
            continue
        name = player.get("name", "?")
        sections = player.get("battlefield_sections") or []
        if not isinstance(sections, list):
            continue
        all_cards: list[str] = []
        for section in sections:
            if not isinstance(section, dict):
                continue
            cards = section.get("cards") or []
            if isinstance(cards, list):
                all_cards.extend(_card_descriptions(cards))
        if all_cards:
            grouped[name] = all_cards
    if not grouped:
        return ["- (empty)"]
    return [
        f"- {player}: {', '.join(cards[:12])}{' ...' if len(cards) > 12 else ''}"
        for player, cards in grouped.items()
    ]


def _format_stack(stack: Any) -> str:
    if not isinstance(stack, list) or not stack:
        return "(empty)"
    names: list[str] = []
    for entry in stack[:8]:
        if isinstance(entry, dict):
            name = entry.get("name")
            if isinstance(name, str):
                names.append(name)
    if len(stack) > 8:
        names.append(f"... +{len(stack) - 8} more")
    return ", ".join(names) if names else "(empty)"


def _card_descriptions(cards: Iterable[Any]) -> list[str]:
    out: list[str] = []
    for card in cards:
        if not isinstance(card, dict):
            continue
        name = card.get("name")
        if not isinstance(name, str):
            continue
        extras: list[str] = []
        if card.get("is_tapped"):
            extras.append("tapped")
        damage = _intish(card.get("damage"))
        if damage:
            extras.append(f"damage={damage}")
        power = card.get("power")
        toughness = card.get("toughness")
        if power is not None and toughness is not None:
            extras.append(f"{power}/{toughness}")
        out.append(f"{name} ({', '.join(extras)})" if extras else name)
    return out


def _format_card_list(descriptions: list[str], limit: int) -> str:
    if not descriptions:
        return "(empty)"
    head = descriptions[:limit]
    if len(descriptions) > limit:
        head.append(f"... +{len(descriptions) - limit} more")
    return ", ".join(head)


def _format_mana_pool(mana: dict[str, Any]) -> str:
    parts: list[str] = []
    for color in _MANA_ORDER:
        amount = _intish(mana.get(color))
        if amount:
            parts.append(f"{_MANA_LABELS[color]}={amount}")
    return "empty" if not parts else " ".join(parts)


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
