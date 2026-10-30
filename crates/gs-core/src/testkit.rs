//! Reusable netcode tests for *any* `Game`. Call these from your game's tests so every rule
//! change is also checked against the real netcode path (`crates/game/src/tests.rs`).
//!
//! - [`soak`]: server `RoomHost` + N browser `Predictor`s over simulated links (latency,
//!   jitter, loss), clients driven by the game's own bot policy on their *predicted* state.
//! - [`replay_check`]: checkpoint + durable frame log reproduces the live state bit-for-bit
//!   (what the server relies on to resume rooms from Dragonfly after a restart).

use crate::game::{Game, Setup};
use crate::net::client::{Predictor, decode_welcome};
use crate::net::host::RoomHost;
use crate::net::link::{Link, LinkConfig};
use crate::net::proto::Channel;
use crate::net::server::Authority;

#[derive(Clone, Debug)]
pub struct SoakReport {
    pub welcomed: usize,
    pub server_ticks: u32,
    pub snapshots: u32,
    pub rollbacks: u32,
    pub prediction_hits: u32,
    pub inputs_missed: u64,
    pub inputs_received: u64,
    pub final_slack: Vec<f32>,
    pub resyncs: u32,
}

impl SoakReport {
    /// Fraction of server ticks where a client's input was not there in time.
    pub fn miss_rate(&self) -> f64 {
        self.inputs_missed as f64 / (self.inputs_missed + self.inputs_received).max(1) as f64
    }
}

/// Run `clients` predictors against one room for `seconds` of simulated time.
pub fn soak<G: Game>(setup: Setup, clients: usize, bots: u8, link: LinkConfig, seconds: f64) -> SoakReport {
    let dt = 1000.0 / f64::from(G::TICK_HZ);
    let mut host = RoomHost::<G>::new("soak", setup);
    host.add_bots(bots);
    let mut ups: Vec<Link<(Channel, Vec<u8>)>> =
        (0..clients).map(|i| Link::new(link.clone(), 100 + i as u64)).collect();
    let mut downs: Vec<Link<(Channel, Vec<u8>)>> =
        (0..clients).map(|i| Link::new(link.clone(), 200 + i as u64)).collect();
    let mut preds: Vec<Option<Predictor<G>>> = (0..clients).map(|_| None).collect();
    let mut token = 0u64;

    for (i, up) in ups.iter_mut().enumerate() {
        let hello = crate::net::client::hello::<G>(&format!("p{i}"), "soak", "", 0, None);
        up.send(0.0, true, (Channel::Reliable, hello));
    }

    let mut now = 0.0f64;
    let mut next_tick = 0.0f64;
    let mut next_frame: Vec<f64> = (0..clients).map(|i| i as f64 * 3.7).collect();
    let end = seconds * 1000.0;
    let mut server_ticks = 0;
    while now < end {
        // Server: receive, tick on its fixed clock, send.
        for (c, up) in ups.iter_mut().enumerate() {
            for (_, bytes) in up.recv(now) {
                host.on_message(c as u64, &bytes, || {
                    token += 1;
                    token
                });
            }
        }
        while next_tick <= now {
            host.tick();
            server_ticks += 1;
            next_tick += dt;
        }
        for (conn, ch, bytes) in host.drain_outbox() {
            downs[conn as usize].send(now, ch == Channel::Reliable, (ch, bytes));
        }
        // Clients: ~60 fps display loop with phase offsets.
        for c in 0..clients {
            for (_, bytes) in downs[c].recv(now) {
                match &mut preds[c] {
                    None => {
                        if let Ok(Some(w)) = decode_welcome(&bytes) {
                            preds[c] = Predictor::new(w).ok();
                        }
                    }
                    Some(p) => {
                        let _ = p.receive(&bytes, now);
                    }
                }
            }
            if now >= next_frame[c] {
                next_frame[c] += 16.67;
                if let Some(p) = &mut preds[c] {
                    let input = p.game().bot_input(p.me());
                    p.set_input(input);
                    p.update(now);
                    for (ch, bytes) in p.drain_outbox() {
                        ups[c].send(now, ch == Channel::Reliable, (ch, bytes));
                    }
                    p.drain_events().for_each(drop);
                }
            }
        }
        now += 0.5;
        if std::env::var("GS_SOAK_TRACE").is_ok() && (now % 500.0) < 0.5 {
            let seats: Vec<String> = host
                .auth
                .seats()
                .filter(|(_, s)| !s.bot)
                .map(|(id, s)| format!("{id}:miss={} late={}", s.stats.inputs_missed, s.stats.inputs_late))
                .collect();
            let preds_s: Vec<String> = preds
                .iter()
                .flatten()
                .map(|p| {
                    format!(
                        "tick={} auth={} slack={:.2} scale={:.3} rs={}",
                        p.tick(),
                        p.stats().server_tick,
                        p.stats().slack,
                        p.stats().time_scale,
                        p.stats().resyncs
                    )
                })
                .collect();
            eprintln!("t={now:.0} srv={} {seats:?} {preds_s:?}", host.auth.tick());
        }
    }

    let mut r = SoakReport {
        welcomed: preds.iter().filter(|p| p.is_some()).count(),
        server_ticks,
        snapshots: 0,
        rollbacks: 0,
        prediction_hits: 0,
        inputs_missed: 0,
        inputs_received: 0,
        final_slack: Vec::new(),
        resyncs: 0,
    };
    for p in preds.iter().flatten() {
        let s = p.stats();
        r.snapshots += s.snapshots;
        r.rollbacks += s.rollbacks;
        r.prediction_hits += s.prediction_hits;
        r.final_slack.push(s.slack);
        r.resyncs += s.resyncs;
    }
    for (_, seat) in host.auth.seats() {
        if !seat.bot {
            r.inputs_missed += seat.stats.inputs_missed;
            r.inputs_received += seat.stats.inputs_received;
        }
    }
    r
}

/// Live room vs checkpoint-at-`checkpoint_at` + replayed frames: must match bit-for-bit.
pub fn replay_check<G: Game>(setup: Setup, bots: u8, checkpoint_at: u32, ticks: u32) -> Result<(), String> {
    let mut live = Authority::<G>::new(setup);
    for i in 0..bots {
        live.join(&format!("bot{i}"), true, 0).map_err(|e| format!("{e:?}"))?;
    }
    let mut cp_bytes = None;
    let mut log = Vec::new();
    for t in 0..ticks {
        if t == checkpoint_at {
            cp_bytes = Some(gs_wire::to_bytes(&live.checkpoint()));
        }
        let out = live.step();
        if t >= checkpoint_at {
            log.push(gs_wire::to_bytes(&out.frame));
        }
    }
    let cp = cp_bytes.ok_or("checkpoint tick beyond run")?;
    let mut restored = Authority::<G>::restore_bytes(&cp).map_err(|e| e.to_string())?;
    for bytes in &log {
        let frame = gs_wire::from_bytes(bytes).map_err(|e| e.to_string())?;
        restored.replay(&frame)?;
    }
    if restored.state_hash() != live.state_hash() || restored.tick() != live.tick() {
        return Err(format!(
            "replay diverged: live tick {} hash {:016x}, replayed tick {} hash {:016x}",
            live.tick(),
            live.state_hash(),
            restored.tick(),
            restored.state_hash()
        ));
    }
    Ok(())
}
