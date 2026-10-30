/**
 * Klangrezepte der Lykos-Soundeffekte – zur Laufzeit per Web Audio API synthetisiert.
 *
 * Warum Synthese statt Audiodateien: null Bundle-Gewicht, keine Lizenzfragen, alle Klänge aus
 * denselben drei Klangfarben und derselben Tonleiter (A-Moll-Pentatonik: A C D E G) – dadurch
 * klingen sie wie eine Familie. Lautstärke, Hall und Ausgabegerät lassen sich zentral steuern.
 *
 * Klangfarben (additive Synthese, Teiltöne mit eigenem Abklingen):
 * - `bell`: weiche Glocke/Kalimba – Grundton + leise Oktave, Duodezime und ein unharmonischer
 *   Schimmer-Teilton, der schnell verklingt (die „Mondlicht“-Anmutung).
 * - `wood`: Holz/Marimba – kurzer Anschlag, Teiltöne bei ×3,9 und ×9,2 sterben fast sofort ab.
 * - `soft`: zwei leicht gegeneinander verstimmte Sinustöne – warmes, langsames Schweben.
 * - `glass`: hell und unharmonisch (Vibraphon/Glas) – nur für Übertragung und Kamera, damit sich
 *   Video-Ereignisse deutlich von Voice-, Schalter- und Nachrichtentönen abheben.
 *
 * Zusätzlich gibt es gefiltertes Rauschen (`noise`): ein Luft-„Whoosh“ (Bandpass gleitet auf- bzw.
 * abwärts) für Übertragungen und ein kurzer Auslöser-Klick für die Kamera.
 *
 * Dazu pro Rezept ein Tiefpass (Wärme) und etwas Hall aus einer synthetisch erzeugten
 * Impulsantwort (dunkles, abklingendes Rauschen ≈ 1,2 s, „nächtlicher Raum“).
 *
 * Diese Datei hat keine Seiteneffekte beim Import (Rezepte sind reine Daten, testbar unter Node);
 * `renderSound` braucht einen `BaseAudioContext` und funktioniert auch mit `OfflineAudioContext`.
 */
import type { SoundName } from './soundLogic'

export type Timbre = 'bell' | 'wood' | 'soft' | 'glass'

export interface SynthNote {
  /** Einsatz in Sekunden ab Klangbeginn. */
  at: number
  /** Tonname, z. B. `A4`, `E5` (nur Töne der Pentatonik + H für Kamera). */
  note: string
  /** Abklingzeit bis ≈ −60 dB in Sekunden. */
  dur: number
  /** Relative Lautstärke 0…1. */
  gain: number
  timbre: Timbre
  /** Tonhöhe gleitet bis Notenende auf `frequenz × glide` (z. B. 0.94 = leicht nach unten). */
  glide?: number
}

/** Gefiltertes Rauschen: Bandpass gleitet während `dur` von `from` nach `to` Hz. */
export interface NoiseBurst {
  at: number
  dur: number
  gain: number
  from: number
  to: number
  /** Güte des Bandpasses (höher = tonaler, schmaler). */
  q: number
}

export interface SoundRecipe {
  notes: SynthNote[]
  noise?: NoiseBurst[]
  /** Gesamtpegel des Klangs (vor Gesamtlautstärke). */
  gain: number
  /** Tiefpass-Grenzfrequenz in Hz. */
  lowpass: number
  /** Hallanteil 0…1. */
  reverb: number
}

interface Overtone {
  ratio: number
  gain: number
  /** Faktor auf die Abklingzeit der Note. */
  decay: number
  /** Verstimmung in Cent (für Schwebung). */
  detune?: number
}

const TIMBRES: Record<Timbre, { attack: number; partials: Overtone[] }> = {
  bell: {
    attack: 0.004,
    partials: [
      { ratio: 1, gain: 1, decay: 1 },
      { ratio: 2, gain: 0.3, decay: 0.55 },
      { ratio: 3.01, gain: 0.1, decay: 0.35 },
      { ratio: 5.43, gain: 0.035, decay: 0.18 },
    ],
  },
  wood: {
    attack: 0.002,
    partials: [
      { ratio: 1, gain: 1, decay: 1 },
      { ratio: 3.93, gain: 0.16, decay: 0.2 },
      { ratio: 9.2, gain: 0.035, decay: 0.08 },
    ],
  },
  glass: {
    attack: 0.003,
    partials: [
      { ratio: 1, gain: 1, decay: 1, detune: -3 },
      { ratio: 1, gain: 0.5, decay: 0.9, detune: 5 },
      { ratio: 2.76, gain: 0.32, decay: 0.45 },
      { ratio: 5.4, gain: 0.14, decay: 0.25 },
      { ratio: 8.93, gain: 0.05, decay: 0.12 },
    ],
  },
  soft: {
    attack: 0.012,
    partials: [
      { ratio: 1, gain: 0.7, decay: 1, detune: -4 },
      { ratio: 1, gain: 0.7, decay: 1, detune: 4 },
      { ratio: 2, gain: 0.08, decay: 0.5 },
    ],
  },
}

