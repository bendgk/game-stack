/**
 * gs3d MCP server core (game-stack 3D asset studio): MCP (JSON-RPC 2.0, newline-delimited over stdio) in front of a
 * long-lived headless Blender (`blender/bridge.py`) reached over a localhost socket.
 *
 * No dependencies: the MCP surface used here is small (initialize, tools/list,
 * tools/call, ping) and the bridge protocol is one JSON object per line.
 */

import { spawn } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
export const PROTOCOL_VERSION = '2025-06-18';

export function blenderPath(env = process.env) {
  if (env.GS3D_BLENDER) return env.GS3D_BLENDER;
  const mac = '/Applications/Blender.app/Contents/MacOS/Blender';
  if (existsSync(mac)) return mac;
  if (process.platform === 'win32') {
    // The official installer puts it in "C:\\Program Files\\Blender Foundation\\Blender <ver>\\".
    const base = join(env.ProgramFiles ?? 'C:\\Program Files', 'Blender Foundation');
    try {
      const dirs = readdirSync(base)
        .filter((d) => d.startsWith('Blender'))
        .sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
      for (const d of dirs) if (existsSync(join(base, d, 'blender.exe'))) return join(base, d, 'blender.exe');
    } catch {}
  }
  return 'blender';
}

/** A headless Blender running bridge.py, (re)started on demand. */
export class BlenderBridge {
  constructor({ bin = blenderPath(), log = () => {} } = {}) {
    this.bin = bin;
    this.log = log;
    this.proc = null;
    this.sock = null;
    this.pending = new Map();
    this.nextId = 1;
    this.buf = '';
    this.starting = null;
  }

  start() {
    if (this.sock) return Promise.resolve();
    if (this.starting) return this.starting;
    this.starting = new Promise((resolve, reject) => {
      const proc = spawn(
        this.bin,
        [
          '-b',
          '--factory-startup',
          '--python',
          join(HERE, 'blender', 'bridge.py'),
          '--',
          '--port',
          '0',
        ],
        { stdio: ['ignore', 'pipe', 'pipe'] },
      );
      this.proc = proc;
      let seen = '';
      const onOut = (chunk) => {
        seen += chunk.toString();
        const m = /GS3D_PORT=(\d+)/.exec(seen);
        if (!m) return;
        proc.stdout.off('data', onOut);
        proc.stdout.on('data', (c) => this.log(c.toString()));
        const sock = net.connect(Number(m[1]), '127.0.0.1', () => {
          this.sock = sock;
          this.starting = null;
          resolve();
        });
        sock.on('data', (c) => this.onData(c));
        sock.on('error', (e) => this.fail(e));
        sock.on('close', () => this.fail(new Error('blender bridge closed')));
      };
      proc.stdout.on('data', onOut);
      proc.stderr.on('data', (c) => this.log(c.toString()));
      proc.on('error', (e) => {
        this.starting = null;
        reject(new Error(`cannot start Blender (${this.bin}): ${e.message}`));
      });
      proc.on('exit', (code) => {
        this.starting = null;
        this.fail(new Error(`blender exited (${code})`));
        reject(new Error(`blender exited before the bridge was up (${code})`));
      });
    });
    return this.starting;
  }

  onData(chunk) {
    this.buf += chunk.toString();
    let nl;
    while ((nl = this.buf.indexOf('\n')) >= 0) {
      const line = this.buf.slice(0, nl);
      this.buf = this.buf.slice(nl + 1);
      if (!line.trim()) continue;
      let msg;
      try {
        msg = JSON.parse(line);
      } catch {
        continue;
      }
      const p = this.pending.get(msg.id);
      if (!p) continue;
      this.pending.delete(msg.id);
      if (msg.ok) p.resolve(msg.result);
      else p.reject(Object.assign(new Error(msg.error), { trace: msg.trace }));
    }
  }

  fail(err) {
    for (const p of this.pending.values()) p.reject(err);
    this.pending.clear();
    if (this.sock) this.sock.destroy();
    this.sock = null;
    if (this.proc && this.proc.exitCode === null) this.proc.kill();
    this.proc = null;
  }

  async call(op, args = {}) {
    await this.start();
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.sock.write(JSON.stringify({ id, op, args }) + '\n');
    });
  }

  /** Drop the running Blender so the next call starts fresh (e.g. after ruler.json changed). */
  restart() {
    if (this.sock) this.stop();
  }

  stop() {
    if (this.sock) this.sock.write(JSON.stringify({ id: 0, op: 'quit', args: {} }) + '\n');
    this.fail(new Error('stopped'));
  }
}

