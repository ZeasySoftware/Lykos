import { useCallback, useEffect, useRef, useState } from 'react'
import type { CSSProperties } from 'react'
import { Mic, Square, Video, VideoOff, Volume2 } from 'lucide-react'
import {
  LEVEL_FLOOR_DB,
  MAX_VOLUME,
  PTT_RELEASE_MAX_MS,
  SOUND_OUTPUT_FOLLOWS_VOICE,
  dbToFraction,
} from '../lib/audioLogic'
import { globalPttAvailable, isGlobalShortcutKey, pauseGlobalPtt } from '../lib/globalPtt'
import { suspendEscape } from '../lib/layers'
import { playTestTone, startMicTest, useMicLevel, useMicLevelReporting } from '../lib/micChain'
import type { MicTest } from '../lib/micChain'
import { setCapturingKey, useVoice } from '../lib/voice'
import { SCREEN_QUALITIES } from '../lib/voiceSettings'
import type { InputMode, ScreenQuality, VoiceSettings } from '../lib/voiceSettings'
import { SettingsSection } from './SettingsLayer'
import { SwitchRow } from './Switch'

const canPickOutput = typeof HTMLMediaElement !== 'undefined' && 'setSinkId' in HTMLMediaElement.prototype

/** Einstellungs-Tab „Sprache & Video“. Alles bleibt lokal im Browser (localStorage). */
export function VoiceSettingsTab() {
  const settings = useVoice((s) => s.settings)
  const update = useVoice((s) => s.updateSettings)
  const [inputs, setInputs] = useState<MediaDeviceInfo[]>([])
  const [outputs, setOutputs] = useState<MediaDeviceInfo[]>([])
  const [cameras, setCameras] = useState<MediaDeviceInfo[]>([])
  const [needsPermission, setNeedsPermission] = useState(false)
  const [capturing, setCapturing] = useState(false)

  const refresh = useCallback(() => {
    if (!navigator.mediaDevices?.enumerateDevices) return Promise.resolve()
    return navigator.mediaDevices.enumerateDevices().then((devices) => {
      setInputs(devices.filter((d) => d.kind === 'audioinput'))
      setOutputs(devices.filter((d) => d.kind === 'audiooutput'))
      setCameras(devices.filter((d) => d.kind === 'videoinput'))
      // Ohne Mikrofon-Freigabe liefern Browser keine Gerätenamen.
      setNeedsPermission(devices.some((d) => d.kind === 'audioinput' && !d.label))
    })
  }, [])

  useEffect(() => {
    const onChange = () => void refresh()
    onChange()
    navigator.mediaDevices?.addEventListener?.('devicechange', onChange)
    return () => navigator.mediaDevices?.removeEventListener?.('devicechange', onChange)
  }, [refresh])

  async function requestPermission() {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
      for (const t of stream.getTracks()) t.stop()
    } catch {
      // Abgelehnt – Liste bleibt ohne Namen.
    }
    await refresh()
  }

  // Taste für Push-to-Talk belegen.
  useEffect(() => {
    if (!capturing) return
    setCapturingKey(true)
    suspendEscape(true)
    pauseGlobalPtt(true)
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault()
      e.stopPropagation()
      if (e.code !== 'Escape') {
        update({ pttKey: e.code, pttKeyLabel: keyLabel(e) })
      }
      setCapturing(false)
    }
    window.addEventListener('keydown', onKey, true)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      setCapturingKey(false)
      suspendEscape(false)
      pauseGlobalPtt(false)
    }
  }, [capturing, update])

  return (
    <div className="form">

      {needsPermission && (
        <p className="notice">
          Für Gerätenamen braucht der Browser einmal Zugriff auf das Mikrofon.{' '}
          <button type="button" className="link-button" onClick={() => void requestPermission()}>
            Zugriff erlauben
          </button>
        </p>
      )}

      <SettingsSection title="Mikrofon">
      <div className="field-row">
        <label className="field">
          <span>Eingabegerät</span>
          <select value={settings.inputDeviceId} onChange={(e) => update({ inputDeviceId: e.target.value })}>
            <option value="">Systemstandard</option>
            {inputs
              .filter((d) => d.deviceId && d.deviceId !== 'default')
              .map((d, i) => (
                <option key={d.deviceId} value={d.deviceId}>
                  {d.label || `Mikrofon ${i + 1}`}
                </option>
              ))}
          </select>
        </label>
        <VolumeField
          label="Eingangslautstärke"
          value={settings.inputVolume}
          onChange={(inputVolume) => update({ inputVolume })}
        />
      </div>
      <MicTestPanel settings={settings} />
      </SettingsSection>

      <SettingsSection title="Lautsprecher">
      <div className="field-row">
        <label className="field">
          <span>Ausgabegerät</span>
          <select
            value={settings.outputDeviceId}
            onChange={(e) => update({ outputDeviceId: e.target.value })}
            disabled={!canPickOutput}
            title={canPickOutput ? undefined : 'Dieser Browser unterstützt keine Auswahl des Ausgabegeräts.'}
          >
            <option value="">Systemstandard</option>
            {outputs
              .filter((d) => d.deviceId && d.deviceId !== 'default')
              .map((d, i) => (
                <option key={d.deviceId} value={d.deviceId}>
                  {d.label || `Lautsprecher ${i + 1}`}
                </option>
              ))}
          </select>
        </label>
        <VolumeField
          label="Ausgabelautstärke"
          value={settings.outputVolume}
          onChange={(outputVolume) => update({ outputVolume })}
        />
      </div>
      <div className="field-row">
        <label className="field">
          <span>Gerät für Benachrichtigungstöne</span>
          <select
            value={settings.soundOutputDeviceId}
            onChange={(e) => update({ soundOutputDeviceId: e.target.value })}
            disabled={!canPickOutput}
            title={canPickOutput ? undefined : 'Dieser Browser unterstützt keine Auswahl des Ausgabegeräts.'}
          >
            <option value={SOUND_OUTPUT_FOLLOWS_VOICE}>Wie Ausgabegerät</option>
            <option value="">Systemstandard</option>
            {outputs
              .filter((d) => d.deviceId && d.deviceId !== 'default')
              .map((d, i) => (
                <option key={d.deviceId} value={d.deviceId}>
                  {d.label || `Lautsprecher ${i + 1}`}
                </option>
              ))}
          </select>
        </label>
        <div className="field">
          <span>Ausgabe prüfen</span>
          <div className="copy-row">
            <TestToneButton deviceId={settings.outputDeviceId} volume={settings.outputVolume} />
          </div>
        </div>
      </div>
      <p className="muted small">
        Die Lautstärke einzelner Teilnehmer (bis 200 %) stellst du per Rechtsklick auf sie im Sprachkanal ein.
      </p>
      </SettingsSection>

      <SettingsSection title="Kamera">
      <CameraSettings
        cameras={cameras}
        deviceId={settings.videoDeviceId}
        onChange={(videoDeviceId) => update({ videoDeviceId })}
        onAccess={refresh}
      />
      </SettingsSection>

      <SettingsSection title="Eingabemodus">
      <fieldset className="field">
        <legend className="sr-only">Eingabemodus</legend>
        <div className="radio-group">
          {(
            [
              ['voice', 'Sprachaktivierung'],
              ['ptt', 'Push-to-Talk'],
            ] as [InputMode, string][]
          ).map(([mode, label]) => (
            <label key={mode}>
              <input
                type="radio"
                name="input-mode"
                checked={settings.inputMode === mode}
                onChange={() => update({ inputMode: mode })}
              />
              {label}
            </label>
          ))}
        </div>
      </fieldset>

      {settings.inputMode === 'voice' && (
        <ThresholdField value={settings.vadThreshold} onChange={(vadThreshold) => update({ vadThreshold })} />
      )}

      {settings.inputMode === 'ptt' && (
        <div className="field">
          <span>Push-to-Talk-Taste</span>
          <div className="copy-row">
            <kbd className="key-badge">{capturing ? 'Taste drücken …' : settings.pttKeyLabel}</kbd>
            <button type="button" className="button" onClick={() => setCapturing((c) => !c)}>
              {capturing ? 'Abbrechen' : 'Ändern'}
            </button>
          </div>
          {globalPttAvailable ? (
            <>
              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={settings.pttGlobal}
                  onChange={(e) => update({ pttGlobal: e.target.checked })}
                />
                Auch wenn Lykos nicht im Vordergrund ist (global)
              </label>
              {settings.pttGlobal && !isGlobalShortcutKey(settings.pttKey) ? (
                <p className="notice small">
                  Diese Taste kann nicht global belegt werden (z. B. Strg, Umschalt, Alt) – sie wirkt nur bei aktivem
                  Fenster. Geeignet sind etwa F13–F24, Pause, Rollen oder ^.
                </p>
              ) : (
                <p className="muted small">
                  {settings.pttGlobal
                    ? 'Während eines Gesprächs gehört die Taste systemweit Lykos – andere Programme bekommen sie dann nicht. Am besten eine sonst ungenutzte Taste wählen.'
                    : 'Wirkt nur, solange das Lykos-Fenster im Vordergrund ist.'}
                </p>
              )}
            </>
          ) : (
            <p className="muted small">
              Funktioniert im Browser nur, solange das Lykos-Fenster im Vordergrund ist. Beim Tippen in Textfeldern
              lösen nur Sondertasten (z. B. F-Tasten) aus. Die Desktop-App kann die Taste auch global belegen.
            </p>
          )}
        </div>
      )}

      {settings.inputMode === 'ptt' && (
        <label className="field">
          <span>
            Loslassverzögerung <span className="muted small">({settings.pttReleaseDelay} ms)</span>
          </span>
          <input
            className="voice-range"
            type="range"
            min={0}
            max={PTT_RELEASE_MAX_MS}
            step={20}
            value={settings.pttReleaseDelay}
            onChange={(e) => update({ pttReleaseDelay: Number(e.target.value) })}
            aria-label="Push-to-Talk-Loslassverzögerung in Millisekunden"
          />
          <p className="muted small">
            So lange bleibt das Mikrofon nach dem Loslassen der Taste noch offen – damit Wortenden nicht abgeschnitten
            werden.
          </p>
        </label>
      )}

      </SettingsSection>

      <SettingsSection title="Audioverarbeitung">
        <SwitchRow
          title="Rauschunterdrückung"
          description="Filtert gleichmäßige Hintergrundgeräusche wie Lüfter oder Tastatur heraus."
          checked={settings.noiseSuppression}
          onChange={(v) => update({ noiseSuppression: v })}
        />
        <SwitchRow
          title="Echounterdrückung"
          description="Verhindert, dass andere sich selbst hören, wenn du ohne Kopfhörer sprichst."
          checked={settings.echoCancellation}
          onChange={(v) => update({ echoCancellation: v })}
        />
        <SwitchRow
          title="Automatische Lautstärkeanpassung"
          description="Gleicht leises und lautes Sprechen automatisch aus."
          checked={settings.autoGainControl}
          onChange={(v) => update({ autoGainControl: v })}
        />
      </SettingsSection>

      <SettingsSection title="Bildschirmübertragung">
      <label className="field">
        <span>Qualität</span>
        <select
          value={settings.screenQuality}
          onChange={(e) => update({ screenQuality: e.target.value as ScreenQuality })}
        >
          {(Object.keys(SCREEN_QUALITIES) as ScreenQuality[]).map((q) => (
            <option key={q} value={q}>
              {SCREEN_QUALITIES[q].label}
            </option>
          ))}
        </select>
      </label>
      </SettingsSection>
      <p className="muted small">Einstellungen werden nur auf diesem Gerät gespeichert.</p>
    </div>
  )
}

