//! The one trait a game implements. The same impl runs as the authority on the server and as
//! the predictor in the browser (compiled to wasm), so a feature is written exactly once.
//!
//! Rules a `Game` must follow (enforced by tests + clippy, see `docs/DETERMINISM.md`):
//! 1. `step` is a pure function of `(self, frame)`: no wall-clock, no OS randomness, no
//!    `HashMap` iteration, no platform float functions — use `gs_core::{math, rng}`.
//! 2. Everything `step` reads lives in `self` and round-trips through `Wire`. Derived data
//!    marked `#[wire(skip)]` must be recomputed by `step` before it is read.
//! 3. Presentation (`render`, `hud`) only reads state; it never mutates it.

use crate::render::RenderView;
use gs_wire::Wire;
use std::fmt::Debug;

/// A player slot in a room. Stable for the room's lifetime (survives reconnects).
pub type PlayerId = u16;

/// The id commands issued by the server itself carry (`Authority::server_command`). Never a
/// seat: games that accept server commands check for it explicitly.
pub const SERVER: PlayerId = PlayerId::MAX;

/// How a room was created; known to both server and clients before tick 0.
#[derive(Wire, Clone, Debug, PartialEq, Eq, Default)]
pub struct Setup {
    pub seed: u64,
    /// Free-form game mode chosen at room creation ("coop", "versus", "solo"…).
    pub mode: String,
}

#[derive(Wire, Clone, Debug, PartialEq, Eq)]
pub struct Join {
    pub id: PlayerId,
    pub name: String,
    /// Driven by `Game::bot_input` on the server.
    pub bot: bool,
}

/// Everything that happens in one tick, in canonical order. This is also the durable log
/// record: `state(t+1) = step(state(t), frame(t))`, so `seed + frames` replays a whole match.
#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Frame<I, C> {
    pub tick: u32,
    /// Players entering this tick (apply before inputs).
    pub joins: Vec<Join>,
    /// Players leaving this tick (their input is absent from `inputs`).
    pub leaves: Vec<PlayerId>,
    /// One input per present player, sorted by `PlayerId`.
    pub inputs: Vec<(PlayerId, I)>,
    /// Reliable one-shot actions (chat, pick team, buy item…), in arrival order.
    pub commands: Vec<(PlayerId, C)>,
}

impl<I, C> Frame<I, C> {
    pub fn input(&self, id: PlayerId) -> Option<&I> {
        self.inputs.binary_search_by_key(&id, |(p, _)| *p).ok().map(|i| &self.inputs[i].1)
    }
}

pub type FrameOf<G> = Frame<<G as Game>::Input, <G as Game>::Command>;

/// One-shot events a step emits (sounds, particles, kill-feed). The client fires each
/// (tick, event) at most once even when prediction re-simulates the tick.
pub struct Events<E> {
    pub(crate) list: Vec<E>,
}

impl<E> Events<E> {
    pub fn new() -> Self {
        Events { list: Vec::new() }
    }
    pub fn emit(&mut self, e: E) {
        self.list.push(e);
    }
    pub fn drain(&mut self) -> std::vec::Drain<'_, E> {
        self.list.drain(..)
    }
    pub fn as_slice(&self) -> &[E] {
        &self.list
    }
}

impl<E> Default for Events<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// Final standings, reported once when a match ends (persisted to Postgres by the server).
#[derive(Wire, Clone, Debug, PartialEq, serde::Serialize)]
pub struct MatchResult {
    /// (player, score), best first.
    pub standings: Vec<(PlayerId, i32)>,
    pub summary: String,
}

pub trait Game: Wire + Clone + Debug + 'static {
    /// Per-tick player intent (sampled every tick, sent unreliably with redundancy).
    type Input: Wire + Clone + Default + PartialEq + Debug;
    /// Reliable one-shot actions that enter the sim at a server-chosen tick.
    type Command: Wire + Clone + Debug;
    /// One-shot presentation events.
    type Event: Wire + Clone + Debug + serde::Serialize;
    /// Per-viewer UI data (scores, health, timers) handed to React.
    type Hud: serde::Serialize;

    /// Simulation rate. Inputs, snapshots and prediction all run on this clock.
    const TICK_HZ: u32 = 60;
    const MAX_PLAYERS: usize = 8;
    /// Short id for this game; part of the protocol fingerprint.
    const NAME: &'static str;

    fn new(setup: &Setup) -> Self;

    /// Advance exactly one tick.
    fn step(&mut self, frame: &FrameOf<Self>, events: &mut Events<Self::Event>);

    /// **Critic**: the server's referee for raw client input. Clamp, normalise or reject
    /// anything a hostile client could send. Runs on the server *and* in prediction.
    fn sanitize(input: Self::Input) -> Self::Input {
        input
    }

    /// **Actor**: input for a server-driven bot. Deterministic in `self`.
    fn bot_input(&self, _me: PlayerId) -> Self::Input {
        Self::Input::default()
    }

    /// Optional scalar reward for a player (RL training / bot evaluation via `env::Env`).
    fn reward(&self, _me: PlayerId) -> f32 {
        0.0
    }

    /// `Some` once the match is over. The server persists it and may restart the room.
    fn result(&self) -> Option<MatchResult> {
        None
    }

    /// Describe what to draw. `me` is the local player (None for spectators / server).
    fn render(&self, view: &mut RenderView, me: Option<PlayerId>);

    fn hud(&self, me: Option<PlayerId>) -> Self::Hud;

    /// Debug name of the entity kind `k` (shown in overlays). Usually from your `Kind` enum.
    fn kind_names() -> &'static [&'static str] {
        &[]
    }
}
