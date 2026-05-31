---
title: Interactive UX for controller-driven ChoosePlayer (choose_player)
status: open
priority: 4
issue_type: task
created_at: 2026-05-31T11:48:23.334763305+00:00
updated_at: 2026-05-31T11:48:23.334763305+00:00
---

# Description

Follow-up from mtg-cuf0e (Black Vise). The new PlayerController::choose_player(view, valid_players) -> index method has a deterministic default (picks index 0) and is wired for API completeness; Black Vise's ETB choose-opponent is resolved at the engine level (GameState::pick_chosen_opponent, a deterministic public-state pick) because the as-enters replacement fires in set_card_zone below the controller layer, mirroring the existing ETB ChooseColor path.

When a card needs a genuinely interactive or AI-meaningful ChoosePlayer (3+ player games, or an activated/triggered DB$ ChoosePlayer where the controller is in scope), implement:
- a real interactive prompt (RichInputController / network ChoiceRequest) so a human picks the player, routed through the existing choice_indices Vec / ChoiceEntry (choice_seq) logged path (ONE index = chosen player), and
- a view-only Heuristic choose_player (e.g. AILogic$ MostCardsInHand evaluated from the controller's public view).

For 2-player as-enters ChoosePlayer (the only current consumer, Black Vise) the choice is forced (single opponent), so the engine-level deterministic pick is correct and information-independent today; no behavior gap. This issue tracks the richer multi-player / controller-driven path only.
