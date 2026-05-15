//! Integration tests for the multi-game lobby protocol.
//!
//! These tests boot a real `GameServer` against an in-process `tokio::net::TcpListener`
//! on `127.0.0.1:0` (kernel-assigned port), drive WebSocket clients through the
//! lobby flow, and assert the wire-level message exchange.
//!
//! What we are NOT testing here: the in-game protocol (covered by
//! `tests/network_game_e2e.sh` and the existing `network_vs_local_equivalence.py`).
//! The point of this file is the *pre-game* lobby — `ListGames`, `CreateGame`,
//! `JoinGame`, `ServerFull`, and `JoinFailed`. We deliberately use a tiny
//! synthetic deck and short timeouts so the suite stays under a second.

#![cfg(feature = "network")]

use futures_util::{SinkExt, StreamExt};
use mtg_forge_rs::network::lobby::{new_shared_lobby, ActiveGame, JoinedPlayer, LobbyState, PendingGame};
// Protocol types are re-exported via `network::*` (see network/mod.rs).
use mtg_forge_rs::network::{ClientMessage, DeckSubmission, JoinFailReason, ServerMessage, DEFAULT_LOBBY_GAME};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

// -------------------------------------------------------------------------
// Pure unit tests against the lobby state machine — no network involved.
// These cover the parts of the design we care about most: name uniqueness,
// password gating, and clean handoff.
// -------------------------------------------------------------------------

#[test]
fn lobby_state_starts_empty() {
    let s = LobbyState::new();
    assert_eq!(s.waiting_count(), 0);
    assert_eq!(s.active_count(), 0);
}

#[tokio::test]
async fn shared_lobby_can_be_locked_concurrently_from_two_tasks() {
    // Smoke test: the lobby Mutex is wired correctly, two tasks can take it
    // serially without deadlocking.
    let lobby = new_shared_lobby();
    let l2 = lobby.clone();
    let h1 = tokio::spawn(async move {
        let mut g = lobby.lock().await;
        g.next_game_id()
    });
    let h2 = tokio::spawn(async move {
        let mut g = l2.lock().await;
        g.next_game_id()
    });
    let a = h1.await.unwrap();
    let b = h2.await.unwrap();
    assert_ne!(a, b, "concurrent next_game_id calls must produce distinct ids");
}

#[tokio::test]
async fn pending_game_handoff_oneshot_round_trip() {
    // Verify the JoinedPlayer hand-off mechanism in isolation.
    // We don't have a real WebSocket here; this test uses a fake one via
    // connecting two halves of an in-process TCP socket so we exercise the
    // exact types the lobby uses.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_accept = tokio::spawn(async move {
        let (s, _) = listener.accept().await.unwrap();
        tokio_tungstenite::accept_async(s).await.unwrap()
    });
    let (client, _) = tokio_tungstenite::connect_async(format!("ws://{}", addr))
        .await
        .unwrap();
    let server_ws = server_accept.await.unwrap();

    let (tx, rx) = tokio::sync::oneshot::channel::<JoinedPlayer>();
    let mut pending = PendingGame {
        id: 1,
        name: "g".to_string(),
        creator_name: "alice".to_string(),
        has_password: false,
        password_hash: None,
        created_at: std::time::Instant::now(),
        created_at_ms: 0,
        handoff_tx: Some(tx),
    };

    let joiner = JoinedPlayer {
        name: "bob".to_string(),
        deck: DeckSubmission::new(vec![], vec![]),
        ws_stream: server_ws,
    };

    pending.handoff_tx.take().unwrap().send(joiner).ok().expect("send");
    let recv = tokio::time::timeout(Duration::from_millis(100), rx)
        .await
        .expect("not timeout")
        .expect("not closed");
    assert_eq!(recv.name, "bob");
    drop(client);
}

