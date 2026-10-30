# Architecture

game-stack is one authoritative node on a LAN plus browsers. This document explains every moving
part and why it is shaped the way it is. For "how do I add X", see `RECIPES.md`.

## 1. Layers

```
┌──────────────────────── crates/game ────────────────────────┐   compiled twice:
│ impl Game for OrbArena { step, render, hud, sanitize, bot }  │   ▸ native → gs-server
└──────────────────────────────┬──────────────────────────────┘   ▸ wasm32 → gs-client
┌──────────────────────── crates/gs-core ─────────────────────┐
│ game.rs    Game trait, Frame, Events, MatchResult            │
│ math/rng   deterministic f32 math (glam+libm), PCG32         │
│ physics    spheres + AABBs, sensors, layers, rays            │
│ net/       proto · server(Authority) · client(Predictor)     │
│            host(RoomHost) · delta · link(sim) · clock         │
│ env.rs     headless gym-style Env, determinism probe          │
│ testkit    soak + replay checks any game can call             │
└──────────────────────────────┬──────────────────────────────┘
┌──────────────────────── crates/gs-wire ─────────────────────┐
│ Wire trait + derive, bounded decoder, schema → fingerprint    │
└──────────────────────────────────────────────────────────────┘
      gs-server: tokio · axum · str0m · fred(Dragonfly) · sqlx(Postgres)
      gs-client: wasm-bindgen · tsify → packages/engine (TS) → apps/web (React/R3F)
```

Everything in `gs-core` is **sans-IO**: it takes bytes and timestamps and returns bytes. The
same `RoomHost` runs inside the tokio server, inside the browser for offline play
(`LocalServer`), and inside the tests over simulated links. Transport and persistence are thin
shells around it.

## 2. The wire contract (`gs-wire`)

Cap'n Proto-like goals with zero external tooling: one schema source (Rust types), compact,
fast, safe on hostile input.

- `#[derive(Wire)]` on structs/enums generates `encode`, `decode` and `schema`.
- Encoding: LEB128 varints (zig-zag for signed), raw little-endian IEEE floats (exact
  round-trips, which determinism needs), enum = varint tag, `Option` = tag byte, sequences =
  varint length. `#[wire(skip)]` fields are not sent and decode as `Default`.
- Decoding never panics: lengths are validated against the bytes actually remaining (no
  allocation bombs), nesting depth is capped, unknown tags and trailing bytes are errors. Fuzz
  tests in `crates/gs-wire/src/tests.rs`.
- `schema` writes canonical text (`Input{mx:i8,mz:i8,yaw:f32,buttons:u8}`), recursively, with
  `@Name` back-references. `net::contract_text::<G>()` concatenates client messages, server
  messages and the snapshot type; `fingerprint` hashes it. The client sends its fingerprint in
  `Hello`, and the server rejects a mismatch with a readable message. `contract/schema.txt` is the
  checked-in copy (a test fails when it is stale), so protocol changes show up in diffs.

Transport encryption comes from WebRTC itself (DTLS 1.2+). The codec's job is to be safe to
parse, not to encrypt.

## 3. Networking model

**Server-authoritative snapshots + full-world deterministic prediction + rollback.**

### Server: `Authority` (`gs-core/src/net/server.rs`)

Per tick `t`:
1. Apply pending joins/leaves/commands (these become part of the frame).
2. For each seat in `PlayerId` order, pick an input: buffered input for `t` (first copy wins)
   → repeat-last if late → neutral if disconnected → `bot_input` for bots. Run it through
   `Game::sanitize` (the critic).
3. `game.step(&frame)`; frame `t` is the canonical log record: `state(t+1) = step(state(t), frame(t))`.
4. Encode `WorldSnap { game, roster }`, keep it in a 1-second history ring for delta baselines.
5. Each player gets the snapshot delta-encoded against the newest snapshot they acked, plus
   `slack` = how many ticks early their newest input arrived.

### Client: `Predictor` (`gs-core/src/net/client.rs`)

