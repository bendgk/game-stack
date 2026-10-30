//! The authority: the server's sans-IO room core.
//!
//! It owns the one true game state and turns per-player input streams into a canonical
//! [`Frame`] each tick. It does no I/O and reads no clock — the room actor calls
//! [`Authority::step`] on its fixed timestep and ships the bytes this returns. That makes it
//! trivially testable (see `tests/netcode.rs`) and reusable in the browser for offline play.
//!
//! Per tick, for every seat in `PlayerId` order:
//! - a human whose input for this tick arrived → that input (first copy wins; history can't be
//!   rewritten by a later packet);
//! - a connected human whose input is late → their previous input (repeat-last);
//! - a disconnected human → neutral `Input::default()`;
//! - a bot → `Game::bot_input`.
//!
//! Every input passes through `Game::sanitize` (the critic) before the sim sees it.

use std::collections::{BTreeMap, VecDeque};

use gs_wire::{Blob, Wire, WireError};

use super::delta;
use super::proto::{InputBatch, Snapshot, Welcome, WorldSnap};
use crate::game::{Events, FrameOf, Game, Join, MatchResult, PlayerId, Setup};

/// `Snapshot::slack` value meaning "no input received yet".
pub const SLACK_UNKNOWN: i16 = i16::MIN;

#[derive(Clone, Debug)]
pub struct AuthorityConfig {
    /// Inputs further than this many ticks in the future are dropped.
    pub max_input_ahead: u32,
    /// Snapshots kept as delta baselines.
    pub history: usize,
}

