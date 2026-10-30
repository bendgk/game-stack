//! Server configuration. Every variable is listed (with defaults) in `.env.example`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    /// HTTP: signaling (`POST /api/rtc/offer`), REST, and the built web client.
    pub http_addr: SocketAddr,
    /// The one UDP socket all WebRTC traffic is multiplexed over.
    pub udp_port: u16,
    /// IP advertised to browsers in the ICE candidate (your LAN address).
    pub public_ip: IpAddr,
    /// `redis://…` for Dragonfly. Unset → in-memory store (fine for solo dev; not durable).
    pub redis_url: Option<String>,
    /// `postgres://…`. Unset → no persistence of results / leaderboard.
    pub database_url: Option<String>,
    /// Directory with the built web client (`apps/web/dist`); served at `/`.
    pub web_dir: Option<String>,
    /// How long a dropped player's seat is held for resume.
    pub resume_grace: Duration,
    /// Rooms with no connections shut down (and checkpoint) after this long.
    pub room_idle: Duration,
    /// Checkpoint each room's full state every N ticks (the log covers the gap).
    pub checkpoint_every: u32,
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> anyhow::Result<T> {
    match env(key) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| anyhow::anyhow!("{key}={v:?} is not valid")),
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let public_ip = match env("GS_PUBLIC_IP") {
            Some(ip) => ip.parse()?,
            None => detect_lan_ip(),
        };
        Ok(Config {
            http_addr: env_parse("GS_HTTP_ADDR", SocketAddr::from(([0, 0, 0, 0], 8080)))?,
            udp_port: env_parse("GS_UDP_PORT", 40000)?,
            public_ip,
            redis_url: env("GS_REDIS_URL"),
            database_url: env("DATABASE_URL"),
            web_dir: env("GS_WEB_DIR"),
            resume_grace: Duration::from_secs(env_parse("GS_RESUME_GRACE_SECS", 30)?),
            room_idle: Duration::from_secs(env_parse("GS_ROOM_IDLE_SECS", 120)?),
            checkpoint_every: env_parse("GS_CHECKPOINT_EVERY", 300)?,
        })
    }
}

/// The address the OS would route LAN traffic from (no packet is sent).
fn detect_lan_ip() -> IpAddr {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.168.0.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip())
        .ok()
        .filter(|ip| !ip.is_unspecified())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}
