---
title: 'Refactor HTML DAG: thin index.html to login + release-dispatch only; move lobby to a hashed child page'
status: open
priority: 3
issue_type: task
created_at: 2026-06-02T23:54:07.666054352+00:00
updated_at: 2026-06-02T23:54:07.666054352+00:00
---

# Description

User 2026-06-02 (BACKLOG / future, AFTER the mtg-4irju CAS rework lands). Today index.html serves BOTH login (enter name) AND lobby (list games / create game). Refactor so index.html is JUST the login page (enter name) + the release-token dispatcher to its immediate children via the asset manifest — VERY thin, with almost NO release-divergent functionality. Move the lobby (list/create games) into a separate HASHED child page (e.g. lobby.<hash>.html), reachable as a normal forward DAG edge from index.

WHY: index.html is the ONE mutable, no-cache resource. Minimizing its FUNCTIONALITY (not merely keeping it one file) means it rarely changes between releases, so the deferred multi-release dispatch (mtg-4irju 'DEFERRED' section) becomes trivial — the mutable root contains almost nothing that diverges across releases; nearly all behavior lives in immutable hashed pages. Relates mtg-4irju (CAS DAG / cache-hardening); do AFTER that lands. Design the mtg-4irju index.html dispatcher to be cleanly separable from the lobby so this split is easy.