impl AuthorityConfig {
    pub fn for_game<G: Game>() -> Self {
        AuthorityConfig { max_input_ahead: G::TICK_HZ * 2, history: G::TICK_HZ as usize }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SeatStats {
    pub inputs_received: u64,
    /// Ticks where this seat's input had not arrived in time.
    pub inputs_missed: u64,
    /// Inputs that arrived for a tick already simulated.
    pub inputs_late: u64,
}

#[derive(Clone, Debug)]
pub struct Seat<G: Game> {
    pub name: String,
    pub bot: bool,
    pub connected: bool,
    pub resume: u64,
    /// Joined the sim (its `Join` has been applied by a step).
    pub active: bool,
    buffer: BTreeMap<u32, G::Input>,
    last: G::Input,
    newest: Option<u32>,
    slack: i16,
    ack: Option<u32>,
    pub stats: SeatStats,
}

/// A checkpoint: everything needed to resume a room exactly (durable-object state).
#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Checkpoint<G: Game> {
    pub tick: u32,
    pub setup: Setup,
    pub game: G,
    pub seats: Vec<SeatRecord<G::Input>>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct SeatRecord<I> {
    pub id: PlayerId,
    pub name: String,
    pub bot: bool,
    pub resume: u64,
    pub last: I,
}

/// What one tick produced.
pub struct StepOutput<G: Game> {
    /// The canonical frame (append it to the durable log for replay).
    pub frame: FrameOf<G>,
    pub events: Vec<G::Event>,
    /// Seats whose join was applied this tick — send them a `Welcome` now.
    pub joined: Vec<PlayerId>,
    /// Set on the first tick `Game::result` returns `Some`.
    pub result: Option<MatchResult>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum JoinError {
    Full,
}

pub struct Authority<G: Game> {
    cfg: AuthorityConfig,
    setup: Setup,
    game: G,
    tick: u32,
    seats: BTreeMap<PlayerId, Seat<G>>,
    pending_joins: Vec<Join>,
    pending_leaves: Vec<PlayerId>,
    pending_commands: Vec<(PlayerId, G::Command)>,
    history: VecDeque<(u32, Vec<u8>)>,
    events: Events<G::Event>,
    result_reported: bool,
}

impl<G: Game> Authority<G> {
    pub fn new(setup: Setup) -> Self {
        Self::with_config(setup, AuthorityConfig::for_game::<G>())
    }

    pub fn with_config(setup: Setup, cfg: AuthorityConfig) -> Self {
        let game = G::new(&setup);
        let mut a = Authority {
            cfg,
            setup,
            game,
            tick: 0,
            seats: BTreeMap::new(),
            pending_joins: Vec::new(),
            pending_leaves: Vec::new(),
            pending_commands: Vec::new(),
            history: VecDeque::new(),
            events: Events::new(),
            result_reported: false,
        };
        a.record_history();
        a
    }

    pub fn setup(&self) -> &Setup {
        &self.setup
    }
    pub fn game(&self) -> &G {
        &self.game
    }
    /// Ticks simulated so far.
    pub fn tick(&self) -> u32 {
        self.tick
    }
    pub fn seats(&self) -> impl Iterator<Item = (PlayerId, &Seat<G>)> {
        self.seats.iter().map(|(id, s)| (*id, s))
    }
    pub fn seat(&self, id: PlayerId) -> Option<&Seat<G>> {
        self.seats.get(&id)
    }
    pub fn humans_connected(&self) -> usize {
        self.seats.values().filter(|s| !s.bot && s.connected).count()
    }

    /// Reserve a seat. The join enters the sim on the next `step`. `resume` is an unguessable
    /// token minted by the caller (the authority itself never touches OS randomness).
    pub fn join(&mut self, name: &str, bot: bool, resume: u64) -> Result<PlayerId, JoinError> {
        if self.seats.len() >= G::MAX_PLAYERS {
            return Err(JoinError::Full);
        }
        let id = (0..).find(|i| !self.seats.contains_key(i)).expect("PlayerId space");
        let name: String = name.chars().filter(|c| !c.is_control()).take(24).collect();
        self.seats.insert(
            id,
            Seat {
                name: name.clone(),
                bot,
                connected: !bot,
                resume,
                active: false,
                buffer: BTreeMap::new(),
                last: G::Input::default(),
                newest: None,
                slack: 0,
                ack: None,
                stats: SeatStats::default(),
            },
        );
        self.pending_joins.push(Join { id, name, bot });
        Ok(id)
    }

    /// Reclaim a seat by resume token after a dropped connection.
    pub fn resume(&mut self, token: u64) -> Option<PlayerId> {
        let (id, seat) = self.seats.iter_mut().find(|(_, s)| !s.bot && s.resume == token)?;
        seat.connected = true;
        seat.ack = None; // the new connection holds no baselines
        seat.buffer.clear();
        seat.newest = None;
        Some(*id)
    }

    /// Restore a seat's resume token after rehydrating from the durable log.
    pub fn set_resume(&mut self, id: PlayerId, token: u64) {
        if let Some(s) = self.seats.get_mut(&id) {
            s.resume = token;
        }
    }

    /// Connection dropped: keep the seat (neutral input) so the player can resume.
    pub fn disconnect(&mut self, id: PlayerId) {
        if let Some(s) = self.seats.get_mut(&id) {
            s.connected = false;
            s.buffer.clear();
        }
    }

    /// Remove the player from the sim on the next step.
    pub fn leave(&mut self, id: PlayerId) {
        if self.seats.contains_key(&id) && !self.pending_leaves.contains(&id) {
            self.pending_leaves.push(id);
        }
    }

    pub fn receive_inputs(&mut self, id: PlayerId, batch: InputBatch<G::Input>) {
        let (now, ahead) = (self.tick, self.cfg.max_input_ahead);
        let Some(seat) = self.seats.get_mut(&id) else { return };
        if seat.bot {
            return;
        }
        if let Some(ack) = batch.ack
            && ack <= now
            && seat.ack.is_none_or(|a| ack > a)
        {
            seat.ack = Some(ack);
        }
        for (i, input) in batch.inputs.into_iter().enumerate() {
            let Some(t) = batch.first_tick.checked_add(i as u32) else { break };
            if t < now {
                if seat.newest.is_none_or(|n| t > n) {
                    seat.stats.inputs_late += 1;
                    // Still track it: the (negative) slack is what tells the client to lead more.
                    seat.newest = Some(t);
                }
                continue;
            }
            if t >= now.saturating_add(ahead) {
                break;
            }
            if let std::collections::btree_map::Entry::Vacant(e) = seat.buffer.entry(t) {
                e.insert(input);
                seat.stats.inputs_received += 1;
            }
            seat.newest = Some(seat.newest.map_or(t, |n| n.max(t)));
        }
    }

    /// Inject one input directly (headless env, tests, local loopback).
    pub fn inject(&mut self, id: PlayerId, tick: u32, input: G::Input) {
        self.receive_inputs(id, InputBatch { first_tick: tick, inputs: vec![input], ack: None });
    }

    /// A command from the server itself (an admin tool, an MCP agent, a scripted event). It
    /// enters the log like any other command, attributed to [`crate::SERVER`], so replays and
    /// predictions see it too.
    pub fn server_command(&mut self, cmd: G::Command) {
        if self.pending_commands.len() < 1024 {
            self.pending_commands.push((crate::SERVER, cmd));
        }
    }

    pub fn command(&mut self, id: PlayerId, cmd: G::Command) {
        if self.seats.get(&id).is_some_and(|s| s.active) && self.pending_commands.len() < 1024 {
            self.pending_commands.push((id, cmd));
        }
    }

    /// Simulate one tick.
    pub fn step(&mut self) -> StepOutput<G> {
        let tick = self.tick;
        let joins = std::mem::take(&mut self.pending_joins);
        let leaves = std::mem::take(&mut self.pending_leaves);
        let commands = std::mem::take(&mut self.pending_commands);
        let mut joined = Vec::new();
        for j in &joins {
            if let Some(s) = self.seats.get_mut(&j.id) {
                s.active = true;
                joined.push(j.id);
            }
        }
        for id in &leaves {
            self.seats.remove(id);
        }
        let mut inputs = Vec::with_capacity(self.seats.len());
        for (&id, seat) in self.seats.iter_mut() {
            let raw = if seat.bot {
                self.game.bot_input(id)
            } else if !seat.connected {
                G::Input::default()
            } else if let Some(i) = seat.buffer.remove(&tick) {
                i
            } else {
                seat.stats.inputs_missed += 1;
                seat.last.clone()
            };
            seat.slack = match seat.newest {
                Some(n) => (i64::from(n) - i64::from(tick)).clamp(-1000, 1000) as i16,
                None => SLACK_UNKNOWN,
            };
            // Drop anything older than the next tick (late duplicates).
            while seat.buffer.first_key_value().is_some_and(|(t, _)| *t <= tick) {
                seat.buffer.pop_first();
            }
            let input = G::sanitize(raw);
            seat.last = input.clone();
            inputs.push((id, input));
        }
        let frame = FrameOf::<G> { tick, joins, leaves, inputs, commands };
        self.apply(&frame);
        let events = self.events.drain().collect();
        // Report a result on the tick it appears (None -> Some), so multi-round games
        // report every round.
        let r = self.game.result();
        let result = if self.result_reported { None } else { r.clone() };
        self.result_reported = r.is_some();
        StepOutput { frame, events, joined, result }
    }

    fn apply(&mut self, frame: &FrameOf<G>) {
        self.game.step(frame, &mut self.events);
        self.tick = frame.tick + 1;
        self.record_history();
    }

    /// Re-apply a logged frame (rehydration). Seat bookkeeping follows the frame's joins/leaves.
    pub fn replay(&mut self, frame: &FrameOf<G>) -> Result<(), String> {
        if frame.tick != self.tick {
            return Err(format!("replay gap: at tick {} got frame {}", self.tick, frame.tick));
        }
        for j in &frame.joins {
            self.seats.entry(j.id).or_insert_with(|| Seat {
                name: j.name.clone(),
                bot: j.bot,
                connected: false,
                resume: 0,
                active: true,
                buffer: BTreeMap::new(),
                last: G::Input::default(),
                newest: None,
                slack: 0,
                ack: None,
                stats: SeatStats::default(),
            });
        }
        for id in &frame.leaves {
            self.seats.remove(id);
        }
        for (id, input) in &frame.inputs {
            if let Some(s) = self.seats.get_mut(id) {
                s.last = input.clone();
            }
        }
        self.apply(frame);
        self.events.drain();
        self.result_reported = self.game.result().is_some();
        Ok(())
    }

    fn roster(&self) -> Vec<(PlayerId, G::Input)> {
        self.seats.iter().filter(|(_, s)| s.active).map(|(id, s)| (*id, s.last.clone())).collect()
    }

    fn record_history(&mut self) {
        let snap = WorldSnap::<G> { game: self.game.clone(), roster: self.roster() };
        self.history.push_back((self.tick, gs_wire::to_bytes(&snap)));
        while self.history.len() > self.cfg.history.max(2) {
            self.history.pop_front();
        }
    }

    /// The latest snapshot for `id`, delta-compressed against its newest acked baseline.
    pub fn snapshot_for(&self, id: PlayerId) -> Snapshot {
        let (tick, bytes) = self.history.back().expect("history is never empty");
        let seat = self.seats.get(&id);
        let slack = seat.map_or(0, |s| s.slack);
        let base =
            seat.and_then(|s| s.ack).filter(|a| a != tick).and_then(|a| self.history.iter().find(|(t, _)| *t == a));
        match base {
            Some((bt, bb)) => {
                Snapshot { tick: *tick, baseline: Some(*bt), slack, payload: Blob(delta::encode(bb, bytes)) }
            }
            None => Snapshot { tick: *tick, baseline: None, slack, payload: Blob(bytes.clone()) },
        }
    }

    pub fn welcome_for(&self, id: PlayerId, room: &str) -> Option<Welcome> {
        let seat = self.seats.get(&id)?;
        let (tick, bytes) = self.history.back()?;
        Some(Welcome {
            player: id,
            room: room.to_string(),
            setup: self.setup.clone(),
            tick_hz: G::TICK_HZ as u16,
            resume: seat.resume,
            snapshot: Snapshot { tick: *tick, baseline: None, slack: 0, payload: Blob(bytes.clone()) },
        })
    }

    pub fn checkpoint(&self) -> Checkpoint<G> {
        Checkpoint {
            tick: self.tick,
            setup: self.setup.clone(),
            game: self.game.clone(),
            seats: self
                .seats
                .iter()
                .filter(|(_, s)| s.active)
                .map(|(id, s)| SeatRecord {
                    id: *id,
                    name: s.name.clone(),
                    bot: s.bot,
                    resume: s.resume,
                    last: s.last.clone(),
                })
                .collect(),
        }
    }

    pub fn restore(cp: Checkpoint<G>) -> Self {
        let mut a = Authority {
            cfg: AuthorityConfig::for_game::<G>(),
            setup: cp.setup,
            result_reported: cp.game.result().is_some(),
            game: cp.game,
            tick: cp.tick,
            seats: BTreeMap::new(),
            pending_joins: Vec::new(),
            pending_leaves: Vec::new(),
            pending_commands: Vec::new(),
            history: VecDeque::new(),
            events: Events::new(),
        };
        for r in cp.seats {
            a.seats.insert(
                r.id,
                Seat {
                    name: r.name,
                    bot: r.bot,
                    connected: false,
                    resume: r.resume,
                    active: true,
                    buffer: BTreeMap::new(),
                    last: r.last,
                    newest: None,
                    slack: 0,
                    ack: None,
                    stats: SeatStats::default(),
                },
            );
        }
        a.record_history();
        a
    }

    pub fn restore_bytes(bytes: &[u8]) -> Result<Self, WireError> {
        Ok(Self::restore(gs_wire::from_bytes(bytes)?))
    }

    /// Hash of the full state (for determinism checks).
    pub fn state_hash(&self) -> u64 {
        gs_wire::fnv1a64(&gs_wire::to_bytes(&self.game))
    }
}
