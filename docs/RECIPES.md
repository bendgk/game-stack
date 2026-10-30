# Recipes

Small, complete how-tos. Each one is "change Rust, follow the TypeScript errors". After any
recipe: `cargo test -p game` while iterating, `pnpm verify` before you call it done.

Files you'll touch most:
- `crates/game/src/lib.rs`: types (`Input`, `Command`, `Event`, `Kind`, `Hud`) + `impl Game`
- `apps/web/src/game/`: `input.ts`, `renderers.tsx`, `Fx.tsx`, `Hud.tsx`, `GameView.tsx`

---

## Add a button (e.g. "shoot")

1. `crates/game/src/lib.rs`: add a variant with the next power of two (literal values only):
   ```rust
   pub enum Button { Jump = 1, Dash = 2, Shoot = 4 }
   pub mod buttons { … pub const SHOOT: u8 = Button::Shoot as u8; }
   ```
2. Allow it through the referee in `sanitize`:
   `input.buttons &= buttons::JUMP | buttons::DASH | buttons::SHOOT;`
3. Use it in `apply_input`: `if input.pressed(buttons::SHOOT) { … }`.
4. `apps/web/src/game/input.ts`: `if (kb.held('KeyF')) buttons |= Button.Shoot;`

Buttons are sampled every tick as **held** state. For "on press" behaviour, store the previous
buttons in the player struct and compare inside `step` (that stays deterministic and rollback-safe).

## Add an analog input (e.g. aim pitch)

1. Add a field to `Input` (`pub aim: i8`). Prefer small integers (quantised) over floats.
2. Clamp it in `sanitize`.
3. `pnpm dev` rebuilds; TypeScript now errors in `input.ts` until you set `aim`.
4. `pnpm contract` (the wire changed) and commit `contract/schema.txt`.

## Add an entity kind (e.g. a bouncing ball)

1. Rust: add `Ball = 5` to `enum Kind`, append `"Ball"` to `KIND_NAMES` and `Kind::Ball` to `Kind::ALL`.
2. Spawn it (e.g. in `arena.rs`, with `use gs_physics::d3::{Body, Shape};`):
   `g.world.add(Body::dynamic(Shape::Ball { r: 0.4 }, pos).with_restitution(0.9))`
   and keep its `BodyId` + an `eid` from `g.eid()` in your state.
3. In `render`, push it: `view.push(RenderEntity::new(eid, Kind::Ball, pos).color(0xff8800));`
4. TypeScript now fails in `renderers.tsx` (the map is total over `Kind`). Add a component:
   ```tsx
   function Ball({ entity }: EntityProps) {
     const color = useColor(entity.color);
     return <mesh castShadow><sphereGeometry args={[0.4, 24, 16]} /><meshStandardMaterial color={color} /></mesh>;
   }
   export const renderers = { …, Ball };
   ```
Position/rotation/scale are applied for you. Animate details with `useEntityFrame(entity, (e) => …)`
using the `a`/`b` channels or `flags` you set in Rust.

## Add a one-shot action (Command), e.g. "pick team"

1. Rust: add a variant to `enum Command` (`SetTeam { team: u8 }`) and handle it in the
   `for (id, cmd) in &frame.commands` loop in `step`. Validate it there; it comes from a client.
