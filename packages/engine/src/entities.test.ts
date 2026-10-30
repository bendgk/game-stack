import assert from 'node:assert/strict';
import { test } from 'node:test';
import { EntityTable, FLAG_ME } from './entities.ts';

const STRIDE = 16;
function buf(rows: number[][]) {
  const b = new Float32Array(rows.length * STRIDE);
  rows.forEach((r, i) => b.set(r, i * STRIDE));
  return b;
}
const row = (id: number, kind: number, flags = 0, x = 0) => [id, kind, flags, x, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0xffffff, 0, 0];

test('entity table keeps object identity and only notifies on membership changes', () => {
  const t = new EntityTable();
  let notified = 0;
  t.subscribe(() => notified++);
  t.update(buf([row(1, 0, FLAG_ME), row(2, 1)]), 2, STRIDE);
  assert.equal(notified, 1);
  const e1 = t.byId.get(1);
  assert.equal(t.me, e1);

  t.update(buf([row(1, 0, FLAG_ME, 5), row(2, 1)]), 2, STRIDE);
  assert.equal(notified, 1, 'moving does not notify');
  assert.equal(t.byId.get(1), e1, 'same object');
  assert.equal(e1?.pos[0], 5);

  t.update(buf([row(2, 1)]), 1, STRIDE);
  assert.equal(notified, 2, 'despawn notifies');
  assert.equal(t.me, null);
  assert.equal(t.list.length, 1);
});
