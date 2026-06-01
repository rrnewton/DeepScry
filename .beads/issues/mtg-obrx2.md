---
title: 'Deployed systemd unit: duplicate --cardsfolder, and --trusted-bug-report-password unset (bug-report upload disabled live)'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-31T21:37:21.294873034+00:00
updated_at: 2026-05-31T21:37:21.294873034+00:00
---

# Description

Audit of live ~/.config/systemd/user/deepscry.service (2026-05-31): ExecStart passes '--cardsfolder' TWICE and does NOT pass --trusted-bug-report-password. Effect: trusted_bug_report_password defaults to empty; server.rs:746 treats empty as 'bug-report upload DISABLED' (safe, not an auth hole — empty does NOT mean anyone-can-upload). But it means playtesters currently CANNOT upload bug-report snapshots. DECISION for user: if we want playtest bug reports, set a real --trusted-bug-report-password (store in <parent>/.deepscry-deploy.env, have scripts/deploy-cloud.sh config render it into the unit). Also dedupe the --cardsfolder arg in the unit template (scripts/deploy-cloud.sh config). No secret is currently deployed.
