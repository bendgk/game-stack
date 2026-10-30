import { NetStatsOverlay, useSession, useStatus } from '@gs/engine';
import { Controls, Entities, FollowCamera, GameCanvas, SessionDriver } from '@gs/engine/three';
import { useEffect, useState } from 'react';
import { Fx } from './Fx.tsx';
import { Hud } from './Hud.tsx';
import { buildInput } from './input.ts';
import { renderers } from './renderers.tsx';

function Scene() {
  return (
    <>
      <color attach="background" args={['#0f1324']} />
      <fog attach="fog" args={['#0f1324', 30, 70]} />
      <hemisphereLight args={['#bcd4ff', '#20243a', 0.9]} />
      <directionalLight
        castShadow
        position={[12, 20, 8]}
        intensity={2.2}
        shadow-mapSize={[2048, 2048]}
        shadow-camera-left={-20}
        shadow-camera-right={20}
        shadow-camera-top={20}
        shadow-camera-bottom={-20}
      />
      <mesh rotation-x={-Math.PI / 2} receiveShadow>
        <planeGeometry args={[120, 120]} />
        <meshStandardMaterial color="#1b2140" roughness={0.95} />
      </mesh>
      <gridHelper args={[30, 30, '#3a4680', '#262d52']} position-y={0.01} />
    </>
  );
}

export function GameView({ onLeave }: { onLeave: () => void }) {
  const session = useSession();
  const { status, error } = useStatus();
  const [stats, setStats] = useState(true);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code >= 'Digit1' && e.code <= 'Digit4') session.command({ type: 'Emote', emote: Number(e.code.slice(5)) });
      if (e.code === 'F3') {
        e.preventDefault();
        setStats((s) => !s);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [session]);

  return (
    <div className="game">
      <GameCanvas>
        <Scene />
        <Controls build={buildInput}>
          <SessionDriver />
          <Entities renderers={renderers} />
          <FollowCamera />
          <Fx />
        </Controls>
      </GameCanvas>
      <Hud />
      {stats && <NetStatsOverlay />}
      <button className="leave" onClick={onLeave}>
        Leave
      </button>
      {status !== 'playing' && (
        <div className="overlay">
          <div className="card">
            <h2>{status === 'error' ? 'Cannot join' : status === 'reconnecting' ? 'Reconnecting…' : 'Connecting…'}</h2>
            {error && <p className="dim">{error}</p>}
            {status === 'error' && <button onClick={onLeave}>Back to lobby</button>}
          </div>
        </div>
      )}
    </div>
  );
}
