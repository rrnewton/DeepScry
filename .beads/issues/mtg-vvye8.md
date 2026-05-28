---
title: 'Release protocol: pre-deploy smoke + deploy-from-main'
status: open
priority: 2
issue_type: task
created_at: 2026-05-28T16:46:16.847101101+00:00
updated_at: 2026-05-28T16:46:16.847101101+00:00
---

# Description

Establish the release protocol the user approved 2026-05-28: pre-deploy smoke test, then switch to deploying from `main` for ALL future deployments.

CURRENT STATE: scripts/deploy-cloud.sh deploys from the local primary checkout (= integration); `main` is ~140 commits behind and unused; there is a POST-deploy probe (run_post_deploy_probe, now with -k) but NO pre-deploy smoke gate.

TARGET PROTOCOL:
  integration green on CI
  -> PRE-DEPLOY SMOKE: build the release binary locally, boot it on a temp port, assert /health + index.json + a quick 2-client headless game, fail-fast BEFORE rsync (hermetic, local — distinct from the live post-deploy probe)
  -> promote integration -> main (ff-only; the integration ceremony; main protected)
  -> DEPLOY FROM main (deploy-cloud.sh builds/ships the main SHA, not integration)
  -> POST-deploy probe (existing).

PIECES:
1. Add a pre-deploy smoke step to deploy-cloud.sh (or a `make pre-deploy-smoke` invoked by it): local boot + health/game assertions; abort deploy on failure.
2. deploy-cloud.sh: deploy from `main` (checkout/build the main ref) instead of whatever the primary checkout is on; record the deployed SHA.
3. A promote-integration-to-main step (rebase/ff + CI-green gate) — or reuse the existing integration ceremony / ci-integration-monitor.

SEQUENCING: do this AFTER mtg-571 (trunk) lands — both edit deploy-cloud.sh, so serialize to avoid conflicts. Also gated on CI being stable (main-promotion needs green CI -> depends on the mtg-p9o5z desync fix). Until this lands, deploys continue from integration (status quo).

This unblocks DEPLOYING the already-merged native-layout GUI fix (it will ship via the first main-based deploy)."