export const RULER_PATH = process.env.GS3D_RULER ?? join(HERE, 'ruler.json');
export const RULER_KEYS = [
  'avatar_radius',
  'avatar_height',
  'camera_fov_deg',
  'camera_dist_min',
  'camera_dist',
  'camera_dist_max',
  'camera_pitch_deg',
  'camera_pitch_far_deg',
  'tile_m',
  'step_up',
];

/** Merge `args` into ruler.json (only known numeric keys); returns the new config. */
export function writeRuler(args, { read = readFileSync, write = writeFileSync, path = RULER_PATH } = {}) {
  let cfg = {};
  try {
    cfg = JSON.parse(read(path, 'utf8'));
  } catch {}
  for (const k of RULER_KEYS) {
    if (typeof args?.[k] === 'number' && Number.isFinite(args[k]) && args[k] > 0) cfg[k] = args[k];
  }
  write(path, JSON.stringify(cfg, null, 2) + '\n');
  return cfg;
}

const NAMES = {
  type: 'array',
  items: { type: 'string' },
  description: 'Object names to act on (default: every mesh in the scene).',
};

/** The tool table: name → { description, inputSchema, op, image? }. */
export const TOOLS = {
  configure: {
    description:
      'Set the ruler for YOUR game (writes tools/mcp-3d/ruler.json and restarts Blender): avatar_radius/avatar_height (metres, your player), camera_fov_deg, camera_dist(_min/_max), camera_pitch_deg (degrees down), camera_pitch_far_deg, tile_m, step_up. Every probe, proportion band and the `game` render view use it. Call with no args to read the current values.',
    inputSchema: {
      type: 'object',
      properties: Object.fromEntries(RULER_KEYS.map((k) => [k, { type: 'number' }])),
    },
    local: 'configure',
  },
  blender_status: {
    description:
      'Blender version, object count, biome and the output directory. Starts Blender if needed.',
    inputSchema: { type: 'object', properties: {} },
    op: 'ping',
  },
  scene_reset: {
    description:
      'Empty the scene and set up the toon look (cel ramp, ink, sky, sun) for a mood/biome: meadow | pine | desert | jungle.',
    inputSchema: { type: 'object', properties: { biome: { type: 'string' } } },
    op: 'reset',
  },
  run_python: {
    description:
      'Run bpy Python in the live scene with `from gs3d import *` preloaded (geo, kit, look, probe, rig, ruler, export, Model, to_object). Assign `result` to return JSON. Build with Model parts + to_object so the probes see the parts.',
    inputSchema: {
      type: 'object',
      properties: { code: { type: 'string' } },
      required: ['code'],
    },
    op: 'exec',
  },
  scene_outline: {
    description: 'Every object: name, type, gs tag and location.',
    inputSchema: { type: 'object', properties: {} },
    op: 'outline',
  },
  measure: {
    description:
      'Measure models: size in metres, height in avatar heights (1 ah = the player height from ruler.json), ground gap, parts, triangles, tags.',
    inputSchema: { type: 'object', properties: { names: NAMES } },
    op: 'measure',
  },
  lint: {
    description:
      'Run the geometry probes and return fix / warn / info notes with literal deltas: hygiene, hair, ground, support, burial (hidden inside other solids), z-fight, proportion (avatar ruler bands), sightline (signs/doors/emblems from the real play camera at 8 yaws), walk (street hidden from the default camera), clearance (intersections, dead alleys), budget.',
    inputSchema: {
      type: 'object',
      properties: {
        names: NAMES,
        probes: { type: 'array', items: { type: 'string' } },
        limit: { type: 'number' },
        measures: {
          type: 'boolean',
          description: 'Include per-object measures (heights, sightline shares).',
        },
      },
    },
    op: 'lint',
  },
  gate: {
    description:
      'The terminal check: the same reads as lint; pass only with zero fix notes. Run before export.',
    inputSchema: { type: 'object', properties: { names: NAMES } },
    op: 'gate',
  },
  snap_to_ground: {
    description:
      'Ground snap (from the old world harness): move models floating above or sunk below their ground by a small slip onto it; anything off by more than max_gap (default 3 m) is reported, not silently fixed.',
    inputSchema: { type: 'object', properties: { names: NAMES, max_gap: { type: 'number' } } },
    op: 'snap',
  },
  render: {
    description:
      'Render one view in the game look and return the image. view: game (the play camera from ruler.json; yaw 0 = north-up) | hero | aerial | top | front | side.',
    inputSchema: {
      type: 'object',
      properties: {
        view: { type: 'string' },
        names: NAMES,
        target: { type: 'array', items: { type: 'number' } },
        radius: { type: 'number' },
        yaw: { type: 'number' },
        pitch: { type: 'number' },
        dist: { type: 'number' },
        res: { type: 'array', items: { type: 'number' } },
        path: { type: 'string' },
      },
    },
    op: 'render',
    image: true,
  },
  contact_sheet: {
    description: 'Render several views (default game, hero, front, top) tiled into one image.',
    inputSchema: {
      type: 'object',
      properties: {
        names: NAMES,
        views: { type: 'array', items: { type: 'string' } },
        target: { type: 'array', items: { type: 'number' } },
        radius: { type: 'number' },
        yaw: { type: 'number' },
        res: { type: 'array', items: { type: 'number' } },
        path: { type: 'string' },
      },
    },
    op: 'sheet',
    image: true,
  },
  export_glb: {
    description:
      'Export models as one GLB: vertex-painted (COLOR_0), +Y up, no ink hull (the client draws its own).',
    inputSchema: {
      type: 'object',
      properties: { path: { type: 'string' }, names: NAMES, draco: { type: 'boolean' } },
      required: ['path'],
    },
    op: 'export',
  },
  save_blend: {
    description: 'Save the scene as a .blend.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' } }, required: ['path'] },
    op: 'save',
  },
  open_blend: {
    description: 'Open a .blend into the live session.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' } }, required: ['path'] },
    op: 'open',
  },
  rig: {
    description:
      'Rig a mesh: kind biped (skeleton placed from measured landmarks: hips, neck, shoulders, reach, stance) or chain (bones through `points`), skinned by heat weights, with a report (unweighted verts, >4 influences, bones outside the volume).',
    inputSchema: {
      type: 'object',
      properties: {
        name: { type: 'string' },
        kind: { type: 'string' },
        points: { type: 'array', items: { type: 'array', items: { type: 'number' } } },
      },
      required: ['name'],
    },
    op: 'rig',
  },
  rig_report: {
    description:
      'Check a skinned mesh: unweighted verts, >4 influences, bones whose heads sit outside the mesh.',
    inputSchema: { type: 'object', properties: { name: { type: 'string' } }, required: ['name'] },
    op: 'rig_report',
  },
};

