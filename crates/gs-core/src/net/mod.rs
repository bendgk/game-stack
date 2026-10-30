//! Netcode primitives. All sans-IO: they take bytes and a clock reading, and hand back bytes.
//! Transport (WebRTC on the server, RTCDataChannel in the browser) lives outside.

pub mod client;
pub mod clock;
pub mod delta;
pub mod host;
pub mod link;
pub mod proto;
pub mod server;

pub use client::{NetStats, Predictor, PredictorConfig};
pub use host::{ConnId, RoomHost};
pub use link::{Link, LinkConfig};
pub use proto::{Channel, ClientMsg, ClientMsgOf, Hello, ServerMsg, contract_text, fingerprint};
pub use server::{Authority, Checkpoint, StepOutput};
