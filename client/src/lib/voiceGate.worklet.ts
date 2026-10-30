/**
 * AudioWorklet „lykos-voice-gate“: misst den Pegel des Mikrofons (nach der Eingangslautstärke) und
 * schließt bei Sprachaktivierung das Signal unter dem Schwellwert (weiche Rampe, Haltezeit siehe
 * `stepGate`). Läuft im Audio-Thread, damit es auch bei gedrosselten Hintergrund-Tabs zuverlässig ist.
 *
 * Nachrichten an den Port: `{ threshold?: number (dBFS), enabled?: boolean, report?: boolean }`.
 * Nachrichten vom Port (~20×/s, nur mit `report: true`, d. h. solange eine Pegelanzeige sichtbar ist):
 * `{ level: number (dBFS, Spitze seit der letzten Meldung), open: boolean }`.
 *
 * Wird von Vite als eigenes Bundle gebaut (`?worker&url`) und per `audioWorklet.addModule` geladen.
 */
import { GATE_INITIAL, LEVEL_FLOOR_DB, rmsToDb, stepGate } from './audioLogic.ts'
import type { GateState } from './audioLogic.ts'

// Globale Namen des AudioWorkletGlobalScope (nicht in der DOM-Bibliothek von TypeScript).
declare const sampleRate: number
declare function registerProcessor(name: string, ctor: unknown): void
declare class AudioWorkletProcessor {
  readonly port: MessagePort
}

/** Abstand der Pegelmeldungen in ms. */
const REPORT_MS = 50
/** Zeitkonstanten der Rampe (öffnen schnell, schließen sanft) in Sekunden. */
const ATTACK_S = 0.005
const RELEASE_S = 0.04

class VoiceGateProcessor extends AudioWorkletProcessor {
  private threshold = LEVEL_FLOOR_DB
  private enabled = false
  /** Pegel melden? Nur solange eine Anzeige ihn braucht – sonst weckt jede Meldung den Haupt-Thread. */
  private report = false
  private gate: GateState = GATE_INITIAL
  private gain = 1
  private time = 0
  private peak = LEVEL_FLOOR_DB
  private sinceReport = 0
  private readonly attack = 1 - Math.exp(-1 / (ATTACK_S * sampleRate))
  private readonly release = 1 - Math.exp(-1 / (RELEASE_S * sampleRate))

  constructor() {
    super()
    this.port.onmessage = (e: MessageEvent<{ threshold?: number; enabled?: boolean; report?: boolean }>) => {
      if (typeof e.data.threshold === 'number') this.threshold = e.data.threshold
      if (typeof e.data.enabled === 'boolean') this.enabled = e.data.enabled
      if (typeof e.data.report === 'boolean') this.report = e.data.report
    }
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const input = inputs[0] ?? []
    const output = outputs[0] ?? []
    const frames = output[0]?.length ?? 128

    let sum = 0
    let count = 0
    for (const ch of input) {
      for (let i = 0; i < ch.length; i++) sum += ch[i] * ch[i]
      count += ch.length
    }
    const level = rmsToDb(count > 0 ? Math.sqrt(sum / count) : 0)
    this.time += (frames / sampleRate) * 1000
    this.gate = stepGate(this.gate, level, this.enabled ? this.threshold : LEVEL_FLOOR_DB, this.time)
    const target = this.gate.open ? 1 : 0
    const coef = target > this.gain ? this.attack : this.release

    const startGain = this.gain
    for (let c = 0; c < output.length; c++) {
      const out = output[c]
      const src = input[c] ?? input[0]
      let g = startGain
      for (let i = 0; i < out.length; i++) {
        g += (target - g) * coef
        out[i] = src ? src[i] * g : 0
      }
      if (c === output.length - 1) this.gain = g
    }

    this.peak = Math.max(this.peak, level)
    this.sinceReport += (frames / sampleRate) * 1000
    if (this.sinceReport >= REPORT_MS) {
      if (this.report) this.port.postMessage({ level: this.peak, open: this.gate.open })
      this.peak = LEVEL_FLOOR_DB
      this.sinceReport = 0
    }
    return true
  }
}

registerProcessor('lykos-voice-gate', VoiceGateProcessor)
