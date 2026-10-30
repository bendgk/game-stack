// Drop-in DOM overlays for development.

import type { CSSProperties } from 'react';
import { useNetStats, useSession, useStatus } from './react.tsx';

const box: CSSProperties = {
  position: 'fixed',
  font: '11px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace',
  background: 'rgba(10,12,20,.72)',
  color: '#cfe3ff',
  padding: '6px 8px',
  borderRadius: 6,
  pointerEvents: 'none',
  whiteSpace: 'pre',
  zIndex: 10,
};

/** RTT, input slack, rollbacks, prediction hits… press nothing, just look. */
export function NetStatsOverlay({ corner = 'bottom-left' }: { corner?: 'bottom-left' | 'bottom-right' | 'top-left' | 'top-right' }) {
  const session = useSession();
  const s = useNetStats();
  const { status } = useStatus();
  const [v, h] = corner.split('-') as ['top' | 'bottom', 'left' | 'right'];
  const pos = { [v]: 8, [h]: 8 };
  if (!s) return <div style={{ ...box, ...pos }}>{status}</div>;
  const hitRate = s.prediction_hits + s.rollbacks > 0 ? (100 * s.prediction_hits) / (s.prediction_hits + s.rollbacks) : 100;
  return (
    <div style={{ ...box, ...pos }}>
      {[
        `${session.offline ? 'local' : 'webrtc'} · ${status} · build ${session.fingerprint.slice(0, 8)}`,
        `tick ${s.tick}  server ${s.server_tick}  lead ${s.tick - s.server_tick}`,
        `rtt ${s.rtt_ms.toFixed(1)}ms ±${s.jitter_ms.toFixed(1)}  slack ${s.slack.toFixed(2)}  ×${s.time_scale.toFixed(3)}`,
        `predicted ${hitRate.toFixed(0)}%  rollbacks ${s.rollbacks} (last ${s.last_rollback_ticks}t)`,
        `snap ${s.snapshot_bytes}B  dropped ${s.dropped_snapshots}  resyncs ${s.resyncs}${s.stalled ? '  STALLED' : ''}`,
      ].join('\n')}
    </div>
  );
}
