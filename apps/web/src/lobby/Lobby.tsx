import type { NetSim, SessionOptions } from '@gs/engine';
import { useEffect, useState } from 'react';

interface Health {
  ok: boolean;
  fingerprint: string;
  store: string;
  db: boolean;
  udp: string;
  peers: number;
}
interface RoomInfo {
  name: string;
  mode: string;
  players: string[];
  connected: number;
}
interface LeaderRow {
  name: string;
  matches: number;
  wins: number;
  total_score: number;
}

const NETSIMS: Record<string, NetSim> = {
  perfect: { latencyMs: 0, jitterMs: 0, loss: 0 },
  lan: { latencyMs: 2, jitterMs: 1, loss: 0 },
  wifi: { latencyMs: 15, jitterMs: 10, loss: 0.01 },
  awful: { latencyMs: 80, jitterMs: 40, loss: 0.08 },
};

function useJson<T>(url: string, every = 3000): T | null {
  const [v, setV] = useState<T | null>(null);
  useEffect(() => {
    let live = true;
    const load = () =>
      fetch(url)
        .then((r) => (r.ok ? r.json() : null))
        .then((j) => live && setV(j))
        .catch(() => live && setV(null));
    void load();
    const t = setInterval(load, every);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [url, every]);
  return v;
}

const saved = (k: string, d: string) => localStorageGet(`gs:${k}`) ?? d;
function localStorageGet(k: string) {
  try {
    return localStorage.getItem(k);
  } catch {
    return null;
  }
}
function localStorageSet(k: string, v: string) {
  try {
    localStorage.setItem(k, v);
  } catch {
    /* private mode */
  }
}

export function Lobby({ onPlay }: { onPlay: (o: SessionOptions) => void }) {
  const health = useJson<Health>('/api/health');
  const rooms = useJson<RoomInfo[]>('/api/rooms');
  const leaders = useJson<LeaderRow[]>('/api/leaderboard', 10000);
  const [name, setName] = useState(() => saved('name', `player${Math.floor(Math.random() * 900 + 100)}`));
  const [room, setRoom] = useState(() => saved('room', 'lobby'));
  const [mode, setMode] = useState(() => saved('mode', 'versus'));
  const [bots, setBots] = useState(() => Number(saved('bots', '2')));
  const [netsim, setNetsim] = useState('perfect');

  const play = (offline: boolean) => {
    localStorageSet('gs:name', name);
    localStorageSet('gs:room', room);
    localStorageSet('gs:mode', mode);
    localStorageSet('gs:bots', String(bots));
    onPlay({ name, room, mode, bots, offline, netsim: offline ? NETSIMS[netsim] : undefined });
  };

  return (
    <div className="lobby">
      <header>
        <h1>Orb Arena</h1>
        <p className="sub">a game-stack starter · server-authoritative · predicted · WebRTC over your LAN</p>
      </header>
      <section className="card">
        <label>
          Name <input value={name} maxLength={24} onChange={(e) => setName(e.target.value)} />
        </label>
        <label>
          Room <input value={room} maxLength={32} onChange={(e) => setRoom(e.target.value)} />
        </label>
        <label>
          Mode
          <select value={mode} onChange={(e) => setMode(e.target.value)}>
            <option value="versus">versus — most orbs wins</option>
            <option value="coop">co-op — fill the team goal</option>
          </select>
        </label>
        <label>
          Bots (new rooms) <input type="number" min={0} max={7} value={bots} onChange={(e) => setBots(Math.max(0, Math.min(7, Number(e.target.value))))} />
        </label>
        <div className="row">
          <button className="primary" disabled={!health?.ok} onClick={() => play(false)}>
            Play on server
          </button>
          <button onClick={() => play(true)}>Practice offline</button>
          <select value={netsim} onChange={(e) => setNetsim(e.target.value)} title="Offline only: simulated network">
            {Object.keys(NETSIMS).map((k) => (
              <option key={k} value={k}>
                net: {k}
              </option>
            ))}
          </select>
        </div>
        <p className="hint">
          {health?.ok
            ? `server online · UDP ${health.udp} · store ${health.store} · db ${health.db ? 'on' : 'off'} · build ${health.fingerprint.slice(0, 8)}`
            : 'server offline — start it with `pnpm dev` (offline practice still works)'}
        </p>
      </section>
      <div className="cols">
        <section className="card">
          <h2>Rooms</h2>
          {rooms?.length ? (
            <ul className="list">
              {rooms.map((r) => (
                <li key={r.name}>
                  <button className="link" onClick={() => setRoom(r.name)}>
                    {r.name}
                  </button>{' '}
                  <span className="dim">
                    {r.mode} · {r.players.length} in room · {r.connected} connected
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="dim">No rooms yet — yours will be created when you join.</p>
          )}
        </section>
        <section className="card">
          <h2>Leaderboard</h2>
          {leaders?.length ? (
            <ol className="list">
              {leaders.map((l) => (
                <li key={l.name}>
                  <b>{l.name}</b> <span className="dim">{l.total_score} orbs · {l.wins} wins · {l.matches} rounds</span>
                </li>
              ))}
            </ol>
          ) : (
            <p className="dim">Finish a round to get on the board (needs Postgres).</p>
          )}
        </section>
      </div>
      <footer className="dim">WASD move · mouse look (click to lock) · Space jump · Shift / click dash · 1–4 emote · Esc release mouse</footer>
    </div>
  );
}