- Runs the same `step` **ahead** of the server: its tick ≈ server tick + one-way latency + ~2
  ticks of cushion. Its own input is real; other players use their last known input (from the
  snapshot roster).
- On each snapshot (state at tick `T`): if the hash of our predicted state at `T` equals the
  snapshot's and the other players' inputs are unchanged, nothing happens (a **prediction hit**).
  Otherwise we **roll back**: load the snapshot, re-simulate `T..now` with recorded local
  inputs. On a LAN that is 2–4 ticks, microseconds of work.
- **Smoothing:** the per-entity visual jump from a rollback becomes an offset that decays with τ
  ≈ 100 ms. Large jumps (> 3 units, or `FLAG_TELEPORT`) snap.
- **Interpolation:** rendering lerps between the last two predicted ticks by the fractional
  accumulator, so motion is smooth at any refresh rate.
- **Clock steering:** the client runs 0.95×–1.05× real time to hold `slack` near 2 ticks. A
  slack error > 3 ticks triggers a hard resync that shifts by exactly the error (slack measures
  it precisely). Startup converges in under a second even on a 40 ms / 5% loss link (see the
  soak tests).
- **Events** are de-duplicated by `(tick, hash)`, so a re-simulated tick never replays a sound.
- **Input redundancy:** every input packet carries the last 8 ticks. Loss bursts heal without
  retransmits or acks.
- **Stall guard:** if the server goes quiet, prediction stops 30 ticks past the last snapshot
  instead of drifting into fiction.

### Why this model

Rollback-only (GGPO) needs every peer's input before it is "true", which fits 1v1 but not
drop-in co-op. Pure snapshot interpolation makes your own movement lag by RTT. Snapshot +
whole-world prediction gets server authority (no cheating, late joiners, bots, persistence)
*and* instant local response, and because the client runs the identical deterministic sim, a
misprediction is only ever about other players' inputs. Running alone, prediction is exact:
`netcode_solo_prediction_is_exact` asserts zero rollbacks.

### Channels

| channel | WebRTC options | carries |
| --- | --- | --- |
| `rel` | ordered, reliable | Hello, Welcome, Command, Notice, Reject |
| `unrel` | unordered, `maxRetransmits: 0` | Inputs, Snapshot, Ping/Pong |

## 4. Determinism

Both targets compile the same Rust, so determinism comes down to not using operations whose
results differ across CPUs and runtimes:

- **Floats are fine** when only IEEE-754 correctly-rounded ops are used (`+ - * / sqrt`), with no
  fused multiply-add. Rust never contracts `a*b+c` implicitly.
- **Transcendentals** (`sin`, `atan2`, `exp`, …) come from the platform's libm natively but
  from a different implementation in wasm. `gs_core::math` routes them through the pure-Rust
  `libm` crate on all targets, and `glam` is built with `libm` + `scalar-math`.
- **Iteration order**: `BTreeMap` only; physics iterates by body index; broadphase pairs are sorted.
- **Randomness**: `Rng` (PCG32) lives in the game state and is snapshotted with it.

Enforcement: `clippy.toml` in `crates/game` and `crates/gs-core` bans the dangerous methods and
types. `pnpm verify` runs `env::probe` (4 bots, 1200 ticks, physics) natively and in wasm and
requires identical hashes.

## 5. Server process (`gs-server`)

```
axum ──POST /api/rtc/offer──▶ rtc task ── one UDP socket, N str0m::Rtc (ICE-lite, host candidate)
                                 │ RtcEvent::{Open, Data, Closed}
                                 ▼
                              router ── binds a peer to a room on its Hello
                                 ▼
                     room actor (one tokio task per room)
                       ├ RoomHost<TheGame>, fixed 60 Hz interval (burst catch-up)
                       ├ outbox → rtc task
                       ├ frames + checkpoints → persist task → Dragonfly
                       └ round results → Postgres (spawned, never blocks the tick)
```