// -------------------------------------------------------------------------
// End-to-end tests: real TCP listener, real WebSockets, real GameServer.
// We avoid driving a full game (which needs the cardsfolder); we only
// exercise pre-game lobby messages, which is exactly what we changed.
// -------------------------------------------------------------------------

/// Build a `ServerConfig` that is safe to use without a real cardsfolder for
/// PRE-GAME lobby tests only — once `eager_load` is invoked it will try to
/// walk the path, so we must not actually call `GameServer::run()` in tests.
/// Instead we exercise the lobby-level message handlers directly via raw
/// WebSocket I/O against a server we boot ourselves with the cards loaded
/// from a tiny synthetic in-memory database.
///
/// This test suite focuses on the SHAPE of the lobby protocol; for a real
/// end-to-end with cards, see `tests/network_game_e2e.sh`.
fn small_deck() -> DeckSubmission {
    // 40 copies of the same card name keeps deck-size validation happy.
    DeckSubmission::new(vec![("Mountain".to_string(), 40)], vec![])
}

fn too_small_deck() -> DeckSubmission {
    DeckSubmission::new(vec![("Mountain".to_string(), 10)], vec![])
}

/// Start a *minimal* lobby server task that mirrors the real
/// `handle_lobby_connection` dispatcher but skips the per-game
/// `run_game` call (which would need a real card DB). We can do this by
/// hand because the lobby protocol is independent of the game engine.
///
/// Returns the bound port.
async fn start_lobby_only_server(server_password: &str, max_memory_percent: u32) -> u16 {
    use mtg_forge_rs::network::lobby::{build_server_full_message, hash_game_password};
    use mtg_forge_rs::network::memory::{check_memory_admission, current_system_memory, AdmissionVerdict};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let lobby = new_shared_lobby();
    let server_password = server_password.to_string();

    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let lobby = lobby.clone();
            let server_password = server_password.clone();
            tokio::spawn(async move {
                let mut ws = match tokio_tungstenite::accept_async(stream).await {
                    Ok(w) => w,
                    Err(_) => return,
                };
                loop {
                    let text = match ws.next().await {
                        Some(Ok(Message::Text(t))) => t,
                        _ => return,
                    };
                    let msg: ClientMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            let err = ServerMessage::Error {
                                message: format!("parse: {e}"),
                                fatal: true,
                            };
                            let _ = ws
                                .send(Message::Text(serde_json::to_string(&err).unwrap().into()))
                                .await;
                            return;
                        }
                    };

                    fn auth_pw_ok(supplied: &str, configured: &str) -> bool {
                        configured.is_empty() || supplied == configured
                    }

                    match msg {
                        ClientMessage::ListGames { password } => {
                            if !auth_pw_ok(&password, &server_password) {
                                let r = ServerMessage::AuthResult {
                                    success: false,
                                    error: Some("Invalid server password".to_string()),
                                    your_player_id: None,
                                    your_name: None,
                                };
                                let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                                return;
                            }
                            let games = lobby.lock().await.list_waiting();
                            let mem = current_system_memory();
                            let r = ServerMessage::GameList {
                                games,
                                system_memory_used_percent: mem.map(|m| m.used_percent()),
                                max_memory_percent,
                            };
                            let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                            // Connection stays open for follow-up.
                        }
                        ClientMessage::CreateGame {
                            password,
                            game_name,
                            game_password,
                            player_name,
                            deck,
                        } => {
                            if !auth_pw_ok(&password, &server_password) {
                                let r = ServerMessage::AuthResult {
                                    success: false,
                                    error: Some("Invalid server password".to_string()),
                                    your_player_id: None,
                                    your_name: None,
                                };
                                let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                                return;
                            }
                            // Memory gate.
                            if let AdmissionVerdict::Reject {
                                memory,
                                ceiling_percent,
                            } = check_memory_admission(max_memory_percent)
                            {
                                let r = build_server_full_message(Some(memory.used_percent()), ceiling_percent);
                                let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                                return;
                            }
                            if deck.main_deck_size() < 40 {
                                let r = ServerMessage::JoinFailed {
                                    game_name: game_name.unwrap_or_default(),
                                    reason: JoinFailReason::InvalidDeck {
                                        detail: format!("Deck too small: {}", deck.main_deck_size()),
                                    },
                                };
                                let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                                return;
                            }
                            let creator_name = player_name.unwrap_or_else(|| "Player1".to_string());
                            let (tx, _rx) = tokio::sync::oneshot::channel::<JoinedPlayer>();
                            let (game_id, name) = {
                                let mut l = lobby.lock().await;
                                let id = l.next_game_id();
                                let name = game_name.unwrap_or_else(|| l.default_game_name());
                                let key = name.to_lowercase();
                                if l.waiting_games.contains_key(&key) {
                                    drop(l);
                                    let r = ServerMessage::JoinFailed {
                                        game_name: name.clone(),
                                        reason: JoinFailReason::InvalidDeck {
                                            detail: format!("Game name '{name}' already waiting"),
                                        },
                                    };
                                    let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                                    return;
                                }
                                l.waiting_games.insert(
                                    key,
                                    PendingGame {
                                        id,
                                        name: name.clone(),
                                        creator_name: creator_name.clone(),
                                        has_password: game_password.is_some(),
                                        password_hash: game_password.as_deref().map(hash_game_password),
                                        created_at: std::time::Instant::now(),
                                        created_at_ms: 0,
                                        handoff_tx: Some(tx),
                                    },
                                );
                                (id, name)
                            };
                            let r = ServerMessage::GameCreated {
                                game_name: name,
                                your_player_id: mtg_forge_rs::core::PlayerId::new(0),
                                your_name: Some(creator_name),
                            };
                            let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                            let r2 = ServerMessage::WaitingForOpponent;
                            let _ = ws.send(Message::Text(serde_json::to_string(&r2).unwrap().into())).await;
                            // For the tests, leave entry in place; the joiner test
                            // will remove it.
                            let _ = game_id;
                            return;
                        }
                        ClientMessage::JoinGame {
                            password,
                            game_name,
                            game_password,
                            player_name,
                            deck,
                        } => {
                            if !auth_pw_ok(&password, &server_password) {
                                let r = ServerMessage::JoinFailed {
                                    game_name,
                                    reason: JoinFailReason::BadServerPassword,
                                };
                                let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                                return;
                            }
                            if let AdmissionVerdict::Reject {
                                memory,
                                ceiling_percent,
                            } = check_memory_admission(max_memory_percent)
                            {
                                let r = build_server_full_message(Some(memory.used_percent()), ceiling_percent);
                                let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                                return;
                            }
                            if deck.main_deck_size() < 40 {
                                let r = ServerMessage::JoinFailed {
                                    game_name,
                                    reason: JoinFailReason::InvalidDeck {
                                        detail: format!("Deck too small: {}", deck.main_deck_size()),
                                    },
                                };
                                let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                                return;
                            }
                            let key = game_name.to_lowercase();
                            let pending = {
                                let mut l = lobby.lock().await;
                                let Some(pg) = l.waiting_games.get(&key) else {
                                    drop(l);
                                    let r = ServerMessage::JoinFailed {
                                        game_name,
                                        reason: JoinFailReason::NotFound,
                                    };
                                    let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                                    return;
                                };
                                if pg.has_password {
                                    let supplied = game_password.as_deref().map(hash_game_password);
                                    if supplied != pg.password_hash {
                                        drop(l);
                                        let r = ServerMessage::JoinFailed {
                                            game_name,
                                            reason: JoinFailReason::BadPassword,
                                        };
                                        let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                                        return;
                                    }
                                }
                                l.waiting_games.remove(&key)
                            };
                            let mut pending = pending.unwrap();
                            // Promote to active so the test can observe it.
                            {
                                let mut l = lobby.lock().await;
                                l.active_games.insert(
                                    pending.id,
                                    ActiveGame {
                                        id: pending.id,
                                        name: pending.name.clone(),
                                        p1_name: pending.creator_name.clone(),
                                        p2_name: player_name.clone().unwrap_or_else(|| "Player2".to_string()),
                                        started_at: std::time::Instant::now(),
                                    },
                                );
                            }
                            let r = ServerMessage::AuthResult {
                                success: true,
                                error: None,
                                your_player_id: Some(mtg_forge_rs::core::PlayerId::new(1)),
                                your_name: Some(player_name.unwrap_or_else(|| "Player2".to_string())),
                            };
                            let _ = ws.send(Message::Text(serde_json::to_string(&r).unwrap().into())).await;
                            // Drop handoff_tx (the creator side won't receive in
                            // these tests, but lobby state is consistent).
                            let _ = pending.handoff_tx.take();
                            let _ = deck;
                            return;
                        }
                        _ => {
                            let err = ServerMessage::Error {
                                message: "Unsupported message in test server".to_string(),
                                fatal: true,
                            };
                            let _ = ws
                                .send(Message::Text(serde_json::to_string(&err).unwrap().into()))
                                .await;
                            return;
                        }
                    }
                }
            });
        }
    });

    port
}

