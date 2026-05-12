---
title: 'Client: Submit bug report to server and display result'
status: closed
priority: 0
issue_type: task
created_at: 2026-04-04T02:16:36.511556309+00:00
updated_at: 2026-05-12T13:57:57.515118038+00:00
closed_at: 2026-05-12T13:57:57.515117938+00:00
---

# Description

Files: mtg-forge-rs/web/fancy.html

Action: Wire up the bug report form submission:
1. Send bug report data over the existing WebSocket connection as a structured message (matching the server's expected format from server-bug-report-infra)
2. Include: user text, game logs, console logs, optional trusted password
3. Handle server response:
   a. On success: display the GitHub issue URL to the user (clickable link)
   b. On failure: display error message
4. Reset the form after submission

Why: Complete the client-server integration for bug reports.

Verify:
- Bug report submits over WebSocket
- Success response shows GitHub issue URL
- Error handling works for failed submissions
- Form resets after submission
