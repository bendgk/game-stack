# game-stack

A starter kit for **fast, server-authoritative multiplayer browser games on your local network**.
Fork it, change the example game, and have friends join from their laptops and phones over Wi-Fi.

- **One Rust codebase** for everything that matters: game rules, physics, netcode, wire
  format. It runs as the authoritative server *and* is compiled to WebAssembly so every browser
  runs the exact same code for prediction. Write a feature once and it works on both sides.
- **WebRTC over UDP.** Game traffic goes over DataChannels (one reliable, one unreliable), never
  WebSockets/TCP. Signaling is a single HTTP request.
- **Server-authoritative + predicted + deterministic.** The server owns the truth. Your browser
  predicts ahead so controls feel instant, and when the server disagrees it rolls back and
  re-simulates, smoothing out the correction. Same inputs give the same result, bit for bit, on
  every machine. A test enforces this.
- **Durable rooms.** Every tick is streamed to **Dragonfly**. Restart the server and rooms pick up
  exactly where they were. **Postgres** keeps match history and a leaderboard.
- **React 19 + React Three Fiber 9 + three.js r187 (WebGPU)** for rendering and UI. TypeScript
  types are *generated from Rust*, so the client can't drift from the server.

Supports single-player, co-op and versus out of the box, with server-side bots to fill seats.

```
  browser (React + three.js)                         one machine on your LAN
 ┌─────────────────────────────┐   HTTP: one SDP   ┌──────────────────────────────────┐
 │ apps/web   UI, scene, input │ ───── offer ────▶ │ gs-server (Rust, tokio)          │
 │ @gs/engine transport, hooks │                   │  ├ WebRTC (str0m, 1 UDP socket)  │
 │ ┌─────────────────────────┐ │◀═ DataChannels ══▶│  ├ room actors @ 60 Hz           │
 │ │ wasm: game + Predictor  │ │     over UDP      │  │   └ game + Authority (Rust)    │
 │ └─────────────────────────┘ │                   │  ├ Dragonfly: tick log, snapshots │
 └─────────────────────────────┘                   │  └ Postgres: matches, leaderboard │
                                                    └──────────────────────────────────┘
```

---

## 1. Get running (10 minutes)

