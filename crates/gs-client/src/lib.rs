//! gs-client — the browser half, compiled to wasm32 by `scripts/build-wasm.mjs`.
//!
//! TypeScript never decodes a wire message or steps the sim. It shuttles bytes between the
//! RTCDataChannels and [`GameClient`], feeds it typed `Input`s, and reads back a zero-copy
//! render buffer + typed HUD/events. All of those types are generated from Rust (`tsify`), so
//! a change in `crates/game` shows up as a TypeScript type error, not a runtime surprise.
//!
//! Packets crossing the boundary are `Uint8Array`s whose first byte is the channel
//! (0 = `rel`, 1 = `unrel`) — or, for [`LocalServer`], `[conn u32 LE][channel][payload]`.

use gs_core::net::client::{ClientError, Predictor, decode_welcome, hello};
use gs_core::net::{Channel, RoomHost};
use gs_core::{Game, RenderEntity, Setup};
use wasm_bindgen::prelude::*;

pub use game::{Command, Event, Hud, Input, Kind};

// Referenced so tsify emits the `Kind` and `Hud` declarations into the .d.ts.
#[allow(dead_code)]
fn _ts_types(_: Kind, _: Hud) {}
type G = game::TheGame;

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
export type PlayerId = number;
export interface NetStats {
  tick: number; server_tick: number; rtt_ms: number; jitter_ms: number; slack: number;
  time_scale: number; rollbacks: number; last_rollback_ticks: number; prediction_hits: number;
  snapshots: number; snapshot_bytes: number; dropped_snapshots: number; resyncs: number; stalled: boolean;
}
export interface TickEvent { tick: number; event: Event }
"#;

#[derive(serde::Serialize)]
struct TickEvent {
    tick: u32,
    event: Event,
}

fn packets(out: impl Iterator<Item = (Channel, Vec<u8>)>) -> js_sys::Array {
    let arr = js_sys::Array::new();
    for (ch, bytes) in out {
        let mut v = Vec::with_capacity(bytes.len() + 1);
        v.push(ch as u8);
        v.extend_from_slice(&bytes);
        arr.push(&js_sys::Uint8Array::from(v.as_slice()));
    }
    arr
}

fn err(e: ClientError) -> JsError {
    JsError::new(&e.to_string())
}

/// One player's connection to a room (online or local).
#[wasm_bindgen]
pub struct GameClient {
    pred: Option<Predictor<G>>,
    render: Vec<f32>,
    scratch: Vec<RenderEntity>,
}

impl Default for GameClient {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl GameClient {
    #[wasm_bindgen(constructor)]
    pub fn new() -> GameClient {
        GameClient { pred: None, render: Vec::new(), scratch: Vec::new() }
    }

    /// The first packet to send on the reliable channel.
    pub fn hello(&self, name: &str, room: &str, mode: &str, bots: u8, resume: Option<u64>) -> js_sys::Uint8Array {
        let bytes = hello::<G>(name, room, mode, bots, resume);
        packets(std::iter::once((Channel::Reliable, bytes))).get(0).into()
    }

    /// Feed one message from a DataChannel (raw payload, no channel prefix).
    pub fn receive(&mut self, bytes: &[u8], now: f64) -> Result<(), JsError> {
        match &mut self.pred {
            Some(p) => p.receive(bytes, now).map_err(err),
            None => {
                if let Some(w) = decode_welcome(bytes).map_err(err)? {
                    self.pred = Some(Predictor::new(w).map_err(err)?);
                }
                Ok(())
            }
        }
    }

    /// True once the server's `Welcome` arrived.
    pub fn connected(&self) -> bool {
        self.pred.is_some()
    }

    pub fn me(&self) -> Option<u16> {
        self.pred.as_ref().map(|p| p.me())
    }

    pub fn resume_token(&self) -> Option<u64> {
        self.pred.as_ref().map(|p| p.resume_token())
    }

    pub fn mode(&self) -> String {
        self.pred.as_ref().map(|p| p.setup().mode.clone()).unwrap_or_default()
    }

    pub fn set_input(&mut self, #[wasm_bindgen(unchecked_param_type = "Input")] input: JsValue) -> Result<(), JsError> {
        let input: Input = serde_wasm_bindgen::from_value(input)?;
        if let Some(p) = &mut self.pred {
            p.set_input(input);
        }
        Ok(())
    }

    pub fn command(&mut self, #[wasm_bindgen(unchecked_param_type = "Command")] cmd: JsValue) -> Result<(), JsError> {
        let cmd: Command = serde_wasm_bindgen::from_value(cmd)?;
        if let Some(p) = &mut self.pred {
            p.send_command(cmd);
        }
        Ok(())
    }