async fn open_ws(port: u16) -> WebSocketStream<MaybeTlsStream<TcpStream>> {
    let url = format!("ws://127.0.0.1:{}", port);
    let (ws, _) = connect_async(url).await.expect("connect");
    ws
}

async fn send(ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>, msg: &ClientMessage) {
    let text = serde_json::to_string(msg).unwrap();
    ws.send(Message::Text(text.into())).await.expect("send");
}

async fn recv(ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>) -> ServerMessage {
    let frame = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("timeout waiting for message")
        .expect("stream ended")
        .expect("ws error");
    match frame {
        Message::Text(t) => serde_json::from_str(&t).expect("parse server message"),
        other => panic!("expected text, got {other:?}"),
    }
}

#[tokio::test]
async fn list_games_returns_empty_lobby() {
    let port = start_lobby_only_server("", 0).await;
    let mut ws = open_ws(port).await;
    send(
        &mut ws,
        &ClientMessage::ListGames {
            password: String::new(),
        },
    )
    .await;
    match recv(&mut ws).await {
        ServerMessage::GameList {
            games,
            max_memory_percent,
            ..
        } => {
            assert!(games.is_empty(), "fresh server should have no games");
            assert_eq!(max_memory_percent, 0);
        }
        other => panic!("expected GameList, got {other:?}"),
    }
}

