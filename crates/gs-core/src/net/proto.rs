//! Wire messages between the browser and the authoritative server.
//!
//! Two WebRTC DataChannels carry them (both SCTP over DTLS over UDP; there is no TCP game
//! traffic):
//! - `rel`   — reliable + ordered: handshake, commands, notices.
//! - `unrel` — unordered, `maxRetransmits: 0`: inputs, snapshots, pings. A lost packet is
//!   simply superseded by the next one (inputs are sent redundantly; snapshots delta against
//!   the last one the client acknowledged).

use crate::game::{Game, PlayerId, Setup};
use gs_wire::{Blob, Wire};

/// Bump on any change to the netcode semantics that the schema hash cannot see.
pub const PROTOCOL_VERSION: u32 = 1;

/// Inputs re-sent with every packet (covers bursts of loss without retransmits).
pub const INPUT_REDUNDANCY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Reliable = 0,
    Unreliable = 1,
}

impl Channel {
    pub const fn label(self) -> &'static str {
        match self {
            Channel::Reliable => "rel",
            Channel::Unreliable => "unrel",
        }
    }
    pub fn from_label(l: &str) -> Option<Self> {
        match l {
            "rel" => Some(Channel::Reliable),
            "unrel" => Some(Channel::Unreliable),
            _ => None,
        }
    }
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Hello {
    /// [`fingerprint`] of the client build. Must equal the server's.
    pub fingerprint: u64,
    pub name: String,
    pub room: String,
    /// Mode for a room this hello creates (ignored if the room exists).
    pub mode: String,
    /// Token from a previous `Welcome`, to reclaim the same player slot after a drop.
    pub resume: Option<u64>,
    /// Number of server bots to add when this hello creates the room.
    pub bots: u8,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct InputBatch<I> {
    /// Tick of `inputs[0]`; the batch covers `first_tick .. first_tick + len`.
    pub first_tick: u32,
    pub inputs: Vec<I>,
    /// Newest snapshot tick fully received (delta baseline), if any.
    pub ack: Option<u32>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub enum ClientMsg<I, C> {
    Hello(Hello),
    Inputs(InputBatch<I>),
    Command(C),
    Ping { t: u32 },
    Bye,
}

impl<I, C> ClientMsg<I, C> {
    pub fn channel(&self) -> Channel {
        match self {
            ClientMsg::Inputs(_) | ClientMsg::Ping { .. } => Channel::Unreliable,
            _ => Channel::Reliable,
        }
    }
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// Ticks simulated so far: the state is *before* frame `tick` is applied.
    pub tick: u32,
    /// Payload is a delta against this previously-acked snapshot (`None` = full).
    pub baseline: Option<u32>,
    /// How many ticks early this player's newest input arrived when the server needed it
    /// (negative = late). The client steers its clock to keep this small and positive.
    pub slack: i16,
    /// Encoded [`WorldSnap`] (or its delta).
    pub payload: Blob,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Welcome {
    pub player: PlayerId,
    pub room: String,
    pub setup: Setup,
    pub tick_hz: u16,
    pub resume: u64,
    pub snapshot: Snapshot,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub enum ServerMsg {
    Welcome(Welcome),
    Snapshot(Snapshot),
    Pong { t: u32, tick: u32 },
    Notice(String),
    Reject(String),
}

impl ServerMsg {
    pub fn channel(&self) -> Channel {
        match self {
            ServerMsg::Snapshot(_) | ServerMsg::Pong { .. } => Channel::Unreliable,
            _ => Channel::Reliable,
        }
    }
}

/// What a snapshot payload decodes to: the full game state plus the engine's roster (who is
/// in the room and the last input the server applied for each — used to predict others).
#[derive(Wire, Clone, Debug, PartialEq)]
pub struct WorldSnap<G: Game> {
    pub game: G,
    pub roster: Vec<(PlayerId, G::Input)>,
}

pub type ClientMsgOf<G> = ClientMsg<<G as Game>::Input, <G as Game>::Command>;

/// The full wire contract for game `G`, as canonical text. Checked in at
/// `contract/schema.txt`; any diff there is a protocol change.
pub fn contract_text<G: Game>() -> String {
    format!(
        "game: {}\nprotocol: {}\ntick_hz: {}\nclient: {}\nserver: {}\nsnapshot: {}\n",
        G::NAME,
        PROTOCOL_VERSION,
        G::TICK_HZ,
        gs_wire::describe::<ClientMsgOf<G>>(),
        gs_wire::describe::<ServerMsg>(),
        gs_wire::describe::<WorldSnap<G>>(),
    )
}

/// Hash of [`contract_text`]: client and server must agree exactly.
pub fn fingerprint<G: Game>() -> u64 {
    gs_wire::fnv1a64(contract_text::<G>().as_bytes())
}
