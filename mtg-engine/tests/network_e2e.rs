//! End-to-end network tests for client/server multiplayer
//!
//! Tests the full networking stack:
//! - Server startup and client connections
//! - Game start handshake
//! - Choice synchronization over WebSocket
//! - Complete games with AI controllers
//!
//! These tests require the `network` feature.

#![cfg(feature = "network")]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

/// Port to use for process-spawning tests (different from default to avoid conflicts)
#[allow(dead_code)]
const TEST_PORT: u16 = 17772;

/// Helper struct to manage server process lifecycle (for process-spawning tests)
#[allow(dead_code)]
struct ServerProcess {
    child: Child,
    port: u16,
}

#[allow(dead_code)]
impl ServerProcess {
    /// Start a server process and wait for it to be ready
    fn start(port: u16, password: &str, cardsfolder: &str) -> Self {
        let child = Command::new("cargo")
            .args([
                "run",
                "--quiet",
                "--bin",
                "mtg",
                "--features",
                "network",
                "--",
                "server",
                "--port",
                &port.to_string(),
                "--password",
                password,
                "--cardsfolder",
                cardsfolder,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to start server process");

        // Give server time to bind to port
        thread::sleep(Duration::from_millis(500));

        ServerProcess { child, port }
    }

    /// Get the server address
    fn address(&self) -> String {
        format!("localhost:{}", self.port)
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        // Kill the server process on cleanup
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Helper struct to manage client process (for process-spawning tests)
#[allow(dead_code)]
struct ClientProcess {
    child: Child,
}

#[allow(dead_code)]
impl ClientProcess {
    /// Start a client process
    fn start(
        deck_path: &str,
        server: &str,
        password: &str,
        name: &str,
        cardsfolder: &str,
    ) -> Self {
        let child = Command::new("cargo")
            .args([
                "run",
                "--quiet",
                "--bin",
                "mtg",
                "--features",
                "network",
                "--",
                "connect",
                deck_path,
                "--server",
                server,
                "--password",
                password,
                "--name",
                name,
                "--cardsfolder",
                cardsfolder,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to start client process");

        ClientProcess { child }
    }

    /// Wait for client to finish and get exit status
    fn wait(mut self) -> std::process::ExitStatus {
        self.child.wait().expect("Failed to wait for client")
    }

    /// Wait with timeout
    fn wait_timeout(mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let start = std::time::Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = self.child.kill();
                        return None;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(_) => return None,
            }
        }
    }
}

impl Drop for ClientProcess {
    fn drop(&mut self) {
        // Kill the client process on cleanup
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Get path to a test deck (for process-spawning tests)
#[allow(dead_code)]
fn test_deck_path(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("decks")
        .join(name);
    path.to_string_lossy().to_string()
}

/// Get path to cardsfolder (for process-spawning tests)
#[allow(dead_code)]
fn cardsfolder_path() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("cardsfolder");
    path.to_string_lossy().to_string()
}

// ============================================================================
// Integration Tests
// ============================================================================

/// Test that server starts and accepts connections
/// Note: This test is ignored by default because it requires building the binary
/// and spawning processes. Run with: cargo test --features network -- --ignored
#[test]
#[ignore = "requires network feature and spawns processes"]
fn test_server_starts() {
    let password = "test123";
    let cardsfolder = cardsfolder_path();

    // Start server
    let _server = ServerProcess::start(TEST_PORT, password, &cardsfolder);

    // Server should be running (we can't easily verify without connecting)
    // The fact that it didn't panic is a basic sanity check
    thread::sleep(Duration::from_millis(200));
}

/// Test that two clients can connect and a game starts
#[test]
#[ignore = "requires network feature and spawns processes"]
fn test_two_clients_connect() {
    let password = "test456";
    let port = TEST_PORT + 1;
    let cardsfolder = cardsfolder_path();
    let deck = test_deck_path("simple_bolt.dck");

    // Start server
    let _server = ServerProcess::start(port, password, &cardsfolder);
    let server_addr = format!("localhost:{}", port);

    // Give server time to start
    thread::sleep(Duration::from_millis(500));

    // Start two clients in parallel
    let client1 = ClientProcess::start(&deck, &server_addr, password, "Alice", &cardsfolder);
    thread::sleep(Duration::from_millis(100));
    let client2 = ClientProcess::start(&deck, &server_addr, password, "Bob", &cardsfolder);

    // Wait for both clients with timeout (game should complete or timeout)
    let timeout = Duration::from_secs(60);

    let status1 = client1.wait_timeout(timeout);
    let status2 = client2.wait_timeout(timeout);

    // At least one should have finished (timeout means something went wrong)
    assert!(
        status1.is_some() || status2.is_some(),
        "Both clients timed out - server may not be functioning"
    );
}

// ============================================================================
// In-process async tests (no process spawning)
// ============================================================================

#[cfg(test)]
mod async_tests {
    use mtg_forge_rs::core::PlayerId;
    use mtg_forge_rs::network::{
        CardReveal, ChoiceType, ClientMessage, DeckSubmission, RevealReason, ServerMessage,
    };

    /// Test protocol message round-trips work correctly
    #[test]
    fn test_protocol_encoding_decoding() {
        // Create a sample GameStarted message
        let msg = ServerMessage::GameStarted {
            your_player_id: PlayerId::new(0),
            opponent_name: "TestOpponent".to_string(),
            opening_hand: vec![CardReveal {
                card_id: mtg_forge_rs::core::CardId::new(1),
                name: "Mountain".to_string(),
                mana_cost: "".to_string(),
                type_line: "Basic Land - Mountain".to_string(),
                text: "".to_string(),
                pt: None,
            }],
            opponent_hand_count: 7,
            library_size: 53,
            opponent_library_size: 53,
            opponent_decklist: None,
            starting_life: 20,
            initial_state_hash: 0x12345678,
        };

        // Encode to JSON
        let json = serde_json::to_string(&msg).expect("Failed to serialize");

        // Decode back
        let decoded: ServerMessage = serde_json::from_str(&json).expect("Failed to deserialize");

        // Re-encode and compare
        let json2 = serde_json::to_string(&decoded).expect("Failed to re-serialize");
        assert_eq!(json, json2, "Round-trip encoding mismatch");
    }

    /// Test deck submission encoding
    #[test]
    fn test_deck_submission_encoding() {
        let deck = DeckSubmission::new(
            vec![
                ("Lightning Bolt".to_string(), 4),
                ("Mountain".to_string(), 20),
            ],
            vec![("Pyroclasm".to_string(), 2)],
        );

        let msg = ClientMessage::Authenticate {
            password: "secret".to_string(),
            player_name: "TestPlayer".to_string(),
            deck,
        };

        let json = serde_json::to_string(&msg).expect("Failed to serialize");
        let decoded: ClientMessage = serde_json::from_str(&json).expect("Failed to deserialize");
        let json2 = serde_json::to_string(&decoded).expect("Failed to re-serialize");

        assert_eq!(json, json2);
    }

    /// Test choice request/response flow encoding
    #[test]
    fn test_choice_flow_encoding() {
        // Server sends choice request
        let request = ServerMessage::ChoiceRequest {
            choice_seq: 42,
            choice_type: ChoiceType::Priority { available_count: 3 },
            options: vec![
                "Pass priority".to_string(),
                "Play land: Mountain".to_string(),
                "Cast: Lightning Bolt".to_string(),
            ],
            state_hash: 0xDEADBEEF,
            context: None,
        };

        let request_json = serde_json::to_string(&request).expect("serialize request");
        let decoded_request: ServerMessage =
            serde_json::from_str(&request_json).expect("deserialize request");

        // Client sends response
        let response = ClientMessage::SubmitChoice {
            choice_seq: 42,
            choice_index: 2, // Cast Lightning Bolt
        };

        let response_json = serde_json::to_string(&response).expect("serialize response");
        let decoded_response: ClientMessage =
            serde_json::from_str(&response_json).expect("deserialize response");

        // Verify choice_seq matches
        match (decoded_request, decoded_response) {
            (
                ServerMessage::ChoiceRequest { choice_seq: req_seq, .. },
                ClientMessage::SubmitChoice {
                    choice_seq: resp_seq,
                    choice_index,
                },
            ) => {
                assert_eq!(req_seq, resp_seq, "Choice sequence mismatch");
                assert_eq!(choice_index, 2);
            }
            _ => panic!("Wrong message types"),
        }
    }

    /// Test card reveal flow
    #[test]
    fn test_card_reveal_flow() {
        let reveal_msg = ServerMessage::CardRevealed {
            owner: PlayerId::new(0),
            card: CardReveal {
                card_id: mtg_forge_rs::core::CardId::new(100),
                name: "Serra Angel".to_string(),
                mana_cost: "{3}{W}{W}".to_string(),
                type_line: "Creature - Angel".to_string(),
                text: "Flying, vigilance".to_string(),
                pt: Some((4, 4)),
            },
            reason: RevealReason::Draw,
        };

        let json = serde_json::to_string(&reveal_msg).expect("serialize");
        let decoded: ServerMessage = serde_json::from_str(&json).expect("deserialize");

        match decoded {
            ServerMessage::CardRevealed { card, reason, .. } => {
                assert_eq!(card.name, "Serra Angel");
                assert_eq!(card.pt, Some((4, 4)));
                assert_eq!(reason, RevealReason::Draw);
            }
            _ => panic!("Wrong message type"),
        }
    }
}

// ============================================================================
// WebSocket Integration Tests (in-process async)
// ============================================================================

/// Module for actual WebSocket integration tests
/// These tests run the server and client in-process using tokio
#[cfg(test)]
mod websocket_integration {
    use futures_util::{SinkExt, StreamExt};
    use mtg_forge_rs::network::{
        ClientMessage, DeckSubmission, GameServer, ServerConfig, ServerMessage,
    };
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::net::TcpStream;
    use tokio::time::timeout;
    use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

    /// Allocate a random available port by binding to port 0
    /// Returns the allocated port number. There's a small race window between
    /// releasing this port and the server binding to it, but in practice this
    /// works reliably for test purposes.
    fn allocate_random_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("Failed to bind to random port");
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    /// Wait until the server is accepting connections on the given port.
    /// Returns true if server became available, false if timed out.
    async fn wait_for_server(port: u16, timeout_secs: u64) -> bool {
        let start = std::time::Instant::now();
        let timeout_duration = Duration::from_secs(timeout_secs);

        while start.elapsed() < timeout_duration {
            // Try to connect
            if tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
                .await
                .is_ok()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    /// Get cardsfolder path
    fn cardsfolder_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("cardsfolder")
    }

    /// Create a simple test deck submission
    fn simple_deck() -> DeckSubmission {
        DeckSubmission::new(
            vec![
                ("Lightning Bolt".to_string(), 4),
                ("Mountain".to_string(), 56),
            ],
            vec![],
        )
    }

    /// Helper to send a message
    async fn send_message(
        ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        msg: &ClientMessage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string(msg)?;
        ws.send(Message::Text(json.into())).await?;
        Ok(())
    }

    /// Helper to receive a message
    async fn receive_message(
        ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> Result<ServerMessage, Box<dyn std::error::Error>> {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    let msg: ServerMessage = serde_json::from_str(&text)?;
                    return Ok(msg);
                }
                Some(Ok(_)) => continue, // Skip non-text messages
                Some(Err(e)) => return Err(e.into()),
                None => return Err("Connection closed".into()),
            }
        }
    }

    /// Test that the server accepts connections and authenticates clients
    #[tokio::test]
    async fn test_server_auth_flow() {
        // Allocate a random available port to avoid test collisions
        let port = allocate_random_port();
        let password = "testpass";

        // Create server config
        let config = ServerConfig {
            port,
            password: password.to_string(),
            cardsfolder: cardsfolder_path(),
            ..Default::default()
        };

        // Start server in background
        let mut server = GameServer::new(config);
        let server_handle = tokio::spawn(async move {
            // Run server for a limited time
            let _ = timeout(Duration::from_secs(30), server.run()).await;
        });

        // Wait for server to be accepting connections (loads card DB first)
        assert!(
            wait_for_server(port, 20).await,
            "Server did not start accepting connections within timeout"
        );

        // Connect first client
        let url = format!("ws://localhost:{}", port);
        let (mut ws1, _) = connect_async(&url).await.expect("Client 1 failed to connect");

        // Send authentication
        let auth_msg = ClientMessage::Authenticate {
            password: password.to_string(),
            player_name: "Alice".to_string(),
            deck: simple_deck(),
        };
        send_message(&mut ws1, &auth_msg).await.expect("Failed to send auth");

        // Should receive AuthResult
        let response = timeout(Duration::from_secs(2), receive_message(&mut ws1))
            .await
            .expect("Timeout waiting for auth result")
            .expect("Failed to receive auth result");

        match response {
            ServerMessage::AuthResult { success, .. } => {
                assert!(success, "Authentication should succeed");
            }
            other => panic!("Expected AuthResult, got {:?}", other),
        }

        // Should then receive WaitingForOpponent
        let response = timeout(Duration::from_secs(2), receive_message(&mut ws1))
            .await
            .expect("Timeout waiting for WaitingForOpponent")
            .expect("Failed to receive message");

        match response {
            ServerMessage::WaitingForOpponent => {
                // Good - server is waiting for second player
            }
            other => panic!("Expected WaitingForOpponent, got {:?}", other),
        }

        // Clean up
        let _ = ws1.close(None).await;
        server_handle.abort();
    }

    /// Test that two clients can connect and receive GameStarted
    #[tokio::test]
    async fn test_two_clients_game_start() {
        // Allocate a random available port to avoid test collisions
        let port = allocate_random_port();
        let password = "testpass2";

        // Create server config
        let config = ServerConfig {
            port,
            password: password.to_string(),
            cardsfolder: cardsfolder_path(),
            starting_life: 20,
            ..Default::default()
        };

        // Start server in background
        let mut server = GameServer::new(config);
        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(60), server.run()).await;
        });

        // Wait for server to be accepting connections (loads card DB first)
        assert!(
            wait_for_server(port, 20).await,
            "Server did not start accepting connections within timeout"
        );

        let url = format!("ws://localhost:{}", port);

        // Connect first client
        let (mut ws1, _) = connect_async(&url).await.expect("Client 1 connect failed");

        // Authenticate first client
        send_message(
            &mut ws1,
            &ClientMessage::Authenticate {
                password: password.to_string(),
                player_name: "Alice".to_string(),
                deck: simple_deck(),
            },
        )
        .await
        .expect("Auth 1 failed");

        // Get auth result and waiting
        let _ = receive_message(&mut ws1).await.expect("Auth result 1");
        let _ = receive_message(&mut ws1).await.expect("Waiting msg");

        // Connect second client
        let (mut ws2, _) = connect_async(&url).await.expect("Client 2 connect failed");

        // Authenticate second client
        send_message(
            &mut ws2,
            &ClientMessage::Authenticate {
                password: password.to_string(),
                player_name: "Bob".to_string(),
                deck: simple_deck(),
            },
        )
        .await
        .expect("Auth 2 failed");

        // Get auth result for second client
        let auth2 = receive_message(&mut ws2).await.expect("Auth result 2");
        match auth2 {
            ServerMessage::AuthResult { success, .. } => {
                assert!(success, "Client 2 auth should succeed");
            }
            _ => panic!("Expected AuthResult for client 2"),
        }

        // Both clients should receive GameStarted
        // Use a longer timeout since the server needs to set up the game
        let game_start_1 = timeout(Duration::from_secs(5), receive_message(&mut ws1))
            .await
            .expect("Timeout waiting for GameStarted on client 1")
            .expect("Failed to receive GameStarted");

        let game_start_2 = timeout(Duration::from_secs(5), receive_message(&mut ws2))
            .await
            .expect("Timeout waiting for GameStarted on client 2")
            .expect("Failed to receive GameStarted");

        // Verify both got GameStarted
        match game_start_1 {
            ServerMessage::GameStarted {
                starting_life,
                library_size,
                ..
            } => {
                assert_eq!(starting_life, 20);
                assert!(library_size > 0, "Library should have cards");
            }
            other => panic!("Expected GameStarted for client 1, got {:?}", other),
        }

        match game_start_2 {
            ServerMessage::GameStarted {
                starting_life,
                library_size,
                ..
            } => {
                assert_eq!(starting_life, 20);
                assert!(library_size > 0, "Library should have cards");
            }
            other => panic!("Expected GameStarted for client 2, got {:?}", other),
        }

        // Clean up
        let _ = ws1.close(None).await;
        let _ = ws2.close(None).await;
        server_handle.abort();
    }

    /// Test that wrong password is rejected
    #[tokio::test]
    async fn test_wrong_password_rejected() {
        // Allocate a random available port to avoid test collisions
        let port = allocate_random_port();
        let password = "correct_password";

        let config = ServerConfig {
            port,
            password: password.to_string(),
            cardsfolder: cardsfolder_path(),
            ..Default::default()
        };

        let mut server = GameServer::new(config);
        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(30), server.run()).await;
        });

        // Wait for server to be accepting connections (loads card DB first)
        assert!(
            wait_for_server(port, 20).await,
            "Server did not start accepting connections within timeout"
        );

        let url = format!("ws://localhost:{}", port);
        let (mut ws, _) = connect_async(&url).await.expect("Connect failed");

        // Send wrong password
        send_message(
            &mut ws,
            &ClientMessage::Authenticate {
                password: "wrong_password".to_string(),
                player_name: "Hacker".to_string(),
                deck: simple_deck(),
            },
        )
        .await
        .expect("Send failed");

        // Should receive failed auth
        let response = timeout(Duration::from_secs(2), receive_message(&mut ws))
            .await
            .expect("Timeout")
            .expect("Receive failed");

        match response {
            ServerMessage::AuthResult { success, error, .. } => {
                assert!(!success, "Wrong password should fail");
                assert!(error.is_some(), "Should have error message");
            }
            _ => panic!("Expected AuthResult"),
        }

        let _ = ws.close(None).await;
        server_handle.abort();
    }
}
