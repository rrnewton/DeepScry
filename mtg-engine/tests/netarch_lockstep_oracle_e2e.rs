// TODO(mtg-211): Remove once wildcard patterns are audited
#![allow(clippy::wildcard_enum_match_arm)]

//! CLASS-A server<->shadow per-action LOCKSTEP ORACLE (mtg-728 CHUNK 2).
//!
//! ## What this proves / enumerates
//!
//! `docs/NETWORK_ARCHITECTURE.md`: any server<->shadow desync is FATAL. The
//! browser e2e (`web/test_network_gui_e2e.js --deck decks/old_school/
//! 03_robots_jesseisbak.dck --seed N`) deterministically desyncs on a CLASS of
//! seeds (2,5,6,9,11,18,19,20 + now-exposed 1,7) with a per-action server<->shadow
//! STATE-HASH mismatch (NOT a within-side rewind hole — that family, class-B, is
//! closed: sig-2e counters / sig-2f damage / sig-2g x_paid). The residual class-A
//! cause is the WASM/native shadow BRANCHING ON ABSENCE for opponent hidden-info
//! events (library-search fetch, mass-draw/shuffle): the authoritative reveal/move
//! is not yet materialised on the shadow, so `try_get -> None` drives a divergent
//! Fisher-Yates draw COUNT / library decrement (the mtg-725 anti-pattern).
//!
//! That browser repro is ~30-280s per seed and only reports the final hash. This
//! oracle runs a FAST, PURE-RUST, in-process game: a real `GameServer` (the golden
//! full-information authority) plus two real native `NetworkClient` shadows driven
//! by `RandomController`s, over a localhost socket inside one tokio runtime, with
//! `network_debug: true` so the server validates the per-choice state hash every
//! action and returns a FATAL `Err` the instant a shadow diverges. The seed is
//! pinned exactly as the browser harness pins it (server RNG `seed = N`; each
//! client's controller master seed `= N`, per-slot-salted via
//! `derive_player_seed`), so the in-process game is the SAME server-side game the
//! browser plays.
//!
//! ## CRITICAL SCOPE FINDING (2026-06-03, slot03) — this is a GREEN GUARD, not the class-A RED
//!
//! Empirically this whole suite is GREEN across every class-A seed (1,2,5,6,7,9,
//! 11,18,19,20) AND the controls — the native shadows stay in PERFECT per-action
//! lockstep with the golden server for the entire game. That is NOT a class-A
//! regression: it is structural. The NATIVE `NetworkClient`
//! (`src/network/client.rs`) uses a BLOCKING-THREAD model with **no client-side
//! rewind** — it frontier-WAITS (condvar) for the authoritative reveal before
//! consuming it, so its `try_get` always sees `Some`. The class-A bug is the
//! shadow BRANCHING ON ABSENCE during REWIND+REPLAY, and only the **WASM** shadow
//! (`src/wasm/network/client.rs`, the action_count-keyed reveal-history buffer +
//! `rewind_to_turn_start`/`unwind_state_sync_to`) does client-side rewind. That
//! module is gated `#[cfg(all(feature = "wasm-network", target_arch = "wasm32"))]`
//! — wasm32-only, so it is NOT reachable from a native `cargo test`. Hence the
//! native client cannot reproduce class-A no matter the seed.
//!
//! So this file's role is a **native-shadow lockstep REGRESSION GUARD**: it proves
//! the non-rewinding native shadow path stays byte-locked to the server across the
//! full class-A seed set (a real, valuable invariant — and a control that any
//! eventual engine-level fix must not break). The class-A RED repro + per-action
//! field enumeration must instead drive the engine's shadow rewind+replay reveal
//! path directly (the `game/game_loop/mod.rs` `opponent_library_search_fetch_*` /
//! `shuffle_replay_byte_*` / `mass_draw_replay_*` native oracle pattern), since
//! the WASM client is a thin wasm32 transport+buffer wrapper over those native
//! engine primitives. See mtg-728.
//!
//! ALL tests here are therefore expected GREEN. A FAILURE means a genuine NEW
//! desync in the non-rewinding native shadow path — surface it, never paper over
//! it (`docs/NETWORK_ARCHITECTURE.md`: desync is ALWAYS fatal).
//!
//! NOTE: the older in-process full-game test pinned `network_debug: false` citing
//! a tokio reveal-ordering race (the async `reveal_pusher`/`opponent_reveal_tx`
//! channels). mtg-610 replaced receipt-race inference with the action_count-keyed
//! reveal stamping, so we re-enable `network_debug` here and it is stable across
//! the suite. If a seed ever proves flaky, that is itself a finding (a residual
//! ordering race) — surface it, do not disable the hash check.

