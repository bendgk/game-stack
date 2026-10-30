import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { PassThrough } from 'node:stream';
import test from 'node:test';
import { BlenderBridge, blenderPath, handle, PROTOCOL_VERSION, serve, TOOLS } from './lib.mjs';

function fakeBridge(results) {
  const calls = [];
  return {
    calls,
    async call(op, args) {
      calls.push({ op, args });
      const r = results[op];
      if (r instanceof Error) throw r;
      return r;
    },
  };
}

test('initialize advertises tools and the protocol version', async () => {
  const res = await handle(
    { jsonrpc: '2.0', id: 1, method: 'initialize', params: {} },
    fakeBridge({}),
  );
  assert.equal(res.result.protocolVersion, PROTOCOL_VERSION);
  assert.ok(res.result.capabilities.tools);
  assert.equal(res.result.serverInfo.name, 'gs3d');
});

test('notifications get no reply', async () => {
  assert.equal(
    await handle({ jsonrpc: '2.0', method: 'notifications/initialized' }, fakeBridge({})),
    null,
  );
});

test('tools/list exposes every tool with an object schema', async () => {
  const res = await handle({ jsonrpc: '2.0', id: 2, method: 'tools/list' }, fakeBridge({}));
  const names = res.result.tools.map((t) => t.name);
  for (const want of [
    'run_python',
    'lint',
    'gate',
    'render',
    'contact_sheet',
    'export_glb',
    'rig',
    'measure',
  ]) {
    assert.ok(names.includes(want), want);
  }
  for (const t of res.result.tools) assert.equal(t.inputSchema.type, 'object', t.name);
});

test('tools/call routes to the bridge op with the arguments', async () => {
  const b = fakeBridge({ lint: { counts: { fix: 0 } } });
  const res = await handle(
    {
      jsonrpc: '2.0',
      id: 3,
      method: 'tools/call',
      params: { name: 'lint', arguments: { names: ['a'] } },
    },
    b,
  );
  assert.deepEqual(b.calls, [{ op: 'lint', args: { names: ['a'] } }]);
  assert.match(res.result.content[0].text, /"fix": 0/);
  assert.equal(res.result.isError, undefined);
});

test('image tools attach the rendered PNG', async () => {
  const b = fakeBridge({ render: { path: '/x.png' } });
  const res = await handle(
    { jsonrpc: '2.0', id: 4, method: 'tools/call', params: { name: 'render', arguments: {} } },
    b,
    () => Buffer.from('png'),
  );
  assert.equal(res.result.content[0].type, 'image');
  assert.equal(res.result.content[0].data, Buffer.from('png').toString('base64'));
  assert.equal(res.result.content[0].mimeType, 'image/png');
});

test('bridge errors come back as tool errors with the Python trace', async () => {
  const err = Object.assign(new Error('KeyError: nope'), { trace: 'Traceback …' });
  const res = await handle(
    {
      jsonrpc: '2.0',
      id: 5,
      method: 'tools/call',
      params: { name: 'run_python', arguments: { code: 'x' } },
    },
    fakeBridge({ exec: err }),
  );
  assert.equal(res.result.isError, true);
  assert.match(res.result.content[0].text, /KeyError: nope\nTraceback/);
});

test('unknown tools and methods are JSON-RPC errors', async () => {
  const a = await handle(
    { jsonrpc: '2.0', id: 6, method: 'tools/call', params: { name: 'nope' } },
    fakeBridge({}),
  );
  assert.equal(a.error.code, -32602);
  const b = await handle({ jsonrpc: '2.0', id: 7, method: 'nope' }, fakeBridge({}));
  assert.equal(b.error.code, -32601);
});

test('serve frames newline-delimited JSON and survives a garbage line', async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  serve(input, output, fakeBridge({}));
  const lines = [];
  output.on('data', (c) => lines.push(...c.toString().trim().split('\n')));
  input.write('not json\n');
  input.write(JSON.stringify({ jsonrpc: '2.0', id: 9, method: 'ping' }) + '\n');
  await new Promise((r) => setTimeout(r, 20));
  const msgs = lines.map((l) => JSON.parse(l));
  assert.equal(msgs[0].error.code, -32700);
  assert.deepEqual(msgs[1], { jsonrpc: '2.0', id: 9, result: {} });
});

test('every tool maps to a bridge op or a local handler', () => {
  for (const [name, t] of Object.entries(TOOLS)) assert.equal(typeof (t.op ?? t.local), 'string', name);
});

// Real Blender: opt in with GS3D_IT=1 (CI has no Blender). Proves each probe catches the
// defect it exists for, and passes a clean kit building.
const IT =
  process.env.GS3D_IT === '1' && (existsSync(blenderPath()) || blenderPath() === 'blender');

