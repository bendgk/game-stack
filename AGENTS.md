# AGENTS.md — working in game-stack

Instructions for coding agents (Claude Code, Codex, Cursor…) and humans who want to move fast
without breaking multiplayer. Read this before editing. `README.md` is the human onboarding.

## The one idea

**Game logic exists once, in Rust.** `crates/game` is compiled natively into the authoritative
server (`crates/gs-server`) and to wasm32 for the browser's predictor (`crates/gs-client`).
TypeScript renders and builds input. It never simulates, never encodes wire messages, never
duplicates rules. If you are about to write game logic in TypeScript, stop and put it in
`crates/game`.

Types flow one way: **Rust → generated TypeScript** (`packages/engine/src/wasm/gs_client.d.ts`,
via `tsify` + `wasm-bindgen`). Change a Rust type and TS fails to typecheck until the client
matches. Never hand-write a TS type that mirrors a Rust one.

## Where things go

| Task | Files | Notes |
| --- | --- | --- |
| Rules, state, scoring, physics bodies, spawns | `crates/game/src/lib.rs`, `arena.rs` | `Game::step` is the only place state changes |
| Physics (optional) | `gs_physics::d3` (3D) or `gs_physics::d2` (2D): a `World` field in your game state | engine code in `crates/gs-physics`; tests run every behaviour in both modes |
| 2D game rendering | `RenderEntity::new2d(..).angle(..).size2d(..)` in Rust; `<GameCanvas orthographic>` + `<Camera2D>` + `<Sprite>` in TS | `docs/RECIPES.md#make-a-2d-game` |
| 2D art (sprites, tiles, icons, FX) | **`gs2d` MCP** → `apps/web/public/sprites/*.json/png`, source in `assets/2d/*.gs2d.json` | follow `tools/mcp-2d/SKILL.md`; must `gate` before export |
| 3D art (props, characters, rigs) | **`gs3d` MCP** → `apps/web/public/models/*.glb` | follow `tools/mcp-3d/SKILL.md`; needs Blender; must `gate` before export |
| Bot behaviour | `crates/game/src/bot.rs` | `Game::bot_input`, deterministic in state |
| Input fields / buttons | `Input` + `Button` in `crates/game/src/lib.rs`, then `apps/web/src/game/input.ts` | TS errors guide you |
| New entity kind | `Kind` enum + `render()` in Rust, then `apps/web/src/game/renderers.tsx` | the renderer map is total over `Kind` |
| One-shot actions (chat, pick team, buy) | `Command` enum + handling in `step`; send with `session.command(...)` | reliable channel, enters sim at a server tick |
| Sounds / particles / screen shake | `Event` enum, emit in `step`; consume with `useGameEvent` in `apps/web/src/game/Fx.tsx` | fired once per tick even across rollbacks |
| UI numbers (score, timer, health) | `Hud` struct + `hud()` in Rust; read with `useHud()` in `apps/web/src/game/Hud.tsx` | |
| Look & feel, camera, lights | `apps/web/src/game/*` | R3F + three.js r187 (`three/webgpu`) |
| Lobby / menus | `apps/web/src/lobby/Lobby.tsx` | plain React |
| Persistent records (accounts, stats) | `migrations/NNNN_*.sql` (new file, never edit an applied one) + `crates/gs-server/src/db.rs` | Postgres via sqlx |
| Fast durable KV / ordered channels | `crates/gs-server/src/store.rs` (`kv_*`, `channel_*`) | Dragonfly |
| Server HTTP endpoints | `crates/gs-server/src/http.rs` | REST only; never game traffic |
| Engine changes (netcode, physics, codec) | `crates/gs-core`, `crates/gs-wire`, `packages/engine` | rare; add tests in the same crate |

Step-by-step recipes for each row: `docs/RECIPES.md`. Architecture and the "why": `docs/ARCHITECTURE.md`.

## Hard rules (break these and multiplayer breaks)

1. **Determinism in the sim.** Anything `Game::step` calls must give bit-identical results on
   x86/ARM native and wasm32:
   - math: `gs_core::math::{sin, cos, atan2, sqrt, …}`, never `f32::sin` etc. (clippy denies them in `crates/game` and `crates/gs-core`)
   - randomness: a `gs_core::Rng` stored in the game state, never `rand`/`getrandom`/`Math.random`
   - collections: `BTreeMap`/`BTreeSet`/`Vec`, never `HashMap`/`HashSet` (clippy denies)
   - time: the tick counter, never `Instant`/`SystemTime` (clippy denies)
   - no FMA (`mul_add`), no threads, no I/O, no globals/statics with mutable state
2. **All sim state is in the game struct and derives `Wire`.** If `step` reads it, it is
   serialized. A `#[wire(skip)]` field must be recomputed by `step` before it is read.
