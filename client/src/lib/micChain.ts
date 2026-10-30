/**
 * Mikrofon-Kette per Web Audio: Quelle → Eingangslautstärke (GainNode, bis 200 %) → Sprachaktivierungs-
 * Gate mit Pegelmessung (AudioWorklet, siehe `voiceGate.worklet.ts`) → Ausgang.
 *
 * Genutzt von
 * - `createMicProcessor`: LiveKit-`TrackProcessor` für die gesendete Mikrofonspur im Call,
 * - `startMicTest`: Mikrofontest in den Einstellungen mit Wiedergabe über das Ausgabegerät (Loopback),
 * - `playTestTone`: Testton für das Ausgabegerät.
 *
 * Der aktuelle Pegel (dBFS) liegt im kleinen Store `useMicLevel` (für Pegelanzeige und Schwellwert-Regler).
 * Gemeldet wird er nur, solange eine Anzeige `useMicLevelReporting()` nutzt – sonst schickt das Worklet im
 * Call nicht 20×/s Nachrichten an den Haupt-Thread, die niemand liest.
 * Lässt sich das AudioWorklet nicht laden, fehlen Gate und Pegel – die Eingangslautstärke wirkt trotzdem.
 */
import { useEffect } from 'react'
import { create } from 'zustand'
import type { AudioProcessorOptions, Track, TrackProcessor } from 'livekit-client'
import { clampVolume } from './audioLogic'
import gateWorkletUrl from './voiceGate.worklet.ts?worker&url'

// ---------------------------------------------------------------------------
// Pegel-Store
// ---------------------------------------------------------------------------

interface MicLevel {
  /** Letzter Pegel in dBFS; `null`, solange keine Kette misst. */
  level: number | null
  /** Gate offen (Sprache wird übertragen). */
  open: boolean
}

export const useMicLevel = create<MicLevel>()(() => ({ level: null, open: false }))
let reporters = 0

/** Gate-Knoten aller laufenden Ketten. */
const gates = new Set<AudioWorkletNode>()
/** Gemountete Pegelanzeigen. */
let viewers = 0

/** In Komponenten aufrufen, die den Pegel anzeigen: Solange eine gemountet ist, melden die Worklets ihn. */
export function useMicLevelReporting() {
  useEffect(() => {
    viewers += 1
    if (viewers === 1) for (const gate of gates) gate.port.postMessage({ report: true })
    return () => {
      viewers -= 1
      if (viewers > 0) return
      for (const gate of gates) gate.port.postMessage({ report: false })
      useMicLevel.setState({ level: null, open: false })
    }
  }, [])
}

// ---------------------------------------------------------------------------
// Kette
// ---------------------------------------------------------------------------

type SinkContext = AudioContext & { setSinkId?: (id: string) => Promise<void> }

const workletLoaded = new WeakMap<BaseAudioContext, Promise<boolean>>()

function loadGateWorklet(ctx: BaseAudioContext): Promise<boolean> {
  let p = workletLoaded.get(ctx)
  if (!p) {
    p = ctx.audioWorklet
      ? ctx.audioWorklet.addModule(gateWorkletUrl).then(
          () => true,
          () => false,
        )
      : Promise.resolve(false)
    workletLoaded.set(ctx, p)
  }
  return p
}

export interface MicChain {
  /** Ausgang der Kette (zum Weiterverbinden). */
  output: AudioNode
  setGain: (volume: number) => void
  /** Gate an (Sprachaktivierung, mit Schwellwert in dBFS) oder aus (immer offen, z. B. Push-to-Talk). */
  setGate: (enabled: boolean, thresholdDb: number) => void
  dispose: () => void
}

export interface MicChainOptions {
  gain: number
  gateEnabled: boolean
  threshold: number
  /** Kanalzahl des Ausgangs (2 für Stereo-Kanäle). */
  channels?: number
}

export async function createMicChain(ctx: AudioContext, source: AudioNode, opts: MicChainOptions): Promise<MicChain> {
  const channels = opts.channels ?? 1
  const gain = ctx.createGain()
  gain.channelCount = channels
  gain.channelCountMode = 'explicit'
  gain.gain.value = clampVolume(opts.gain)
  source.connect(gain)

  let gate: AudioWorkletNode | null = null
  if (await loadGateWorklet(ctx)) {
    try {
      gate = new AudioWorkletNode(ctx, 'lykos-voice-gate', {
        numberOfInputs: 1,
        numberOfOutputs: 1,
        outputChannelCount: [channels],
      })
    } catch {
      gate = null
    }
  }

  let reporting = false
  if (gate) {
    gain.connect(gate)
    gate.port.postMessage({ enabled: opts.gateEnabled, threshold: opts.threshold, report: viewers > 0 })
    gates.add(gate)
    reporting = true
    reporters += 1
    gate.port.onmessage = (e: MessageEvent<{ level: number; open: boolean }>) => {
      useMicLevel.setState({ level: e.data.level, open: e.data.open })
    }
  }
  const output: AudioNode = gate ?? gain

  return {
    output,
    setGain(volume) {
      gain.gain.setTargetAtTime(clampVolume(volume), ctx.currentTime, 0.02)
    },
    setGate(enabled, threshold) {
      gate?.port.postMessage({ enabled, threshold })
    },
    dispose() {
      try {
        source.disconnect(gain)
      } catch {
        // schon getrennt
      }
      gain.disconnect()
      if (gate) {
        gate.port.onmessage = null
        gate.disconnect()
        gates.delete(gate)
      }
      if (reporting) {
        reporting = false
        reporters -= 1
        if (reporters <= 0) {
          reporters = 0
          useMicLevel.setState({ level: null, open: false })
        }
      }
    },
  }
}