test('probes catch planted defects in real Blender', { skip: !IT, timeout: 300_000 }, async () => {
  const bridge = new BlenderBridge();
  try {
    await bridge.call('reset', { biome: 'meadow' });
    const code = `
from gs3d.kit import Body
m = Model("defects", tag="house")
m.add("base", geo.box(4, 4, 3), 0xddccbb)
m.add("float", geo.box(0.5, 0.5, 0.5, 6.0), 0xaa3333)
m.add("buried", geo.translate(geo.box(0.6, 0.6, 0.6, 1.0), 0, 0), 0x3333aa)
m.add("fight", geo.box(4, 4, 3), 0x33aa33)
m.add("hair", geo.translate(geo.box(0.02, 0.02, 1.0, 3.0), 1.5, 1.5), 0x111111)
m.add("lamp", geo.translate(geo.box(0.3, 0.3, 9.0), 4, 4), 0x444444, tag="lamp")
V, F = geo.box(1, 1, 1, 0.0)
m.add("open", (V + [6, 0, 0], F[:-1]), 0x888888)
to_object(m)
c = Model("clean", tag="house")
kit.plinth(c, 5, 4, 0.4, 0x9c958a)
b = Body(5, 4, 0.4, 3.2)
kit.storey(c, b, 0xede2c8)
kit.door(c, b, "f", 0, 1.1, 2.15, 0x6a3f2a, 0x5a3a26, step=0x9c958a)
kit.gable(c, b, 2.6, 0xc2593b, along="x")
ob = to_object(c)
ob.location.x = 20
result = "ok"
`;
    const ran = await bridge.call('exec', { code });
    assert.equal(ran.result, 'ok');
    const rep = await bridge.call('lint', { names: ['defects'], limit: 200 });
    const has = (probe, target) =>
      rep.notes.some((n) => n.probe === probe && n.target.endsWith(target));
    assert.ok(has('support', '/float'), 'floating part');
    assert.ok(has('burial', '/buried'), 'buried part');
    assert.ok(has('z-fight', '/base') || has('z-fight', '/fight'), 'coplanar faces');
    assert.ok(has('hair', '/hair'), 'hair');
    assert.ok(has('proportion', '/lamp'), 'lamp taller than its band');
    assert.ok(has('hygiene', '/open'), 'open solid');
    // Ground snap: a 0.4 m float is a slip and is fixed; a 9 m float is refused.
    await bridge.call('exec', {
      code: `m = Model("slip", tag="prop")\nm.add("box", geo.box(1, 1, 1), 0x888888)\nob = to_object(m)\nob.location = (40, 0, 0.4)\nf = Model("lost", tag="prop")\nf.add("box", geo.box(1, 1, 1), 0x888888)\nob2 = to_object(f)\nob2.location = (44, 0, 9.0)\nbpy.context.view_layer.update()\nresult = "ok"`,
    });
    const snapped = await bridge.call('snap', { names: ['slip', 'lost'] });
    assert.deepEqual(
      snapped.snapped.map((s) => s.name),
      ['slip'],
    );
    assert.equal(snapped.snapped[0].dz, -0.4);
    assert.deepEqual(
      snapped.refused.map((s) => s.name),
      ['lost'],
    );
    const clean = await bridge.call('gate', { names: ['clean'] });
    assert.equal(clean.pass, true, clean.lines.join('\n'));
  } finally {
    bridge.stop();
  }
});

test(
  'the biped rigger measures a low-poly cloaked figure and binds it cleanly',
  { skip: !IT, timeout: 300_000 },
  async () => {
    // Box legs (vertices only at their ends), a cloak hanging behind them and a tabard before:
    // the crotch must be where the legs meet, every bone inside the body, every vertex weighted.
    const bridge = new BlenderBridge();
    try {
      await bridge.call('reset', { biome: 'meadow' });
      const code = `
import math
m = Model("figure", tag="character")
for s in (-1, 1):
    m.add(f"leg{s}", geo.translate(geo.box(0.17, 0.2, 0.86), s * 0.12, 0), 0x3a3a48)
    a = math.radians(42)
    arm = geo.box_span(0.0, -0.06, -0.06, 0.62, 0.06, 0.06) if s > 0 else geo.box_span(-0.62, -0.06, -0.06, 0.0, 0.06, 0.06)
    m.add(f"arm{s}", geo.translate(geo.rot_y(arm, s * a), s * 0.2, 0, 1.33), 0x2f4fa0)
m.add("torso", geo.box(0.46, 0.28, 0.6, 0.82), 0x2f4fa0)
m.add("tabard", geo.box_span(-0.13, -0.16, 0.6, 0.13, -0.14, 1.36), 0xf2ede2)
m.add("cloak", geo.box_span(-0.27, 0.15, 0.45, 0.27, 0.19, 1.42), 0x23397a)
m.add("neck", geo.cylinder(0.07, 0.16, 8, z=1.4), 0xe8b894)
m.add("head", geo.sphere(0.15, 12, 6, z=1.5), 0xe8b894)
m.add("crest", geo.box_span(-0.02, -0.1, 1.78, 0.02, 0.1, 1.86), 0xe6b54a)
to_object(m)
result = "ok"
`;
      assert.equal((await bridge.call('exec', { code })).result, 'ok');
      const out = await bridge.call('rig', { name: 'figure', kind: 'biped' });
      assert.ok(Math.abs(out.landmarks.crotch - 0.84) < 0.06, `crotch at ${out.landmarks.crotch}`);
      assert.ok(
        out.landmarks.neck > 1.35 && out.landmarks.neck < 1.55,
        `neck at ${out.landmarks.neck}`,
      );
      assert.equal(out.report.unweighted, 0);
      assert.deepEqual(out.report.bones_outside, []);
      assert.equal(out.report.ok, true);
    } finally {
      bridge.stop();
    }
  },
);

test('configure writes only known positive numeric ruler keys and restarts the bridge', async () => {
  const { writeRuler } = await import('./lib.mjs');
  let saved = '';
  const cfg = writeRuler(
    { avatar_height: 2, camera_fov_deg: 70, bogus: 1, tile_m: -1, step_up: 'x' },
    { read: () => '{"avatar_radius":0.5}', write: (_p, s) => (saved = s), path: '/tmp/ruler.json' },
  );
  assert.deepEqual(cfg, { avatar_radius: 0.5, avatar_height: 2, camera_fov_deg: 70 });
  assert.deepEqual(JSON.parse(saved), cfg);
});