3. **`render` / `hud` / `bot_input` are pure reads.** They take `&self`.
4. **Sanitize input in `Game::sanitize`.** It is the server's referee. Clamp every field; treat
   floats as hostile (NaN/inf).
5. **Never hand-edit generated files:** `packages/engine/src/wasm/*` (run `pnpm build:wasm`),
   `contract/schema.txt` (run `pnpm contract`), exported sprite sheets (re-export from the
   `assets/2d/*.gs2d.json` project with gs2d).
6. **No game traffic over HTTP/WebSocket.** HTTP is for signaling and REST. Gameplay goes through
   the DataChannels via the engine.
7. **Keep TS a renderer.** If TS needs a number the sim knows, add it to `Hud`, `Event` or the
   render entity (`a`/`b` channels, flags). Don't re-derive it.

## The loop

```sh
pnpm dev                 # everything, rebuilds wasm + restarts server on save
cargo test -p game       # fast: rules + netcode soak + replay + determinism + contract
pnpm verify              # full gate — run before you say you're done
```

`pnpm verify` = rustfmt → clippy (native + wasm, `-D warnings`) → `cargo test --workspace` →
contract drift → wasm build → `tsc` (TypeScript 7) → TS tests → native-vs-wasm determinism
probe. **It must pass.** If the contract check fails and you meant to change the protocol, run
`pnpm contract` and include `contract/schema.txt` in the change.

More checks when relevant:
- `pnpm netbot -- --room x --clients 4 --seconds 20`: headless players over real WebRTC against a running server.
- `pnpm e2e`: drives the installed Chrome against `pnpm dev` (offline + 2-player online) and saves screenshots to `target/e2e/`.
- In the browser console, `__gs.stats`, `__gs.hud` and `__gs.entities.list` show live session state.

## Making assets (MCP)

The repo's `.mcp.json` provides `gs2d` (2D, pure Node) and `gs3d` (3D, headless Blender).
Skills: `.claude/skills/gs-2d`, `.claude/skills/gs-3d`. The rules:
- `configure` once per game (player size, camera, background) so probes measure against the real game.
- Look at every step (`render`/`preview`/`contact_sheet`). Probes don't see taste.
- `gate` must pass before export. Keep the editable source (gs2d `save_project` to `assets/2d/`,
  gs3d `save_blend` to `assets/3d/`).
- Match sizes with physics: a sprite/GLB that is `h` units tall needs a collider of that size in Rust.

## Tests to write

- **Every rule:** a headless test in `crates/game/src/tests.rs` using `gs_core::env::Env`
  (add players/bots, `set_input`, `step`/`run`, assert on `env.game()`). No network needed.
- **Anything touching state shape or physics:** the existing netcode tests already cover you.
  `netcode_solo_prediction_is_exact` fails if client and server ever step differently, and
  `durable_log_replays_bit_exactly` fails if replay from Dragonfly would diverge. Keep them green.
- **Engine changes:** unit tests beside the code. Hostile-input fuzz for anything that decodes bytes.
  Physics changes: write the test once as a generic `fn x<V: Vector>()` and run it in both
  modes (`in_both_modes!` in `crates/gs-physics/src/tests.rs`).
- **MCP server changes:** `node --test tools/mcp-2d/lib.test.mjs tools/mcp-3d/lib.test.mjs`
  (`GS3D_IT=1` adds real-Blender tests).

## Diagnosing

| Symptom | Likely cause |
| --- | --- |
| `netcode_solo_prediction_is_exact` fails | non-determinism or state not in the struct (rules 1–2) |
| native vs wasm probe mismatch in `pnpm verify` | a platform-dependent op (transcendental, HashMap order, FMA) |
| `durable_log_replays_bit_exactly` fails | `step` depends on something not in `Frame` or state |
| TS error after a Rust change | expected: update the TS side to the new generated types |
| "client build does not match server" in browser | stale wasm or server; `pnpm dev` rebuilds both, then reload |
| many rollbacks with one player | same as the first row |
| snapshots huge (`snap` in F3 overlay) | large or volatile state; keep render-only data out of state |

## Conventions

- Rust 2024, `rustfmt.toml` (120 cols), clippy clean. TS strict, ES modules, `.ts` import suffixes.
- Comments explain *why*. Match the surrounding density.
- Keep `crates/game` readable for students: plain structs, small functions, no clever generics.
- Env vars: declare in `crates/gs-server/src/config.rs` **and** `.env.example`.
- Don't add dependencies to `crates/game`/`gs-core`/`gs-physics` that use floats, threads or
  OS APIs in ways that could differ across targets. A different physics engine needs
  cross-platform determinism (for example rapier with `enhanced-determinism`); the probe in
  `pnpm verify` checks it.
