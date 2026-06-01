---
title: 'deploy-cloud.sh: systemd --user restart fails over SSH (''Failed to connect to bus'') → split-brain (new web, old binary)'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-01T06:55:30.051530888+00:00
updated_at: 2026-06-01T06:55:30.051530888+00:00
---

# Description

FINAL DEPLOY 2026-06-01 (main 18b2941d): scripts/deploy-cloud.sh rsynced web/+binary fine but the 'systemctl --user restart deepscry' step FAILED with 'Failed to connect to bus: No medium found' (exit 1). Result = SPLIT-BRAIN: new content-addressed web/ live (new native_game/index/deck_editor hashes, catalog, lobby Register) but /health still old binary 9d125ae2 — the new lobby.html would send Register/SetDeck to an old server that can't parse them = broken. Coordinator recovered manually: ssh + 'XDG_RUNTIME_DIR=/run/user/$(id -u) systemctl --user restart deepscry.service' → now live sha=18b2941d. ROOT CAUSE: the deploy script's restart runs systemctl --user without XDG_RUNTIME_DIR/DBUS_SESSION_BUS_ADDRESS set for the non-interactive SSH session (and/or loginctl linger not enabled), so the user-bus isn't reachable. FIX deploy-cloud.sh restart step: export XDG_RUNTIME_DIR=/run/user/$(id -u) (and DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus) before systemctl --user, OR enable-linger once in 'config' phase, OR fall back to a non-failing restart + VERIFY /health sha post-restart (deploy should FAIL LOUDLY if /health sha != just-built sha, instead of exit-1-but-leave-old-binary-running). The post-deploy probe should assert health-sha==build-sha.
