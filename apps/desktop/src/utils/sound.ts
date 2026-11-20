//! Lightweight Web Audio synthesizer for crisp tactile feedback sounds (toggle on/off,
//! inject success, uninject rollback, and error alerts) with zero external asset files.

let sharedAudioCtx: AudioContext | null = null;

function getAudioContext(): AudioContext | null {
  if (typeof window === "undefined") return null;
  const Ctor =
    window.AudioContext ||
    (window as unknown as { webkitAudioContext?: typeof AudioContext })
      .webkitAudioContext;
  if (!Ctor) return null;
  if (!sharedAudioCtx) {
    sharedAudioCtx = new Ctor();
  }
  if (sharedAudioCtx.state === "suspended") {
    void sharedAudioCtx.resume();
  }
  return sharedAudioCtx;
}

function playTone(
  freqHz: number,
  startOffsetSec: number,
  durationSec: number,
  gainPeak = 0.08,
  type: OscillatorType = "sine",
) {
  const ctx = getAudioContext();
  if (!ctx) return;
  const now = ctx.currentTime + startOffsetSec;
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = type;
  osc.frequency.setValueAtTime(freqHz, now);
  gain.gain.setValueAtTime(0.0001, now);
  gain.gain.exponentialRampToValueAtTime(gainPeak, now + 0.015);
  gain.gain.exponentialRampToValueAtTime(0.0001, now + durationSec);
  osc.connect(gain);
  gain.connect(ctx.destination);
  osc.start(now);
  osc.stop(now + durationSec + 0.02);
}

export const soundFx = {
  toggleOn() {
    playTone(587.33, 0, 0.09, 0.07, "triangle");
    playTone(880.0, 0.06, 0.14, 0.08, "triangle");
  },
  toggleOff() {
    playTone(523.25, 0, 0.09, 0.06, "triangle");
    playTone(349.23, 0.06, 0.14, 0.06, "triangle");
  },
  injectSuccess() {
    playTone(523.25, 0, 0.1, 0.07, "sine");
    playTone(659.25, 0.07, 0.1, 0.07, "sine");
    playTone(783.99, 0.14, 0.22, 0.09, "sine");
  },
  uninject() {
    playTone(587.33, 0, 0.09, 0.06, "sine");
    playTone(440.0, 0.08, 0.16, 0.06, "sine");
  },
  error() {
    playTone(220.0, 0, 0.14, 0.07, "sawtooth");
    playTone(185.0, 0.11, 0.16, 0.06, "sawtooth");
  },
};
