// Transports carry opaque bytes between the wasm client and an authority.
//
// - `connectWebRtc` — the real thing: two RTCDataChannels to gs-server over UDP.
//   `rel` (reliable, ordered) and `unrel` (unordered, no retransmits). Signaling is one
//   HTTP POST; no game data ever goes over TCP.
// - `LoopbackTransport` — talks to a `LocalServer` (the same authority code, in wasm) for
//   offline play, with optional simulated latency / jitter / loss to rehearse bad networks.

import type { Core, LocalServer } from './wasm.ts';

export const RELIABLE = 0;
export const UNRELIABLE = 1;
export type ChannelId = typeof RELIABLE | typeof UNRELIABLE;

export interface Transport {
  send(channel: ChannelId, data: Uint8Array): void;
  onmessage: ((data: Uint8Array) => void) | null;
  onclose: ((reason: string) => void) | null;
  close(): void;
  readonly kind: 'webrtc' | 'local';
}

/** Max bytes queued on the unreliable channel before newer packets are dropped (newest wins). */
const MAX_UNRELIABLE_BUFFER = 64 * 1024;

class RtcTransport implements Transport {
  readonly kind = 'webrtc';
  onmessage: ((data: Uint8Array) => void) | null = null;
  onclose: ((reason: string) => void) | null = null;
  #closed = false;
  constructor(
    private pc: RTCPeerConnection,
    private channels: [RTCDataChannel, RTCDataChannel],
  ) {
    for (const ch of channels) {
      ch.binaryType = 'arraybuffer';
      ch.onmessage = (e) => this.onmessage?.(new Uint8Array(e.data as ArrayBuffer));
      ch.onclose = () => this.#fail('datachannel closed');
    }
    pc.onconnectionstatechange = () => {
      if (pc.connectionState === 'failed' || pc.connectionState === 'closed') this.#fail(`peer ${pc.connectionState}`);
    };
  }
  send(channel: ChannelId, data: Uint8Array) {
    const ch = this.channels[channel];
    if (ch.readyState !== 'open') return;
    if (channel === UNRELIABLE && ch.bufferedAmount > MAX_UNRELIABLE_BUFFER) return;
    ch.send(data as Uint8Array<ArrayBuffer>);
  }
  #fail(reason: string) {
    if (this.#closed) return;
    this.#closed = true;
    this.onclose?.(reason);
    this.close();
  }
  close() {
    this.#closed = true;
    for (const ch of this.channels) ch.close();
    this.pc.close();
  }
}

/** Open a WebRTC connection to gs-server. `base` is the server origin ('' = same origin). */
export async function connectWebRtc(base = '', timeoutMs = 8000): Promise<Transport> {
  // LAN only: no STUN/TURN. The server is ICE-lite with one host candidate.
  const pc = new RTCPeerConnection({ iceServers: [] });
  const rel = pc.createDataChannel('rel', { ordered: true });
  const unrel = pc.createDataChannel('unrel', { ordered: false, maxRetransmits: 0 });
  await pc.setLocalDescription(await pc.createOffer());
  const res = await fetch(`${base}/api/rtc/offer`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(pc.localDescription),
  });
  if (!res.ok) {
    pc.close();
    throw new Error(`signaling failed: ${res.status} ${await res.text()}`);
  }
  await pc.setRemoteDescription(await res.json());
  const open = (ch: RTCDataChannel) =>
    new Promise<void>((resolve, reject) => {
      if (ch.readyState === 'open') return resolve();
      ch.addEventListener('open', () => resolve(), { once: true });
      ch.addEventListener('error', () => reject(new Error(`${ch.label} failed`)), { once: true });
    });
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () => reject(new Error('WebRTC connect timed out — is UDP to the server allowed? (GS_PUBLIC_IP / firewall)')),
      timeoutMs,
    );
  });
  try {
    await Promise.race([Promise.all([open(rel), open(unrel)]), timeout]);
  } catch (e) {
    pc.close();
    throw e;
  } finally {
    clearTimeout(timer);
  }
  return new RtcTransport(pc, [rel, unrel]);
}

export interface NetSim {
  /** One-way latency, ms. */
  latencyMs: number;
  jitterMs: number;
  /** 0..1, unreliable channel only. */
  loss: number;
}

export const NO_NETSIM: NetSim = { latencyMs: 0, jitterMs: 0, loss: 0 };

/** An in-browser authority (offline / practice). Ticks on its own fixed clock. */
export class LocalHost {
  readonly server: LocalServer;
  #conns = new Map<number, LoopbackTransport>();
  #timer: ReturnType<typeof setInterval>;
  #next = performance.now();
  #nextConn = 1;

  constructor(core: Core, mode: string, bots: number, public netsim: NetSim = NO_NETSIM) {
    this.server = new core.LocalServer(mode, bots, BigInt(Math.floor(Math.random() * 2 ** 52)));
    const dt = 1000 / core.tick_hz();
    // A fast poll with an accumulator keeps the tick rate exact despite timer jitter.
    this.#timer = setInterval(() => {
      const now = performance.now();
      let n = 0;
      while (this.#next <= now && n++ < 8) {
        this.server.tick();
        this.#next += dt;
      }
      if (now - this.#next > 250) this.#next = now;
      this.#flush();
    }, 4);
  }

  connect(): LoopbackTransport {
    const conn = this.#nextConn++;
    const t = new LoopbackTransport(this, conn);
    this.#conns.set(conn, t);
    return t;
  }

  /** @internal */
  deliverToServer(conn: number, channel: ChannelId, data: Uint8Array) {
    this.#delay(channel, () => {
      this.server.receive(conn, data);
      this.#flush();
    });
  }

  /** @internal */
  drop(conn: number) {
    this.#conns.delete(conn);
    this.server.disconnect(conn);
  }

  #flush() {
    for (const pkt of this.server.drain_outbox() as Uint8Array[]) {
      const conn = new DataView(pkt.buffer, pkt.byteOffset).getUint32(0, true);
      const channel = pkt[4] as ChannelId;
      const t = this.#conns.get(conn);
      if (t) this.#delay(channel, () => t.onmessage?.(pkt.subarray(5)));
    }
  }

  #lastReliable = 0;
  #delay(channel: ChannelId, fn: () => void) {
    const { latencyMs, jitterMs, loss } = this.netsim;
    if (latencyMs === 0 && jitterMs === 0 && loss === 0) return fn();
    if (channel === UNRELIABLE && Math.random() < loss) return;
    let at = performance.now() + latencyMs + Math.random() * jitterMs;
    if (channel === RELIABLE) at = this.#lastReliable = Math.max(at, this.#lastReliable);
    setTimeout(fn, at - performance.now());
  }

  dispose() {
    clearInterval(this.#timer);
    this.server.free();
  }
}

export class LoopbackTransport implements Transport {
  readonly kind = 'local';
  onmessage: ((data: Uint8Array) => void) | null = null;
  onclose: ((reason: string) => void) | null = null;
  constructor(
    private host: LocalHost,
    private conn: number,
  ) {}
  send(channel: ChannelId, data: Uint8Array) {
    this.host.deliverToServer(this.conn, channel, data.slice());
  }
  close() {
    this.host.drop(this.conn);
  }
}