function text(value) {
  return { type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value, null, 1) };
}

/** Handle one JSON-RPC message; returns the response object (or null for notifications). */
export async function handle(msg, bridge, readFile = readFileSync) {
  const { id, method, params } = msg;
  const reply = (result) => ({ jsonrpc: '2.0', id, result });
  const error = (code, message) => ({ jsonrpc: '2.0', id, error: { code, message } });
  if (id === undefined || id === null) return null;
  switch (method) {
    case 'initialize':
      return reply({
        protocolVersion: PROTOCOL_VERSION,
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: 'gs3d', version: '1.0.0' },
        instructions:
          'game-stack 3D asset studio: headless Blender with a toon look and measured probes. Loop: configure (once per game) → build with Model parts → lint → render/contact_sheet → fix → gate → export_glb into apps/web/public/models/. Read tools/mcp-3d/SKILL.md first.',
      });
    case 'ping':
      return reply({});
    case 'tools/list':
      return reply({
        tools: Object.entries(TOOLS).map(([name, t]) => ({
          name,
          description: t.description,
          inputSchema: t.inputSchema,
        })),
      });
    case 'tools/call': {
      const name = params?.name;
      const tool = TOOLS[name];
      if (!tool) return error(-32602, `unknown tool ${name}`);
      try {
        if (tool.local === 'configure') {
          const cfg = writeRuler(params.arguments ?? {});
          bridge.restart?.();
          return reply({ content: [text({ ruler: cfg, path: RULER_PATH, note: 'Blender restarts on the next call with these values.' })] });
        }
        const out = await bridge.call(tool.op, params.arguments ?? {});
        const content = [];
        if (tool.image && out && typeof out.path === 'string') {
          content.push({
            type: 'image',
            data: readFile(out.path).toString('base64'),
            mimeType: 'image/png',
          });
        }
        content.push(text(out));
        return reply({ content });
      } catch (e) {
        const trace = e && typeof e === 'object' && 'trace' in e ? `\n${e.trace}` : '';
        return reply({
          content: [text(`${e instanceof Error ? e.message : String(e)}${trace}`)],
          isError: true,
        });
      }
    }
    default:
      return error(-32601, `method not found: ${method}`);
  }
}

/** Serve MCP on a readable/writable pair (stdio in production). */
export function serve(input, output, bridge) {
  let buf = '';
  input.on('data', (chunk) => {
    buf += chunk.toString();
    let nl;
    while ((nl = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (!line) continue;
      let msg;
      try {
        msg = JSON.parse(line);
      } catch {
        output.write(
          JSON.stringify({
            jsonrpc: '2.0',
            id: null,
            error: { code: -32700, message: 'parse error' },
          }) + '\n',
        );
        continue;
      }
      handle(msg, bridge).then((res) => {
        if (res) output.write(JSON.stringify(res) + '\n');
      });
    }
  });
}