#![cfg(feature = "network")]

#[cfg(test)]
mod lockstep_oracle {
    use mtg_engine::core::PlayerId;
    use mtg_engine::game::{derive_player_seed, PlayerSlot, RandomController};
    use mtg_engine::network::{ClientConfig, GameServer, NetworkClient, ServerConfig};
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::time::timeout;

    /// The deck every class-A seed is reproduced on.
    const ROBOTS_DECK: &str = "decks/old_school/03_robots_jesseisbak.dck";

    /// The class-A seeds that desync the WASM (rewinding) shadow in-browser. Here
    /// they exercise the NATIVE (non-rewinding) shadow, which stays green — see the
    /// SCOPE FINDING in the module doc. Kept as a guard: the native shadow must
    /// stay byte-locked to the server across exactly this set.
    const CLASS_A_SEEDS: &[u64] = &[1, 2, 5, 6, 7, 9, 11, 18, 19, 20];

    /// CONTROL seeds: already green in the deterministic 20-seed sweep (green for
    /// BOTH shadow kinds). Guard against a future fix that only reshuffles the RNG.
    const CONTROL_SEEDS: &[u64] = &[3, 13, 16];

    fn repo_path(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)
    }

    fn allocate_random_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind random port");
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    async fn wait_for_server(port: u16, timeout_secs: u64) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(timeout_secs) {
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

    /// Outcome of one client's `run_game` in lockstep with the server.
    type ClientOutcome = Result<Option<PlayerId>, String>;

    /// Run the robots deck at a pinned `seed` as a real in-process networked game
    /// (golden server + two native shadow clients), with per-action state-hash
    /// validation enabled. Returns each client's outcome (winner or the FATAL
    /// desync string). The pinning mirrors `web/test_network_gui_e2e.js`:
    ///   - server RNG seed  = seed
    ///   - client master seed = seed, per-slot salted via `derive_player_seed`.
    async fn run_robots_seed(seed: u64) -> (ClientOutcome, ClientOutcome) {
        let port = allocate_random_port();
        let password = "lockstep";

        let config = ServerConfig {
            port,
            password: password.to_string(),
            cardsfolder: repo_path("cardsfolder"),
            starting_life: 20,
            // The whole point: validate the per-choice server<->shadow state hash.
            network_debug: true,
            // Pin the shuffle RNG exactly like the browser harness `--seed`.
            seed: Some(seed),
            ..Default::default()
        };

        let mut server = GameServer::new(config);
        let server_handle = tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(300), server.run()).await;
        });

        assert!(
            wait_for_server(port, 20).await,
            "seed {seed}: server did not start accepting connections"
        );

        let deck_path = repo_path(ROBOTS_DECK);
        let cardsfolder = repo_path("cardsfolder");

        let make_client = |name: &str| {
            let mut c = ClientConfig::new(
                format!("localhost:{}", port),
                password.to_string(),
                Some(name.to_string()),
                deck_path.clone(),
            );
            c.cardsfolder = cardsfolder.clone();
            c
        };

        let c1 = make_client("ShadowBot1");
        let c2 = make_client("ShadowBot2");

        // Each shadow derives its controller seed from the SAME master seed,
        // per-slot salted — exactly as `main.rs` does for `--seed-player`.
        let run_client = |cfg: ClientConfig| async move {
            let mut client = NetworkClient::new(cfg);
            client.connect().await.map_err(|e| e.to_string())?;
            client.wait_for_game_start().await.map_err(|e| e.to_string())?;
            let pid = client.our_player_id().expect("player id after game start");
            let slot = PlayerSlot::from_index(pid.as_u32() as usize).unwrap_or(PlayerSlot::P1);
            let controller = RandomController::with_seed(pid, derive_player_seed(seed, slot));
            client.run_game(controller).await.map_err(|e| e.to_string())
        };

        let h1 = tokio::spawn(run_client(c1));
        let h2 = tokio::spawn(run_client(c2));

        let t = Duration::from_secs(300);
        let (r1, r2) = tokio::join!(timeout(t, h1), timeout(t, h2));

        let unwrap = |label: &str,
                      r: Result<Result<ClientOutcome, tokio::task::JoinError>, tokio::time::error::Elapsed>|
         -> ClientOutcome {
            match r {
                Err(_) => Err(format!("{label}: TIMEOUT after {}s", t.as_secs())),
                Ok(Err(join)) => Err(format!("{label}: task panicked: {join}")),
                Ok(Ok(outcome)) => outcome,
            }
        };

        let out1 = unwrap("ShadowBot1", r1);
        let out2 = unwrap("ShadowBot2", r2);

        server_handle.abort();
        (out1, out2)
    }

    /// Assert a seed runs to a clean, agreed finish with NO server<->shadow
    /// desync on the native (non-rewinding) shadow path. On failure the panic
    /// message carries the captured divergence signature (which client, and the
    /// server's `FATAL: P{1,2} state hash mismatch ... action_count=...` string).
    async fn assert_lockstep_green(seed: u64) {
        let (out1, out2) = run_robots_seed(seed).await;
        match (&out1, &out2) {
            (Ok(w1), Ok(w2)) => {
                assert_eq!(w1, w2, "seed {seed}: clients disagree on winner: {w1:?} vs {w2:?}");
            }
            _ => panic!(
                "seed {seed}: native-shadow server<->shadow lockstep FAILURE \
                 (NEW desync in the non-rewinding native path — investigate, never paper over).\n  \
                 ShadowBot1 = {out1:?}\n  ShadowBot2 = {out2:?}"
            ),
        }
    }

    // ── default-run guard: ONE class-A seed + ONE control ──
    //
    // Each robots game is a full ~28s in-process networked match, so the DEFAULT
    // `cargo test` (and `make validate`) runs only a lean representative pair to
    // keep validation fast (slot02's validate-overhaul, mtg-717). The FULL
    // enumerated sweep over every class-A + control seed lives in the `#[ignore]`d
    // `native_shadow_lockstep_full_sweep` below — NOT excluded, just opt-in via
    // `cargo test --features network --test netarch_lockstep_oracle_e2e -- --ignored`.

    /// Class-A seed 2 against the native (non-rewinding) shadow: must stay locked.
    #[tokio::test]
    async fn native_shadow_lockstep_class_a_seed_02() {
        assert_lockstep_green(2).await;
    }

    /// Control seed 3 (green for both shadow kinds): RNG-perturbation guard.
    #[tokio::test]
    async fn native_shadow_lockstep_control_seed_03() {
        assert_lockstep_green(3).await;
    }

    /// FULL native-shadow lockstep sweep over every class-A + control seed.
    /// Opt-in (`--ignored`): ~28s/seed × 13 seeds. Proves the non-rewinding native
    /// shadow stays byte-locked to the golden server across the entire seed set.
    #[tokio::test]
    #[ignore = "heavy: ~28s/seed × 13 seeds; run with --ignored for the full native-shadow sweep"]
    async fn native_shadow_lockstep_full_sweep() {
        for &seed in CLASS_A_SEEDS.iter().chain(CONTROL_SEEDS.iter()) {
            assert_lockstep_green(seed).await;
        }
    }

    /// Guard: the seed lists are non-empty and disjoint.
    #[test]
    fn seed_lists_are_disjoint_and_nonempty() {
        assert!(!CLASS_A_SEEDS.is_empty());
        assert!(!CONTROL_SEEDS.is_empty());
        for s in CONTROL_SEEDS {
            assert!(
                !CLASS_A_SEEDS.contains(s),
                "seed {s} cannot be both control and class-A"
            );
        }
    }
}
