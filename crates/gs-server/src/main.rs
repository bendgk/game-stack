//! gs-server — the one authoritative node.
//!
//! ```text
//!  browser ──HTTP POST offer──▶ axum ──▶ rtc task (str0m, one UDP socket)
//!     ▲                                      │ RtcEvent
//!     └──── DataChannels (UDP) ◀─────────────┤
//!                                            ▼
//!                                   router ─▶ room actors (RoomHost<TheGame>, fixed tick)
//!                                                 │ frames/checkpoints   │ results
//!                                                 ▼                      ▼
//!                                             Dragonfly              Postgres
//! ```
//!
//! Subcommands: `gs-server contract` prints the wire contract; `gs-server probe` prints the
//! determinism probe hash (compared against the wasm build by `pnpm verify`).

use std::net::SocketAddr;
use std::sync::Arc;

use tracing_subscriber::EnvFilter;

mod config;
mod db;
mod http;
mod rooms;
mod rtc;
mod store;

use rooms::G;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("contract") => {
            print!("{}", gs_core::net::contract_text::<G>());
            return Ok(());
        }
        Some("probe") => {
            println!("{:016x}", gs_core::env::probe::<G>(1, "versus", 4, 1200));
            return Ok(());
        }
        Some(other) if other != "serve" => anyhow::bail!("unknown command {other:?} (serve | contract | probe)"),
        _ => {}
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,str0m=warn,dimpl=warn")),
        )
        .init();

    let cfg = Arc::new(config::Config::from_env()?);
    tracing::info!(game = <G as gs_core::Game>::NAME, fingerprint = %format!("{:016x}", gs_core::net::fingerprint::<G>()), "starting");

    let store = store::Store::connect(cfg.redis_url.as_deref()).await?;
    let db = db::Db::connect(cfg.database_url.as_deref()).await?;

    let bind = SocketAddr::from(([0, 0, 0, 0], cfg.udp_port));
    let advertise = SocketAddr::new(cfg.public_ip, cfg.udp_port);
    let (rtc, events) = rtc::start(bind, advertise).await?;

    let rooms = rooms::Rooms::new(store.clone(), db.clone(), rtc.clone(), cfg.clone());
    tokio::spawn(rooms.clone().route(events));

    let app = http::router(http::AppState { rtc, rooms, store, db, cfg: cfg.clone() });
    let listener = tokio::net::TcpListener::bind(cfg.http_addr).await?;
    tracing::info!("open http://{}:{} on any device on this network", cfg.public_ip, cfg.http_addr.port());
    axum::serve(listener, app).with_graceful_shutdown(shutdown()).await?;
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}
