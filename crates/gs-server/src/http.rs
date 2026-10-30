//! HTTP: WebRTC signaling + a small REST API + the built web client.
//! (Game traffic never touches HTTP — only the one-time SDP offer/answer does.)

use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;
use str0m::change::SdpOffer;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::config::Config;
use crate::db::Db;
use crate::rooms::{G, Rooms};
use crate::rtc::RtcHandle;
use crate::store::Store;

#[derive(Clone)]
pub struct AppState {
    pub rtc: RtcHandle,
    pub rooms: Rooms,
    pub store: Store,
    pub db: Db,
    pub cfg: Arc<Config>,
}

struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{:#}", self.0) }))).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(e.into())
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/api/rtc/offer", post(offer))
        .route("/api/health", get(health))
        .route("/api/rooms", get(rooms))
        .route("/api/leaderboard", get(leaderboard))
        .route("/api/matches", get(matches))
        .route("/api/contract", get(contract))
        .route("/api/channel/{name}", get(channel_read).post(channel_push));
    let app = match &state.cfg.web_dir {
        Some(dir) => {
            let index = format!("{dir}/index.html");
            api.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => api,
    };
    app.layer(CorsLayer::permissive()).with_state(state)
}

/// Browser POSTs `pc.localDescription` ({type:"offer", sdp}); we return the answer.
async fn offer(State(s): State<AppState>, Json(offer): Json<SdpOffer>) -> Result<Json<serde_json::Value>, ApiError> {
    let answer = s.rtc.accept(offer).await?;
    Ok(Json(serde_json::to_value(answer)?))
}

async fn health(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "ok": true,
        "game": <G as gs_core::Game>::NAME,
        "fingerprint": format!("{:016x}", gs_core::net::fingerprint::<G>()),
        "tick_hz": <G as gs_core::Game>::TICK_HZ,
        "peers": s.rtc.peer_count(),
        "rooms": s.rooms.list().len(),
        "store": s.store.kind(),
        "db": s.db.enabled(),
        "udp": format!("{}:{}", s.cfg.public_ip, s.cfg.udp_port),
    }))
}

async fn rooms(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(json!(s.rooms.list()))
}

async fn leaderboard(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(json!(s.db.leaderboard(20).await?)))
}

async fn matches(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(json!(s.db.recent_matches(20).await?)))
}

async fn contract() -> String {
    gs_core::net::contract_text::<G>()
}

#[derive(Deserialize)]
struct ChanQuery {
    after: Option<String>,
}

/// Durable channel (Dragonfly stream) read: `GET /api/channel/lobby?after=<id>`.
async fn channel_read(
    State(s): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<ChanQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let msgs = s.store.channel_read(&name, q.after.as_deref().unwrap_or("0"), 100).await?;
    let out: Vec<_> =
        msgs.into_iter().map(|(id, m)| json!({ "id": id, "text": String::from_utf8_lossy(&m) })).collect();
    Ok(Json(json!(out)))
}

/// Durable channel append: `POST /api/channel/lobby` with a text body.
async fn channel_push(
    State(s): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    body: String,
) -> Result<Json<serde_json::Value>, ApiError> {
    let text: String = body.chars().take(500).collect();
    let id = s.store.channel_push(&name, text.into_bytes(), 1000).await?;
    Ok(Json(json!({ "id": id })))
}
