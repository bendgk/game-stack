//! Durable room state in Dragonfly — the "durable object" layer.
//!
//! Per room:
//! - `gs:room:{name}:log`  — a stream of committed frames. Entry ids are `{tick}-1`, so the
//!   stream's order *is* the simulation order (and a duplicate append is rejected, not doubled).
//! - `gs:room:{name}:ckpt` — the newest full checkpoint (`Checkpoint<G>` bytes).
//!
//! On restart a room is rebuilt as `checkpoint + replay(log entries after it)`, which is exact
//! because the sim is deterministic. The log is trimmed to the checkpoint each time one lands.
//!
//! Plus a generic durable channel primitive (`channel_push` / `channel_read`) for business
//! logic that needs ordered, persistent messages (lobby chat, match queues, audit trails).
//!
//! Without `GS_REDIS_URL` the same API is served from memory (not durable across restarts).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use fred::prelude::*;
use fred::types::streams::XID;

const ROOM_TTL_SECS: i64 = 60 * 60 * 6;

#[derive(Clone)]
pub enum Store {
    Dragonfly(Client),
    Memory(Arc<Mutex<Mem>>),
}

#[derive(Default)]
#[allow(dead_code)] // `kv` backs the kv_* extension API
pub struct Mem {
    logs: HashMap<String, BTreeMap<u32, Vec<u8>>>,
    ckpts: HashMap<String, (u32, Vec<u8>)>,
    kv: HashMap<String, Vec<u8>>,
    channels: HashMap<String, Vec<(u64, Vec<u8>)>>,
}

fn lock(m: &Mutex<Mem>) -> std::sync::MutexGuard<'_, Mem> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What `load_room` returns: checkpoint (if any) and the frames logged after it, in order.
pub struct RoomRecord {
    pub checkpoint: Option<Vec<u8>>,
    pub log: Vec<Vec<u8>>,
}