/** Kamera-Auswahl mit Live-Vorschau. Die Vorschau startet erst auf Klick (Kameralicht). */
function CameraSettings({
  cameras,
  deviceId,
  onChange,
  onAccess,
}: {
  cameras: MediaDeviceInfo[]
  deviceId: string
  onChange: (deviceId: string) => void
  /** Nach erteilter Freigabe: Geräteliste neu laden (jetzt mit Namen). Muss stabil sein. */
  onAccess: () => unknown
}) {
  const [preview, setPreview] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const videoRef = useRef<HTMLVideoElement>(null)

  useEffect(() => {
    if (!preview) return
    let stream: MediaStream | null = null
    let cancelled = false
    navigator.mediaDevices
      .getUserMedia({
        video: {
          deviceId: deviceId ? { exact: deviceId } : undefined,
          width: { ideal: 640 },
          height: { ideal: 360 },
        },
      })
      .then((s) => {
        if (cancelled) {
          for (const t of s.getTracks()) t.stop()
          return
        }
        stream = s
        if (videoRef.current) videoRef.current.srcObject = s
        void onAccess()
      })
      .catch(() => {
        if (!cancelled) setError('Kein Zugriff auf die Kamera.')
      })
    return () => {
      cancelled = true
      for (const t of stream?.getTracks() ?? []) t.stop()
    }
  }, [preview, deviceId, onAccess])

  return (
    <div className="field camera-settings">
      <span>Kamera</span>
      <div className="camera-settings-row">
        <select
          value={deviceId}
          onChange={(e) => {
            setError(null)
            onChange(e.target.value)
          }}
          aria-label="Kamera"
        >
          <option value="">Systemstandard</option>
          {cameras
            .filter((d) => d.deviceId)
            .map((d, i) => (
              <option key={d.deviceId} value={d.deviceId}>
                {d.label || `Kamera ${i + 1}`}
              </option>
            ))}
        </select>
        <button type="button" className="button" onClick={() => {
            setError(null)
            setPreview((p) => !p)
          }} aria-pressed={preview}>
          {preview ? <VideoOff size={16} aria-hidden="true" /> : <Video size={16} aria-hidden="true" />}
          {preview ? 'Vorschau beenden' : 'Vorschau'}
        </button>
      </div>
      {preview && (
        <div className="camera-preview">
          <video ref={videoRef} autoPlay playsInline muted />
          {error && <div className="camera-preview-error error small">{error}</div>}
        </div>
      )}
    </div>
  )
}

