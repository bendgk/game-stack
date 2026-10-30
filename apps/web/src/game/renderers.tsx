// How each Rust `Kind` looks. Rust decides where things are (position, rotation, scale and the
// a/b channels); these components only decide what they look like. The `renderers` map is
// total over `Kind`, so adding a kind in Rust is a type error here until you draw it.

import type { Kind } from '@gs/engine';
import { Sprite, THREE, useColor, useEntityFrame, type EntityComponent, type EntityProps } from '@gs/engine/three';
import { Suspense, useRef, useState } from 'react';

/** Emote bubbles: drawn with the gs2d MCP (assets/2d/emotes.gs2d.json → public/sprites/emotes.*). */
function EmoteBubble({ entity }: EntityProps) {
  const [id, setId] = useState(0);
  const group = useRef<THREE.Group>(null);
  useEntityFrame(entity, (e) => {
    const next = Math.round(e.b); // b = emote id (0 = none), set in Rust `render`
    if (next !== id) setId(next);
    if (group.current) group.current.position.y = 1.15 + Math.sin(performance.now() / 150) * 0.06;
  });
  if (!id) return null;
  return (
    <group ref={group}>
      <Suspense fallback={null}>
        <Sprite sheet="/sprites/emotes.json" frame={`emote/${(id - 1) % 4}`} billboard />
      </Suspense>
    </group>
  );
}

function Player({ entity }: EntityProps) {
  const color = useColor(entity.color);
  const ring = useRef<THREE.Mesh>(null);
  useEntityFrame(entity, (e) => {
    // a = dash charge (0..1)
    if (ring.current) {
      ring.current.scale.setScalar(0.6 + 0.4 * e.a);
      (ring.current.material as THREE.MeshBasicMaterial).opacity = e.a >= 1 ? 0.85 : 0.25;
    }
  });
  return (
    <>
      <mesh castShadow>
        <sphereGeometry args={[0.5, 32, 16]} />
        <meshStandardMaterial color={color} roughness={0.35} metalness={0.1} />
      </mesh>
      {/* eyes face -Z, the sim's forward */}
      <mesh position={[-0.17, 0.12, -0.42]}>
        <sphereGeometry args={[0.09, 12, 8]} />
        <meshStandardMaterial color="white" />
      </mesh>
      <mesh position={[0.17, 0.12, -0.42]}>
        <sphereGeometry args={[0.09, 12, 8]} />
        <meshStandardMaterial color="white" />
      </mesh>
      <mesh ref={ring} rotation-x={-Math.PI / 2} position-y={-0.48}>
        <ringGeometry args={[0.55, 0.7, 32]} />
        <meshBasicMaterial color={color} transparent opacity={0.5} />
      </mesh>
      <EmoteBubble entity={entity} />
    </>
  );
}

function Orb({ entity }: EntityProps) {
  const color = useColor(entity.color);
  return (
    <mesh castShadow>
      <icosahedronGeometry args={[0.38, 1]} />
      <meshStandardMaterial color={color} emissive={color} emissiveIntensity={1.4} roughness={0.2} />
    </mesh>
  );
}

/** Unit cube; the entity's scale (= Rust box size) is applied by <Entities>. */
function Block({ entity }: EntityProps) {
  const color = useColor(entity.color);
  return (
    <mesh castShadow receiveShadow>
      <boxGeometry args={[1, 1, 1]} />
      <meshStandardMaterial color={color} roughness={0.8} />
    </mesh>
  );
}

export const renderers: Record<Kind, EntityComponent | null> = {
  Player,
  Orb,
  Crate: Block,
  Wall: Block,
  Pad: Block,
};
