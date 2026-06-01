---
title: 'TRACK: Lobby+server-protocol redesign to deployed prototype (AFK build 2026-05-31)'
status: open
priority: 1
issue_type: task
created_at: 2026-06-01T00:34:25.997406631+00:00
updated_at: 2026-06-01T01:04:12.502929005+00:00
---

# Description

## Status: PHASE 1 COMPLETE — awaiting make validate + merge

### Phase 1 deliverables (lobby-server-protocol branch):

**1. Authoritative lobby state (mtg-dw9j3):**
- ClientMessage::Register: unique-name reservation tied to WS connection; released on disconnect
- ServerMessage::RegisterResult: success/failure response
- WS-drop eviction: creator disconnect immediately removes waiting game from ListGames
- Per-player deck + ready state: WaitingPlayerState in PendingGame
- ClientMessage::SetDeck / SetReady: update server-side state, send WaitingRoomUpdate to both
- ServerMessage::WaitingRoomUpdate: snapshot pushed to both players on state change
- Reconnect tokens: ClientMessage::Reconnect + ServerMessage::ReconnectResult
  - ReconnectToken type (128-bit random hex), issued in GameStarted
  - Stored in ActiveGame (p1/p2_reconnect_token), validated via LobbyState::validate_reconnect_token
  - Phase 1 stub: token lifecycle fully implemented, in-game resume deferred to Phase 3

**2. Bug-report infallibility (mtg-obrx2):**
- validate_trusted_bug_report_password: Result<bool> → bool (infallible)
- Wrong password → untrusted (stored), not Err (rejected)
- Test updated: test_store_bug_report_stores_with_wrong_password_as_untrusted

**3. Help text rewrite (mtg-57hso):**
- mtg server: clear headless WS-only about
- mtg server-web: thorough long_about (all features documented)
- web_server/mod.rs: Python/dual-process docstring removed

**4. Deploy: trusted-password + cardsfolder dedup (mtg-obrx2):**
- render_systemd_unit: reads TRUSTED_BUG_REPORT_PASSWORD, adds flag when present
- render_env_file: writes password to env file
- cmd_config: warns-but-proceeds when password not set
- deepscry-deploy.env.example: documented TRUSTED_BUG_REPORT_PASSWORD var

Sub-issues: mtg-dw9j3 CLOSED, mtg-57hso CLOSED, mtg-obrx2 CLOSED