/** Lautstärkeregler 0–200 % (100 % = unverändert). */
function VolumeField({ label, value, onChange }: { label: string; value: number; onChange: (v: number) => void }) {
  const percent = Math.round(value * 100)
  const max = MAX_VOLUME * 100
  return (
    <label className="field">
      <span>
        {label} <span className="muted small">({percent} %)</span>
      </span>
      <input
        className="voice-range"
        type="range"
        min={0}
        max={max}
        step={1}
        value={percent}
        style={{ '--fill': `${(percent / max) * 100}%` } as CSSProperties}
        onChange={(e) => onChange(Number(e.target.value) / 100)}
        onDoubleClick={() => onChange(1)}
        aria-label={label}
        title="Doppelklick: 100 %"
      />
    </label>
  )
}

/** Pegelanzeige des Mikrofons (dBFS), optional mit Markierung des Schwellwerts. */
function LevelMeter({ threshold }: { threshold?: number }) {
  useMicLevelReporting()
  const level = useMicLevel((s) => s.level)
  const open = useMicLevel((s) => s.open)
  const active = level !== null
  const gated = threshold !== undefined && threshold > LEVEL_FLOOR_DB
  return (
    <div
      className={`level-meter${active ? '' : ' idle'}${active && gated && !open ? ' below' : ''}`}
      role="meter"
      aria-label="Mikrofonpegel"
      aria-valuemin={LEVEL_FLOOR_DB}
      aria-valuemax={0}
      aria-valuenow={Math.round(level ?? LEVEL_FLOOR_DB)}
    >
      <div className="level-meter-fill" style={{ width: `${dbToFraction(level ?? LEVEL_FLOOR_DB) * 100}%` }} />
      {gated && <div className="level-meter-threshold" style={{ left: `${dbToFraction(threshold) * 100}%` }} />}
    </div>
  )
}

