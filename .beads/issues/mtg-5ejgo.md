---
title: 'Bug-report widget hangs on submit: confirm disk-write first, timeout-bound github, install gh on VM'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-04T01:58:06.296710873+00:00
updated_at: 2026-06-04T01:58:06.296710873+00:00
---

# Description

USER-REPORTED 2026-06-04 (live deepscry.net game). The bug-report widget HUNG on submit (spinning ball, Chrome/Windows). Investigated the VM logs.

ROOT CAUSE (confirmed from server journal @01:46:35):
- store_bug_report SUCCEEDED: "Stored bug report from Some(0) in bug_reports/1780537595823". Disk write works.
- GitHub step FAILED INSTANTLY (not a hang): `gh auth status preflight failed ... No such file or directory (os error 2)` + same for labels fetch + gist upload + issue create. => the `gh` CLI is NOT INSTALLED on the VM, so Command::new("gh") returns ENOENT immediately.
- Server returned ServerMessage::BugReportResult { success:true, issue_url:None, error:Some("...gh not found...") } (submit_bug_report, server.rs:3838). So the SERVER responded fine and fast.
- => The HANG is CLIENT-SIDE: web/bug_report.js doesn't handle the "stored, but github failed / issue_url:None" case and leaves the spinner running (likely waits for an issue_url).

THREE FIXES (user's desired architecture: confirm disk-write IMMEDIATELY, THEN dicier github with a TIMEOUT, never a forever-spinner):

1. CLIENT (web/bug_report.js) — PRIMARY user-facing fix: on the server's stored-confirmation, STOP the spinner immediately and show "✓ Bug report saved to server". Handle github outcomes gracefully: issue_url present → show the link; github failed/absent/timed-out → show "saved ✓ (GitHub issue not filed: <reason>)", NOT a spinner. Add a CLIENT-SIDE timeout backstop (e.g. 15s) so a missing/late response can never leave a forever-spinner.

2. SERVER (server.rs submit_bug_report + protocol.rs) — two-phase + timeout: send an immediate BugReportStored confirmation right after store_bug_report succeeds (so the widget confirms disk-write WITHOUT waiting on github), THEN attempt github wrapped in a tokio::time::timeout (e.g. 10-15s) so a real network hang in some env can't block; send a follow-up BugReportIssueResult (issue_url | failed | timed-out). Requires a small protocol addition (a stored-ack message and/or a status field). Currently the single BugReportResult is sent only AFTER github completes (the design flaw the user called out, even though here github failed fast).

3. VM CONFIG (infra) — so issues actually file: install + `gh auth login` on the VM, OR (cleaner, removes the gh-binary dependency + gives a native timeout) switch create_github_issue to the GitHub REST API via reqwest with a GITHUB_TOKEN env + a request timeout. Either way, fold gh-install (or the token env) into scripts/deploy-cloud.sh `config` so a fresh VM provisions it. Recommend the reqwest+token route — no gh binary, native timeout, no spawn_blocking.

ACCEPTANCE: (a) widget shows "saved ✓" within ~1s of submit even with github down, never a forever-spinner; (b) a github hang is bounded by the timeout; (c) a real bug report files a GitHub issue from the VM (after gh-install or token). make validate green (test_bug_report.js extended for the stored-then-github two-phase + the github-failed-no-spinner case + a server timeout test). Repro context: the live submit that hung is bug_reports/1780537595823 on the VM.