2. TS: `session.command({ type: 'SetTeam', team: 1 })` (typed; wrong fields don't compile).

Commands go over the reliable channel and enter the sim at the next server tick. They aren't
predicted locally, so the effect appears after ~1 RTT (milliseconds on a LAN).

## Add a sound / particle / shake (Event)

1. Rust: add a variant to `enum Event` and `events.emit(Event::Boom { x, y, z })` in `step`.
2. TS (`Fx.tsx` or any component inside `<GameProvider>`):
   ```tsx
   useGameEvent('Boom', (e) => { add([e.x, e.y, e.z], 0xff4400); sfx.bump(1); });
   ```
Events come from the *predicted* sim, so they fire instantly, and once per tick even across
rollbacks. For things that must never "un-happen" (a win banner), read the HUD instead.

## Show a number in the UI (HUD)

1. Rust: add a field to `struct Hud` and fill it in `hud()`.
2. TS: `const hud = useHud(); hud?.yourField`.

## Add a game mode

`Setup.mode` is the string chosen in the lobby when the room is created. Match it in
`Game::new` (see `Mode`) and branch in `step`. Add the option to the `<select>` in `Lobby.tsx`.

## Better bots

Edit `crates/game/src/bot.rs`. It's a pure function `(&state, me) → Input`. It runs on the
server, and the chosen inputs are logged like human ones, so replays stay exact. Tune it headless:
```rust
let mut env = Env::<OrbArena>::new(Setup { seed: 1, mode: "versus".into() });
env.add_bot("a"); env.add_bot("b");
let result = env.run(ROUND_TICKS + 1).unwrap();
```

## Test a rule

```rust
#[test]
fn dash_has_a_cooldown() {
    let mut e = Env::<OrbArena>::new(Setup::default());
    let me = e.add_player("me");
    e.run(60);
    e.set_input(me, Input { buttons: buttons::DASH, ..Default::default() });
    e.step();
    assert!(e.game().players[&me].dash_cd > 0);
}
```
The netcode tests in `tests.rs` (`netcode_*`, `durable_log_replays_bit_exactly`) re-check
prediction and replay against your rules automatically. Keep them green.

## Store something in Postgres (e.g. per-player stats)

1. New migration file `migrations/0002_stats.sql` (never edit an applied migration).
2. Add a method on `Db` in `crates/gs-server/src/db.rs` using `sqlx::query(...)`.
3. Call it from the room actor in `crates/gs-server/src/rooms.rs`, where `out.result` is
   handled, inside the spawned task, so the tick never waits on the database.
4. Expose it with a route in `crates/gs-server/src/http.rs` and `fetch` it from React.

## Durable chat / message feed (Dragonfly)

Already wired as REST:
```ts
await fetch('/api/channel/lobby', { method: 'POST', body: 'hello!' });
const msgs = await (await fetch('/api/channel/lobby?after=0')).json(); // [{id, text}]
```
Server-side: `store.channel_push(name, bytes, max_len)` / `store.channel_read(name, after, n)`.
For in-game chat that everyone sees at the same tick, use a `Command` instead.

## Tune the netcode

| Knob | Where | Default |
| --- | --- | --- |
| Tick rate | `const TICK_HZ` in `crates/game/src/lib.rs` | 60 |
| Max players per room | `Game::MAX_PLAYERS` | 8 |
| Jitter cushion (ticks) | `PredictorConfig::target_slack` | 2 |
| Correction smoothing | `PredictorConfig::smoothing_tau` / `snap_distance` | 0.1 s / 3 units |
| Input redundancy | `INPUT_REDUNDANCY` in `gs-core/src/net/proto.rs` | 8 |
| Resume grace | `GS_RESUME_GRACE_SECS` | 30 s |
| Checkpoint interval | `GS_CHECKPOINT_EVERY` | 300 ticks |

Rehearse bad networks with **Practice offline → net: awful**, or in Rust with
`gs_core::testkit::soak(…, LinkConfig { latency_ms, jitter_ms, loss }, seconds)`.

## Physics recipes (`gs-physics`)

```rust
use gs_physics::d3::*;                       // or d2::* for 2D: same API with Vec2
let floor  = world.add(Body::floor(0.0));
let ramp   = world.add(Body::plane(vec3(0.0, 0.87, -0.5), Vec3::ZERO));       // 30° slope
let lift   = world.add(Body::kinematic(Shape::Box { half: vec3(1.0, 0.2, 1.0) }, pos).with_vel(vec3(0.0, 1.0, 0.0)));
let coin   = world.add(Body::sensor(Shape::Ball { r: 0.4 }, pos));
let bullet = world.add(Body::dynamic(Shape::Ball { r: 0.1 }, pos).with_vel(dir * 30.0).with_gravity_scale(0.0).with_layers(BULLETS, !BULLETS));
world.add_joint(Joint { a: anchor, b: lamp, rest: 2.0, kind: JointKind::Rope });
world.substeps = 2;                           // fast things / tall stacks
// after world.step(DT):
for (other, normal) in world.began_contacts_of(coin) { /* enter event */ }
let hit = world.raycast(eye, aim, 50.0, u32::MAX);       // Option<RayHit>
let near = world.overlap_ball(center, 3.0, ENEMIES);       // Vec<BodyId>
```
Use `body.user` to map a body back to your entity (`with_user(TAG | id)`). Don't want physics?
Remove `gs-physics` from `crates/game/Cargo.toml` and move things directly in `step`.

## Make a 2D game

1. **Rust:** use `gs_physics::d2::*` (`World::new(vec2(0.0, -30.0))` for a platformer,
   `World::zero_g()` with `with_damping(8.0)` bodies for top-down). Render with
   `RenderEntity::new2d(eid, Kind::Hero, pos, z_order).angle(rot).size2d(vec2(w, h))`.
   Keep `Input` 2D (e.g. `mx`, `my`, buttons). There's no camera yaw to send.
2. **Client** (`apps/web/src/game/GameView.tsx`):
   ```tsx
   <GameCanvas orthographic>
     <Controls build={buildInput}>
       <SessionDriver />
       <Entities renderers={renderers} />
       <Camera2D zoom={48} />            {/* screen px per world unit */}
     </Controls>
   </GameCanvas>
   ```
   and draw kinds with sprites: `Hero: () => <Suspense><Sprite sheet="/sprites/hero.json" anim="hero/run" size={[1, 1]} /></Suspense>`
   (`size={[1,1]}` because `size2d` already scales the entity).
3. **Art:** make the sprites with the `gs2d` MCP (below), with `configure {screen_ppu: 48}`
   matching the camera zoom.
4. Try the physics headless first: `cargo run -p gs-physics --example platformer2d`.

## Make art with the MCP servers

Open Claude Code in the repo. `.mcp.json` starts the servers on demand. Then ask, e.g.:
- *"Configure gs2d for our game (player 1×1 units, background #0f1324), then make a 6-frame
  coin spin, gate it, and export to apps/web/public/sprites/coin.json."*
- *"Configure gs3d (player radius 0.5 m), model a stack of three crates, lint and gate it, and
  export apps/web/public/models/crates.glb."*

Then use the output: `<Sprite sheet="/sprites/coin.json" anim="coin" billboard />` (in 3D) or
load the GLB (see `tools/mcp-3d/SKILL.md`). Keep the gs2d project (`assets/2d/*.gs2d.json`) so
anyone can reopen and tweak the art.

## Start a different game from scratch

1. In `crates/game/src/lib.rs`, replace `OrbArena` with your own struct that derives
   `Wire, Clone, Debug` and implements `Game`. Keep `pub type TheGame = YourGame;` at the bottom.
2. Define `Input`, `Command`, `Event`, `Kind`, `Hud` (keep the `cfg_attr(feature = "ts", …)` lines
   so TypeScript types are generated).
3. Rewrite `apps/web/src/game/*` to match (TypeScript errors will list what's needed).
4. `pnpm contract`, `pnpm verify`.

Need tumbling rigid bodies (rotating boxes, ragdolls)? `gs-physics` keeps boxes axis-aligned.
Anything you add to the sim must be deterministic across native and wasm (e.g. `rapier3d` /
`rapier2d` with the `enhanced-determinism` feature, stored in your state). Add it to the
probes and `pnpm verify` will tell you.
