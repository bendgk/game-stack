//! Round-trip-time estimation (client side).

/// Rolling RTT estimator: tracks the minimum of a recent window (the least-queued sample is the
/// best estimate of the true path delay) plus a smoothed mean and jitter.
#[derive(Clone, Debug)]
pub struct Rtt {
    window: [f32; 16],
    n: usize,
    head: usize,
    pub ema_ms: f32,
    pub jitter_ms: f32,
}

impl Default for Rtt {
    fn default() -> Self {
        Rtt { window: [0.0; 16], n: 0, head: 0, ema_ms: 0.0, jitter_ms: 0.0 }
    }
}

impl Rtt {
    pub fn sample(&mut self, rtt_ms: f32) {
        let rtt_ms = rtt_ms.max(0.0);
        self.window[self.head] = rtt_ms;
        self.head = (self.head + 1) % self.window.len();
        self.n = (self.n + 1).min(self.window.len());
        if self.n == 1 {
            self.ema_ms = rtt_ms;
        } else {
            let dev = (rtt_ms - self.ema_ms).abs();
            self.ema_ms += (rtt_ms - self.ema_ms) * 0.125;
            self.jitter_ms += (dev - self.jitter_ms) * 0.25;
        }
    }
    pub fn ready(&self) -> bool {
        self.n >= 3
    }
    pub fn min_ms(&self) -> f32 {
        self.window[..self.n].iter().copied().fold(f32::INFINITY, f32::min).min(self.ema_ms.max(0.0))
    }
}
