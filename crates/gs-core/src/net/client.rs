//! The predictor: the client's sans-IO netcode core (runs in wasm in the browser).
//!
//! Model: **server-authoritative snapshots + full-world deterministic prediction.**
//!
//! - The client runs the *same* `Game::step` ahead of the server by a small lead (≈ RTT/2 +
//!   a jitter cushion), using its own real input and repeat-last input for everyone else.
//! - Each snapshot is the truth at tick `T`. If our prediction for `T` already matches it
//!   bit-for-bit (the common case on a LAN) nothing happens. Otherwise we roll back: load the
//!   snapshot and re-simulate `T..now` with our recorded inputs.
//! - The lead is steered by the server's `slack` feedback with gentle time dilation (±5%), so
//!   inputs arrive just in time without visible hitches. Big errors hard-resync.
//! - Corrections never pop: the visual difference is carried as a per-entity offset that
//!   decays over ~100 ms. Render is interpolated between the last two predicted ticks.
//! - Events fire once per (tick, event) even when a tick is re-simulated.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use gs_wire::WireError;

use super::clock::Rtt;
use super::delta;
use super::proto::{Channel, ClientMsgOf, INPUT_REDUNDANCY, InputBatch, ServerMsg, Snapshot, Welcome, WorldSnap};
use crate::game::{Events, FrameOf, Game, PlayerId, Setup};
use crate::math::{self, Vec3};
use crate::render::{FLAG_TELEPORT, RenderEntity, RenderView};

#[derive(Clone, Debug)]
pub struct PredictorConfig {
    /// Desired ticks of input slack at the server (jitter cushion).
    pub target_slack: f32,
    /// Max time-dilation either way (0.05 = ±5%).
    pub max_dilation: f32,
    /// Max ticks simulated per `update` (spiral-of-death guard).
    pub max_steps_per_update: u32,
    /// Correction smoothing time constant (seconds).
    pub smoothing_tau: f32,
    /// Corrections larger than this (world units) snap instead of gliding.
    pub snap_distance: f32,
    /// Max ticks the prediction may run ahead of the newest snapshot before it holds.
    pub max_prediction: u32,
}

impl Default for PredictorConfig {
    fn default() -> Self {
        PredictorConfig {
            target_slack: 2.0,
            max_dilation: 0.05,
            max_steps_per_update: 8,
            smoothing_tau: 0.1,
            snap_distance: 3.0,
            max_prediction: 30,
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct NetStats {
    pub tick: u32,
    pub server_tick: u32,
    pub rtt_ms: f32,
    pub jitter_ms: f32,
    pub slack: f32,
    pub time_scale: f32,
    pub rollbacks: u32,
    pub last_rollback_ticks: u32,
    pub prediction_hits: u32,
    pub snapshots: u32,
    pub snapshot_bytes: u32,
    pub dropped_snapshots: u32,
    pub resyncs: u32,
    pub stalled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientError {
    Wire(WireError),
    Rejected(String),
}

impl From<WireError> for ClientError {
    fn from(e: WireError) -> Self {
        ClientError::Wire(e)
    }
}

impl std::error::Error for ClientError {}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Wire(e) => write!(f, "bad message: {e}"),
            ClientError::Rejected(r) => write!(f, "rejected by server: {r}"),
        }
    }
}

pub struct Predictor<G: Game> {
    cfg: PredictorConfig,
    me: PlayerId,
    setup: Setup,
    resume: u64,
    dt_ms: f64,

    // Authoritative (newest snapshot).
    auth_tick: u32,
    auth: WorldSnap<G>,
    auth_hash: u64,
    auth_dirty: bool,
    baselines: VecDeque<(u32, Vec<u8>)>,

    // Prediction.
    game: G,
    tick: u32,
    prev_view: RenderView,
    view: RenderView,
    inputs: VecDeque<(u32, G::Input)>,
    input: G::Input,
    roster_used: Vec<(PlayerId, G::Input)>,
    pred_hashes: VecDeque<(u32, u64)>,

    // Time.
    acc_ms: f64,
    last_now: Option<f64>,
    slack_ema: f32,
    resync_cooldown: u32,
    rtt: Rtt,
    last_ping: f64,

