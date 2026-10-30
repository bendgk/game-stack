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
  use,
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

// --- 2D --------------------------------------------------------------------------------------

/**
 * Orthographic camera for 2D games (`<GameCanvas orthographic>`): looks down -Z at the XY
 * plane and follows the local player. `zoom` = screen pixels per world unit.
 */
export function Camera2D({ zoom = 48, follow = true, stiffness = 10 }: { zoom?: number; follow?: boolean; stiffness?: number }) {
  const session = useSession();
  const camera = useThree((s) => s.camera);
  useEffect(() => {
    camera.position.set(0, 0, 100);
    camera.lookAt(0, 0, 0);
    camera.zoom = zoom;
    camera.updateProjectionMatrix();
  }, [camera, zoom]);
  useFrame((_, dt) => {
    const me = session.entities.me;
    if (!follow || !me) return;
    const k = 1 - Math.exp(-stiffness * dt);
    camera.position.x += (me.pos[0] - camera.position.x) * k;
    camera.position.y += (me.pos[1] - camera.position.y) * k;
  }, -1);
  return null;
}

/** A sprite sheet as written by the gs2d MCP (`export_sheet`): one PNG + this JSON. */
export interface SpriteSheet {
  image: string;
  size: [number, number];
  /** Pixels per world unit the art was drawn at. */
  ppu: number;
  frames: Record<string, { x: number; y: number; w: number; h: number; pivot: [number, number] }>;
  animations: Record<string, { frames: string[]; fps: number; loop: boolean }>;
}

const sheets = new Map<string, Promise<{ sheet: SpriteSheet; texture: THREE.Texture }>>();
const tmpQ = new THREE.Quaternion();

/** Load (once, cached) a sprite sheet JSON and its texture. Suspends while loading. */
export function loadSpriteSheet(url: string) {
  let p = sheets.get(url);
  if (!p) {
    p = (async () => {
      const sheet = (await (await fetch(url)).json()) as SpriteSheet;
      const imageUrl = new URL(sheet.image, new URL(url, location.href)).href;
      const texture = await new THREE.TextureLoader().loadAsync(imageUrl);
      texture.magFilter = THREE.NearestFilter; // crisp pixel art
      texture.minFilter = THREE.NearestFilter;
      texture.colorSpace = THREE.SRGBColorSpace;
      texture.generateMipmaps = false;
      return { sheet, texture };
    })();
    sheets.set(url, p);
  }
  return p;
}

export function useSpriteSheet(url: string) {
  return use(loadSpriteSheet(url));
}

/**
 * Draw one frame or an animation from a sprite sheet on a quad facing +Z.
 * `size` (world units) defaults to the frame's pixel size / `ppu`; pass `size={[1, 1]}` when
 * the entity's scale already carries the size (`RenderEntity::size2d`). Wrap in <Suspense>.
 */
export function Sprite({
  sheet: url,
  frame,
  anim,
  size,
  flipX = false,
  tint = 0xffffff,
  billboard = false,
}: {
  sheet: string;
  frame?: string;
  anim?: string;
  size?: [number, number];
  flipX?: boolean;
  tint?: number;
  /** Always face the camera (sprites in a 3D scene). */
  billboard?: boolean;
}) {
  const { sheet, texture } = useSpriteSheet(url);
  const tex = useMemo(() => {
    const t = texture.clone();
    t.needsUpdate = true;
    return t;
  }, [texture]);
  const mesh = useRef<THREE.Mesh>(null);
  const camera = useThree((s) => s.camera);
  const first = anim ? sheet.animations[anim]?.frames[0] : frame;
  const f0 = first ? sheet.frames[first] : undefined;
  const [w, h] = size ?? (f0 ? [f0.w / sheet.ppu, f0.h / sheet.ppu] : [1, 1]);
  const start = useMemo(() => performance.now(), []);
  useFrame(() => {
    let name = frame;
    const a = anim ? sheet.animations[anim] : undefined;
    if (a && a.frames.length) {
      const i = Math.floor(((performance.now() - start) / 1000) * a.fps);
      name = a.frames[a.loop ? i % a.frames.length : Math.min(i, a.frames.length - 1)];
    }
    const f = name ? sheet.frames[name] : undefined;
    if (!f) return;
    const [sw, sh] = sheet.size;
    tex.repeat.set(((flipX ? -1 : 1) * f.w) / sw, f.h / sh);
    tex.offset.set((flipX ? f.x + f.w : f.x) / sw, 1 - (f.y + f.h) / sh);
    if (mesh.current) {
      mesh.current.position.set((0.5 - f.pivot[0]) * w, (f.pivot[1] - 0.5) * h, 0);
      if (billboard) {
        // Face the camera in world space, whatever the parent's rotation.
        const parent = mesh.current.parent;
        if (parent) mesh.current.quaternion.copy(parent.getWorldQuaternion(tmpQ).invert().multiply(camera.quaternion));
        else mesh.current.quaternion.copy(camera.quaternion);
      }
    }
  });
  return (
    <mesh ref={mesh}>
      <planeGeometry args={[w, h]} />
      <meshBasicMaterial map={tex} color={tint} transparent alphaTest={0.01} toneMapped={false} />
    </mesh>
  );
}
