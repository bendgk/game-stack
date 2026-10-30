// React Three Fiber primitives on three.js r187's WebGPURenderer (falls back to WebGL2
// automatically when WebGPU is unavailable — e.g. over plain http on a LAN IP).
//
//   <GameCanvas>                    WebGPU canvas
//     <Controls build={…}>          keyboard + mouse-look + gamepad → your Rust `Input`
//       <SessionDriver />           steps prediction + network once per frame
//       <Entities renderers={…} />  one React component per entity kind (from Rust `Kind`)
//       <FollowCamera />            third-person camera on the FLAG_ME entity
//
// Transforms are written straight into three.js objects in useFrame — React only re-renders
// when entities spawn or despawn.

import { Canvas, extend, useFrame, useThree, type ThreeToJSXElements } from '@react-three/fiber';
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentProps,
  type ComponentType,
  type ReactNode,
} from 'react';
import * as THREE from 'three/webgpu';
import { FLAG_HIDDEN, type EntityView } from './entities.ts';
import { Keyboard, MouseLook, readGamepad } from './input.ts';
import { useEntities, useSession } from './react.tsx';
import type { Input, Kind } from './wasm.ts';

// Make three/webgpu classes (node materials etc.) available as JSX elements. `pMREMGenerator`
// is declared differently by the WebGL typings, so it is left out.
declare module '@react-three/fiber' {
  interface ThreeElements extends Omit<ThreeToJSXElements<typeof THREE>, 'pMREMGenerator'> {}
}
extend(THREE as unknown as Parameters<typeof extend>[0]);

export { THREE };

type CanvasProps = Omit<ComponentProps<typeof Canvas>, 'gl'>;

export function GameCanvas({ children, ...props }: CanvasProps & { children: ReactNode }) {
  return (
    <Canvas
      shadows
      dpr={[1, 2]}
      camera={{ fov: 60, near: 0.1, far: 500, position: [0, 10, 14] }}
      {...props}
      gl={async (glProps) => {
        const renderer = new THREE.WebGPURenderer({ ...(glProps as object), antialias: true });
        await renderer.init();
        return renderer;
      }}
    >
      {children}
    </Canvas>
  );
}

/** Steps the session (prediction, network, render buffer) every frame, before rendering. */
export function SessionDriver() {
  const session = useSession();
  useFrame(() => session.frame(performance.now()), -10);
  return null;
}

// --- entities --------------------------------------------------------------------------------

export interface EntityProps {
  entity: EntityView;
}
export type EntityComponent = ComponentType<EntityProps>;

/**
 * Renders every entity with the component registered for its kind. The map is *total*
 * over the Rust `Kind` enum: add a kind in Rust and TypeScript tells you to draw it here
 * (use `null` to draw nothing).
 */
export function Entities({ renderers, applyScale = true }: { renderers: Record<Kind, EntityComponent | null>; applyScale?: boolean }) {
  const session = useSession();
  const list = useEntities();
  return (
    <>
      {list.map((e) => {
        const C = renderers[session.kinds[e.kind] as Kind];
        return C ? <EntityNode key={e.id} entity={e} component={C} applyScale={applyScale} /> : null;
      })}
    </>
  );
}

function EntityNode({ entity, component: C, applyScale }: { entity: EntityView; component: EntityComponent; applyScale: boolean }) {
  const ref = useRef<THREE.Group>(null);
  useFrame(() => {
    const g = ref.current;
    if (!g) return;
    g.position.set(entity.pos[0], entity.pos[1], entity.pos[2]);
    g.quaternion.set(entity.rot[0], entity.rot[1], entity.rot[2], entity.rot[3]);
    if (applyScale) g.scale.set(entity.scale[0], entity.scale[1], entity.scale[2]);
    g.visible = (entity.flags & FLAG_HIDDEN) === 0;
  }, -5);
  return (
    <group ref={ref}>
      <C entity={entity} />
    </group>
  );
}

/** Run `fn` every frame with this entity (animate children from `a`/`b`, flags…). */
export function useEntityFrame(entity: EntityView, fn: (e: EntityView, dt: number) => void) {
  const ref = useRef(fn);
  ref.current = fn;
  useFrame((_, dt) => ref.current(entity, dt));
}

/** 0xRRGGBB → THREE.Color, memoised per value. */
export function useColor(hex: number) {
  return useMemo(() => new THREE.Color(hex), [hex]);
}

// --- input -----------------------------------------------------------------------------------

export interface Devices {
  keyboard: Keyboard;
  look: MouseLook;
  gamepad: ReturnType<typeof readGamepad>;
}

const ControlsCtx = createContext<Devices | null>(null);

/** Builds the Rust `Input` from devices each frame. `build` is your game's input mapping. */
export function Controls({ build, children }: { build: (d: Devices, prev: Input) => Input; children?: ReactNode }) {
  const session = useSession();
  const dom = useThree((s) => s.gl.domElement);
  const [devices, setDevices] = useState<Devices | null>(null);
  useEffect(() => {
    const d: Devices = { keyboard: new Keyboard(), look: new MouseLook(dom), gamepad: null };
    setDevices(d);
    return () => {
      d.keyboard.dispose();
      d.look.dispose();
    };
  }, [dom]);
  useFrame(() => {
    if (!devices) return;
    devices.gamepad = readGamepad();
    session.input = build(devices, session.input);
    devices.keyboard.endFrame();
  }, -20);
  return <ControlsCtx.Provider value={devices}>{devices ? children : null}</ControlsCtx.Provider>;
}

export function useDevices(): Devices {
  const d = useContext(ControlsCtx);
  if (!d) throw new Error('useDevices() outside <Controls>');
  return d;
}

// --- camera ----------------------------------------------------------------------------------

/** Third-person orbit camera around the local player, steered by mouse-look yaw/pitch. */
export function FollowCamera({ distance = 7.5, height = 1.0, stiffness = 12 }: { distance?: number; height?: number; stiffness?: number }) {
  const session = useSession();
  const { look } = useDevices();
  const camera = useThree((s) => s.camera);
  const target = useMemo(() => new THREE.Vector3(), []);
  const desired = useMemo(() => new THREE.Vector3(), []);
  useFrame((_, dt) => {
    const me = session.entities.me;
    if (me) target.lerp(desired.set(me.pos[0], me.pos[1] + height, me.pos[2]), 1 - Math.exp(-stiffness * 2 * dt));
    const cp = Math.cos(look.pitch);
    desired.set(
      target.x + Math.sin(look.yaw) * cp * distance,
      target.y - Math.sin(look.pitch) * distance,
      target.z + Math.cos(look.yaw) * cp * distance,
    );
    camera.position.lerp(desired, 1 - Math.exp(-stiffness * dt));
    camera.lookAt(target);
  }, -1);
  return null;
}
