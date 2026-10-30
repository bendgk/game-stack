//! Rule tests run headless through `gs_core::env::Env` — no network, no browser.

use super::*;
use gs_core::env::{Env, probe};

fn env(mode: &str) -> Env<OrbArena> {
    Env::new(Setup { seed: 7, mode: mode.into() })
}

#[test]
fn players_spawn_and_fall_to_the_floor() {
    let mut e = env("versus");
    let me = e.add_player("alice");
    e.run(120);
    let p = &e.game().players[&me];
    let b = &e.game().world[p.body];
    assert!(b.grounded, "player settled, y = {}", b.pos.y);
}

#[test]
fn walking_forward_moves_along_camera_yaw() {
    let mut e = env("versus");
    let me = e.add_player("alice");
    e.run(60);
    let body = e.game().players[&me].body;
    let start = e.game().world[body].pos;
    for _ in 0..30 {
        e.set_input(me, Input { mz: 127, yaw: 0.0, ..Default::default() });
        e.step();
    }
    let end = e.game().world[body].pos;
    assert!(end.z < start.z - 2.0, "yaw 0 is -Z: {start} -> {end}");
}

#[test]
fn bots_collect_orbs_and_rounds_end() {
    let mut e = env("versus");
    e.add_bot("a");
    e.add_bot("b");
    let res = e.run(ROUND_TICKS + 10).expect("round ends with a result");
    let total: i32 = res.standings.iter().map(|s| s.1).sum();
    assert!(total > 0, "bots scored: {res:?}");
    assert!(matches!(e.game().phase, Phase::Intermission { .. }));
    e.run(INTERMISSION_TICKS + 2);
    assert_eq!(e.game().round, 2);
}

#[test]
fn coop_ends_when_goal_reached() {
    let mut e = env("coop");
    e.add_bot("a");
    e.add_bot("b");
    e.add_bot("c");
    let res = e.run(ROUND_TICKS).expect("coop round ends");
    assert!(res.summary.contains('/'), "{}", res.summary);
}

#[test]
fn sanitize_is_the_referee() {
    let bad = Input { mx: i8::MIN, mz: 0, yaw: f32::NAN, buttons: 0xff };
    let ok = OrbArena::sanitize(bad);
    assert_eq!(ok.mx, -127);
    assert_eq!(ok.yaw, 0.0);
    assert_eq!(ok.buttons, buttons::JUMP | buttons::DASH);
}

#[test]
fn state_roundtrips_through_the_wire_bit_exactly() {
    let mut e = env("versus");
    e.add_bot("a");
    e.add_bot("b");
    e.run(200);
    let bytes = gs_wire::to_bytes(e.game());
    let back: OrbArena = gs_wire::from_bytes(&bytes).unwrap();
    assert_eq!(gs_wire::to_bytes(&back), bytes);
}

#[test]
fn deterministic_across_runs() {
    assert_eq!(probe::<OrbArena>(1, "versus", 4, 900), probe::<OrbArena>(1, "versus", 4, 900));
    assert_ne!(probe::<OrbArena>(1, "versus", 4, 900), probe::<OrbArena>(2, "versus", 4, 900));
}

// ---- Netcode: the real server/client path for this game, over simulated networks ----------

use gs_core::net::LinkConfig;
use gs_core::testkit::{replay_check, soak};

#[test]
fn netcode_soak_lan() {
    let r = soak::<OrbArena>(Setup { seed: 3, mode: "versus".into() }, 3, 1, LinkConfig::lan(), 8.0);
    eprintln!("{r:#?} miss_rate={:.4}", r.miss_rate());
    assert_eq!(r.welcomed, 3);
    assert!(r.snapshots > 3 * 400, "snapshots flow");
    assert!(r.miss_rate() < 0.02, "inputs arrive in time on a LAN: {}", r.miss_rate());
    assert!(r.final_slack.iter().all(|s| (0.5..5.0).contains(s)), "lead converged: {:?}", r.final_slack);
}

#[test]
fn netcode_soak_rough_network() {
    // 40ms ± 15ms with 5% loss: much worse than any LAN. Must stay connected and converge.
    let r = soak::<OrbArena>(Setup { seed: 4, mode: "coop".into() }, 2, 1, LinkConfig::rough(), 8.0);
    eprintln!("{r:#?} miss_rate={:.4}", r.miss_rate());
    assert_eq!(r.welcomed, 2);
    assert!(r.snapshots > 2 * 300);
    assert!(r.miss_rate() < 0.10, "redundant inputs cover loss: {}", r.miss_rate());
}

#[test]
fn durable_log_replays_bit_exactly() {
    replay_check::<OrbArena>(Setup { seed: 9, mode: "versus".into() }, 4, 300, 1500).unwrap();
}

#[test]
fn netcode_solo_prediction_is_exact() {
    // One client, no bots: nothing is unknown to the predictor, so every snapshot must match
    // the prediction bit-for-bit. A rollback here means client and server stepped differently.
    let r = soak::<OrbArena>(Setup { seed: 5, mode: "versus".into() }, 1, 0, LinkConfig::lan(), 6.0);
    eprintln!("{r:#?}");
    assert!(r.prediction_hits > 300, "{r:?}");
    assert!(r.rollbacks <= 2, "solo prediction diverged {} times", r.rollbacks);
}

#[test]
fn wire_contract_is_checked_in() {
    // The wire contract (every type that crosses the network) is checked in so protocol changes
    // are visible in review. If this fails you changed a networked type: run `pnpm contract`,
    // rebuild the client (`pnpm build:wasm`), and commit contract/schema.txt.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../contract/schema.txt");
    let want = gs_core::net::contract_text::<OrbArena>();
    let have = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        have == want,
        "contract/schema.txt is stale — run `pnpm contract`.\n--- checked in ---\n{have}\n--- current ---\n{want}"
    );
}

#[test]
fn kinds_are_consistent() {
    assert_eq!(Kind::ALL.len(), KIND_NAMES.len(), "add the new Kind to Kind::ALL and KIND_NAMES");
    for (i, k) in Kind::ALL.iter().enumerate() {
        assert_eq!(*k as usize, i, "Kind discriminants must be 0..n in order");
        assert_eq!(format!("{k:?}"), KIND_NAMES[i], "KIND_NAMES[{i}] must be the variant name");
    }
}