- **WebRTC:** `str0m` is sans-IO. We own the socket and the clock, and every browser is one `Rtc`
  multiplexed on UDP 40000. The server is ICE-lite with a single host candidate
  (`GS_PUBLIC_IP:40000`). Browsers that hide their own addresses behind mDNS still connect,
  because the server learns their address from their first STUN request. In Docker, inbound
  packets are re-addressed to the advertised candidate so port-mapping works.
- **Backpressure:** unreliable sends are dropped when a channel's SCTP buffer exceeds 64 KB
  (newest snapshot wins). Persistence uses a bounded queue; if Dragonfly is slow the room skips
  log frames and forces a checkpoint instead of stalling the tick.
- **Resume:** each seat has a 64-bit token from OS randomness (outside the sim). A dropped
  player keeps their seat for `GS_RESUME_GRACE_SECS` with neutral input, and a `Hello` carrying
  the token reclaims it. Browsers keep the token in `sessionStorage`, so even a page reload
  resumes.

## 6. Durable rooms (Dragonfly)

Dragonfly is used as a durable-object layer:

- `gs:room:{name}:log`: a stream whose entry ids are `{tick}-1`, so stream order *is* simulation
  order and duplicate appends are rejected. Each entry is a `LogRecord { frame, resumes }`.
- `gs:room:{name}:ckpt`: the latest `Checkpoint` (full state + seats), written every
  `GS_CHECKPOINT_EVERY` ticks. The log is trimmed (`XTRIM MINID`) to the checkpoint.
- **Rehydration:** when a room starts, `checkpoint + replay(log)` reproduces the exact pre-crash
  state, because the sim is deterministic. `durable_log_replays_bit_exactly` tests this, and it
  has been verified live: kill the server mid-game, restart it, and the room resumes at the same
  tick.
- `gs:chan:{name}`: generic durable ordered channels (`Store::channel_push/read`,
  `/api/channel/{name}`) for business logic: lobby chat, queues, audit trails.
- `gs:kv:{key}`: small TTL'd key/values.

Without `GS_REDIS_URL` the same API is served from memory, so solo development needs no Docker.

## 7. Postgres

Long-lived records only: `players`, `matches`, `match_players` (see `migrations/`). Rounds are
recorded when `Game::result()` transitions from `None` to `Some`. `/api/leaderboard` and
`/api/matches` read them. Migrations run on startup.

## 8. Browser (`packages/engine`, `apps/web`)

- `GameSession` owns the wasm `GameClient` + a `Transport` (`connectWebRtc` or the in-tab
  `LocalHost` loopback with simulated latency/jitter/loss). It reconnects with backoff and goes
  neutral when the tab is hidden.
- Each frame: `set_input` → `update(now)` → flush packets → dispatch events → `render()` fills
  a Float32Array in wasm memory, which `EntityTable` reads **zero-copy** into stable
  `EntityView` objects. React re-renders only on spawn/despawn; transforms go straight into
  three.js objects in `useFrame`.
- R3F uses three.js r187's `WebGPURenderer`, which falls back to WebGL2 when WebGPU is
  unavailable (plain `http://` on a LAN IP is not a secure context).
- HUD/stats are pushed to React at ~15 Hz via `useSyncExternalStore`.

## 9. "Actor–critic" in this codebase

The server is an async **actor** system: one tokio task per room, one for the WebRTC socket, one
persistence writer per room, all communicating by message passing. Inside the sim, each seat's
**actor** (a human's input stream or `Game::bot_input`) proposes an input, and the **critic**
`Game::sanitize` referees it before the deterministic `step` applies it. For learning agents,
`gs_core::env::Env` exposes the game gym-style: `set_input` (act), `step`, `game()` (observe),
`reward` (`Game::reward`, the critic signal).

## 10. Performance notes (defaults)

60 Hz ticks and snapshots; ~1.8 KB per raw snapshot for the example (8 players, ~25 entities),
much less after delta. Prediction cost per snapshot is one state hash, plus a 2–4 tick re-sim
on a miss. The physics world is cloned on rollback, so keep it to hundreds of bodies, not tens
of thousands. For bigger worlds, send interest-managed snapshots and predict only nearby
entities. Both are possible extensions of `Authority::snapshot_for` and `Predictor`.