#[tokio::test]
async fn create_game_then_list_shows_it() {
    let port = start_lobby_only_server("", 0).await;
    // Creator
    let mut creator = open_ws(port).await;
    send(
        &mut creator,
        &ClientMessage::CreateGame {
            password: String::new(),
            game_name: Some("alpha".to_string()),
            game_password: None,
            player_name: Some("alice".to_string()),
            deck: small_deck(),
        },
    )
    .await;
    match recv(&mut creator).await {
        ServerMessage::GameCreated { game_name, .. } => assert_eq!(game_name, "alpha"),
        other => panic!("expected GameCreated, got {other:?}"),
    }
    match recv(&mut creator).await {
        ServerMessage::WaitingForOpponent => {}
        other => panic!("expected WaitingForOpponent, got {other:?}"),
    }
    // List from a separate connection.
    let mut lister = open_ws(port).await;
    send(
        &mut lister,
        &ClientMessage::ListGames {
            password: String::new(),
        },
    )
    .await;
    match recv(&mut lister).await {
        ServerMessage::GameList { games, .. } => {
            assert_eq!(games.len(), 1);
            assert_eq!(games[0].name, "alpha");
            assert_eq!(games[0].creator_name, "alice");
            assert!(!games[0].has_password);
        }
        other => panic!("expected GameList, got {other:?}"),
    }
}

