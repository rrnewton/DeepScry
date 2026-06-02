---
title: 'Refactor HTML DAG: thin index.html to login + release-dispatch only; move lobby to a hashed child page'
status: open
priority: 3
issue_type: task
created_at: 2026-06-02T23:54:07.666054352+00:00
updated_at: 2026-06-02T23:55:31.579153837+00:00
---

# Description

User 2026-06-02 (BACKLOG / future, AFTER mtg-4irju). REFINED per team-lead advice + user request: make index.html a PURE FORWARDER — NO login UI at all, not even a login+dispatch hybrid. index.html's entire job: (1) determine release (release= param else baked latest-token), (2) resolve the entry child via the manifest, (3) location.replace() to it. ZERO UI, ZERO release-divergent logic.

Move BOTH the login (enter name) AND the lobby (list/create games) into HASHED child pages: login.<hash>.html (name entry) -> lobby.<hash>.html (list/create) -> launcher.<hash> -> game pages. All immutable, release-versioned.

WHY pure-forwarder (stronger than the original 'thin login+dispatch'): the login form/validation/lobby-protocol/styling are release-divergent; keeping them in the mutable no-cache index.html means changing login = changing the mutable root. Putting them in hashed children means every release's index.html is structurally IDENTICAL (same tiny forwarder, differing only in the baked latest-token + manifest hash). That makes the deferred multi-release dispatch (mtg-4irju) trivial — the forwarder is release-agnostic and can resolve ANY retained release's children; minimal cache-attack surface; even the login becomes a release=-pinnable immutable artifact (provenance).

TRADEOFF (accepted, mitigable): one redirect hop on fresh visit deepscry.net/ -> login.<hash>.html. Mitigate with location.replace() (keeps back-button clean) + a minimal inline loading state (logo, never changes) so no flash. Sub-100ms same-origin.

SEQUENCING: do AFTER mtg-4irju lands. Asked cas-dev to keep the mtg-4irju index.html dispatcher cleanly SEPARABLE from the lobby so this becomes a small follow-on, not a rework. Relates mtg-4irju.