    // Presentation.
    smoothing: BTreeMap<u32, Vec3>,
    seen_events: BTreeSet<(u32, u64)>,
    events_out: Vec<(u32, G::Event)>,
    notices: Vec<String>,
    outbox: Vec<(Channel, Vec<u8>)>,
    scratch: Events<G::Event>,
    stats: NetStats,
}

impl<G: Game> Predictor<G> {
    pub fn new(welcome: Welcome) -> Result<Self, ClientError> {
        Self::with_config(welcome, PredictorConfig::default())
    }

    pub fn with_config(welcome: Welcome, cfg: PredictorConfig) -> Result<Self, ClientError> {
        let bytes = welcome.snapshot.payload.0;
        let auth: WorldSnap<G> = gs_wire::from_bytes(&bytes)?;
        let tick = welcome.snapshot.tick;
        let mut p = Predictor {
            me: welcome.player,
            setup: welcome.setup,
            resume: welcome.resume,
            dt_ms: 1000.0 / f64::from(welcome.tick_hz.max(1)),
            auth_tick: tick,
            auth_hash: gs_wire::fnv1a64(&gs_wire::to_bytes(&auth.game)),
            game: auth.game.clone(),
            roster_used: auth.roster.clone(),
            auth,
            auth_dirty: false,
            baselines: VecDeque::from([(tick, bytes)]),
            tick,
            prev_view: RenderView::default(),
            view: RenderView::default(),
            inputs: VecDeque::new(),
            input: G::Input::default(),
            pred_hashes: VecDeque::new(),
            acc_ms: 0.0,
            last_now: None,
            slack_ema: cfg.target_slack,
            resync_cooldown: 0,
            rtt: Rtt::default(),
            last_ping: f64::NEG_INFINITY,
            smoothing: BTreeMap::new(),
            seen_events: BTreeSet::new(),
            events_out: Vec::new(),
            notices: Vec::new(),
            outbox: Vec::new(),
            scratch: Events::new(),
            stats: NetStats::default(),
            cfg,
        };
        p.render_into_view();
        p.prev_view = p.view.clone();
        // Start a few ticks ahead; slack feedback converges the exact lead within a second.
        let lead = p.cfg.target_slack.ceil() as u32 + 1;
        for _ in 0..lead {
            p.predict_one();
        }
        Ok(p)
    }

    pub fn me(&self) -> PlayerId {
        self.me
    }
    pub fn setup(&self) -> &Setup {
        &self.setup
    }
    pub fn resume_token(&self) -> u64 {
        self.resume
    }
    /// Predicted present state (what the player sees).
    pub fn game(&self) -> &G {
        &self.game
    }
    /// Newest authoritative state.
    pub fn authoritative(&self) -> &G {
        &self.auth.game
    }
    pub fn tick(&self) -> u32 {
        self.tick
    }
    pub fn stats(&self) -> &NetStats {
        &self.stats
    }

    /// Set the input for the next simulated tick(s). Sanitized here exactly as the server will.
    pub fn set_input(&mut self, input: G::Input) {
        self.input = G::sanitize(input);
    }

    pub fn send_command(&mut self, cmd: G::Command) {
        let msg: ClientMsgOf<G> = super::proto::ClientMsg::Command(cmd);
        self.outbox.push((Channel::Reliable, gs_wire::to_bytes(&msg)));
    }

    /// Feed one message from either DataChannel.
    pub fn receive(&mut self, bytes: &[u8], now_ms: f64) -> Result<(), ClientError> {
        match gs_wire::from_bytes::<ServerMsg>(bytes)? {
            ServerMsg::Snapshot(s) => self.on_snapshot(s),
            ServerMsg::Pong { t, .. } => {
                let sent = f64::from(t);
                let now32 = (now_ms as u64 & 0xffff_ffff) as f64;
                let rtt = if now32 >= sent { now32 - sent } else { now32 + 4_294_967_296.0 - sent };
                if rtt < 10_000.0 {
                    self.rtt.sample(rtt as f32);
                }
            }
            ServerMsg::Notice(n) => self.notices.push(n),
            ServerMsg::Reject(r) => return Err(ClientError::Rejected(r)),
            ServerMsg::Welcome(_) => {} // only the first one matters (handled by the caller)
        }
        Ok(())
    }

