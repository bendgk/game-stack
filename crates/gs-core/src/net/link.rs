//! A deterministic simulated network link (latency, jitter, loss, reordering).
//! Used by the netcode tests and by the browser's offline mode to rehearse bad networks.

use crate::rng::Rng;

#[derive(Clone, Debug, PartialEq)]
pub struct LinkConfig {
    pub latency_ms: f64,
    pub jitter_ms: f64,
    /// 0..1 probability a message on the unreliable channel is dropped.
    pub loss: f32,
}

impl Default for LinkConfig {
    fn default() -> Self {
        LinkConfig { latency_ms: 0.0, jitter_ms: 0.0, loss: 0.0 }
    }
}

impl LinkConfig {
    pub fn lan() -> Self {
        LinkConfig { latency_ms: 1.0, jitter_ms: 0.5, loss: 0.0 }
    }
    pub fn rough() -> Self {
        LinkConfig { latency_ms: 40.0, jitter_ms: 15.0, loss: 0.05 }
    }
}

/// One direction of a connection. `reliable` messages are never dropped and keep their order.
pub struct Link<T> {
    pub cfg: LinkConfig,
    rng: Rng,
    queue: Vec<(f64, u64, T)>,
    seq: u64,
    last_reliable_at: f64,
}

impl<T> Link<T> {
    pub fn new(cfg: LinkConfig, seed: u64) -> Self {
        Link { cfg, rng: Rng::new(seed), queue: Vec::new(), seq: 0, last_reliable_at: 0.0 }
    }

    pub fn send(&mut self, now_ms: f64, reliable: bool, msg: T) {
        if !reliable && self.rng.chance(self.cfg.loss) {
            return;
        }
        let jitter = f64::from(self.rng.f32()) * self.cfg.jitter_ms;
        let mut at = now_ms + self.cfg.latency_ms + jitter;
        if reliable {
            at = at.max(self.last_reliable_at);
            self.last_reliable_at = at;
        }
        self.seq += 1;
        self.queue.push((at, self.seq, msg));
    }

    /// Everything due by `now_ms`, in delivery order.
    pub fn recv(&mut self, now_ms: f64) -> Vec<T> {
        self.queue.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let n = self.queue.iter().take_while(|(at, _, _)| *at <= now_ms).count();
        self.queue.drain(..n).map(|(_, _, m)| m).collect()
    }

    pub fn in_flight(&self) -> usize {
        self.queue.len()
    }
}
