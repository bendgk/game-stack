/**
 * gs2d MCP server core: MCP (JSON-RPC 2.0, newline-delimited over stdio) in front of an
 * in-process 2D asset studio (`studio.mjs`). No dependencies.
 */

import { join } from 'node:path';

import { encodePng } from './png.mjs';
import { OUT, Studio, writePng } from './studio.mjs';

export const PROTOCOL_VERSION = '2025-06-18';

const SPRITES = { type: 'array', items: { type: 'string' }, description: 'Sprite names (default: all).' };
const FRAME = {
  description: "Frame index, list of indices, or 'all' (default 0 for draw, 'all' for previews).",
  anyOf: [{ type: 'number' }, { type: 'string' }, { type: 'array', items: { type: 'number' } }],
};
const OPS_DOC = `Array of ops, applied in order. Coordinates are pixels, origin top-left, y down. Colours: '#rrggbb', '#rrggbbaa', [r,g,b,a], names, or '@i' (palette index). fill/stroke may be a gradient {linear:[x0,y0,x1,y1]|radial:[cx,cy,r], stops:[[0,'#f00'],[1,'#00f']]}. Optional per op: aa (bool), transform {translate,scale,rotate,origin}.
- {op:'rect', x,y,w,h, r?, fill?, stroke?, width?}
- {op:'circle', cx,cy,r, fill?, stroke?, width?}   {op:'ellipse', cx,cy,rx,ry, rotation?, …}
- {op:'poly', points:[[x,y]…], fill?, stroke?, width?, closed?}
- {op:'line', x1,y1,x2,y2 | points, color, width?, cap?:'round'|'square'|'butt'}
- {op:'path', d:'<SVG path data>', fill?, stroke?, width?, rule?:'nonzero'|'evenodd'}
- {op:'pixels', x?,y?, rows:['..ab..', …], key:{a:'#f00', b:'@3'}}  ('.'/' ' = clear; hex digits = palette index)
- {op:'pixel', x,y | points, color}   {op:'text', x,y, text, color?, scale?} (3×5 font)
- {op:'clear', color?}   {op:'fill_bucket', x,y, color, tolerance?}   {op:'replace_color', from, to, tolerance?}
- {op:'outline', color?, width?, corners?}   {op:'shadow', dx?,dy?, color?}   {op:'shade', light?:[-1,-1], highlight?, dark?}
- {op:'quantize', palette?, dither?:0..1}   {op:'grain', scale?, amount?, seed?, octaves?}
- {op:'flip', axis:'x'|'y'}   {op:'rotate', turns}   {op:'shift', dx,dy, wrap?}   {op:'blit', sprite, frame?, x?,y?, flipX?, flipY?, scale?}`;

