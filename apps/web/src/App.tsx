import { GameProvider, useCreateSession, type SessionOptions } from '@gs/engine';
import { useState } from 'react';
import { GameView } from './game/GameView.tsx';
import { Lobby } from './lobby/Lobby.tsx';

export function App() {
  const [opts, setOpts] = useState<SessionOptions | null>(null);
  const session = useCreateSession(opts);
  if (!opts) return <Lobby onPlay={setOpts} />;
  if (!session) return <div className="center">Loading engine…</div>;
  // Console handle for debugging and the e2e smoke test: `__gs.stats`, `__gs.hud`, `__gs.entities`.
  (window as unknown as { __gs: unknown }).__gs = session;
  return (
    <GameProvider session={session}>
      <GameView onLeave={() => setOpts(null)} />
    </GameProvider>
  );
}