const NOTE_INDEX: Record<string, number> = { C: -9, D: -7, E: -5, F: -4, G: -2, A: 0, B: 2, H: 2 }

/** Tonname (`A4`, `C#5`, `E6`) → Frequenz in Hz (A4 = 440 Hz, gleichstufig). */
export function noteToHz(note: string): number {
  const m = /^([A-H])(#|b)?(-?\d)$/.exec(note)
  if (!m) throw new Error(`Unbekannter Ton: ${note}`)
  const semis = NOTE_INDEX[m[1]] + (m[2] === '#' ? 1 : m[2] === 'b' ? -1 : 0) + (Number(m[3]) - 4) * 12
  return 440 * 2 ** (semis / 12)
}

const whoosh = (at: number, dur: number, gain: number, from: number, to: number, q = 1.4): NoiseBurst => ({
  at,
  dur,
  gain,
  from,
  to,
  q,
})

const n = (at: number, note: string, dur: number, gain: number, timbre: Timbre, glide?: number): SynthNote => ({
  at,
  note,
  dur,
  gain,
  timbre,
  glide,
})

/**
 * Die Klangfamilie. Grundidee: aufsteigend = etwas beginnt/geht an, absteigend = endet/geht aus;
 * Glocke für eigene Aktionen, weiches Schweben für andere Leute, Holz für Schalter.
 */
export const RECIPES: Record<SoundName, SoundRecipe> = {
  // Eigener Beitritt: Quarte aufwärts D5 → A5, Glocke.
  voiceJoin: {
    notes: [n(0, 'D5', 0.3, 0.8, 'bell'), n(0.09, 'A5', 0.3, 0.75, 'bell')],
    gain: 0.5,
    lowpass: 4200,
    reverb: 0.22,
  },
  // Eigenes Verlassen: dieselben Töne abwärts, etwas dunkler.
  voiceLeave: {
    notes: [n(0, 'A5', 0.26, 0.75, 'bell'), n(0.09, 'D5', 0.3, 0.8, 'bell')],
    gain: 0.5,
    lowpass: 2800,
    reverb: 0.22,
  },
  // Andere kommen/gehen: leiser, weich schwebend, Quarte E5 ↔ A5.
  peerJoin: {
    notes: [n(0, 'E5', 0.2, 0.7, 'soft'), n(0.07, 'A5', 0.22, 0.65, 'soft')],
    gain: 0.36,
    lowpass: 3200,
    reverb: 0.25,
  },
  peerLeave: {
    notes: [n(0, 'A5', 0.2, 0.65, 'soft'), n(0.07, 'E5', 0.22, 0.7, 'soft')],
    gain: 0.36,
    lowpass: 2600,
    reverb: 0.25,
  },
  // Mikrofon: zwei Holzschläge, stumm abwärts / an aufwärts.
  mute: {
    notes: [n(0, 'E5', 0.12, 0.8, 'wood', 0.97), n(0.055, 'A4', 0.14, 0.9, 'wood', 0.97)],
    gain: 0.45,
    lowpass: 3000,
    reverb: 0.1,
  },
  unmute: {
    notes: [n(0, 'A4', 0.12, 0.85, 'wood'), n(0.055, 'E5', 0.14, 0.85, 'wood', 1.02)],
    gain: 0.45,
    lowpass: 3600,
    reverb: 0.1,
  },
  // Taub: Holz + tiefes Schweben, Klang „schließt sich“ (dunkler Tiefpass).
  deafen: {
    notes: [n(0, 'D5', 0.12, 0.7, 'wood'), n(0.06, 'A4', 0.2, 0.7, 'soft'), n(0.13, 'D4', 0.25, 0.85, 'soft')],
    gain: 0.45,
    lowpass: 1700,
    reverb: 0.12,
  },
  undeafen: {
    notes: [n(0, 'D4', 0.18, 0.8, 'soft'), n(0.06, 'A4', 0.2, 0.7, 'soft'), n(0.13, 'D5', 0.16, 0.75, 'wood')],
    gain: 0.45,
    lowpass: 3400,
    reverb: 0.12,
  },
  // Push-to-Talk: kaum hörbares Holzklicken.
  pttDown: {
    notes: [n(0, 'A5', 0.06, 1, 'wood')],
    gain: 0.14,
    lowpass: 3500,
    reverb: 0,
  },
  pttUp: {
    notes: [n(0, 'E5', 0.06, 1, 'wood')],
    gain: 0.12,
    lowpass: 3000,
    reverb: 0,
  },
  // Eigene Übertragung: Luft-Whoosh aufwärts + helles Glas-Arpeggio A5 E6 A6 (eine Oktave über
  // den Voice-Tönen) – unverwechselbar „etwas geht auf Sendung“.
  screenShareStart: {
    notes: [n(0.05, 'A5', 0.2, 0.6, 'glass'), n(0.11, 'E6', 0.2, 0.6, 'glass'), n(0.17, 'A6', 0.22, 0.55, 'glass')],
    noise: [whoosh(0, 0.26, 0.5, 500, 4200)],
    gain: 0.48,
    lowpass: 6500,
    reverb: 0.24,
  },
  screenShareStop: {
    notes: [n(0, 'A6', 0.18, 0.5, 'glass'), n(0.06, 'E6', 0.2, 0.55, 'glass'), n(0.12, 'A5', 0.24, 0.6, 'glass')],
    noise: [whoosh(0.02, 0.28, 0.45, 4000, 450)],
    gain: 0.48,
    lowpass: 5000,
    reverb: 0.24,
  },
  // Fremde Übertragung: derselbe Whoosh, leiser, Glas-Zweiklang E6 → A6 bzw. zurück.
  peerStreamStart: {
    notes: [n(0.06, 'E6', 0.2, 0.55, 'glass'), n(0.13, 'A6', 0.22, 0.5, 'glass')],
    noise: [whoosh(0, 0.24, 0.35, 600, 3600)],
    gain: 0.38,
    lowpass: 6000,
    reverb: 0.26,
  },
  peerStreamStop: {
    notes: [n(0, 'A6', 0.2, 0.5, 'glass'), n(0.07, 'E6', 0.22, 0.55, 'glass')],
    noise: [whoosh(0.02, 0.24, 0.3, 3600, 550)],
    gain: 0.38,
    lowpass: 4800,
    reverb: 0.26,
  },
  // Kamera: kurzer, heller Auslöser-Klick (schmales Rauschen) + Glas-Quinte E6/H6 („Blick“).
  cameraOn: {
    notes: [n(0.03, 'E6', 0.16, 0.6, 'glass'), n(0.075, 'H6', 0.22, 0.5, 'glass')],
    noise: [whoosh(0, 0.035, 0.55, 5200, 3000, 3), whoosh(0.045, 0.03, 0.35, 3800, 2400, 3)],
    gain: 0.45,
    lowpass: 7000,
    reverb: 0.16,
  },
  cameraOff: {
    notes: [n(0.03, 'H6', 0.14, 0.45, 'glass'), n(0.075, 'E6', 0.2, 0.55, 'glass', 0.98)],
    noise: [whoosh(0, 0.03, 0.4, 3000, 1800, 3)],
    gain: 0.45,
    lowpass: 5000,
    reverb: 0.16,
  },
  // Nachricht: ein einzelner, leiser Glockenton mit weichem Unterton.
  message: {
    notes: [n(0, 'E6', 0.3, 0.6, 'bell'), n(0, 'A5', 0.28, 0.25, 'soft')],
    gain: 0.32,
    lowpass: 4500,
    reverb: 0.2,
  },
  // Erwähnung: deutlicher – zwei Glocken A5 → E6 plus heller Schimmer (Mondlicht), mehr Hall.
  mention: {
    notes: [n(0, 'A5', 0.26, 0.8, 'bell'), n(0.085, 'E6', 0.3, 0.8, 'bell'), n(0.085, 'E7', 0.22, 0.08, 'soft')],
    gain: 0.5,
    lowpass: 6000,
    reverb: 0.28,
  },
  // Direktnachricht: Holz-Anschlag G5, dann Glocke D6 (Quinte aufwärts).
  dm: {
    notes: [n(0, 'G5', 0.12, 0.6, 'wood'), n(0.06, 'D6', 0.28, 0.65, 'bell')],
    gain: 0.4,
    lowpass: 4800,
    reverb: 0.22,
  },
  // Freundschaftsanfrage: zwei Holz-Anschläge D5 → G5, dann Glocken-Terz C6/E6 – freundlich „anklopfen“.
  friendRequest: {
    notes: [
      n(0, 'D5', 0.1, 0.55, 'wood'),
      n(0.06, 'G5', 0.1, 0.55, 'wood'),
      n(0.12, 'C6', 0.24, 0.6, 'bell'),
      n(0.12, 'E6', 0.26, 0.45, 'bell'),
    ],
    gain: 0.4,
    lowpass: 4800,
    reverb: 0.22,
  },
  // Klingelton (eingehender Anruf): Glocken-Arpeggio A5 C6 E6 A6 – hell und auffordernd. Läuft als Schleife
  // „ring-ring … ring-ring“ (siehe `callAlerts.ts`), daher kurz und ohne lange Fahne.
  incomingCall: {
    notes: [
      n(0, 'A5', 0.16, 0.7, 'bell'),
      n(0.065, 'C6', 0.16, 0.65, 'bell'),
      n(0.13, 'E6', 0.16, 0.65, 'bell'),
      n(0.195, 'A6', 0.2, 0.55, 'bell'),
    ],
    gain: 0.5,
    lowpass: 6000,
    reverb: 0.18,
  },
  // Freizeichen (man ruft selbst an): leises, weiches Schweben E5 → A5, ruhig wiederholt.
  outgoingCall: {
    notes: [n(0, 'E5', 0.3, 0.6, 'soft'), n(0.12, 'A5', 0.26, 0.55, 'soft')],
    gain: 0.3,
    lowpass: 3000,
    reverb: 0.22,
  },
  // Verbindung verloren: kleine Terz abwärts, gedämpft, leicht absinkend – ruhig, aber „anders“.
  connectionLost: {
    notes: [n(0, 'E5', 0.25, 0.8, 'soft', 0.97), n(0.12, 'C5', 0.28, 0.85, 'soft', 0.97)],
    gain: 0.45,
    lowpass: 1600,
    reverb: 0.2,
  },
  // Wiederhergestellt: C5 → E5 → A5, löst die Terz in den Grundton auf.
  reconnected: {
    notes: [n(0, 'C5', 0.18, 0.7, 'soft'), n(0.07, 'E5', 0.2, 0.7, 'bell'), n(0.14, 'A5', 0.24, 0.7, 'bell')],
    gain: 0.42,
    lowpass: 4000,
    reverb: 0.2,
  },
  // Update bereit: langsames, freundliches E5 A5 E6.
  updateReady: {
    notes: [n(0, 'E5', 0.2, 0.6, 'bell'), n(0.1, 'A5', 0.2, 0.6, 'bell'), n(0.2, 'E6', 0.2, 0.55, 'bell')],
    gain: 0.4,
    lowpass: 4500,
    reverb: 0.25,
  },
}

/** Trockene Länge eines Rezepts in Sekunden (ohne Hallfahne). */
export function recipeDuration(r: SoundRecipe): number {
  return Math.max(...r.notes.map((x) => x.at + x.dur), ...(r.noise ?? []).map((x) => x.at + x.dur))
}

// ---------------------------------------------------------------------------
// Wiedergabe
// ---------------------------------------------------------------------------

const reverbCache = new WeakMap<BaseAudioContext, AudioBuffer>()
const noiseCache = new WeakMap<BaseAudioContext, AudioBuffer>()

/** Weißes Rauschen (0,5 s, deterministisch) als Quelle für Whoosh und Klick. */
function noiseBuffer(ctx: BaseAudioContext): AudioBuffer {
  const cached = noiseCache.get(ctx)
  if (cached) return cached
  const length = Math.floor(ctx.sampleRate * 0.5)
  const buf = ctx.createBuffer(1, length, ctx.sampleRate)
  const data = buf.getChannelData(0)
  let seed = 0x5eed11
  for (let i = 0; i < length; i++) {
    seed = (seed * 1664525 + 1013904223) >>> 0
    data[i] = (seed / 0x100000000) * 2 - 1
  }
  noiseCache.set(ctx, buf)
  return buf
}

/** Synthetische Impulsantwort: dunkles, exponentiell abklingendes Stereo-Rauschen (~1,2 s). */
function reverbImpulse(ctx: BaseAudioContext): AudioBuffer {
  const cached = reverbCache.get(ctx)
  if (cached) return cached
  const seconds = 1.2
  const length = Math.floor(ctx.sampleRate * seconds)
  const buf = ctx.createBuffer(2, length, ctx.sampleRate)
  // Deterministisches Rauschen (gleicher Raum bei jedem Start).
  let seed = 0x2f6b1d
  const rand = () => {
    seed = (seed * 1664525 + 1013904223) >>> 0
    return seed / 0x100000000 - 0.5
  }
  for (let ch = 0; ch < 2; ch++) {
    const data = buf.getChannelData(ch)
    let low = 0
    for (let i = 0; i < length; i++) {
      const t = i / ctx.sampleRate
      // Einpoliger Tiefpass auf dem Rauschen → warmer, dunkler Raum.
      low += 0.18 * (rand() - low)
      const predelay = t < 0.012 ? 0 : 1
      data[i] = low * 2.2 * predelay * Math.exp(-t * 5.2)
    }
  }
  reverbCache.set(ctx, buf)
  return buf
}

/**
 * Einen Klang einplanen. `destination` ist der Eingang der Gesamtlautstärke; `when` die
 * Kontextzeit des Einsatzes. Alle Knoten trennen sich nach dem Ausklingen selbst.
 */
export function renderSound(ctx: BaseAudioContext, destination: AudioNode, recipe: SoundRecipe, when: number) {
  const out = ctx.createGain()
  out.gain.value = recipe.gain
  const tone = ctx.createBiquadFilter()
  tone.type = 'lowpass'
  tone.frequency.value = recipe.lowpass
  tone.Q.value = 0.5
  tone.connect(out)
  out.connect(destination)

  let reverb: ConvolverNode | null = null
  if (recipe.reverb > 0) {
    const send = ctx.createGain()
    send.gain.value = recipe.reverb
    reverb = ctx.createConvolver()
    reverb.buffer = reverbImpulse(ctx)
    out.connect(send)
    send.connect(reverb)
    reverb.connect(destination)
  }

  let end = when
  for (const note of recipe.notes) {
    const timbre = TIMBRES[note.timbre]
    const freq = noteToHz(note.note)
    const start = when + note.at
    for (const p of timbre.partials) {
      const f = freq * p.ratio
      if (f > ctx.sampleRate / 2 - 500) continue
      const decay = Math.max(0.03, note.dur * p.decay)
      const osc = ctx.createOscillator()
      osc.type = 'sine'
      osc.frequency.setValueAtTime(f, start)
      if (note.glide) osc.frequency.exponentialRampToValueAtTime(f * note.glide, start + decay)
      if (p.detune) osc.detune.value = p.detune
      const env = ctx.createGain()
      const peak = note.gain * p.gain
      env.gain.setValueAtTime(0, start)
      env.gain.linearRampToValueAtTime(peak, start + timbre.attack)
      // Exponentielles Abklingen: nach `decay` Sekunden ≈ −60 dB.
      env.gain.setTargetAtTime(0, start + timbre.attack, decay / 6.9)
      osc.connect(env)
      env.connect(tone)
      const stop = start + timbre.attack + decay + 0.05
      osc.start(start)
      osc.stop(stop)
      end = Math.max(end, stop)
    }
  }

  for (const nb of recipe.noise ?? []) {
    const start = when + nb.at
    const src = ctx.createBufferSource()
    src.buffer = noiseBuffer(ctx)
    const band = ctx.createBiquadFilter()
    band.type = 'bandpass'
    band.Q.value = nb.q
    band.frequency.setValueAtTime(nb.from, start)
    band.frequency.exponentialRampToValueAtTime(nb.to, start + nb.dur)
    const env = ctx.createGain()
    // Weich ein- und ausblenden (Whoosh), bei sehr kurzen Bursts wie ein Klick.
    env.gain.setValueAtTime(0, start)
    env.gain.linearRampToValueAtTime(nb.gain, start + Math.min(0.004 + nb.dur * 0.35, nb.dur / 2))
    env.gain.linearRampToValueAtTime(0, start + nb.dur)
    src.connect(band)
    band.connect(env)
    env.connect(tone)
    src.start(start)
    src.stop(start + nb.dur + 0.02)
    end = Math.max(end, start + nb.dur + 0.02)
  }

  // Aufräumen nach Ausklingen inkl. Hallfahne.
  const tail = end + (reverb ? 1.3 : 0.05)
  const marker = ctx.createConstantSource()
  marker.offset.value = 0
  marker.connect(out)
  marker.onended = () => {
    out.disconnect()
    reverb?.disconnect()
  }
  marker.start(when)
  marker.stop(tail)
}
