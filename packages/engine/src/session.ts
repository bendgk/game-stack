// GameSession: one player's live connection to a room. It owns the wasm `GameClient`
// (prediction, rollback, smoothing — all Rust) and a Transport, and exposes plain data to React.
//
// Resilience: if the transport drops, the session reconnects with backoff and presents its
// resume token, so the server hands back the same seat (within GS_RESUME_GRACE_SECS). The
// token is kept in sessionStorage, so a page reload resumes too.

import { EntityTable } from './entities.ts';
import { LocalHost, type NetSim, type Transport, connectWebRtc } from './transport.ts';
import type { Command, Core, Event, GameClient, Hud, Input, Kind, NetStats } from './wasm.ts';
import { loadCore } from './wasm.ts';

export type SessionStatus = 'connecting' | 'joining' | 'playing' | 'reconnecting' | 'error' | 'closed';

export interface SessionOptions {
  name: string;
  room: string;
  /** Mode for a room this session creates (e.g. 'coop', 'versus'). */
  mode: string;
  /** Server bots to add when this session creates the room. */
  bots?: number;
  /** Run the authority in this tab instead of connecting to gs-server. */
  offline?: boolean;
  /** Server origin; '' = same origin (Vite proxies /api in dev). */
  server?: string;
  /** Offline only: simulated network. */
  netsim?: NetSim;
}

type EventOf<T extends Event['type']> = Extract<Event, { type: T }>;

const RESUME_KEY = (room: string) => `gs:resume:${room}`;

