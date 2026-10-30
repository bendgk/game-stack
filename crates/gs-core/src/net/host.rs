//! `RoomHost`: the per-room message protocol on top of [`Authority`].
//!
//! Connections are opaque ids; bytes in, bytes out. The real server wraps this in a tokio actor
//! with WebRTC + persistence (`gs-server/src/room.rs`), and the browser wraps the very same
//! code for offline/practice play (`gs-client`). One implementation, no drift.

use std::collections::BTreeMap;

use super::proto::{Channel, ClientMsg, ClientMsgOf, Hello, ServerMsg, fingerprint};
use super::server::{Authority, JoinError, StepOutput};
use crate::game::{Game, PlayerId, Setup};

pub type ConnId = u64;

#[derive(Debug, Clone, Default)]
struct Conn {
    player: Option<PlayerId>,
    welcomed: bool,
}

/// Something worth telling the room's owner (for logging / persistence / presence).
#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    Joined { conn: ConnId, player: PlayerId, name: String, resumed: bool },
    Left { conn: ConnId, player: PlayerId },
    Rejected { conn: ConnId, reason: String },
}

pub struct RoomHost<G: Game> {
    pub auth: Authority<G>,
    room: String,
    conns: BTreeMap<ConnId, Conn>,
    outbox: Vec<(ConnId, Channel, Vec<u8>)>,
    events: Vec<HostEvent>,
}

impl<G: Game> RoomHost<G> {
    pub fn new(room: &str, setup: Setup) -> Self {
        Self::from_authority(room, Authority::new(setup))
    }

    pub fn from_authority(room: &str, auth: Authority<G>) -> Self {
        RoomHost { auth, room: room.to_string(), conns: BTreeMap::new(), outbox: Vec::new(), events: Vec::new() }
    }

    pub fn room(&self) -> &str {
        &self.room
    }

    pub fn add_bots(&mut self, n: u8) {
        for i in 0..n {
            if self.auth.join(&format!("bot-{}", i + 1), true, 0).is_err() {
                break;
            }
        }
    }

    pub fn connections(&self) -> usize {
        self.conns.len()
    }

    pub fn player_of(&self, conn: ConnId) -> Option<PlayerId> {
        self.conns.get(&conn).and_then(|c| c.player)
    }

    fn send(&mut self, conn: ConnId, msg: &ServerMsg) {
        self.outbox.push((conn, msg.channel(), gs_wire::to_bytes(msg)));
    }

    /// Handle one inbound message. `mint` creates an unguessable resume token (OS randomness
    /// lives with the caller, outside the deterministic core).
    pub fn on_message(&mut self, conn: ConnId, bytes: &[u8], mut mint: impl FnMut() -> u64) {
        let Ok(msg) = gs_wire::from_bytes::<ClientMsgOf<G>>(bytes) else {
            return; // garbage from a peer is dropped, never trusted
        };
        match msg {
            ClientMsg::Hello(h) => self.on_hello(conn, h, &mut mint),
            ClientMsg::Inputs(batch) => {
                if let Some(p) = self.player_of(conn) {
                    self.auth.receive_inputs(p, batch);
                }
            }
            ClientMsg::Command(c) => {
                if let Some(p) = self.player_of(conn) {
                    self.auth.command(p, c);
                }
            }
            ClientMsg::Ping { t } => {
                let tick = self.auth.tick();
                self.send(conn, &ServerMsg::Pong { t, tick });
            }
            ClientMsg::Bye => {
                if let Some(c) = self.conns.remove(&conn)
                    && let Some(p) = c.player
                {
                    self.auth.leave(p);
                    self.events.push(HostEvent::Left { conn, player: p });
                }
            }
        }
    }

    fn on_hello(&mut self, conn: ConnId, h: Hello, mint: &mut impl FnMut() -> u64) {
        if self.conns.get(&conn).is_some_and(|c| c.player.is_some()) {
            return; // duplicate hello on the reliable channel
        }
        if h.fingerprint != fingerprint::<G>() {
            let reason = format!(
                "client build does not match server (fingerprint {:016x} != {:016x}); rebuild the client",
                h.fingerprint,
                fingerprint::<G>()
            );
            self.send(conn, &ServerMsg::Reject(reason.clone()));
            self.events.push(HostEvent::Rejected { conn, reason });
            return;
        }
        let resumed = h.resume.and_then(|t| self.auth.resume(t));
        let (player, was_resumed) = match resumed {
            Some(p) => (p, true),
            None => match self.auth.join(&h.name, false, mint()) {
                Ok(p) => (p, false),
                Err(JoinError::Full) => {
                    self.send(conn, &ServerMsg::Reject("room is full".into()));
                    self.events.push(HostEvent::Rejected { conn, reason: "full".into() });
                    return;
                }
            },
        };
        // A resumed seat may still be bound to a dead connection — unbind it.
        for c in self.conns.values_mut() {
            if c.player == Some(player) {
                c.player = None;
            }
        }
        self.conns.insert(conn, Conn { player: Some(player), welcomed: false });
        let name = self.auth.seat(player).map(|s| s.name.clone()).unwrap_or_default();
        self.events.push(HostEvent::Joined { conn, player, name, resumed: was_resumed });
        if was_resumed {
            self.welcome(conn, player);
        }
    }

    fn welcome(&mut self, conn: ConnId, player: PlayerId) {
        if let Some(w) = self.auth.welcome_for(player, &self.room) {
            self.send(conn, &ServerMsg::Welcome(w));
            if let Some(c) = self.conns.get_mut(&conn) {
                c.welcomed = true;
            }
        }
    }

    /// The transport closed (no `Bye`): keep the seat for resume.
    pub fn on_disconnect(&mut self, conn: ConnId) {
        if let Some(c) = self.conns.remove(&conn)
            && let Some(p) = c.player
        {
            self.auth.disconnect(p);
            self.events.push(HostEvent::Left { conn, player: p });
        }
    }

    /// Remove a player's seat entirely (e.g. after the resume grace period).
    pub fn evict(&mut self, player: PlayerId) {
        self.auth.leave(player);
    }

    /// Simulate one tick and queue snapshots for every welcomed connection.
    pub fn tick(&mut self) -> StepOutput<G> {
        let out = self.auth.step();
        let joined: Vec<(ConnId, PlayerId)> = self
            .conns
            .iter()
            .filter_map(|(c, s)| s.player.filter(|p| out.joined.contains(p)).map(|p| (*c, p)))
            .collect();
        for (c, p) in joined {
            self.welcome(c, p);
        }
        let targets: Vec<(ConnId, PlayerId)> =
            self.conns.iter().filter(|(_, s)| s.welcomed).filter_map(|(c, s)| s.player.map(|p| (*c, p))).collect();
        for (c, p) in targets {
            let snap = self.auth.snapshot_for(p);
            self.send(c, &ServerMsg::Snapshot(snap));
        }
        out
    }

    pub fn notice(&mut self, text: &str) {
        let conns: Vec<ConnId> = self.conns.keys().copied().collect();
        for c in conns {
            self.send(c, &ServerMsg::Notice(text.to_string()));
        }
    }

    pub fn drain_outbox(&mut self) -> std::vec::Drain<'_, (ConnId, Channel, Vec<u8>)> {
        self.outbox.drain(..)
    }

    pub fn drain_events(&mut self) -> std::vec::Drain<'_, HostEvent> {
        self.events.drain(..)
    }
}