/** Schwellwert der Sprachaktivierung, direkt unter der Pegelanzeige. */
function ThresholdField({ value, onChange }: { value: number; onChange: (v: number) => void }) {
  const level = useMicLevel((s) => s.level)
  const always = value <= LEVEL_FLOOR_DB
  return (
    <div className="field">
      <span>
        Empfindlichkeit der Sprachaktivierung{' '}
        <span className="muted small">({always ? 'immer an' : `${value} dB`})</span>
      </span>
      <div className="threshold-field">
        <LevelMeter threshold={value} />
        <input
          className="voice-range threshold-range"
          type="range"
          min={LEVEL_FLOOR_DB}
          max={0}
          step={1}
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
          aria-label="Schwellwert der Sprachaktivierung in dB"
        />
      </div>
      <p className="muted small">
        {level === null
          ? 'Starte oben den Mikrofontest (oder tritt einem Sprachkanal bei), um deinen Pegel zu sehen.'
          : 'Grün: du wirst übertragen. Schieb den Regler knapp über dein Hintergrundrauschen.'}{' '}
        Ganz links = Mikrofon immer offen.
      </p>
    </div>
  )
}

/** Mikrofontest: eigene Stimme mit allen Einstellungen über das Ausgabegerät hören. */
function MicTestPanel({ settings }: { settings: VoiceSettings }) {
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const testRef = useRef<MicTest | null>(null)

  // Neustart bei Gerät/Verarbeitung, sonst nur live nachführen.
  const { inputDeviceId, outputDeviceId, noiseSuppression, echoCancellation, autoGainControl } = settings
  useEffect(() => {
    if (!running) return
    let cancelled = false
    // Aktuelle Werte; Lautstärken/Schwellwert führt der zweite Effekt live nach.
    const s = useVoice.getState().settings
    startMicTest({
      constraints: {
        deviceId: inputDeviceId ? { exact: inputDeviceId } : undefined,
        noiseSuppression,
        echoCancellation,
        autoGainControl,
      },
      outputDeviceId,
      inputVolume: s.inputVolume,
      outputVolume: s.outputVolume,
      gateEnabled: s.inputMode === 'voice',
      threshold: s.vadThreshold,
    })
      .then((test) => {
        if (cancelled) test.stop()
        else testRef.current = test
      })
      .catch(() => {
        if (cancelled) return
        setError('Kein Zugriff auf das Mikrofon.')
        setRunning(false)
      })
    return () => {
      cancelled = true
      testRef.current?.stop()
      testRef.current = null
    }
  }, [running, inputDeviceId, outputDeviceId, noiseSuppression, echoCancellation, autoGainControl])

  useEffect(() => {
    testRef.current?.update({
      inputVolume: settings.inputVolume,
      outputVolume: settings.outputVolume,
      gateEnabled: settings.inputMode === 'voice',
      threshold: settings.vadThreshold,
    })
  }, [settings.inputVolume, settings.outputVolume, settings.inputMode, settings.vadThreshold])

  return (
    <div className="field mic-test">
      <span>Mikrofontest</span>
      <div className="mic-test-row">
        <button
          type="button"
          className="button"
          aria-pressed={running}
          onClick={() => {
            setError(null)
            setRunning((r) => !r)
          }}
        >
          {running ? <Square size={16} aria-hidden="true" /> : <Mic size={16} aria-hidden="true" />}
          {running ? 'Test beenden' : 'Lass uns prüfen, wie du klingst'}
        </button>
        <LevelMeter />
      </div>
      {error ? (
        <p className="error small">{error}</p>
      ) : (
        <p className="muted small">
          {running
            ? 'Sprich etwas – du hörst dich so, wie andere dich hören. Mit Kopfhörern testen, sonst kann es pfeifen.'
            : 'Spielt dein Mikrofon mit Lautstärke, Sprachaktivierung und Filtern direkt wieder ab.'}
        </p>
      )}
    </div>
  )
}

/** Kurzer Testton auf dem gewählten Ausgabegerät. */
function TestToneButton({ deviceId, volume }: { deviceId: string; volume: number }) {
  const [playing, setPlaying] = useState(false)
  return (
    <button
      type="button"
      className="button"
      disabled={playing}
      onClick={() => {
        setPlaying(true)
        void playTestTone(deviceId, volume)
          .catch(() => {})
          .finally(() => setPlaying(false))
      }}
    >
      <Volume2 size={16} aria-hidden="true" />
      {playing ? 'Spielt …' : 'Testton abspielen'}
    </button>
  )
}

function keyLabel(e: KeyboardEvent): string {
  if (e.key === ' ') return 'Leertaste'
  if (e.key.length === 1) return e.key.toUpperCase()
  return e.key
}