    fn on_snapshot(&mut self, s: Snapshot) {
        if s.tick <= self.auth_tick {
            return; // stale or duplicate (unordered channel)
        }
        let bytes = match s.baseline {
            None => s.payload.0,
            Some(bt) => match self.baselines.iter().find(|(t, _)| *t == bt) {
                Some((_, base)) => match delta::decode(base, &s.payload.0) {
                    Ok(b) => b,
                    Err(_) => {
                        self.stats.dropped_snapshots += 1;
                        return;
                    }
                },
                None => {
                    self.stats.dropped_snapshots += 1;
                    return;
                }
            },
        };
        let Ok(snap) = gs_wire::from_bytes::<WorldSnap<G>>(&bytes) else {
            self.stats.dropped_snapshots += 1;
            return;
        };
        self.stats.snapshots += 1;
        self.stats.snapshot_bytes = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        self.auth_hash = gs_wire::fnv1a64(&gs_wire::to_bytes(&snap.game));
        self.auth = snap;
        self.auth_tick = s.tick;
        self.auth_dirty = true;
        self.baselines.push_back((s.tick, bytes));
        while self.baselines.len() > 32 {
            self.baselines.pop_front();
        }
        if s.slack != super::server::SLACK_UNKNOWN {
            // Converge fast right after connecting, then filter jitter.
            let alpha = if self.stats.snapshots < 20 { 0.4 } else { 0.1 };
            self.slack_ema += (f32::from(s.slack) - self.slack_ema) * alpha;
        }
        self.stats.server_tick = s.tick;
    }

    /// Advance to wall-clock `now_ms` (any monotonic ms clock, e.g. `performance.now()`).
    pub fn update(&mut self, now_ms: f64) {
        let elapsed = match self.last_now {
            Some(l) => (now_ms - l).clamp(0.0, 250.0),
            None => 0.0,
        };
        self.last_now = Some(now_ms);

        self.reconcile();
        self.steer();

        // Time dilation: run slightly fast when our inputs arrive late, slightly slow when early.
        let err = self.cfg.target_slack - self.slack_ema;
        let scale = 1.0 + (err * 0.02).clamp(-self.cfg.max_dilation, self.cfg.max_dilation);
        self.stats.time_scale = scale;
        self.acc_ms += elapsed * f64::from(scale);

        let mut steps = 0;
        let mut produced = false;
        self.stats.stalled = false;
        while self.acc_ms >= self.dt_ms && steps < self.cfg.max_steps_per_update {
            if self.tick >= self.auth_tick + self.cfg.max_prediction {
                // Server has gone quiet; hold rather than predict into fiction.
                self.stats.stalled = true;
                self.acc_ms = self.dt_ms;
                break;
            }
            self.acc_ms -= self.dt_ms;
            self.predict_one();
            steps += 1;
            produced = true;
        }
        if self.acc_ms > self.dt_ms * 2.0 {
            self.acc_ms = self.dt_ms; // drop backlog beyond the step cap
        }
        if produced {
            self.send_inputs();
        }
        if now_ms - self.last_ping >= 250.0 {
            self.last_ping = now_ms;
            let msg: ClientMsgOf<G> = super::proto::ClientMsg::Ping { t: (now_ms as u64 & 0xffff_ffff) as u32 };
            self.outbox.push((Channel::Unreliable, gs_wire::to_bytes(&msg)));
        }

        // Decay correction offsets.
        let k = math::exp(-(elapsed as f32 / 1000.0) / self.cfg.smoothing_tau.max(1e-3));
        self.smoothing.retain(|_, o| {
            *o *= k;
            o.length_squared() > 1e-8
        });

        self.stats.tick = self.tick;
        self.stats.rtt_ms = self.rtt.ema_ms;
        self.stats.jitter_ms = self.rtt.jitter_ms;
        self.stats.slack = self.slack_ema;
    }

