// Tiny synthesized sound effects (no assets). Replace with real audio whenever you like.

let ctx: AudioContext | null = null;
function audio() {
  if (!ctx) {
    try {
      ctx = new AudioContext();
    } catch {
      return null;
    }
  }
  if (ctx.state === 'suspended') void ctx.resume();
  return ctx;
}

function blip(freq: number, to: number, dur: number, type: OscillatorType = 'sine', gain = 0.12) {
  const a = audio();
  if (!a) return;
  const o = a.createOscillator();
  const g = a.createGain();
  o.type = type;
  o.frequency.setValueAtTime(freq, a.currentTime);
  o.frequency.exponentialRampToValueAtTime(to, a.currentTime + dur);
  g.gain.setValueAtTime(gain, a.currentTime);
  g.gain.exponentialRampToValueAtTime(0.0001, a.currentTime + dur);
  o.connect(g).connect(a.destination);
  o.start();
  o.stop(a.currentTime + dur);
}

export const sfx = {
  orb: () => {
    blip(660, 1320, 0.12, 'triangle');
    setTimeout(() => blip(990, 1760, 0.1, 'triangle'), 60);
  },
  jump: () => blip(220, 440, 0.12, 'square', 0.04),
  dash: () => blip(180, 60, 0.18, 'sawtooth', 0.05),
  bump: (k: number) => blip(120, 50, 0.15, 'square', 0.03 + 0.06 * k),
  fanfare: () => [523, 659, 784, 1046].forEach((f, i) => setTimeout(() => blip(f, f, 0.18, 'triangle', 0.08), i * 110)),
};