/** The tool table: name → { description, inputSchema, run(studio, args), image? }. */
export const TOOLS = {
  configure: {
    description:
      "Set the 2D ruler for YOUR game (writes tools/mcp-2d/ruler.json): ppu (art pixels per world unit; 1 unit = 1 m in gs-physics), screen_ppu (screen px per unit = <Camera2D zoom>), avatar_w/avatar_h (your player in world units), background (game bg colour, for readability checks), palette_budget. No args = read current.",
    inputSchema: {
      type: 'object',
      properties: {
        ppu: { type: 'number' },
        screen_ppu: { type: 'number' },
        avatar_w: { type: 'number' },
        avatar_h: { type: 'number' },
        background: { type: 'string' },
        palette_budget: { type: 'number' },
      },
    },
    run: (s, a) => s.configure(a, Object.keys(a).length > 0),
  },
  status: {
    description: 'The ruler, active palette and every sprite (size, frames, tag, pivot, anims, world size).',
    inputSchema: { type: 'object', properties: {} },
    run: (s) => s.outline(),
  },
  palette: {
    description:
      "Choose or define the active palette ('@i' colour references and lint use it). Built-ins: gs-default (matches the example game), pico8, sweetie16, gameboy, grayscale8. Pass colors to define a new one.",
    inputSchema: { type: 'object', properties: { name: { type: 'string' }, colors: { type: 'array', items: { type: 'string' } } } },
    run: (s, a) => s.setPalette(a),
  },
  sprite_new: {
    description:
      "Create a sprite. tag: character | enemy | pickup | prop | projectile | tile | ui | fx | background (drives the probes). pivot: [x,y] 0..1 (default [0.5,1] = feet). aa: false = crisp pixel art (default), true = smooth vector art. Size it from the ruler: a 1-unit-tall player at ppu 32 is 32px tall.",
    inputSchema: {
      type: 'object',
      properties: {
        name: { type: 'string' },
        w: { type: 'number' },
        h: { type: 'number' },
        frames: { type: 'number' },
        tag: { type: 'string' },
        pivot: { type: 'array', items: { type: 'number' } },
        aa: { type: 'boolean' },
        fps: { type: 'number' },
        fill: { type: 'string' },
      },
      required: ['name', 'w', 'h'],
    },
    run: (s, a) => s.newSprite(a),
  },
  sprite_set: {
    description: 'Change a sprite’s tag, pivot, aa or fps.',
    inputSchema: {
      type: 'object',
      properties: { sprite: { type: 'string' }, tag: { type: 'string' }, pivot: { type: 'array', items: { type: 'number' } }, aa: { type: 'boolean' }, fps: { type: 'number' } },
      required: ['sprite'],
    },
    run: (s, a) => s.setSprite(a),
  },
  frame_add: {
    description: 'Append frames (empty, or copies of frame `copy_from`) — for animations, start from a copy and change it.',
    inputSchema: { type: 'object', properties: { sprite: { type: 'string' }, count: { type: 'number' }, copy_from: { type: 'number' } }, required: ['sprite'] },
    run: (s, a) => s.addFrames(a),
  },
  draw: {
    description: `Draw on a sprite's frame(s).\n${OPS_DOC}`,
    inputSchema: {
      type: 'object',
      properties: { sprite: { type: 'string' }, frame: FRAME, ops: { type: 'array', items: { type: 'object' } } },
      required: ['sprite', 'ops'],
    },
    run: (s, a) => s.draw(a),
  },
  anim: {
    description: 'Define a named animation: frames (indices in order), fps, loop. Exported as "<sprite>/<name>".',
    inputSchema: {
      type: 'object',
      properties: { sprite: { type: 'string' }, name: { type: 'string' }, frames: { type: 'array', items: { type: 'number' } }, fps: { type: 'number' }, loop: { type: 'boolean' } },
      required: ['sprite', 'name', 'frames'],
    },
    run: (s, a) => s.setAnim(a),
  },
  run_js: {
    description:
      "Run JavaScript with the studio API for procedural work (loops over frames, generated tiles). In scope: draw(sprite, ops, frame), newSprite({...}), sprite(name) (→ {frames: Canvas[]…}), R (raster: Canvas, fillPolys, outline, noise2…), color(c), palette, Math. Assign `result` to return JSON; console.log is captured.",
    inputSchema: { type: 'object', properties: { code: { type: 'string' } }, required: ['code'] },
    run: (s, a) => s.runJs(a),
  },
  render: {
    description:
      "Look at sprites: frames side by side, scaled up (nearest), labelled. bg: 'checker' or a colour. tile: n repeats each frame n×n (see seams). grid: pixel grid when zoomed.",
    inputSchema: {
      type: 'object',
      properties: { sprites: SPRITES, frame: FRAME, scale: { type: 'number' }, bg: { type: 'string' }, tile: { type: 'number' }, grid: { type: 'boolean' } },
    },
    image: (s, a) => s.render(a),
  },
  preview: {
    description:
      'See a sprite at TRUE in-game size: on the game background, at the screen scale (<Camera2D zoom>), next to a player-sized block. Use it to judge readability and scale — what players will actually see.',
    inputSchema: { type: 'object', properties: { sprite: { type: 'string' }, frame: FRAME, zoom: { type: 'number' } }, required: ['sprite'] },
    image: (s, a) => s.preview(a),
  },
  lint: {
    description:
      'Measured checks with fix / warn / info notes: empty, clipped (content touching edges), padding, alpha (half-transparent pixels in pixel art), orphans (stray pixels), palette (colour budget), contrast (silhouette vs the game background), scale (height in player heights by tag), tile (ppu multiple + seams), frames (feet jitter across frames), anim.',
    inputSchema: { type: 'object', properties: { sprites: SPRITES, probes: { type: 'array', items: { type: 'string' } } } },
    run: (s, a) => s.lint(a),
  },
  gate: {
    description: 'The terminal check before export: the same reads as lint; pass only with zero fix notes.',
    inputSchema: { type: 'object', properties: { sprites: SPRITES } },
    run: (s, a) => s.gate(a),
  },
  export_sheet: {
    description:
      'Pack sprites (all frames + animations) into one PNG atlas + JSON that @gs/engine/three <Sprite sheet="/sprites/x.json" anim="hero/run"/> loads. path: e.g. apps/web/public/sprites/hero.json (PNG written next to it).',
    inputSchema: { type: 'object', properties: { sprites: SPRITES, path: { type: 'string' }, padding: { type: 'number' } }, required: ['path'] },
    run: (s, a) => s.exportSheet(a),
  },
  export_png: {
    description: 'Write one frame as a PNG (optionally scaled up, nearest).',
    inputSchema: { type: 'object', properties: { sprite: { type: 'string' }, frame: { type: 'number' }, path: { type: 'string' }, scale: { type: 'number' } }, required: ['sprite', 'path'] },
    run: (s, a) => s.exportPng(a),
  },
  import_png: {
    description: 'Load a PNG as a sprite; frame_w/frame_h slice a sheet into frames (row-major).',
    inputSchema: {
      type: 'object',
      properties: { path: { type: 'string' }, name: { type: 'string' }, frame_w: { type: 'number' }, frame_h: { type: 'number' }, tag: { type: 'string' }, aa: { type: 'boolean' } },
      required: ['path', 'name'],
    },
    run: (s, a) => s.importPng(a),
  },
  sprite_delete: {
    description: 'Remove a sprite from the session.',
    inputSchema: { type: 'object', properties: { sprite: { type: 'string' } }, required: ['sprite'] },
    run: (s, a) => s.remove(a),
  },
  save_project: {
    description: 'Save every sprite (frames, tags, anims, palette) to a .gs2d.json project file to continue later.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' } }, required: ['path'] },
    run: (s, a) => s.save(a),
  },
  open_project: {
    description: 'Open a .gs2d.json project (replaces the session).',
    inputSchema: { type: 'object', properties: { path: { type: 'string' } }, required: ['path'] },
    run: (s, a) => s.open(a),
  },
};

