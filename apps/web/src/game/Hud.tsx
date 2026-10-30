// Plain React UI over the canvas. `Hud` is generated from the Rust `Hud` struct.

import { useHud, useNotices } from '@gs/engine';

const hex = (c: number) => `#${c.toString(16).padStart(6, '0')}`;

export function Hud() {
  const hud = useHud();
  const notices = useNotices();
  if (!hud) return null;
  const mm = Math.floor(hud.seconds_left / 60);
  const ss = String(hud.seconds_left % 60).padStart(2, '0');
  return (
    <div className="hud">
      <div className="top">
        <div className="pill">
          {hud.mode} · round {hud.round} · {hud.playing ? `${mm}:${ss}` : `next round in ${hud.seconds_left}s`}
        </div>
        {hud.mode === 'coop' && (
          <div className="goal">
            <div className="bar" style={{ width: `${Math.min(100, (100 * hud.team_score) / hud.team_goal)}%` }} />
            <span>
              team {hud.team_score} / {hud.team_goal}
            </span>
          </div>
        )}
      </div>
      <ol className="scores">
        {hud.scores.map((s) => (
          <li key={s.id} className={s.me ? 'me' : ''}>
            <i style={{ background: hex(s.color) }} />
            {s.name}
            {s.bot ? ' 🤖' : ''}
            <b>{s.score}</b>
          </li>
        ))}
      </ol>
      <div className="dash">
        <div style={{ width: `${hud.dash * 100}%` }} />
        <span>dash</span>
      </div>
      {!hud.playing && hud.winner && <div className="banner">{hud.winner}</div>}
      <ul className="notices">
        {notices.map((n) => (
          <li key={n.id}>{n.text}</li>
        ))}
      </ul>
    </div>
  );
}
