//! Rooms: one tokio actor per room, plus the router that binds WebRTC peers to rooms.
//!
//! A room actor owns a `RoomHost<TheGame>` (sans-IO core from gs-core) and does the I/O around
//! it: a fixed-timestep tick loop, sending its outbox over WebRTC, streaming committed frames +
//! checkpoints to Dragonfly (via a per-room persistence task so the tick never waits on I/O),
//! and writing round results to Postgres. On startup it rehydrates from Dragonfly, so a server
//! restart resumes every recent room exactly where it was.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gs_core::net::host::HostEvent;
use gs_core::net::{Authority, ClientMsg, ClientMsgOf, RoomHost};
use gs_core::{FrameOf, Game, PlayerId, Setup};
use gs_wire::Wire;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::db::Db;
use crate::rtc::{PeerId, RtcEvent, RtcHandle};
use crate::store::Store;

pub type G = game::TheGame;

/// One durable log entry: the canonical frame plus server-private seat data.
#[derive(Wire, Clone, Debug)]
pub struct LogRecord<G: Game> {
    pub frame: FrameOf<G>,
    pub resumes: Vec<(PlayerId, u64)>,
}

enum RoomMsg {
    Data(PeerId, Vec<u8>),
    Closed(PeerId),
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct RoomInfo {
    pub name: String,
    pub mode: String,
    pub tick: u32,
    pub players: Vec<String>,
    pub connected: usize,
}

struct RoomEntry {
    tx: mpsc::UnboundedSender<RoomMsg>,
    info: Arc<Mutex<RoomInfo>>,
}

#[derive(Clone)]
pub struct Rooms {
    rooms: Arc<Mutex<HashMap<String, RoomEntry>>>,
    store: Store,
    db: Db,
    rtc: RtcHandle,
    cfg: Arc<Config>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn mint_token() -> u64 {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("OS randomness");
    u64::from_le_bytes(b) | 1
}

fn clean_room_name(s: &str) -> String {
    let s: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(32).collect();
    if s.is_empty() { "lobby".into() } else { s.to_lowercase() }
}

impl Rooms {
    pub fn new(store: Store, db: Db, rtc: RtcHandle, cfg: Arc<Config>) -> Self {
        Rooms { rooms: Arc::default(), store, db, rtc, cfg }
    }

