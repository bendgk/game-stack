//! # Orb Arena — the example game
//!
//! Collect glowing orbs in a walled arena. Shove rivals with a dash, hop on crates.
//! - `coop`   — everyone fills one team score before the clock runs out.
//! - `versus` — most orbs when the clock runs out wins.
//! - `solo`   — like versus; add bots from the lobby.
//!
//! This one file (plus `arena.rs` and `bot.rs`) *is* the game. It runs as the authority on the
//! server and as the predictor in every browser. To add a feature: change the state + `step`
//! here, describe how it looks in `render`, and give it a React component (see `AGENTS.md`).

use std::collections::BTreeMap;

use gs_core::prelude::*;
use gs_physics::d3::{Body, BodyId, Shape, World};
use serde::{Deserialize, Serialize};

mod arena;
mod bot;
#[cfg(test)]
mod tests;

pub const TICK_HZ: u32 = 60;
pub const DT: f32 = 1.0 / TICK_HZ as f32;

const ROUND_TICKS: u32 = 90 * TICK_HZ;
const INTERMISSION_TICKS: u32 = 6 * TICK_HZ;
const MOVE_SPEED: f32 = 7.5;
const GROUND_ACCEL: f32 = 70.0;
const AIR_ACCEL: f32 = 18.0;
const JUMP_SPEED: f32 = 8.5;
const DASH_SPEED: f32 = 15.0;
const DASH_COOLDOWN: u16 = 50;
const PLAYER_RADIUS: f32 = 0.5;
const ORB_RADIUS: f32 = 0.45;

/// Body.user tags so contacts know what they hit.
const TAG_PLAYER: u32 = 1 << 16;
const TAG_ORB: u32 = 2 << 16;

// ---------------------------------------------------------------------------------------------
// Wire types shared with TypeScript (the `ts` feature generates their .d.ts)
// ---------------------------------------------------------------------------------------------

/// Input buttons — the single source of truth for both Rust and TypeScript
/// (`import { Button } from '@gs/engine'` in the web client).
#[cfg_attr(feature = "ts", wasm_bindgen::prelude::wasm_bindgen)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Button {
    Jump = 1,
    Dash = 2,
}

pub mod buttons {
    use super::Button;
    pub const JUMP: u8 = Button::Jump as u8;
    pub const DASH: u8 = Button::Dash as u8;
}

/// Sampled every tick from keyboard/gamepad (see `apps/web/src/game/input.ts`).
#[derive(Wire, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct Input {
    /// Strafe, -127..127 (right positive).
    pub mx: i8,
    /// Forward, -127..127 (forward positive).
    pub mz: i8,
    /// Camera yaw in radians; movement is camera-relative.
    pub yaw: f32,
    /// Bitset of [`buttons`].
    pub buttons: u8,
}

impl Input {
    pub fn pressed(&self, b: u8) -> bool {
        self.buttons & b != 0
    }
}

#[derive(Wire, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
#[serde(tag = "type")]
pub enum Command {
    SetColor { color: u32 },
    Emote { emote: u8 },
}

#[derive(Wire, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
#[serde(tag = "type")]
pub enum Event {
    Orb { player: PlayerId, x: f32, y: f32, z: f32 },
    Jump { player: PlayerId },
    Dash { player: PlayerId },
    Bump { x: f32, y: f32, z: f32, strength: f32 },
    RoundStart { round: u32 },
    RoundOver { winner: Option<String> },
    Emote { player: PlayerId, emote: u8 },
}

/// Entity kinds the renderer maps to React components (`apps/web/src/game/renderers.tsx`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
#[repr(u16)]
pub enum Kind {
    Player = 0,
    Orb = 1,
    Crate = 2,
    Wall = 3,
    Pad = 4,
}

/// Names indexed by discriminant (shown to TypeScript). Keep in the same order as `Kind`;
/// `kinds_are_consistent` in tests.rs checks it.
pub const KIND_NAMES: &[&str] = &["Player", "Orb", "Crate", "Wall", "Pad"];

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Player, Kind::Orb, Kind::Crate, Kind::Wall, Kind::Pad];
}