    /// Advance prediction to `now` (`performance.now()`).
    pub fn update(&mut self, now: f64) {
        if let Some(p) = &mut self.pred {
            p.update(now);
        }
    }

    /// Packets to send: `Uint8Array[]`, byte 0 = channel.
    pub fn drain_outbox(&mut self) -> js_sys::Array {
        match &mut self.pred {
            Some(p) => packets(p.drain_outbox()),
            None => js_sys::Array::new(),
        }
    }

    /// Fill the render buffer; returns the entity count. Read it zero-copy with
    /// `new Float32Array(memory.buffer, render_ptr(), count * RENDER_STRIDE)`.
    pub fn render(&mut self) -> usize {
        self.render.clear();
        if let Some(p) = &self.pred {
            p.render(&mut self.scratch);
            for e in &self.scratch {
                e.pack(&mut self.render);
            }
        }
        self.render.len() / RenderEntity::STRIDE
    }

    pub fn render_ptr(&self) -> *const f32 {
        self.render.as_ptr()
    }

    #[wasm_bindgen(unchecked_return_type = "Hud | null")]
    pub fn hud(&self) -> JsValue {
        match &self.pred {
            Some(p) => serde_wasm_bindgen::to_value(&p.hud()).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }

    #[wasm_bindgen(unchecked_return_type = "TickEvent[]")]
    pub fn drain_events(&mut self) -> JsValue {
        let evs: Vec<TickEvent> = match &mut self.pred {
            Some(p) => p.drain_events().map(|(tick, event)| TickEvent { tick, event }).collect(),
            None => vec![],
        };
        serde_wasm_bindgen::to_value(&evs).unwrap_or(JsValue::NULL)
    }

    pub fn drain_notices(&mut self) -> Vec<String> {
        match &mut self.pred {
            Some(p) => p.drain_notices().collect(),
            None => vec![],
        }
    }

    #[wasm_bindgen(unchecked_return_type = "NetStats | null")]
    pub fn stats(&self) -> JsValue {
        match &self.pred {
            Some(p) => serde_wasm_bindgen::to_value(p.stats()).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }
}

/// The authoritative room running *inside the browser* — offline / practice play with the
/// exact same server code path (connections are small integers).
#[wasm_bindgen]
pub struct LocalServer {
    host: RoomHost<G>,
    token: u64,
}

#[wasm_bindgen]
impl LocalServer {
    #[wasm_bindgen(constructor)]
    pub fn new(mode: &str, bots: u8, seed: u64) -> LocalServer {
        let mut host = RoomHost::<G>::new("local", Setup { seed, mode: mode.into() });
        host.add_bots(bots);
        LocalServer { host, token: seed }
    }

    pub fn receive(&mut self, conn: u32, bytes: &[u8]) {
        let token = &mut self.token;
        self.host.on_message(u64::from(conn), bytes, || {
            *token = token.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            *token
        });
    }

    pub fn disconnect(&mut self, conn: u32) {
        self.host.on_disconnect(u64::from(conn));
    }

    pub fn tick(&mut self) {
        self.host.tick();
    }

    /// `Uint8Array[]`: `[conn u32 LE][channel u8][payload]`.
    pub fn drain_outbox(&mut self) -> js_sys::Array {
        let arr = js_sys::Array::new();
        for (conn, ch, bytes) in self.host.drain_outbox() {
            let mut v = Vec::with_capacity(bytes.len() + 5);
            v.extend_from_slice(&(conn as u32).to_le_bytes());
            v.push(ch as u8);
            v.extend_from_slice(&bytes);
            arr.push(&js_sys::Uint8Array::from(v.as_slice()));
        }
        arr
    }
}

#[wasm_bindgen]
pub fn tick_hz() -> u32 {
    G::TICK_HZ
}

/// f32s per entity in the render buffer.
#[wasm_bindgen]
pub fn render_stride() -> usize {
    RenderEntity::STRIDE
}

/// Kind names, indexed by kind id.
#[wasm_bindgen]
pub fn kind_names() -> Vec<String> {
    G::kind_names().iter().map(|s| s.to_string()).collect()
}

#[wasm_bindgen]
pub fn fingerprint() -> String {
    format!("{:016x}", gs_core::net::fingerprint::<G>())
}

#[wasm_bindgen]
pub fn contract() -> String {
    gs_core::net::contract_text::<G>()
}

/// Same probe as `gs-server probe`; `pnpm verify` asserts they match (native == wasm).
#[wasm_bindgen]
pub fn determinism_probe() -> String {
    format!(
        "game:{:016x} physics2d:{:016x} physics3d:{:016x}",
        gs_core::env::probe::<G>(1, "versus", 4, 1200),
        gs_physics::probe::<gs_physics::d2::Vec2>(600),
        gs_physics::probe::<gs_physics::d3::Vec3>(600)
    )
}
