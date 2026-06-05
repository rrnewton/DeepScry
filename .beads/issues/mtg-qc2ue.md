---
title: 'NETARCH follow-up: fold library-search found-CardId into buffer/state-sync log, then remove ChoiceAccepted block (audit E2/Q2)'
status: open
priority: 3
issue_type: task
depends_on:
  mtg-o99ow: related
created_at: 2026-06-05T14:08:25.839751182+00:00
updated_at: 2026-06-05T14:08:25.839751182+00:00
---

# Description

From audit Q2/§E2. ChoiceAccepted (protocol.rs:864-887) is NOT a generic UI-wait handshake — it is a DATA-FETCH for one choice type: NetworkLocalController::choose_from_library is the ONLY method that blocks on it (local_controller.rs:843-863 via wait_for_choice_accepted) to obtain the server-authoritative library_search_result (the hidden CardId the tutor moved to hand). Every other choice is fire-and-forget. It is load-bearing TODAY (do NOT delete now). Under the buffer-as-sole-source model, fold the library-search found-CardId into the buffer/state-sync log at its true resolution action_count (the server already delivers the searcher's own found card at its true ac via collect_reveals_since_last_choice — see server.rs:3088-3109), then the block + the ChoiceAccepted message can be removed (or demoted to a debug-only receipt). GATED on the deep-AC reveal work (slot03-deepac2). Senders to retire afterward: server.rs:2632/2824/3111.