// ---------------------------------------------------------------------------
// LiveKit-Prozessor für die Mikrofonspur
// ---------------------------------------------------------------------------

export interface MicProcessorSettings {
  gain: number
  gateEnabled: boolean
  threshold: number
  stereo: boolean
}

export type MicProcessor = TrackProcessor<Track.Kind.Audio, AudioProcessorOptions> & {
  update: (s: MicProcessorSettings) => void
}

/** Prozessor, der Eingangslautstärke und Sprachaktivierung vor dem Senden anwendet. */
export function createMicProcessor(initial: MicProcessorSettings): MicProcessor {
  let settings = initial
  let chain: MicChain | null = null
  let source: MediaStreamAudioSourceNode | null = null
  let dest: MediaStreamAudioDestinationNode | null = null

  function teardown() {
    chain?.dispose()
    source?.disconnect()
    for (const t of dest?.stream.getTracks() ?? []) t.stop()
    chain = null
    source = null
    dest = null
  }

  const processor: MicProcessor = {
    name: 'lykos-mic',
    async init({ track, audioContext }) {
      teardown()
      const channels = settings.stereo ? 2 : 1
      source = audioContext.createMediaStreamSource(new MediaStream([track]))
      dest = audioContext.createMediaStreamDestination()
      dest.channelCount = channels
      chain = await createMicChain(audioContext, source, {
        gain: settings.gain,
        gateEnabled: settings.gateEnabled,
        threshold: settings.threshold,
        channels,
      })
      chain.output.connect(dest)
      processor.processedTrack = dest.stream.getAudioTracks()[0]
      if (audioContext.state !== 'running') void audioContext.resume().catch(() => {})
    },
    async restart(opts) {
      await processor.init(opts)
    },
    async destroy() {
      teardown()
      processor.processedTrack = undefined
    },
    update(next) {
      settings = next
      chain?.setGain(next.gain)
      chain?.setGate(next.gateEnabled, next.threshold)
    },
  }
  return processor
}

// ---------------------------------------------------------------------------
// Ausgabe: Kontext auf einem bestimmten Gerät
// ---------------------------------------------------------------------------

/** Neuer `AudioContext`, der auf `sinkId` ausgibt (`''` = Systemstandard), soweit der Browser das kann. */
export function createOutputContext(sinkId: string): SinkContext {
  const ctx: SinkContext = new AudioContext({ latencyHint: 'interactive' })
  if (sinkId && typeof ctx.setSinkId === 'function') void ctx.setSinkId(sinkId).catch(() => {})
  return ctx
}

// ---------------------------------------------------------------------------
// Mikrofontest (Loopback) & Testton
// ---------------------------------------------------------------------------

export interface MicTestOptions {
  constraints: MediaTrackConstraints
  outputDeviceId: string
  inputVolume: number
  outputVolume: number
  gateEnabled: boolean
  threshold: number
}

export interface MicTest {
  update: (o: Pick<MicTestOptions, 'inputVolume' | 'outputVolume' | 'gateEnabled' | 'threshold'>) => void
  stop: () => void
}

/**
 * Mikrofon aufnehmen, durch dieselbe Kette wie im Call schicken und über das Ausgabegerät wiedergeben.
 * Wirft, wenn kein Zugriff auf das Mikrofon besteht.
 */
export async function startMicTest(o: MicTestOptions): Promise<MicTest> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: o.constraints })
  const ctx = createOutputContext(o.outputDeviceId)
  try {
    if (ctx.state !== 'running') await ctx.resume().catch(() => {})
    const source = ctx.createMediaStreamSource(stream)
    const chain = await createMicChain(ctx, source, {
      gain: o.inputVolume,
      gateEnabled: o.gateEnabled,
      threshold: o.threshold,
    })
    const out = ctx.createGain()
    out.gain.value = clampVolume(o.outputVolume)
    chain.output.connect(out)
    out.connect(ctx.destination)
    let stopped = false
    return {
      update(next) {
        chain.setGain(next.inputVolume)
        chain.setGate(next.gateEnabled, next.threshold)
        out.gain.setTargetAtTime(clampVolume(next.outputVolume), ctx.currentTime, 0.02)
      },
      stop() {
        if (stopped) return
        stopped = true
        chain.dispose()
        out.disconnect()
        for (const t of stream.getTracks()) t.stop()
        void ctx.close().catch(() => {})
      },
    }
  } catch (e) {
    for (const t of stream.getTracks()) t.stop()
    void ctx.close().catch(() => {})
    throw e
  }
}

/** Kurzer Zweiklang (ca. 1,2 s) auf dem Ausgabegerät in der eingestellten Ausgabelautstärke. */
export async function playTestTone(outputDeviceId: string, volume: number): Promise<void> {
  const ctx = createOutputContext(outputDeviceId)
  const level = 0.18 * clampVolume(volume)
  const start = ctx.currentTime + 0.05
  const notes = [
    { hz: 523.25, at: 0 },
    { hz: 659.25, at: 0.35 },
    { hz: 783.99, at: 0.7 },
  ]
  for (const n of notes) {
    const osc = ctx.createOscillator()
    osc.type = 'sine'
    osc.frequency.value = n.hz
    const env = ctx.createGain()
    const t = start + n.at
    env.gain.setValueAtTime(0, t)
    env.gain.linearRampToValueAtTime(level, t + 0.02)
    env.gain.exponentialRampToValueAtTime(0.0001, t + 0.45)
    osc.connect(env).connect(ctx.destination)
    osc.start(t)
    osc.stop(t + 0.5)
  }
  void ctx.resume().catch(() => {})
  return new Promise((resolve) => {
    setTimeout(() => {
      void ctx.close().catch(() => {})
      resolve()
    }, 1400)
  })
}