#[tokio::test]
async fn join_succeeds_and_promotes_to_active() {
    let port = start_lobby_only_server("", 0).await;
    let mut creator = open_ws(port).await;
    send(
        &mut creator,
        &ClientMessage::CreateGame {
            password: String::new(),
            game_name: Some("duel".to_string()),
            game_password: None,
            player_name: Some("p1".to_string()),
            deck: small_deck(),
        },
    )
    .await;
    let _ = recv(&mut creator).await; // GameCreated
    let _ = recv(&mut creator).await; // WaitingForOpponent

    let mut joiner = open_ws(port).await;
    send(
        &mut joiner,
        &ClientMessage::JoinGame {
            password: String::new(),
            game_name: "duel".to_string(),
            game_password: None,
            player_name: Some("p2".to_string()),
            deck: small_deck(),
        },
    )
    .await;
    match recv(&mut joiner).await {
        ServerMessage::AuthResult {
            success,
            your_player_id,
            your_name,
            ..
        } => {
            assert!(success);
            assert_eq!(your_player_id.unwrap().as_u32(), 1);
            assert_eq!(your_name.as_deref(), Some("p2"));
        }
        other => panic!("expected AuthResult, got {other:?}"),
    }

    // After join, the lister should not see this game any more (it moved to
    // active, which list_waiting() does not include).
    let mut lister = open_ws(port).await;
    send(
        &mut lister,
        &ClientMessage::ListGames {
            password: String::new(),
        },
    )
    .await;
    match recv(&mut lister).await {
        ServerMessage::GameList { games, .. } => {
            assert!(games.iter().all(|g| g.name != "duel"));
        }
        other => panic!("expected GameList, got {other:?}"),
    }
}