    pub fn list(&self) -> Vec<RoomInfo> {
        let mut v: Vec<RoomInfo> = lock(&self.rooms).values().map(|r| lock(&r.info).clone()).collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    fn get_or_spawn(&self, name: &str, mode: &str, bots: u8) -> mpsc::UnboundedSender<RoomMsg> {
        let mut rooms = lock(&self.rooms);
        if let Some(r) = rooms.get(name)
            && !r.tx.is_closed()
        {
            return r.tx.clone();
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let info = Arc::new(Mutex::new(RoomInfo { name: name.into(), mode: mode.into(), ..Default::default() }));
        rooms.insert(name.to_string(), RoomEntry { tx: tx.clone(), info: info.clone() });
        let me = self.clone();
        let (name, mode) = (name.to_string(), mode.to_string());
        tokio::spawn(async move {
            if let Err(e) = room_actor(&me, &name, &mode, bots, rx, info).await {
                tracing::error!(room = %name, "room actor failed: {e:#}");
            }
            let mut rooms = lock(&me.rooms);
            if rooms.get(&name).is_some_and(|r| r.tx.is_closed()) {
                rooms.remove(&name);
            }
        });
        tx
    }

    /// Route transport events to rooms. A peer is bound to a room by its first `Hello`.
    pub async fn route(self, mut events: mpsc::UnboundedReceiver<RtcEvent>) {
        let mut bound: HashMap<PeerId, mpsc::UnboundedSender<RoomMsg>> = HashMap::new();
        while let Some(ev) = events.recv().await {
            match ev {
                RtcEvent::Open(peer) => tracing::debug!(peer, "datachannels open"),
                RtcEvent::Data(peer, _ch, bytes) => {
                    if let Some(tx) = bound.get(&peer)
                        && tx.send(RoomMsg::Data(peer, bytes.clone())).is_ok()
                    {
                        continue;
                    }
                    // Unbound (or the room shut down): only a Hello can bind.
                    if let Ok(ClientMsg::Hello(h)) = gs_wire::from_bytes::<ClientMsgOf<G>>(&bytes) {
                        let room = clean_room_name(&h.room);
                        let mode = if h.mode.is_empty() { "versus".to_string() } else { h.mode.clone() };
                        let tx = self.get_or_spawn(&room, &mode, h.bots.min(G::MAX_PLAYERS as u8));
                        let _ = tx.send(RoomMsg::Data(peer, bytes));
                        bound.insert(peer, tx);
                    }
                }
                RtcEvent::Closed(peer) => {
                    if let Some(tx) = bound.remove(&peer) {
                        let _ = tx.send(RoomMsg::Closed(peer));
                    }
                }
            }
        }
    }
}

enum PersistOp {
    Frame(u32, Vec<u8>),
    Checkpoint(u32, Vec<u8>),
}

/// Writes a room's log + checkpoints to the store in order, batching frames.
async fn persist_task(store: Store, room: String, mut rx: mpsc::Receiver<PersistOp>) {
    let mut frames = Vec::new();
    while let Some(op) = rx.recv().await {
        let mut ops = vec![op];
        while let Ok(op) = rx.try_recv() {
            ops.push(op);
        }
        for op in ops {
            match op {
                PersistOp::Frame(t, b) => frames.push((t, b)),
                PersistOp::Checkpoint(t, b) => {
                    if !frames.is_empty()
                        && let Err(e) = store.append_log(&room, std::mem::take(&mut frames)).await
                    {
                        tracing::warn!(%room, "log append failed: {e:#}");
                    }
                    if let Err(e) = store.checkpoint(&room, t, b).await {
                        tracing::warn!(%room, "checkpoint failed: {e:#}");
                    }
                }
            }
        }
        if !frames.is_empty()
            && let Err(e) = store.append_log(&room, std::mem::take(&mut frames)).await
        {
            tracing::warn!(%room, "log append failed: {e:#}");
        }
    }
}

/// Rebuild a room from Dragonfly: checkpoint + replayed log. `None` if nothing stored.
async fn rehydrate(store: &Store, room: &str) -> anyhow::Result<Option<Authority<G>>> {
    let rec = store.load_room(room).await?;
    let Some(cp) = rec.checkpoint else { return Ok(None) };
    let mut auth = Authority::<G>::restore_bytes(&cp)?;
    let mut replayed = 0;
    for bytes in rec.log {
        let r: LogRecord<G> = gs_wire::from_bytes(&bytes)?;
        if r.frame.tick < auth.tick() {
            continue;
        }
        auth.replay(&r.frame).map_err(anyhow::Error::msg)?;
        for (p, t) in r.resumes {
            auth.set_resume(p, t);
        }
        replayed += 1;
    }
    tracing::info!(%room, tick = auth.tick(), replayed, "room rehydrated from store");
    Ok(Some(auth))
}

async fn room_actor(
    rooms: &Rooms,
    name: &str,
    mode: &str,
    bots: u8,
    mut rx: mpsc::UnboundedReceiver<RoomMsg>,
    info: Arc<Mutex<RoomInfo>>,
) -> anyhow::Result<()> {
    let cfg = rooms.cfg.clone();
    let auth = match rehydrate(&rooms.store, name).await {
        Ok(Some(a)) => Some(a),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(room = %name, "rehydrate failed ({e:#}); starting fresh");
            None
        }
    };
    let resumed = auth.is_some();
    let mut host = match auth {
        Some(a) => RoomHost::from_authority(name, a),
        None => {
            let mut h = RoomHost::<G>::new(name, Setup { seed: mint_token(), mode: mode.to_string() });
            h.add_bots(bots);
            h
        }
    };
    let mode = host.auth.setup().mode.clone();
    lock(&info).mode = mode.clone();
    tracing::info!(room = %name, %mode, resumed, "room started");

    let (ptx, prx) = mpsc::channel(8192);
    tokio::spawn(persist_task(rooms.store.clone(), name.to_string(), prx));
    let _ = ptx.try_send(PersistOp::Checkpoint(host.auth.tick(), gs_wire::to_bytes(&host.auth.checkpoint())));

    let dt = Duration::from_secs_f64(1.0 / f64::from(G::TICK_HZ));
    let mut interval = tokio::time::interval(dt);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut disconnected_at: HashMap<PlayerId, Instant> = HashMap::new();
    let mut new_resumes: Vec<(PlayerId, u64)> = Vec::new();
    let mut empty_since = Some(Instant::now());
    let mut persist_broken = false;

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    RoomMsg::Data(peer, bytes) => host.on_message(peer, &bytes, mint_token),
                    RoomMsg::Closed(peer) => host.on_disconnect(peer),
                }
            }
            _ = interval.tick() => {
                let out = host.tick();
                let tick = out.frame.tick;
                // Persist the committed frame. If the writer is backed up, skip frames and force a
                // checkpoint instead — durability degrades gracefully, the tick never blocks.
                let rec = LogRecord::<G> { frame: out.frame, resumes: std::mem::take(&mut new_resumes) };
                if ptx.try_send(PersistOp::Frame(tick, gs_wire::to_bytes(&rec))).is_err() {
                    persist_broken = true;
                }
                if (tick + 1) % cfg.checkpoint_every == 0 || persist_broken {
                    let cp = gs_wire::to_bytes(&host.auth.checkpoint());
                    persist_broken = ptx.try_send(PersistOp::Checkpoint(host.auth.tick(), cp)).is_err();
                }
                if let Some(result) = out.result {
                    let names: Vec<(PlayerId, String, bool)> =
                        host.auth.seats().map(|(id, s)| (id, s.name.clone(), s.bot)).collect();
                    let (db, room, mode) = (rooms.db.clone(), name.to_string(), mode.clone());
                    tracing::info!(room = %name, summary = %result.summary, "round over");
                    tokio::spawn(async move {
                        if let Err(e) = db.record_match(&room, &mode, &result, &names).await {
                            tracing::warn!("recording match failed: {e:#}");
                        }
                    });
                }
                // Drop seats whose owner has been gone longer than the resume grace period.
                let now = Instant::now();
                disconnected_at.retain(|p, at| {
                    let still_gone = host.auth.seat(*p).is_some_and(|s| !s.connected);
                    if still_gone && now.duration_since(*at) > cfg.resume_grace {
                        host.evict(*p);
                        return false;
                    }
                    still_gone
                });
                if tick % G::TICK_HZ == 0 {
                    let mut i = lock(&info);
                    i.tick = host.auth.tick();
                    i.players = host.auth.seats().map(|(_, s)| s.name.clone()).collect();
                    i.connected = host.connections();
                }
                if host.connections() == 0 {
                    let since = *empty_since.get_or_insert(now);
                    if now.duration_since(since) > cfg.room_idle {
                        break;
                    }
                } else {
                    empty_since = None;
                }
            }
        }
        let host_events: Vec<HostEvent> = host.drain_events().collect();
        for ev in host_events {
            match ev {
                HostEvent::Joined { player, name: who, resumed, .. } => {
                    tracing::info!(room = %name, player, %who, resumed, "player joined");
                    disconnected_at.remove(&player);
                    if !resumed && let Some(s) = host.auth.seat(player) {
                        new_resumes.push((player, s.resume));
                    }
                    host.notice(&format!("{who} {}", if resumed { "reconnected" } else { "joined" }));
                }
                HostEvent::Left { player, .. } => {
                    disconnected_at.insert(player, Instant::now());
                }
                HostEvent::Rejected { conn, reason } => {
                    tracing::warn!(room = %name, conn, %reason, "hello rejected");
                }
            }
        }
        for (peer, ch, bytes) in host.drain_outbox() {
            rooms.rtc.send(peer, ch, bytes);
        }
    }

    // Idle shutdown: final checkpoint so the room resumes exactly here next time.
    let cp = gs_wire::to_bytes(&host.auth.checkpoint());
    let _ = ptx.send(PersistOp::Checkpoint(host.auth.tick(), cp)).await;
    tracing::info!(room = %name, tick = host.auth.tick(), "room idle; checkpointed and stopped");
    Ok(())
}
