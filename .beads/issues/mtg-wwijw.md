---
title: 'Bug-report log artifacts don''t attach: gh gist create fails (fine-grained PAT can''t gist + empty game_logs.txt). Non-fatal — issue filing works.'
status: open
priority: 3
issue_type: bug
created_at: 2026-06-04T07:45:56.272401970+00:00
updated_at: 2026-06-04T07:45:56.272401970+00:00
---

# Description

NON-FATAL P3 follow-up to mtg-zvlpk (gh issue filing now works end-to-end — verified live, rrnewton/DeepScry#13). The optional GIST upload of the bug report's log artifacts (game_logs.txt + console_logs.txt) still fails; the issue files fine and its body falls back to "logs remain stored on the server". So log ATTACHMENT to issues doesn't work yet, but reporting does.

SYMPTOM (from the live verify): bug_report_issue_result.error =
  "command failed: /usr/bin/gh gist create /home/newton/deepscry/bug_reports/<ts>/game_logs.txt /home/newton/deepscry/bug_reports/<ts>/console_logs.txt -d MTG Forge bug report logs <abs>"
(paths are now ABSOLUTE — the mtg-zvlpk fix is working; this is a separate gist failure.)

CHARACTERIZATION (read-only ssh to the VM 178.156.252.200):
1. The VM's gh is authed with a FINE-GRAINED PAT (token prefix github_pat_11A…; `gh auth status` lists NO classic scopes and the X-OAuth-Scopes response header is EMPTY → fine-grained, not classic). Fine-grained PATs do NOT support `gh gist create` — gists are a user-level resource not covered by the fine-grained permission model. So even NON-empty logs would fail to gist with this token. (Issue creation DOES work with the FGPAT — issues:write is supported — which is why #13 filed.)
2. SECONDARY: game_logs.txt is 0 bytes for heuristic-vs-heuristic games (no human play; console_logs.txt was 461 bytes). `gh gist create` also rejects empty files. So an empty-file guard is needed regardless.

OPTIONS (non-secrets preferred):
A. (cleanest, no-secrets, removes the gist dependency entirely) DROP the gh-gist log upload; instead inline a BOUNDED tail of the logs directly into the issue body (e.g. last N KB of console/game logs in a collapsed <details> block), with a hard size cap. No gist, no extra token scope, logs visible on the issue. create_github_issue_with_runner already builds the issue body — append the log tails there and delete upload_bug_report_logs_with_runner.
B. Guard gh gist create to skip empty/missing files (small): filter report_dir.join("game_logs.txt")/("console_logs.txt") to those that exist AND are non-empty before passing to `gh gist create`; if none remain, skip the gist step (no warning). Sketch: in upload_bug_report_logs_with_runner, build the file-arg list by filtering std::fs::metadata(path).map(|m| m.len() > 0). NOTE: this alone does NOT fix attachment because the FGPAT still can't gist — it only removes the empty-file error noise.
C. (user-gated, do NOT auto-provision) switch the VM gh auth to a CLASSIC PAT with the `gist` scope (gh auth login / a token with gist) — then gist works. This is a secret change the USER must make; flag it, don't auto-modify auth.

RECOMMENDATION: Option A (inline bounded log tails into the issue body) — it's no-secrets, removes the fragile gist + spawn dependency, and actually surfaces the logs on the issue. Pair with a size cap. B is a cheap stopgap to silence the empty-file error if A is deferred. C only if the user specifically wants real gists.

Discovered + characterized by slot05 during the mtg-zvlpk live verification, 2026-06-04.
