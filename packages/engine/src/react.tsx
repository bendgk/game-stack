// React bindings. Components read session data through these hooks; nothing re-renders at
// frame rate (transforms flow through refs in ./three.tsx).

import { createContext, useContext, useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import type { EntityView } from './entities.ts';
import { GameSession, type SessionOptions } from './session.ts';
import type { Event, Hud, NetStats } from './wasm.ts';

const Ctx = createContext<GameSession | null>(null);

export function GameProvider({ session, children }: { session: GameSession; children: ReactNode }) {
  return <Ctx.Provider value={session}>{children}</Ctx.Provider>;
}

export function useSession(): GameSession {
  const s = useContext(Ctx);
  if (!s) throw new Error('useSession() outside <GameProvider>');
  return s;
}

/** Create (and dispose) a session for the given options; `null` while loading. */
export function useCreateSession(opts: SessionOptions | null): GameSession | null {
  const [session, setSession] = useState<GameSession | null>(null);
  const key = opts ? JSON.stringify(opts) : null;
  useEffect(() => {
    if (!opts) return;
    let live = true;
    let s: GameSession | null = null;
    void GameSession.create(opts).then((created) => {
      if (live) setSession((s = created));
      else created.dispose();
    });
    return () => {
      live = false;
      s?.dispose();
      setSession(null);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return session;
}

function useSessionValue<T>(read: (s: GameSession) => T): T {
  const s = useSession();
  useSyncExternalStore(s.subscribe, s.getVersion);
  return read(s);
}

/** The game's `Hud` (generated type), refreshed ~15×/s. */
export function useHud(): Hud | null {
  return useSessionValue((s) => s.hud);
}

export function useNetStats(): NetStats | null {
  return useSessionValue((s) => s.stats);
}

export function useStatus() {
  return useSessionValue((s) => ({ status: s.status, error: s.error, offline: s.offline }));
}

export function useNotices() {
  return useSessionValue((s) => s.notices);
}

/** Entities currently alive (re-renders only on spawn / despawn). */
export function useEntities(): readonly EntityView[] {
  const s = useSession();
  return useSyncExternalStore(s.entities.subscribe, s.entities.getSnapshot);
}

/** Run `fn` for every game event of `type` (once per tick, rollback-safe). */
export function useGameEvent<T extends Event['type']>(type: T, fn: (e: Extract<Event, { type: T }>, tick: number) => void) {
  const s = useSession();
  const ref = useRef(fn);
  ref.current = fn;
  useEffect(() => s.on(type, (e, t) => ref.current(e, t)), [s, type]);
}
