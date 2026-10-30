//! Headless environment: run a game with no network, gym-style.
//!
//! Use it for unit tests of game rules, bot tuning, balance sweeps, determinism probes and
//! RL training loops (actor = your policy producing `Input`s, critic = `Game::reward`).
//!
//! ```ignore
//! let mut env = Env::<MyGame>::new(Setup { seed: 1, mode: "versus".into() });
//! let me = env.add_player("alice");
//! env.add_bot("bot");
//! for _ in 0..600 {
//!     env.set_input(me, policy(env.game(), me));
//!     env.step();
//!     let r = env.reward(me);
//! }
//! ```

use crate::game::{Game, MatchResult, PlayerId, Setup};
use crate::net::server::Authority;

pub struct Env<G: Game> {
    pub auth: Authority<G>,
    events: Vec<G::Event>,
}

impl<G: Game> Env<G> {
    pub fn new(setup: Setup) -> Self {
        Env { auth: Authority::new(setup), events: Vec::new() }
    }

    /// A human-controlled seat (drive it with [`Env::set_input`]). Joins on the next step.
    pub fn add_player(&mut self, name: &str) -> PlayerId {
        self.auth.join(name, false, 0).expect("room full")
    }

    /// A seat driven by `Game::bot_input`.
    pub fn add_bot(&mut self, name: &str) -> PlayerId {
        self.auth.join(name, true, 0).expect("room full")
    }

    pub fn remove(&mut self, id: PlayerId) {
        self.auth.leave(id);
    }

    /// Input for the next tick only.
    pub fn set_input(&mut self, id: PlayerId, input: G::Input) {
        let t = self.auth.tick();
        self.auth.inject(id, t, input);
    }

    pub fn command(&mut self, id: PlayerId, cmd: G::Command) {
        self.auth.command(id, cmd);
    }

    /// Advance one tick; returns the match result the first tick it becomes available.
    pub fn step(&mut self) -> Option<MatchResult> {
        let out = self.auth.step();
        self.events.extend(out.events);
        out.result
    }

    /// Step `n` ticks (bots act, humans repeat their last input).
    pub fn run(&mut self, n: u32) -> Option<MatchResult> {
        let mut res = None;
        for _ in 0..n {
            res = res.or(self.step());
        }
        res
    }

    pub fn game(&self) -> &G {
        self.auth.game()
    }
    pub fn tick(&self) -> u32 {
        self.auth.tick()
    }
    pub fn reward(&self, id: PlayerId) -> f32 {
        self.auth.game().reward(id)
    }
    pub fn drain_events(&mut self) -> std::vec::Drain<'_, G::Event> {
        self.events.drain(..)
    }
    pub fn state_hash(&self) -> u64 {
        self.auth.state_hash()
    }
}

/// Determinism probe: `bots` bots play for `ticks` ticks; returns the final state hash.
/// The same call must return the same value natively and in wasm (`scripts/verify.mjs`).
pub fn probe<G: Game>(seed: u64, mode: &str, bots: u8, ticks: u32) -> u64 {
    let mut env = Env::<G>::new(Setup { seed, mode: mode.into() });
    for i in 0..bots {
        env.add_bot(&format!("probe-{i}"));
    }
    env.run(ticks);
    env.state_hash()
}