    /// Hard resync when the lead is clearly off (first connect, a long stall, a clock jump).
    /// Slack measures exactly how early our inputs reach the server, so shift by its error.
    fn steer(&mut self) {
        if self.resync_cooldown > 0 {
            self.resync_cooldown -= 1;
            return;
        }
        let err = self.slack_ema - self.cfg.target_slack;
        let behind_server = self.tick <= self.auth_tick;
        if err.abs() <= 3.0 && !behind_server {
            return;
        }
        let shift = if behind_server {
            (self.auth_tick - self.tick) as i64 + self.cfg.target_slack.ceil() as i64 + 1
        } else {
            (-err).round() as i64
        };
        let target = (self.tick as i64 + shift).max(self.auth_tick as i64 + 1) as u32;
        self.stats.resyncs += 1;
        self.slack_ema = self.cfg.target_slack;
        // Let the effect travel to the server and back before judging again.
        self.resync_cooldown = 20;
        if target > self.tick {
            while self.tick < target {
                self.predict_one();
            }
        } else if target < self.tick {
            self.tick = target;
            self.inputs.retain(|(t, _)| *t < target);
            self.auth_dirty = true;
            self.reconcile();
        }
    }

    fn reconcile(&mut self) {
        if !self.auth_dirty {
            return;
        }
        self.auth_dirty = false;
        let at = self.auth_tick;
        // Inputs and hashes older than the authoritative tick are settled history.
        while self.inputs.front().is_some_and(|(t, _)| *t < at) {
            self.inputs.pop_front();
        }
        while self.pred_hashes.front().is_some_and(|(t, _)| *t < at) {
            self.pred_hashes.pop_front();
        }
        let cutoff = at.saturating_sub(G::TICK_HZ * 2);
        while self.seen_events.first().is_some_and(|(t, _)| *t < cutoff) {
            self.seen_events.pop_first();
        }

        if at >= self.tick {
            // Server is ahead of our prediction: adopt it and re-lead from there.
            self.game = self.auth.game.clone();
            self.roster_used = self.auth.roster.clone();
            self.tick = at;
            self.render_into_view();
            self.prev_view = self.view.clone();
            return;
        }

        let predicted = self.pred_hashes.iter().find(|(t, _)| *t == at).map(|(_, h)| *h);
        let others_same = roster_eq_except(&self.roster_used, &self.auth.roster, self.me);
        if predicted == Some(self.auth_hash) && others_same {
            self.stats.prediction_hits += 1;
            return;
        }

        // Roll back and re-simulate at..tick.
        let old = std::mem::take(&mut self.view);
        let target = self.tick;
        self.game = self.auth.game.clone();
        self.roster_used = self.auth.roster.clone();
        self.tick = at;
        self.pred_hashes.clear();
        while self.tick < target {
            if self.tick + 1 == target {
                // Interpolation needs the state one tick before the present.
                self.render_into_view();
                self.prev_view = std::mem::take(&mut self.view);
            }
            self.step_prediction();
        }
        self.render_into_view();
        self.stats.rollbacks += 1;
        self.stats.last_rollback_ticks = target - at;

        for e in &self.view.entities {
            if e.flags & FLAG_TELEPORT != 0 {
                self.smoothing.remove(&e.id);
                continue;
            }
            if let Some(o) = old.find(e.id) {
                let off = self.smoothing.get(&e.id).copied().unwrap_or(Vec3::ZERO) + (o.pos - e.pos);
                if off.length() < self.cfg.snap_distance {
                    self.smoothing.insert(e.id, off);
                } else {
                    self.smoothing.remove(&e.id);
                }
            }
        }
    }

    fn predict_one(&mut self) {
        let t = self.tick;
        if self.inputs.back().is_none_or(|(lt, _)| *lt < t) {
            self.inputs.push_back((t, self.input.clone()));
        }
        self.prev_view = std::mem::take(&mut self.view);
        self.step_prediction();
        self.render_into_view();
    }

    /// Step the predicted world one tick using recorded local input + roster for others.
    fn step_prediction(&mut self) {
        let t = self.tick;
        let mine = self
            .inputs
            .iter()
            .rev()
            .find(|(it, _)| *it == t)
            .map(|(_, i)| i.clone())
            .unwrap_or_else(|| self.input.clone());
        let inputs = self
            .roster_used
            .iter()
            .map(|(id, i)| (*id, if *id == self.me { mine.clone() } else { G::sanitize(i.clone()) }))
            .collect();
        let frame = FrameOf::<G> { tick: t, joins: vec![], leaves: vec![], inputs, commands: vec![] };
        self.game.step(&frame, &mut self.scratch);
        self.tick = t + 1;
        for (i, ev) in self.scratch.drain().enumerate() {
            let h = gs_wire::fnv1a64(&gs_wire::to_bytes(&ev)) ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            if self.seen_events.insert((t, h)) {
                self.events_out.push((t, ev));
            }
        }
        self.pred_hashes.push_back((self.tick, gs_wire::fnv1a64(&gs_wire::to_bytes(&self.game))));
        while self.pred_hashes.len() > 256 {
            self.pred_hashes.pop_front();
        }
        // Keep the remembered inputs consistent with the roster entry for "me" (repeat-last).
        if let Some(e) = self.roster_used.iter_mut().find(|(id, _)| *id == self.me) {
            e.1 = mine;
        }
    }