function storage(): Storage | null {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

export class GameSession {
  readonly entities = new EntityTable();
  status: SessionStatus = 'connecting';
  error: string | null = null;
  hud: Hud | null = null;
  stats: NetStats | null = null;
  notices: { id: number; text: string }[] = [];
  kinds: Kind[] = [];
  /** Input sent on the next simulated tick. Mutate fields or replace it. */
  input: Input = { mx: 0, mz: 0, yaw: 0, buttons: 0 };

  #core!: Core;
  #client: GameClient | null = null;
  #transport: Transport | null = null;
  #local: LocalHost | null = null;
  #listeners = new Set<() => void>();
  #eventHandlers = new Map<string, Set<(e: Event, tick: number) => void>>();
  #lastHud = 0;
  #noticeId = 0;
  #disposed = false;
  #version = 0;

  private constructor(readonly opts: SessionOptions) {}

  static async create(opts: SessionOptions): Promise<GameSession> {
    const s = new GameSession(opts);
    s.#core = await loadCore();
    s.kinds = s.#core.kind_names() as Kind[];
    s.#watchdog();
    void s.#connect(0);
    return s;
  }

  #lastFrame = 0;
  #watchdogTimer: ReturnType<typeof setInterval> | undefined;
  #onVisibility = () => {
    // A hidden tab stops requestAnimationFrame. Go neutral so our avatar doesn't keep running
    // on the server's repeat-last-input, and keep ticking from a timer.
    if (document.hidden) {
      this.input = { mx: 0, mz: 0, yaw: this.input.yaw, buttons: 0 };
      this.frame();
    }
  };
  #watchdog() {
    document.addEventListener('visibilitychange', this.#onVisibility);
    this.#watchdogTimer = setInterval(() => {
      if (performance.now() - this.#lastFrame > 100) this.frame();
    }, 50);
  }

  get tickHz() {
    return this.#core.tick_hz();
  }
  get fingerprint() {
    return this.#core.fingerprint();
  }
  get me(): number | null {
    return this.#client?.me() ?? null;
  }
  get offline() {
    return !!this.opts.offline;
  }

  // --- subscription (for useSyncExternalStore) --------------------------------------------

  subscribe = (fn: () => void) => {
    this.#listeners.add(fn);
    return () => this.#listeners.delete(fn);
  };
  getVersion = () => this.#version;
  #emit() {
    this.#version++;
    for (const fn of this.#listeners) fn();
  }

  /** Listen for one game event type (fired once per tick even across rollbacks). */
  on<T extends Event['type']>(type: T, fn: (e: EventOf<T>, tick: number) => void): () => void {
    let set = this.#eventHandlers.get(type);
    if (!set) this.#eventHandlers.set(type, (set = new Set()));
    const h = fn as (e: Event, tick: number) => void;
    set.add(h);
    return () => set.delete(h);
  }

  /** Send a reliable command (enters the sim at a server-chosen tick). */
  command(cmd: Command) {
    this.#client?.command(cmd);
  }

  // --- connection lifecycle ----------------------------------------------------------------

  async #connect(attempt: number): Promise<void> {
    if (this.#disposed) return;
    this.status = attempt === 0 ? 'connecting' : 'reconnecting';
    this.#emit();
    try {
      let t: Transport;
      if (this.opts.offline) {
        this.#local ??= new LocalHost(this.#core, this.opts.mode, this.opts.bots ?? 0, this.opts.netsim);
        t = this.#local.connect();
      } else {
        t = await connectWebRtc(this.opts.server ?? '');
      }
      if (this.#disposed) return t.close();
      this.#attach(t);
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
      const delay = Math.min(4000, 400 * 2 ** attempt);
      this.status = 'reconnecting';
      this.#emit();
      setTimeout(() => void this.#connect(attempt + 1), delay);
    }
  }

  #attach(t: Transport) {
    this.#client?.free();
    const client = new this.#core.GameClient();
    this.#client = client;
    this.#transport = t;
    this.status = 'joining';
    this.#emit();
    t.onmessage = (data) => {
      if (this.#client !== client) return;
      try {
        const was = client.connected();
        client.receive(data, performance.now());
        if (!was && client.connected()) {
          this.status = 'playing';
          this.error = null;
          const token = client.resume_token();
          if (token !== undefined) storage()?.setItem(RESUME_KEY(this.opts.room), token.toString());
          this.#emit();
        }
      } catch (e) {
        // A Reject (e.g. stale client build) is fatal; anything else is a dropped bad packet.
        const msg = e instanceof Error ? e.message : String(e);
        if (msg.includes('rejected')) {
          this.status = 'error';
          this.error = msg;
          storage()?.removeItem(RESUME_KEY(this.opts.room));
          this.#emit();
          t.onclose = null;
          t.close();
        }
      }
    };
    t.onclose = (reason) => {
      if (this.#disposed || this.#client !== client) return;
      this.error = `connection lost (${reason})`;
      void this.#connect(1);
    };
    const saved = storage()?.getItem(RESUME_KEY(this.opts.room));
    const hello = client.hello(this.opts.name, this.opts.room, this.opts.mode, this.opts.bots ?? 0, saved ? BigInt(saved) : undefined);
    t.send(0, hello.subarray(1));
  }

  // --- per-frame ---------------------------------------------------------------------------

  /** Call once per display frame (the R3F `<SessionDriver>` does this for you). */
  frame(now = performance.now()) {
    this.#lastFrame = now;
    const c = this.#client;
    if (!c || !c.connected() || !this.#transport) return;
    c.set_input(this.input);
    c.update(now);
    for (const pkt of c.drain_outbox() as Uint8Array[]) {
      this.#transport.send(pkt[0] as 0 | 1, pkt.subarray(1));
    }
    for (const { tick, event } of c.drain_events()) {
      this.#eventHandlers.get(event.type)?.forEach((h) => h(event, tick));
      this.#eventHandlers.get('*')?.forEach((h) => h(event, tick));
    }
    const notices = c.drain_notices();
    if (notices.length) {
      this.notices = [...this.notices, ...notices.map((text) => ({ id: ++this.#noticeId, text }))].slice(-6);
      this.#emit();
    }
    const count = c.render();
    const stride = this.#core.render_stride();
    const view = new Float32Array(this.#core.memory.buffer, c.render_ptr(), count * stride);
    this.entities.update(view, count, stride);
    // UI data at ~15 Hz is plenty and keeps React cheap.
    if (now - this.#lastHud > 66) {
      this.#lastHud = now;
      this.hud = c.hud();
      this.stats = c.stats();
      this.#emit();
    }
  }

  dispose() {
    this.#disposed = true;
    clearInterval(this.#watchdogTimer);
    document.removeEventListener('visibilitychange', this.#onVisibility);
    this.#transport?.close();
    this.#local?.dispose();
    this.#client?.free();
    this.#client = null;
    this.status = 'closed';
    this.#emit();
  }
}