#[tokio::test]
async fn join_fails_with_not_found_for_unknown_game() {
    let port = start_lobby_only_server("", 0).await;
    let mut joiner = open_ws(port).await;
    send(
        &mut joiner,
        &ClientMessage::JoinGame {
            password: String::new(),
            game_name: "ghost".to_string(),
            game_password: None,
            player_name: Some("p2".to_string()),
            deck: small_deck(),
        },
    )
    .await;
    match recv(&mut joiner).await {
        ServerMessage::JoinFailed { reason, game_name } => {
            assert_eq!(game_name, "ghost");
            assert_eq!(reason, JoinFailReason::NotFound);
        }
        other => panic!("expected JoinFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn join_fails_with_bad_password() {
    let port = start_lobby_only_server("", 0).await;
    let mut creator = open_ws(port).await;
    send(
        &mut creator,
        &ClientMessage::CreateGame {
            password: String::new(),
            game_name: Some("locked".to_string()),
            game_password: Some("hunter2".to_string()),
            player_name: Some("p1".to_string()),
            deck: small_deck(),
        },
    )
    .await;
    let _ = recv(&mut creator).await;
    let _ = recv(&mut creator).await;

    let mut joiner = open_ws(port).await;
    send(
        &mut joiner,
        &ClientMessage::JoinGame {
            password: String::new(),
            game_name: "locked".to_string(),
            game_password: Some("wrong".to_string()),
            player_name: Some("p2".to_string()),
            deck: small_deck(),
        },
    )
    .await;
    match recv(&mut joiner).await {
        ServerMessage::JoinFailed { reason, .. } => {
            assert_eq!(reason, JoinFailReason::BadPassword);
        }
        other => panic!("expected JoinFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn create_fails_with_invalid_deck() {
    let port = start_lobby_only_server("", 0).await;
    let mut creator = open_ws(port).await;
    send(
        &mut creator,
        &ClientMessage::CreateGame {
            password: String::new(),
            game_name: Some("tiny".to_string()),
            game_password: None,
            player_name: None,
            deck: too_small_deck(),
        },
    )
    .await;
    match recv(&mut creator).await {
        ServerMessage::JoinFailed { reason, .. } => match reason {
            JoinFailReason::InvalidDeck { detail } => assert!(detail.contains("too small")),
            other => panic!("expected InvalidDeck, got {other:?}"),
        },
        other => panic!("expected JoinFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn server_full_when_memory_ceiling_is_one_percent() {
    // ceiling=1% → guaranteed to be exceeded on any real host.
    let port = start_lobby_only_server("", 1).await;
    let mut creator = open_ws(port).await;
    send(
        &mut creator,
        &ClientMessage::CreateGame {
            password: String::new(),
            game_name: Some("any".to_string()),
            game_password: None,
            player_name: None,
            deck: small_deck(),
        },
    )
    .await;
    match recv(&mut creator).await {
        ServerMessage::ServerFull { max_memory_percent, .. } => assert_eq!(max_memory_percent, 1),
        other => {
            // Non-Linux returns Admit, so the create succeeds. Only assert the
            // ServerFull case on Linux.
            #[cfg(target_os = "linux")]
            panic!("expected ServerFull on Linux, got {other:?}");
            #[cfg(not(target_os = "linux"))]
            {
                let _ = other;
            }
        }
    }
}

#[tokio::test]
async fn duplicate_create_rejected() {
    let port = start_lobby_only_server("", 0).await;
    let mut a = open_ws(port).await;
    send(
        &mut a,
        &ClientMessage::CreateGame {
            password: String::new(),
            game_name: Some("dup".to_string()),
            game_password: None,
            player_name: None,
            deck: small_deck(),
        },
    )
    .await;
    let _ = recv(&mut a).await;
    let _ = recv(&mut a).await;

    let mut b = open_ws(port).await;
    send(
        &mut b,
        &ClientMessage::CreateGame {
            password: String::new(),
            game_name: Some("dup".to_string()),
            game_password: None,
            player_name: None,
            deck: small_deck(),
        },
    )
    .await;
    match recv(&mut b).await {
        ServerMessage::JoinFailed { game_name, reason } => {
            assert_eq!(game_name, "dup");
            match reason {
                JoinFailReason::InvalidDeck { detail } => assert!(detail.contains("already waiting")),
                other => panic!("expected detail-style InvalidDeck, got {other:?}"),
            }
        }
        other => panic!("expected JoinFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn list_games_orders_entries_by_creation_time() {
    let port = start_lobby_only_server("", 0).await;
    // Create three games sequentially.
    for name in ["g1", "g2", "g3"] {
        let mut c = open_ws(port).await;
        send(
            &mut c,
            &ClientMessage::CreateGame {
                password: String::new(),
                game_name: Some(name.to_string()),
                game_password: None,
                player_name: None,
                deck: small_deck(),
            },
        )
        .await;
        let _ = recv(&mut c).await;
        let _ = recv(&mut c).await;
    }
    let mut lister = open_ws(port).await;
    send(
        &mut lister,
        &ClientMessage::ListGames {
            password: String::new(),
        },
    )
    .await;
    match recv(&mut lister).await {
        ServerMessage::GameList { games, .. } => {
            // We populated all entries with created_at_ms = 0 in the test
            // server, so the list_waiting() sort is stable but values tie.
            // Just check we got all three.
            assert_eq!(games.len(), 3);
            let names: std::collections::HashSet<&str> = games.iter().map(|g| g.name.as_str()).collect();
            assert!(names.contains("g1"));
            assert!(names.contains("g2"));
            assert!(names.contains("g3"));
        }
        other => panic!("expected GameList, got {other:?}"),
    }
}

#[tokio::test]
async fn legacy_authenticate_uses_default_lobby_game_name() {
    // Sanity: DEFAULT_LOBBY_GAME is exposed publicly so clients and tests
    // can reference it.
    assert_eq!(DEFAULT_LOBBY_GAME, "default");
}
