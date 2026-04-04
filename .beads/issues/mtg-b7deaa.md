---
title: 'Client: Bug report button, form, and log capture in fancy.html'
status: open
priority: 0
issue_type: task
created_at: 2026-04-04T02:16:36.511219130+00:00
updated_at: 2026-04-04T02:16:36.511219130+00:00
---

# Description

Files: mtg-forge-rs/web/fancy.html

Action: Add bug report UI to the floating controls widget (the one toggled by #btn-toggle-controls):
1. Add a "Report Bug" button to the controls panel
2. Clicking it opens a bug report form/modal with:
   a. Text entry field with prompt: "Describe the expected behavior and the deviant behavior"
   b. Optional password field labeled "Trusted bug-report password"
   c. A "Submit" button and a "Cancel" button
3. On submit, capture:
   a. The user's text description
   b. Game logs from the active game (however they're currently stored/displayed)
   c. Chrome dev console logs if possible (use console override to buffer recent logs)
4. Show a loading/submitting state while waiting for server response

Why: Users need a UI to file bug reports directly from the game interface.

Verify:
- Bug report button visible in controls panel
- Form opens with correct fields
- Console log capture works (override console.log/error/warn to buffer)
- Game log capture pulls from existing game log display/storage
