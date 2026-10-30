// The render bridge: Rust writes a flat Float32Array of entities every frame (see
// gs-core/src/render.rs); this turns it into stable, mutable `EntityView` objects keyed by id.
// Transforms change every frame without React re-rendering; only the *set* of entities
// (spawn / despawn) notifies subscribers.

export const FLAG_ME = 1 << 0;
export const FLAG_HIDDEN = 1 << 1;
export const FLAG_TELEPORT = 1 << 2;
export const FLAG_USER = 1 << 8;

export interface EntityView {
  readonly id: number;
  kind: number;
  flags: number;
  /** Interpolated + correction-smoothed position. */
  pos: [number, number, number];
  /** Quaternion x, y, z, w. */
  rot: [number, number, number, number];
  scale: [number, number, number];
  /** 0xRRGGBB */
  color: number;
  /** Free channels the game uses for whatever it wants (health, charge, emote…). */
  a: number;
  b: number;
}

export class EntityTable {
  readonly byId = new Map<number, EntityView>();
  #list: EntityView[] = [];
  #listeners = new Set<() => void>();
  #me: EntityView | null = null;

  /** Current entities (stable array between membership changes — safe for React). */
  get list(): readonly EntityView[] {
    return this.#list;
  }

  /** The local player's entity (flagged `FLAG_ME` by the game's `render`). */
  get me(): EntityView | null {
    return this.#me;
  }

  subscribe = (fn: () => void) => {
    this.#listeners.add(fn);
    return () => this.#listeners.delete(fn);
  };

  getSnapshot = () => this.#list;

  update(buf: Float32Array, count: number, stride: number) {
    let changed = count !== this.byId.size;
    const seen = new Set<number>();
    this.#me = null;
    for (let i = 0; i < count; i++) {
      const o = i * stride;
      const id = buf[o]!;
      seen.add(id);
      let e = this.byId.get(id);
      if (!e) {
        e = { id, kind: 0, flags: 0, pos: [0, 0, 0], rot: [0, 0, 0, 1], scale: [1, 1, 1], color: 0xffffff, a: 0, b: 0 };
        this.byId.set(id, e);
        changed = true;
      }
      const kind = buf[o + 1]!;
      if (e.kind !== kind) changed = true;
      e.kind = kind;
      e.flags = buf[o + 2]!;
      e.pos[0] = buf[o + 3]!;
      e.pos[1] = buf[o + 4]!;
      e.pos[2] = buf[o + 5]!;
      e.rot[0] = buf[o + 6]!;
      e.rot[1] = buf[o + 7]!;
      e.rot[2] = buf[o + 8]!;
      e.rot[3] = buf[o + 9]!;
      e.scale[0] = buf[o + 10]!;
      e.scale[1] = buf[o + 11]!;
      e.scale[2] = buf[o + 12]!;
      e.color = buf[o + 13]!;
      e.a = buf[o + 14]!;
      e.b = buf[o + 15]!;
      if (e.flags & FLAG_ME) this.#me = e;
    }
    for (const id of this.byId.keys()) {
      if (!seen.has(id)) {
        this.byId.delete(id);
        changed = true;
      }
    }
    if (changed) {
      this.#list = [...this.byId.values()];
      for (const fn of this.#listeners) fn();
    }
  }
}
