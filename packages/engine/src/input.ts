// Device state helpers. The *game* decides how devices map to its Rust `Input` type
// (apps/web/src/game/input.ts); these just make the raw state easy to read.

export class Keyboard {
  readonly down = new Set<string>();
  #pressed = new Set<string>();
  #off: () => void;

  constructor(target: Window = window) {
    const kd = (e: KeyboardEvent) => {
      if (isTyping(e)) return;
      if (!this.down.has(e.code)) this.#pressed.add(e.code);
      this.down.add(e.code);
    };
    const ku = (e: KeyboardEvent) => this.down.delete(e.code);
    const blur = () => this.down.clear();
    target.addEventListener('keydown', kd);
    target.addEventListener('keyup', ku);
    target.addEventListener('blur', blur);
    this.#off = () => {
      target.removeEventListener('keydown', kd);
      target.removeEventListener('keyup', ku);
      target.removeEventListener('blur', blur);
    };
  }

  /** Held right now. Accepts any of several codes ('KeyW', 'ArrowUp'). */
  held(...codes: string[]) {
    return codes.some((c) => this.down.has(c));
  }

  /** Went down since the last `endFrame()`. */
  pressed(...codes: string[]) {
    return codes.some((c) => this.#pressed.has(c));
  }

  /** -1 / 0 / +1 from a negative and positive key set. */
  axis(neg: string[], pos: string[]) {
    return (this.held(...pos) ? 1 : 0) - (this.held(...neg) ? 1 : 0);
  }

  endFrame() {
    this.#pressed.clear();
  }

  dispose() {
    this.#off();
  }
}

function isTyping(e: KeyboardEvent) {
  const t = e.target as HTMLElement | null;
  return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
}

/** Pointer-lock mouse look. Accumulates yaw/pitch in radians. */
export class MouseLook {
  yaw = 0;
  pitch = -0.5;
  sensitivity = 0.0025;
  minPitch = -1.2;
  maxPitch = 0.3;
  buttons = 0;
  #off: () => void;

  constructor(readonly element: HTMLElement) {
    const move = (e: MouseEvent) => {
      if (document.pointerLockElement !== element) return;
      this.yaw -= e.movementX * this.sensitivity;
      this.pitch = Math.min(this.maxPitch, Math.max(this.minPitch, this.pitch - e.movementY * this.sensitivity));
    };
    const down = (e: MouseEvent) => {
      this.buttons = e.buttons;
      if (document.pointerLockElement !== element) element.requestPointerLock?.();
    };
    const up = (e: MouseEvent) => (this.buttons = e.buttons);
    window.addEventListener('mousemove', move);
    element.addEventListener('mousedown', down);
    window.addEventListener('mouseup', up);
    this.#off = () => {
      window.removeEventListener('mousemove', move);
      element.removeEventListener('mousedown', down);
      window.removeEventListener('mouseup', up);
    };
  }

  get locked() {
    return document.pointerLockElement === this.element;
  }

  dispose() {
    this.#off();
  }
}

/** First connected gamepad, with a radial dead-zone applied to the sticks. */
export function readGamepad(deadzone = 0.15) {
  const pad = navigator.getGamepads?.().find((p) => p?.connected);
  if (!pad) return null;
  const dz = (x: number, y: number): [number, number] => {
    const m = Math.hypot(x, y);
    if (m < deadzone) return [0, 0];
    const s = (m - deadzone) / (1 - deadzone) / m;
    return [x * s, y * s];
  };
  const [lx, ly] = dz(pad.axes[0] ?? 0, pad.axes[1] ?? 0);
  const [rx, ry] = dz(pad.axes[2] ?? 0, pad.axes[3] ?? 0);
  return { lx, ly, rx, ry, button: (i: number) => !!pad.buttons[i]?.pressed };
}

/** Float in [-1, 1] → signed byte in [-127, 127] (the usual Rust `i8` axis). */
export function toAxisI8(v: number) {
  return Math.round(Math.max(-1, Math.min(1, v)) * 127);
}