You need **Rust** ([rustup.rs](https://rustup.rs)), **Node 22+** with **pnpm**
(`npm i -g pnpm`), and ideally **Docker** (for Dragonfly + Postgres; without it everything still
runs, just without persistence).

```sh
pnpm setup      # installs the wasm target + wasm-bindgen, JS deps, builds the wasm once
pnpm dev        # Dragonfly + Postgres (docker), the Rust server, and Vite — with hot rebuilds
```

Open the **Network** URL Vite prints (e.g. `http://192.168.1.20:5173`) on any device on the
same Wi-Fi. Enter a name and room and hit **Play on server**. Open a second tab or device with
the same room name and you're in multiplayer. **Practice offline** runs the whole authority
inside your tab, with a simulated network you can make awful on purpose.

Controls: **WASD** move · **mouse** look (click to lock) · **Space** jump · **Shift/click** dash ·
**1–4** emote · **F3** net stats · gamepads work too.

## 2. Host a LAN party

```sh
pnpm stack      # builds one Docker image (server + web client) and starts everything
```

It prints `http://<your-LAN-IP>:8080`, and players open that. Allow **TCP 8080** and
**UDP 40000** through your firewall. That's it: one machine, no cloud.

## 3. Make it your game

Everything game-specific lives in two places:

| You want to change…                         | Edit                                   |
| ------------------------------------------- | -------------------------------------- |
| rules, state, physics, scoring, bots        | `crates/game/src/*.rs`                 |
| what things look like, UI, sounds, controls | `apps/web/src/game/*`                  |
| the lobby / menus                           | `apps/web/src/lobby/Lobby.tsx`         |
| database tables                             | `migrations/*.sql` + `crates/gs-server/src/db.rs` |

The engine (`crates/gs-core`, `crates/gs-wire`, `crates/gs-server`, `crates/gs-client`,
`packages/engine`) is meant to be used, not edited, though it's small and readable when you want to.

### Your first change: a super-jump pad

1. **Rust (the rules)**, `crates/game/src/lib.rs`: in `step`, right after `self.world.step(DT);`:
   ```rust
   // Super-jump pads: launch any player touching a Pad.
   let pads: Vec<BodyId> =
       self.props.iter().filter(|p| p.kind == u16::from(Kind::Pad)).map(|p| p.body).collect();
   let launch: Vec<BodyId> = self
       .players
       .values()
       .map(|p| p.body)
       .filter(|b| self.world.contacts_of(*b).any(|(other, _)| pads.contains(&other)))
       .collect();
   for b in launch {
       self.world[b].vel.y = 14.0;
   }
   ```
2. Save. `pnpm dev` rebuilds the wasm and restarts the server, and the page reloads.
3. That's it. Server and client run the same code, so the jump is predicted on your screen and
   authoritative on the server, with zero networking code.

Then try the recipes in [`docs/RECIPES.md`](docs/RECIPES.md): new input buttons, new entity
kinds, commands, events and effects, HUD fields, game modes, persistence.

### Three rules that keep multiplayer working

1. **The sim must be deterministic.** Use `gs_core::math` (not `f32::sin`), `gs_core::Rng`
   (not OS randomness), `BTreeMap` (not `HashMap`), and the tick (not wall-clock time).
   Clippy enforces this, and `pnpm verify` checks that native and WASM produce the same hash.
2. **All state lives in the game struct and derives `Wire`.** If `step` reads it, it must be
   in there, or prediction and snapshots won't see it.
3. **Rendering only reads.** `render` and `hud` describe state; they never change it.

## 4. Check your work

```sh
cargo test -p game     # your rules + the full netcode path over simulated bad networks
pnpm verify            # everything: fmt, clippy, tests, contract, wasm, types, determinism
pnpm netbot -- --room test --clients 4 --seconds 20   # headless players over real WebRTC
pnpm e2e               # drives your installed Chrome against `pnpm dev`
```

## 5. How it works (short version)

- **Tick loop.** Each room runs at a fixed 60 Hz. Each tick the server gathers everyone's input
  into a `Frame`, calls your `Game::step`, and sends each player a snapshot (delta-compressed
  against the last one they acknowledged).
- **Prediction.** Your browser runs ~2–3 ticks ahead of the server with your real input and a
  guess for everyone else's. When a snapshot arrives, it is compared bit-for-bit with what was
  predicted. If it matches (the usual case for you alone), nothing happens. If not, the browser
  rolls back to the snapshot and re-simulates. Visual pops are smoothed away.
- **Clock steering.** The server reports how early your inputs arrive ("slack"). The client
  speeds up or slows down by up to 5% to keep it around 2 ticks, so inputs are never late and
  never wastefully early.
- **Resilience.** Inputs are sent 8× redundantly, so packet loss heals itself. Dropped
  connections reconnect automatically and reclaim the same seat with a resume token.
  Server restarts resume rooms from Dragonfly.
- **The contract.** Every networked type derives `Wire` (our compact, bounds-checked binary
  format). Its schema is hashed. Client and server refuse to talk if the hashes differ, and
  `contract/schema.txt` shows every protocol change in review.

The long version, with diagrams: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Repo map

```
crates/
  gs-wire/          binary codec + #[derive(Wire)] + schema fingerprints (the contract)
  gs-wire-derive/   the derive macro
  gs-core/          engine: Game trait, math, rng, physics, netcode (authority, predictor,
                    room host, delta, link sim), headless Env, testkit
  game/             ★ YOUR GAME (Orb Arena example)
  gs-server/        the server binary: WebRTC, room actors, Dragonfly, Postgres, HTTP
                    + bin/netbot.rs (headless WebRTC players)
  gs-client/        wasm bindings for the browser (also an in-browser LocalServer)
packages/engine/    @gs/engine: transport, session, input, React hooks, R3F primitives
apps/web/           ★ YOUR CLIENT: lobby, scene, renderers, HUD, input mapping, sfx
migrations/         Postgres schema
contract/           generated wire contract (checked in)
scripts/            dev, build-wasm, verify, contract, stack, e2e, setup
docs/               ARCHITECTURE.md, RECIPES.md
```

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| "WebRTC connect timed out" | The browser can't reach UDP 40000. Check the firewall, and that `GS_PUBLIC_IP` (auto-detected; printed at startup) is the server's LAN address. Guest Wi-Fi networks often block device-to-device traffic. |
| "client build does not match server" | You changed a networked type. Reload the page after the rebuild finishes (`pnpm dev` does both). |
| Rubber-banding in one place | Something in `step` is non-deterministic or reads state that isn't in the struct. Run `pnpm verify` and look at the solo prediction test (`netcode_solo_prediction_is_exact`). |
| Black screen on another device | WebGPU needs a secure context. Over `http://<ip>` three.js falls back to WebGL2 automatically, so check the browser console for other errors. |
| No leaderboard | Postgres isn't running (start Docker), or `DATABASE_URL` is unset. |
| Port already in use | Another `pnpm dev`/`pnpm stack` is running. `pnpm stack down` stops the Docker one. |

Configuration lives in environment variables; see [`.env.example`](.env.example).