    fn render_into_view(&mut self) {
        self.view.clear();
        self.game.render(&mut self.view, Some(self.me));
        self.view.finish();
    }

    fn send_inputs(&mut self) {
        let n = self.inputs.len().min(INPUT_REDUNDANCY);
        if n == 0 {
            return;
        }
        let start = self.inputs.len() - n;
        let first_tick = self.inputs[start].0;
        let inputs = self.inputs.iter().skip(start).map(|(_, i)| i.clone()).collect();
        let msg: ClientMsgOf<G> =
            super::proto::ClientMsg::Inputs(InputBatch { first_tick, inputs, ack: Some(self.auth_tick) });
        self.outbox.push((Channel::Unreliable, gs_wire::to_bytes(&msg)));
    }

    /// Messages to send, as (channel, bytes).
    pub fn drain_outbox(&mut self) -> std::vec::Drain<'_, (Channel, Vec<u8>)> {
        self.outbox.drain(..)
    }

    pub fn drain_events(&mut self) -> std::vec::Drain<'_, (u32, G::Event)> {
        self.events_out.drain(..)
    }

    pub fn drain_notices(&mut self) -> std::vec::Drain<'_, String> {
        self.notices.drain(..)
    }

    /// Interpolated, smoothed render list for this display frame.
    pub fn render(&self, out: &mut Vec<RenderEntity>) {
        out.clear();
        let alpha = (self.acc_ms / self.dt_ms).clamp(0.0, 1.0) as f32;
        for e in &self.view.entities {
            let mut r = *e;
            if e.flags & FLAG_TELEPORT == 0
                && let Some(p) = self.prev_view.find(e.id)
                && p.kind == e.kind
            {
                r.pos = p.pos.lerp(e.pos, alpha);
                r.rot = p.rot.slerp(e.rot, alpha);
                r.scale = p.scale.lerp(e.scale, alpha);
                r.a = math::lerp(p.a, e.a, alpha);
                r.b = math::lerp(p.b, e.b, alpha);
            }
            if let Some(o) = self.smoothing.get(&e.id) {
                r.pos += *o;
            }
            out.push(r);
        }
    }

    pub fn hud(&self) -> G::Hud {
        self.game.hud(Some(self.me))
    }
}

fn roster_eq_except<I: PartialEq>(a: &[(PlayerId, I)], b: &[(PlayerId, I)], me: PlayerId) -> bool {
    let mut ai = a.iter().filter(|(id, _)| *id != me);
    let mut bi = b.iter().filter(|(id, _)| *id != me);
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y => {}
            _ => return false,
        }
    }
}

/// Decode the reliable-channel `Welcome` that starts a session.
pub fn decode_welcome(bytes: &[u8]) -> Result<Option<Welcome>, ClientError> {
    match gs_wire::from_bytes::<ServerMsg>(bytes)? {
        ServerMsg::Welcome(w) => Ok(Some(w)),
        ServerMsg::Reject(r) => Err(ClientError::Rejected(r)),
        _ => Ok(None),
    }
}

/// Encode a `Hello` for game `G`.
pub fn hello<G: Game>(name: &str, room: &str, mode: &str, bots: u8, resume: Option<u64>) -> Vec<u8> {
    let msg: ClientMsgOf<G> = super::proto::ClientMsg::Hello(super::proto::Hello {
        fingerprint: super::proto::fingerprint::<G>(),
        name: name.into(),
        room: room.into(),
        mode: mode.into(),
        resume,
        bots,
    });
    gs_wire::to_bytes(&msg)
}
