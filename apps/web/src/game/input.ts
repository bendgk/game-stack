// Device → Rust `Input` mapping for this game. `Input` and `Button` are generated from
// crates/game/src/lib.rs, so adding a field there makes this file fail to typecheck.

import { Button, toAxisI8, type Input } from '@gs/engine';
import type { Devices } from '@gs/engine/three';

export function buildInput({ keyboard: kb, look, gamepad: pad }: Devices): Input {
  let mx = kb.axis(['KeyA', 'ArrowLeft'], ['KeyD', 'ArrowRight']);
  let mz = kb.axis(['KeyS', 'ArrowDown'], ['KeyW', 'ArrowUp']);
  let buttons = 0;
  if (pad) {
    mx += pad.lx;
    mz -= pad.ly;
    look.yaw -= pad.rx * 0.06;
    look.pitch = Math.min(look.maxPitch, Math.max(look.minPitch, look.pitch - pad.ry * 0.04));
    if (pad.button(0)) buttons |= Button.Jump;
    if (pad.button(2) || pad.button(7)) buttons |= Button.Dash;
  }
  if (kb.held('Space')) buttons |= Button.Jump;
  if (kb.held('ShiftLeft', 'ShiftRight') || (look.locked && look.buttons & 1)) buttons |= Button.Dash;
  return { mx: toAxisI8(mx), mz: toAxisI8(mz), yaw: look.yaw, buttons };
}
