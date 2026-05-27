---
title: Username uniqueness check is best-effort against waiting-game host names only
status: open
priority: 3
issue_type: bug
labels:
- web
- landing-page
created_at: 2026-05-27T18:33:46.478631388+00:00
updated_at: 2026-05-27T18:33:46.478631388+00:00
---

# Description

MAJOR finding from Playwright QA on commit d8b2448f.

nameIsTaken() in web/index.html only checks creator_name fields of games currently returned by ListGames. Two browsers can simultaneously claim the same username (verified). The commit body acknowledges that proper server-side enforcement requires a protocol extension.

Recommended: add ClientMessage::RegisterUsername (and ServerMessage::UsernameAccepted / Rejected) so the server tracks claimed names independently of game creation, and have the lobby send it on 'Enter Lobby'.
