//! Bot "actor": a deterministic policy from state to input. Runs on the server for bot seats.

use super::{Input, OrbArena, buttons};
use gs_core::prelude::*;

pub(super) fn think(g: &OrbArena, me: PlayerId) -> Input {
    let Some(pl) = g.players.get(&me) else { return Input::default() };
    let pos = g.world[pl.body].pos;
    // Nearest orb, ties broken by entity id (deterministic).
    let target = g
        .orbs
        .iter()
        .map(|o| (g.world[o.body].pos, o.eid))
        .min_by(|a, b| (a.0 - pos).length_squared().total_cmp(&(b.0 - pos).length_squared()).then(a.1.cmp(&b.1)));
    let Some((tp, _)) = target else { return Input::default() };
    let to = vec3(tp.x - pos.x, 0.0, tp.z - pos.z);
    let dist = to.length();
    // Bots steer with yaw (camera-relative input with full forward).
    let yaw = math::dir_yaw(to);
    let mut b = 0;
    if dist > 6.0 && (g.tick + u32::from(me) * 17).is_multiple_of(90) {
        b |= buttons::DASH;
    }
    if tp.y - pos.y > 0.6 || (g.tick + u32::from(me) * 7).is_multiple_of(150) {
        b |= buttons::JUMP;
    }
    Input { mx: 0, mz: if dist > 0.3 { 127 } else { 0 }, yaw, buttons: b }
}