function text(value) {
  return { type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value, null, 1) };
}

let shot = 0;

/** Handle one JSON-RPC message; returns the response (or null for notifications). */
export async function handle(msg, studio) {
  const { id, method, params } = msg;
  const reply = (result) => ({ jsonrpc: '2.0', id, result });
  const error = (code, message) => ({ jsonrpc: '2.0', id, error: { code, message } });
  if (id === undefined || id === null) return null;
  switch (method) {
    case 'initialize':
      return reply({
        protocolVersion: PROTOCOL_VERSION,
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: 'gs2d', version: '1.0.0' },
        instructions:
          'game-stack 2D asset studio (sprites, animations, tiles, UI). Loop: configure (once per game) → sprite_new → draw (shapes, SVG paths, pixel rows) → render / preview (true game scale) → lint → fix → gate → export_sheet into apps/web/public/sprites/. Read tools/mcp-2d/SKILL.md first.',
      });
    case 'ping':
      return reply({});
    case 'tools/list':
      return reply({ tools: Object.entries(TOOLS).map(([name, t]) => ({ name, description: t.description, inputSchema: t.inputSchema })) });
    case 'tools/call': {
      const tool = TOOLS[params?.name];
      if (!tool) return error(-32602, `unknown tool ${params?.name}`);
      const args = params.arguments ?? {};
      try {
        if (tool.image) {
          const canvas = tool.image(studio, args);
          const file = writePng(join(OUT, `${params.name}-${++shot}.png`), canvas);
          return reply({
            content: [{ type: 'image', data: encodePng(canvas.w, canvas.h, canvas.data).toString('base64'), mimeType: 'image/png' }, text(file)],
          });
        }
        return reply({ content: [text(await tool.run(studio, args))] });
      } catch (e) {
        return reply({ content: [text(e instanceof Error ? e.message : String(e))], isError: true });
      }
    }
    default:
      return error(-32601, `method not found: ${method}`);
  }
}

/** Serve MCP on a readable/writable pair (stdio in production). */
export function serve(input, output, studio = new Studio()) {
  let buf = '';
  let chain = Promise.resolve();
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
        output.write(JSON.stringify({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'parse error' } }) + '\n');
        continue;
      }
      // Calls run in order: the studio is a stateful session.
      chain = chain.then(() => handle(msg, studio)).then((res) => {
        if (res) output.write(JSON.stringify(res) + '\n');
      });
    }
  });
  return studio;
}
