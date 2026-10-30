// One-shot effects driven by Rust game events. Events are rollback-safe: each fires once per
// tick even when prediction re-simulates, so effects never double up.

import { useGameEvent, useSession } from '@gs/engine';
import { THREE } from '@gs/engine/three';
import { useFrame } from '@react-three/fiber';
import { useRef, useState } from 'react';
import { sfx } from './sfx.ts';

interface Burst {
  id: number;
  pos: [number, number, number];
  color: number;
  born: number;
}

const LIFE = 0.6;
const DIRS = Array.from({ length: 10 }, (_, i) => {
  const a = (i / 10) * Math.PI * 2;
  return new THREE.Vector3(Math.cos(a), 0.8 + (i % 3) * 0.4, Math.sin(a)).normalize();
});

function BurstMesh({ b }: { b: Burst }) {
  const group = useRef<THREE.Group>(null);
  useFrame(() => {
    const t = (performance.now() - b.born) / 1000 / LIFE;
    const g = group.current;
    if (!g) return;
    g.children.forEach((c, i) => {
      c.position.copy(DIRS[i]!).multiplyScalar(t * 2.2);
      c.position.y -= t * t * 1.5;
      c.scale.setScalar(Math.max(0, 1 - t));
    });
  });
  return (
    <group ref={group} position={b.pos}>
      {DIRS.map((_, i) => (
        <mesh key={i}>
          <octahedronGeometry args={[0.09]} />
          <meshBasicMaterial color={b.color} />
        </mesh>
      ))}
    </group>
  );
}

export function Fx() {
  const session = useSession();
  const [bursts, setBursts] = useState<Burst[]>([]);
  const next = useRef(0);
  const add = (pos: [number, number, number], color: number) => {
    const born = performance.now();
    setBursts((bs) => [...bs.filter((b) => born - b.born < LIFE * 1000), { id: next.current++, pos, color, born }]);
  };
  useGameEvent('Orb', (e) => {
    add([e.x, e.y, e.z], 0xffd166);
    if (e.player === session.me) sfx.orb();
  });
  useGameEvent('Bump', (e) => {
    add([e.x, e.y, e.z], 0xffffff);
    sfx.bump(Math.min(1, e.strength / 20));
  });
  useGameEvent('Jump', (e) => e.player === session.me && sfx.jump());
  useGameEvent('Dash', (e) => e.player === session.me && sfx.dash());
  useGameEvent('RoundOver', () => sfx.fanfare());
  return (
    <>
      {bursts.map((b) => (
        <BurstMesh key={b.id} b={b} />
      ))}
    </>
  );
}
