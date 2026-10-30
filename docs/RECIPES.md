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
2. Spawn it (e.g. in `arena.rs`): `g.world.add(Body::dynamic(Shape::Sphere { r: 0.4 }, pos).with_restitution(0.9))`
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

## Start a different game from scratch

1. In `crates/game/src/lib.rs`, replace `OrbArena` with your own struct that derives
   `Wire, Clone, Debug` and implements `Game`. Keep `pub type TheGame = YourGame;` at the bottom.
2. Define `Input`, `Command`, `Event`, `Kind`, `Hud` (keep the `cfg_attr(feature = "ts", …)` lines
   so TypeScript types are generated).
3. Rewrite `apps/web/src/game/*` to match (TypeScript errors will list what's needed).
4. `pnpm contract`, `pnpm verify`.

Need a real physics engine? Anything you add to the sim must be deterministic across native and
wasm (e.g. `rapier3d` with the `enhanced-determinism` feature). The determinism probe in
`pnpm verify` will tell you.