impl From<Kind> for u16 {
    fn from(k: Kind) -> u16 {
        k as u16
    }
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct ScoreRow {
    pub id: PlayerId,
    pub name: String,
    pub score: u32,
    pub color: u32,
    pub bot: bool,
    pub me: bool,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct Hud {
    pub mode: String,
    pub round: u32,
    pub playing: bool,
    pub seconds_left: u32,
    pub team_score: u32,
    pub team_goal: u32,
    pub scores: Vec<ScoreRow>,
    pub winner: Option<String>,
    /// 0..1, 1 = dash ready.
    pub dash: f32,
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

#[derive(Wire, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Coop,
    Versus,
}

#[derive(Wire, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Playing { ends: u32 },
    Intermission { until: u32 },
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Player {
    pub eid: u32,
    pub name: String,
    pub body: BodyId,
    pub score: u32,
    pub yaw: f32,
    pub dash_cd: u16,
    pub color: u32,
    pub emote: u8,
    pub emote_until: u32,
    pub bot: bool,
    pub was_grounded: bool,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Orb {
    pub eid: u32,
    pub body: BodyId,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Prop {
    pub eid: u32,
    pub body: BodyId,
    pub kind: u16,
    pub color: u32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct OrbArena {
    pub tick: u32,
    pub mode: Mode,
    pub mode_name: String,
    pub rng: Rng,
    pub world: World,
    pub players: BTreeMap<PlayerId, Player>,
    pub orbs: Vec<Orb>,
    pub props: Vec<Prop>,
    pub phase: Phase,
    pub round: u32,
    pub team_score: u32,
    pub next_eid: u32,
    pub result: Option<MatchResult>,
}

const PALETTE: [u32; 8] = [0x4cc9f0, 0xf72585, 0x80ed99, 0xffd166, 0x9b5de5, 0xff7b00, 0x00f5d4, 0xe5e5e5];

impl OrbArena {
    fn eid(&mut self) -> u32 {
        self.next_eid += 1;
        self.next_eid
    }

    fn team_goal(&self) -> u32 {
        10 + 6 * self.players.len() as u32
    }

    fn spawn_point(&mut self) -> Vec3 {
        for _ in 0..16 {
            let p = vec3(self.rng.range_f32(-12.0, 12.0), 1.2, self.rng.range_f32(-12.0, 12.0));
            if self.world.overlap_ball(p, 1.0, u32::MAX).is_empty() {
                return p;
            }
        }
        vec3(0.0, 4.0, 0.0)
    }

    fn spawn_orb(&mut self) {
        let mut p = self.spawn_point();
        p.y = 0.9;
        let eid = self.eid();
        let body = self.world.add(Body::sensor(Shape::Ball { r: ORB_RADIUS }, p).with_user(TAG_ORB | eid));
        self.orbs.push(Orb { eid, body });
    }

    fn target_orbs(&self) -> usize {
        3 + self.players.len()
    }

    fn start_round(&mut self, events: &mut Events<Event>) {
        self.round += 1;
        self.team_score = 0;
        self.result = None;
        self.phase = Phase::Playing { ends: self.tick + ROUND_TICKS };
        let ids: Vec<PlayerId> = self.players.keys().copied().collect();
        for id in ids {
            let p = self.spawn_point();
            let body = self.players[&id].body;
            self.world[body].pos = p;
            self.world[body].vel = Vec3::ZERO;
            if let Some(pl) = self.players.get_mut(&id) {
                pl.score = 0;
            }
        }
        events.emit(Event::RoundStart { round: self.round });
    }

    fn end_round(&mut self, events: &mut Events<Event>) {
        let mut standings: Vec<(PlayerId, i32)> = self.players.iter().map(|(id, p)| (*id, p.score as i32)).collect();
        standings.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let winner = match self.mode {
            Mode::Coop => {
                if self.team_score >= self.team_goal() {
                    Some("Team".to_string())
                } else {
                    None
                }
            }
            Mode::Versus => standings.first().filter(|s| s.1 > 0).map(|(id, _)| self.players[id].name.clone()),
        };
        let summary = match (&self.mode, &winner) {
            (Mode::Coop, Some(_)) => format!("Team collected {} / {}", self.team_score, self.team_goal()),
            (Mode::Coop, None) => format!("Out of time at {} / {}", self.team_score, self.team_goal()),
            (Mode::Versus, Some(w)) => format!("{w} wins"),
            (Mode::Versus, None) => "No winner".to_string(),
        };
        self.result = Some(MatchResult { standings, summary });
        self.phase = Phase::Intermission { until: self.tick + INTERMISSION_TICKS };
        events.emit(Event::RoundOver { winner });
    }

    fn add_player(&mut self, j: &Join) {
        let pos = self.spawn_point();
        let eid = self.eid();
        let body = self.world.add(
            Body::dynamic(Shape::Ball { r: PLAYER_RADIUS }, pos)
                .with_friction(0.2)
                .with_restitution(0.3)
                .with_user(TAG_PLAYER | u32::from(j.id)),
        );
        let color = PALETTE[j.id as usize % PALETTE.len()];
        self.players.insert(
            j.id,
            Player {
                eid,
                name: j.name.clone(),
                body,
                score: 0,
                yaw: 0.0,
                dash_cd: 0,
                color,
                emote: 0,
                emote_until: 0,
                bot: j.bot,
                was_grounded: false,
            },
        );
    }

    fn apply_input(&mut self, id: PlayerId, input: &Input, events: &mut Events<Event>) {
        let Some(pl) = self.players.get_mut(&id) else { return };
        let body = &mut self.world[pl.body];
        // Camera-relative move vector on XZ.
        let fwd = math::yaw_dir(input.yaw);
        let right = vec3(-fwd.z, 0.0, fwd.x);
        let mut wish = right * (f32::from(input.mx) / 127.0) + fwd * (f32::from(input.mz) / 127.0);
        if wish.length_squared() > 1.0 {
            wish = wish.normalize();
        }
        let accel = if body.grounded { GROUND_ACCEL } else { AIR_ACCEL } * DT;
        let target = wish * MOVE_SPEED;
        // Only steer toward the target if we're slower than it (keeps dash momentum).
        let horiz = vec3(body.vel.x, 0.0, body.vel.z);
        if horiz.length() <= MOVE_SPEED + 0.01 || wish.length_squared() < 0.01 {
            body.vel.x = math::approach(body.vel.x, target.x, accel);
            body.vel.z = math::approach(body.vel.z, target.z, accel);
        }
        if wish.length_squared() > 0.01 {
            pl.yaw = math::dir_yaw(wish);
        }
        if input.pressed(buttons::JUMP) && body.grounded && body.vel.y <= 0.5 {
            body.vel.y = JUMP_SPEED;
            events.emit(Event::Jump { player: id });
        }
        if pl.dash_cd > 0 {
            pl.dash_cd -= 1;
        } else if input.pressed(buttons::DASH) {
            let dir = math::yaw_dir(pl.yaw);
            body.vel.x = dir.x * DASH_SPEED;
            body.vel.z = dir.z * DASH_SPEED;
            pl.dash_cd = DASH_COOLDOWN;
            events.emit(Event::Dash { player: id });
        }
    }

    fn collect_orbs(&mut self, events: &mut Events<Event>) {
        let mut taken = Vec::new();
        for (i, orb) in self.orbs.iter().enumerate() {
            let hit = self
                .world
                .contacts_of(orb.body)
                .filter_map(|(other, _)| self.world.get(other).map(|b| b.user))
                .filter(|u| u & 0xffff_0000 == TAG_PLAYER)
                .map(|u| (u & 0xffff) as PlayerId)
                .min();
            if let Some(p) = hit {
                taken.push((i, p));
            }
        }
        for &(i, p) in taken.iter().rev() {
            let orb = self.orbs.remove(i);
            let pos = self.world[orb.body].pos;
            self.world.remove(orb.body);
            if let Some(pl) = self.players.get_mut(&p) {
                pl.score += 1;
            }
            self.team_score += 1;
            events.emit(Event::Orb { player: p, x: pos.x, y: pos.y, z: pos.z });
        }
    }

    fn bumps(&mut self, events: &mut Events<Event>) {
        for c in &self.world.contacts {
            let (Some(a), Some(b)) = (self.world.get(c.a), self.world.get(c.b)) else { continue };
            if a.user & 0xffff_0000 != TAG_PLAYER || b.user & 0xffff_0000 != TAG_PLAYER {
                continue;
            }
            let strength = (a.vel - b.vel).length();
            if strength > 6.0 {
                let p = (a.pos + b.pos) * 0.5;
                events.emit(Event::Bump { x: p.x, y: p.y, z: p.z, strength });
            }
        }
    }
}

impl Game for OrbArena {
    type Input = Input;
    type Command = Command;
    type Event = Event;
    type Hud = Hud;

    const TICK_HZ: u32 = TICK_HZ;
    const MAX_PLAYERS: usize = 8;
    const NAME: &'static str = "orb-arena";

    fn new(setup: &Setup) -> Self {
        let mode = match setup.mode.as_str() {
            "coop" => Mode::Coop,
            _ => Mode::Versus,
        };
        let mut g = OrbArena {
            tick: 0,
            mode,
            mode_name: if setup.mode.is_empty() { "versus".into() } else { setup.mode.clone() },
            rng: Rng::new(setup.seed),
            world: World::new(vec3(0.0, -24.0, 0.0)),
            players: BTreeMap::new(),
            orbs: Vec::new(),
            props: Vec::new(),
            phase: Phase::Playing { ends: ROUND_TICKS },
            round: 1,
            team_score: 0,
            next_eid: 0,
            result: None,
        };
        g.world.add(Body::floor(0.0).with_friction(0.3));
        arena::build(&mut g);
        g
    }

    fn step(&mut self, frame: &FrameOf<Self>, events: &mut Events<Event>) {
        for j in &frame.joins {
            self.add_player(j);
        }
        for id in &frame.leaves {
            if let Some(p) = self.players.remove(id) {
                self.world.remove(p.body);
            }
        }
        for (id, cmd) in &frame.commands {
            let Some(p) = self.players.get_mut(id) else { continue };
            match *cmd {
                Command::SetColor { color } => p.color = color & 0xff_ffff,
                Command::Emote { emote } => {
                    p.emote = emote.min(8);
                    p.emote_until = self.tick + 2 * TICK_HZ;
                    events.emit(Event::Emote { player: *id, emote: p.emote });
                }
            }
        }

        let playing = matches!(self.phase, Phase::Playing { .. });
        for (id, input) in &frame.inputs {
            if playing {
                self.apply_input(*id, input, events);
            }
        }
        if !playing {
            // Frozen during intermission: bleed off momentum.
            let bodies: Vec<BodyId> = self.players.values().map(|p| p.body).collect();
            for b in bodies {
                let v = &mut self.world[b].vel;
                v.x *= 0.9;
                v.z *= 0.9;
            }
        }

        self.world.step(DT);
        for p in self.players.values_mut() {
            p.was_grounded = self.world[p.body].grounded;
        }
        if playing {
            self.collect_orbs(events);
            self.bumps(events);
            while self.orbs.len() < self.target_orbs() {
                self.spawn_orb();
            }
        }

        match self.phase {
            Phase::Playing { ends } => {
                let coop_done = self.mode == Mode::Coop && self.team_score >= self.team_goal();
                if (self.tick + 1 >= ends || coop_done) && !self.players.is_empty() {
                    self.end_round(events);
                }
            }
            Phase::Intermission { until } => {
                if self.tick + 1 >= until {
                    self.start_round(events);
                }
            }
        }
        self.tick += 1;
    }

    fn sanitize(mut input: Input) -> Input {
        if !input.yaw.is_finite() {
            input.yaw = 0.0;
        }
        input.yaw = math::wrap_angle(input.yaw);
        input.mx = input.mx.max(-127);
        input.mz = input.mz.max(-127);
        input.buttons &= buttons::JUMP | buttons::DASH;
        input
    }

    fn bot_input(&self, me: PlayerId) -> Input {
        bot::think(self, me)
    }

    fn reward(&self, me: PlayerId) -> f32 {
        self.players.get(&me).map_or(0.0, |p| p.score as f32)
    }

    fn result(&self) -> Option<MatchResult> {
        self.result.clone()
    }

    fn render(&self, view: &mut RenderView, me: Option<PlayerId>) {
        for (id, p) in &self.players {
            let b = &self.world[p.body];
            let dash = 1.0 - f32::from(p.dash_cd) / f32::from(DASH_COOLDOWN);
            let emote = if self.tick < p.emote_until { f32::from(p.emote) } else { 0.0 };
            let mut e =
                RenderEntity::new(p.eid, Kind::Player, b.pos).rot(math::yaw_quat(p.yaw)).color(p.color).ab(dash, emote);
            if Some(*id) == me {
                e = e.flags(FLAG_ME);
            }
            view.push(e);
        }
        for o in &self.orbs {
            let b = &self.world[o.body];
            let t = self.tick as f32 * DT;
            let bob = math::sin(t * 3.0 + o.eid as f32) * 0.15;
            view.push(
                RenderEntity::new(o.eid, Kind::Orb, b.pos + vec3(0.0, bob, 0.0))
                    .rot(math::yaw_quat(t * 2.0))
                    .color(0xffd166),
            );
        }
        for p in &self.props {
            let b = &self.world[p.body];
            view.push(RenderEntity::new(p.eid, p.kind, b.pos).scale(b.shape.half_extents() * 2.0).color(p.color));
        }
    }

    fn hud(&self, me: Option<PlayerId>) -> Hud {
        let mut scores: Vec<ScoreRow> = self
            .players
            .iter()
            .map(|(id, p)| ScoreRow {
                id: *id,
                name: p.name.clone(),
                score: p.score,
                color: p.color,
                bot: p.bot,
                me: Some(*id) == me,
            })
            .collect();
        scores.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
        let (playing, left) = match self.phase {
            Phase::Playing { ends } => (true, ends.saturating_sub(self.tick)),
            Phase::Intermission { until } => (false, until.saturating_sub(self.tick)),
        };
        let dash = me
            .and_then(|m| self.players.get(&m))
            .map_or(1.0, |p| 1.0 - f32::from(p.dash_cd) / f32::from(DASH_COOLDOWN));
        Hud {
            mode: self.mode_name.clone(),
            round: self.round,
            playing,
            seconds_left: left.div_ceil(TICK_HZ),
            team_score: self.team_score,
            team_goal: self.team_goal(),
            scores,
            winner: self.result.as_ref().map(|r| r.summary.clone()),
            dash,
        }
    }

    fn kind_names() -> &'static [&'static str] {
        KIND_NAMES
    }
}

/// The concrete game the server and client binaries are built for.
pub type TheGame = OrbArena;
