import { Play } from 'lucide-react'
import { SOUND_GROUPS, SOUND_INFO, SOUND_NAMES } from '../../lib/soundLogic'
import { playSound, useSoundSettings } from '../../lib/sounds'

/** Einstellungs-Tab „Sounds“: Hauptschalter, Gesamtlautstärke, Ereignisse mit Vorhören. Nur lokal. */
export function SoundSettings() {
  const settings = useSoundSettings((s) => s.settings)
  const setEnabled = useSoundSettings((s) => s.setEnabled)
  const setVolume = useSoundSettings((s) => s.setVolume)
  const setEvent = useSoundSettings((s) => s.setEvent)
  const reset = useSoundSettings((s) => s.reset)
  const percent = Math.round(settings.volume * 100)

  return (
    <div className="form sound-settings">

      <label className="checkbox">
        <input type="checkbox" checked={settings.enabled} onChange={(e) => setEnabled(e.target.checked)} />
        Soundeffekte
      </label>

      <label className="field">
        <span>
          Lautstärke <span className="muted small">({percent} %)</span>
        </span>
        <div className="sound-volume">
          <input
            type="range"
            min={0}
            max={100}
            step={1}
            value={percent}
            onChange={(e) => setVolume(Number(e.target.value) / 100)}
            // Beim Loslassen kurz anspielen, damit man die neue Lautstärke hört.
            onPointerUp={() => playSound('message', { preview: true })}
            onKeyUp={() => playSound('message', { preview: true })}
            aria-label="Lautstärke der Soundeffekte"
          />
        </div>
      </label>

      <div className={settings.enabled ? 'sound-groups' : 'sound-groups disabled'}>
        {SOUND_GROUPS.map((group) => (
          <section key={group.id} className="sound-group">
            <h4>{group.label}</h4>
            <ul className="sound-list">
              {SOUND_NAMES.filter((name) => SOUND_INFO[name].group === group.id).map((name) => (
                <li key={name} className="sound-row">
                  <label className="checkbox">
                    <input
                      type="checkbox"
                      checked={settings.events[name]}
                      disabled={!settings.enabled}
                      onChange={(e) => setEvent(name, e.target.checked)}
                    />
                    {SOUND_INFO[name].label}
                  </label>
                  <button
                    type="button"
                    className="icon-button small"
                    onClick={() => playSound(name, { preview: true })}
                    aria-label={`Vorhören: ${SOUND_INFO[name].label}`}
                    title="Vorhören"
                  >
                    <Play size={14} aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>

      <p className="muted small">
        Die Töne laufen über das Ausgabegerät aus „Sprache &amp; Video“. „Ton aus“ im Sprachkanal schaltet nur das
        Gespräch stumm, Hinweistöne bleiben hörbar. Bei „Bitte nicht stören“ bleiben Nachrichtentöne aus;
        stummgeschaltete Kanäle nur bei Erwähnungen. Einstellungen werden nur auf diesem Gerät gespeichert.
      </p>
      <div className="form-actions">
        <button type="button" className="button ghost" onClick={reset}>
          Standard wiederherstellen
        </button>
      </div>
    </div>
  )
}