impl Store {
    pub async fn connect(url: Option<&str>) -> anyhow::Result<Store> {
        let Some(url) = url else {
            tracing::warn!("GS_REDIS_URL unset: using the in-memory store (rooms will not survive a restart)");
            return Ok(Store::Memory(Arc::default()));
        };
        let cfg = Config::from_url(url)?;
        let client = Builder::from_config(cfg)
            .with_connection_config(|c| {
                c.connection_timeout = Duration::from_secs(3);
            })
            .set_policy(ReconnectPolicy::new_exponential(0, 100, 5_000, 2))
            .build()?;
        // Retry for a while: in docker-compose Dragonfly may still be starting.
        let mut attempt = 0;
        loop {
            match client.init().await {
                Ok(_) => break,
                Err(e) if attempt < 20 => {
                    attempt += 1;
                    tracing::warn!("dragonfly not ready ({e}); retrying");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
        tracing::info!(%url, "connected to dragonfly");
        Ok(Store::Dragonfly(client))
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Store::Dragonfly(_) => "dragonfly",
            Store::Memory(_) => "memory",
        }
    }

    fn log_key(room: &str) -> String {
        format!("gs:room:{room}:log")
    }
    fn ckpt_key(room: &str) -> String {
        format!("gs:room:{room}:ckpt")
    }

    /// Append committed frames (already in tick order).
    pub async fn append_log(&self, room: &str, frames: Vec<(u32, Vec<u8>)>) -> anyhow::Result<()> {
        match self {
            Store::Memory(m) => {
                let mut m = lock(m);
                let log = m.logs.entry(room.to_string()).or_default();
                for (t, f) in frames {
                    log.entry(t).or_insert(f);
                }
            }
            Store::Dragonfly(c) => {
                let key = Self::log_key(room);
                let pipe = c.pipeline();
                for (t, f) in frames {
                    let _: () = pipe
                        .xadd(&key, false, None, XID::Manual(format!("{t}-1").into()), ("f", Bytes::from(f)))
                        .await?;
                }
                let _: () = pipe.expire(&key, ROOM_TTL_SECS, None).await?;
                // A duplicate id (replayed append after a reconnect) errors harmlessly.
                let results: Vec<Result<Value, Error>> = pipe.try_all().await;
                for r in results {
                    if let Err(e) = r
                        && !e.details().contains("equal or smaller")
                    {
                        return Err(e.into());
                    }
                }
            }
        }
        Ok(())
    }

    /// Store a checkpoint and drop the log entries it covers.
    pub async fn checkpoint(&self, room: &str, tick: u32, bytes: Vec<u8>) -> anyhow::Result<()> {
        match self {
            Store::Memory(m) => {
                let mut m = lock(m);
                m.ckpts.insert(room.to_string(), (tick, bytes));
                if let Some(log) = m.logs.get_mut(room) {
                    *log = log.split_off(&tick);
                }
            }
            Store::Dragonfly(c) => {
                let mut v = tick.to_le_bytes().to_vec();
                v.extend_from_slice(&bytes);
                let _: () = c
                    .set(Self::ckpt_key(room), Bytes::from(v), Some(Expiration::EX(ROOM_TTL_SECS)), None, false)
                    .await?;
                // Frames with tick >= checkpoint tick stay (they are applied on top of it).
                let _: Value = c.xtrim(Self::log_key(room), ("MINID", "=", format!("{tick}-0"))).await?;
            }
        }
        Ok(())
    }

    pub async fn load_room(&self, room: &str) -> anyhow::Result<RoomRecord> {
        match self {
            Store::Memory(m) => {
                let m = lock(m);
                let ck = m.ckpts.get(room).cloned();
                let from = ck.as_ref().map_or(0, |c| c.0);
                let log =
                    m.logs.get(room).map(|l| l.range(from..).map(|(_, f)| f.clone()).collect()).unwrap_or_default();
                Ok(RoomRecord { checkpoint: ck.map(|c| c.1), log })
            }
            Store::Dragonfly(c) => {
                let raw: Option<Bytes> = c.get(Self::ckpt_key(room)).await?;
                let (from, checkpoint) = match raw {
                    Some(b) if b.len() >= 4 => (u32::from_le_bytes([b[0], b[1], b[2], b[3]]), Some(b[4..].to_vec())),
                    _ => (0, None),
                };
                let entries: Vec<(String, HashMap<String, Bytes>)> =
                    c.xrange(Self::log_key(room), format!("{from}-0"), "+", None).await?;
                let log = entries.into_iter().filter_map(|(_, mut f)| f.remove("f").map(|b| b.to_vec())).collect();
                Ok(RoomRecord { checkpoint, log })
            }
        }
    }

    #[allow(dead_code)] // extension API: reset a room for good
    pub async fn delete_room(&self, room: &str) -> anyhow::Result<()> {
        match self {
            Store::Memory(m) => {
                let mut m = lock(m);
                m.logs.remove(room);
                m.ckpts.remove(room);
            }
            Store::Dragonfly(c) => {
                let _: i64 = c.del(vec![Self::log_key(room), Self::ckpt_key(room)]).await?;
            }
        }
        Ok(())
    }

    /// Small key/value with TTL (sessions, presence, feature flags…). Extension API.
    #[allow(dead_code)]
    pub async fn kv_set(&self, key: &str, value: Vec<u8>, ttl_secs: i64) -> anyhow::Result<()> {
        match self {
            Store::Memory(m) => {
                lock(m).kv.insert(key.to_string(), value);
            }
            Store::Dragonfly(c) => {
                let _: () = c
                    .set(format!("gs:kv:{key}"), Bytes::from(value), Some(Expiration::EX(ttl_secs)), None, false)
                    .await?;
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn kv_get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        match self {
            Store::Memory(m) => Ok(lock(m).kv.get(key).cloned()),
            Store::Dragonfly(c) => {
                let v: Option<Bytes> = c.get(format!("gs:kv:{key}")).await?;
                Ok(v.map(|b| b.to_vec()))
            }
        }
    }

    /// Durable ordered channel: append a message, returns its sequence id.
    pub async fn channel_push(&self, name: &str, msg: Vec<u8>, max_len: u64) -> anyhow::Result<String> {
        match self {
            Store::Memory(m) => {
                let mut m = lock(m);
                let ch = m.channels.entry(name.to_string()).or_default();
                let seq = ch.last().map_or(1, |l| l.0 + 1);
                ch.push((seq, msg));
                let excess = ch.len().saturating_sub(max_len as usize);
                ch.drain(..excess);
                Ok(format!("{seq}-0"))
            }
            Store::Dragonfly(c) => {
                let id: String = c
                    .xadd(
                        format!("gs:chan:{name}"),
                        false,
                        ("MAXLEN", "~", max_len as i64),
                        "*",
                        ("m", Bytes::from(msg)),
                    )
                    .await?;
                Ok(id)
            }
        }
    }

    /// Read up to `count` messages after id `after` ("0" = from the start).
    pub async fn channel_read(&self, name: &str, after: &str, count: u64) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
        match self {
            Store::Memory(m) => {
                let m = lock(m);
                let after: u64 = after.split('-').next().and_then(|s| s.parse().ok()).unwrap_or(0);
                Ok(m.channels
                    .get(name)
                    .map(|ch| {
                        ch.iter()
                            .filter(|(s, _)| *s > after)
                            .take(count as usize)
                            .map(|(s, b)| (format!("{s}-0"), b.clone()))
                            .collect()
                    })
                    .unwrap_or_default())
            }
            Store::Dragonfly(c) => {
                let start = if after == "0" { "-".to_string() } else { format!("({after}") };
                let entries: Vec<(String, HashMap<String, Bytes>)> =
                    c.xrange(format!("gs:chan:{name}"), start, "+", Some(count)).await?;
                Ok(entries.into_iter().filter_map(|(id, mut f)| f.remove("m").map(|b| (id, b.to_vec()))).collect())
            }
        }
    }
}
