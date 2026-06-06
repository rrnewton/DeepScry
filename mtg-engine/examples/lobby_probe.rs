//! Tiny live probe for the lobby protocol against a running `mtg server`.
//!
//! This is a hand-runnable smoke test we can point at the real binary to
//! confirm new wire messages survive the round-trip through serde and
//! through the lobby dispatcher. The integration test
//! `tests/lobby_protocol.rs` covers the same cases against an in-process
//! server; this binary exists for ad-hoc / staging probes.
//!
//! Usage:
//! ```text
//! cargo run --features network --example lobby_probe -- --port 17810
//! ```

// Probe code matches exactly one expected variant per step and bails on
// anything else; the wildcard arm IS the assertion. Spelling out every other
// `ServerMessage` / `Message` variant would be pure noise.
#![allow(clippy::wildcard_enum_match_arm)]

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use mtg_engine::network::{ClientMessage, DeckSubmission, ServerMessage};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::main]
async fn main() -> Result<()> {
    let mut port = 17810u16;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--port" {
            port = args.next().ok_or_else(|| anyhow!("--port wants a value"))?.parse()?;
        }
    }

    let url = format!("ws://127.0.0.1:{port}");
    eprintln!("[probe] connecting to {url}");
    let (mut ws, _) = connect_async(&url).await?;
    if let tokio_tungstenite::MaybeTlsStream::Plain(ref tcp) = *ws.get_ref() {
        let _ = tcp.set_nodelay(true);
    }

    eprintln!("[probe] sending ListGames");
    let msg = ClientMessage::ListGames {
        password: String::new(),
        query: None,
    };
    ws.send(Message::Text(serde_json::to_string(&msg)?.into())).await?;

    let frame = ws.next().await.ok_or_else(|| anyhow!("eof"))??;
    let text = match frame {
        Message::Text(t) => t,
        other => return Err(anyhow!("expected text, got {other:?}")),
    };
    let reply: ServerMessage = serde_json::from_str(&text)?;
    println!("REPLY: {}", serde_json::to_string_pretty(&reply)?);

    match reply {
        ServerMessage::GameList {
            games,
            total_count,
            system_memory_used_percent,
            max_memory_percent,
        } => {
            println!(
                "OK: GameList with {} games (of {} total, host {:?}% used, ceiling {}%)",
                games.len(),
                total_count,
                system_memory_used_percent,
                max_memory_percent
            );
        }
        other => return Err(anyhow!("expected GameList, got {other:?}")),
    }

    // Quick CreateGame probe on the same connection.
    eprintln!("[probe] sending CreateGame 'probe-game'");
    let msg = ClientMessage::CreateGame {
        password: String::new(),
        game_name: Some("probe-game".to_string()),
        game_password: None,
        player_name: Some("probe".to_string()),
        deck: DeckSubmission::new(vec![("Mountain".to_string(), 40)], vec![]),
        waiting_room: false,
    };
    ws.send(Message::Text(serde_json::to_string(&msg)?.into())).await?;

    // Expect GameCreated then WaitingForOpponent.
    for expected in &["GameCreated", "WaitingForOpponent"] {
        let frame = ws.next().await.ok_or_else(|| anyhow!("eof"))??;
        let text = match frame {
            Message::Text(t) => t,
            other => return Err(anyhow!("expected text, got {other:?}")),
        };
        let reply: ServerMessage = serde_json::from_str(&text)?;
        println!("REPLY {expected}: {}", serde_json::to_string(&reply)?);
    }

    println!("PROBE OK");
    Ok(())
}
