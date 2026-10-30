import { useEffect, useMemo, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { endpoints } from '../lib/api'
import type { AutoModRule, AutoModRuleKind, Channel, CommunityDetail, Role } from '../lib/api'
import { AUTOMOD_LIMITS, AUTOMOD_RULES, completeRules, formatList, invalidTerms, parseList, rulesEqual } from '../lib/automod'
import { errorText } from '../lib/errors'
import { roleColor } from '../lib/permissionLabels'
import { sortRoles } from '../lib/roles'
import { useUnsavedChanges } from '../lib/unsaved'
import { SettingsPage } from './SettingsLayer'
import { Switch, SwitchRow } from './Switch'
import { AtSign, Gauge, Hash, Link2, ShieldCheck, TextSearch, X } from 'lucide-react'

const ICONS: Record<AutoModRuleKind, ReactNode> = {
  keyword: <TextSearch size={20} />,
  links: <Link2 size={20} />,
  mentions: <AtSign size={20} />,
  spam: <Gauge size={20} />,
}

/**
 * Community-Einstellungen → AutoMod (`MANAGE_COMMUNITY`): eine Karte pro Regelart. Gespeichert wird über die
 * Unsaved-Leiste alles auf einmal (`PUT /communities/{id}/automod`).
 */
export function AutoModPage({ detail }: { detail: CommunityDetail }) {
  const communityId = detail.community.id
  const [saved, setSaved] = useState<AutoModRule[] | null>(null)
  const [draft, setDraft] = useState<AutoModRule[] | null>(null)
  // Zählt Zurücksetzen/Speichern: Karten werden dann neu aufgebaut (Textfelder übernehmen den Stand).
  const [version, setVersion] = useState(0)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  // Die Fehlermeldung steht unter allen Karten – beim Speichern über die Leiste sonst außer Sicht.
  const errorRef = useRef<HTMLParagraphElement>(null)
  useEffect(() => {
    if (error) errorRef.current?.scrollIntoView?.({ block: 'center', behavior: 'smooth' })
  }, [error])

  useEffect(() => {
    let alive = true
    endpoints
      .automod(communityId)
      .then((rules) => {
        if (!alive) return
        const complete = completeRules(rules)
        setSaved(complete)
        setDraft(complete)
      })
      .catch((e) => alive && setLoadError(errorText(e)))
    return () => {
      alive = false
    }
  }, [communityId])

  const textChannels = useMemo(() => detail.channels.filter((c) => c.kind === 'text'), [detail.channels])
  const roles = useMemo(() => sortRoles(detail.roles).filter((r) => !r.is_default), [detail.roles])
  const dirty = !!saved && !!draft && !rulesEqual(saved, draft)

  async function save() {
    if (!draft) return
    setBusy(true)
    setError(null)
    try {
      const rules = completeRules(await endpoints.updateAutomod(communityId, draft))
      setSaved(rules)
      setDraft(rules)
      setVersion((v) => v + 1)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  function reset() {
    setDraft(saved)
    setVersion((v) => v + 1)
    setError(null)
  }

  useUnsavedChanges({ dirty, busy, save, reset })

  function change(rule: AutoModRule) {
    setDraft((d) => d && d.map((r) => (r.kind === rule.kind ? rule : r)))
  }

  return (
    <SettingsPage
      title="AutoMod"
      description="Prüft Nachrichten automatisch beim Senden und Bearbeiten – in allen Kanälen dieser Community, nicht in Direktnachrichten."
    >
      <p className="automod-note">
        <ShieldCheck size={16} aria-hidden="true" />
        <span>
          Der Besitzer und Mitglieder mit „Administrator“ oder „Community verwalten“ sind immer ausgenommen.
        </span>
      </p>
      {loadError && <p className="error">{loadError}</p>}
      {!draft && !loadError && <p className="muted">Lädt …</p>}
      {draft &&
        AUTOMOD_RULES.map((meta) => {
          const rule = draft.find((r) => r.kind === meta.kind)
          if (!rule) return null
          return (
            <RuleCard
              key={`${meta.kind}-${version}`}
              meta={meta}
              rule={rule}
              onChange={change}
              textChannels={textChannels}
              roles={roles}
            />
          )
        })}
      {error && (
        <p className="error" ref={errorRef} role="alert">
          {error}
        </p>
      )}
    </SettingsPage>
  )
}

function RuleCard({
  meta,
  rule,
  onChange,
  textChannels,
  roles,
}: {
  meta: (typeof AUTOMOD_RULES)[number]
  rule: AutoModRule
  onChange: (rule: AutoModRule) => void
  textChannels: Channel[]
  roles: Role[]
}) {
  const titleId = `automod-${rule.kind}-title`
  return (
    <section className={`automod-card${rule.enabled ? ' enabled' : ''}`} aria-labelledby={titleId}>
      <header className="automod-card-header">
        <span className="automod-card-icon" aria-hidden="true">
          {ICONS[rule.kind]}
        </span>
        <div className="automod-card-heading">
          <h3 id={titleId}>{meta.title}</h3>
          <p className="settings-description">{meta.description}</p>
        </div>
        <Switch
          checked={rule.enabled}
          onChange={(enabled) => onChange({ ...rule, enabled })}
          label={`${meta.title} ${rule.enabled ? 'ausschalten' : 'einschalten'}`}
        />
      </header>
      {rule.enabled && (
        <div className="automod-card-body">
          <RuleConfigFields rule={rule} onChange={onChange} />
          <ActionFields rule={rule} onChange={onChange} textChannels={textChannels} defaultMessage={meta.blockMessage} />
          <div className="automod-exemptions">
            <ExemptPicker
              label="Ausgenommene Rollen"
              addLabel="Rolle hinzufügen …"
              items={roles.map((r) => ({ id: r.id, name: r.name, dot: roleColor(r.color) ?? 'var(--text-muted)' }))}
              selected={rule.exempt_role_ids}
              onChange={(exempt_role_ids) => onChange({ ...rule, exempt_role_ids })}
            />
            <ExemptPicker
              label="Ausgenommene Kanäle"
              addLabel="Kanal hinzufügen …"
              items={textChannels.map((c) => ({ id: c.id, name: c.name, channel: true }))}
              selected={rule.exempt_channel_ids}
              onChange={(exempt_channel_ids) => onChange({ ...rule, exempt_channel_ids })}
            />
          </div>
        </div>
      )}
    </section>
  )
}

/** Art-spezifische Einstellungen einer Regel. */
function RuleConfigFields({ rule, onChange }: { rule: AutoModRule; onChange: (rule: AutoModRule) => void }) {
  switch (rule.kind) {
    case 'keyword':
      return (
        <div className="automod-fields">
          <ListField
            label="Begriffe"
            hint={
              <>
                Ein Begriff pro Zeile. Ohne <code>*</code> zählen nur ganze Wörter; <code>spam*</code> = Wortanfang,{' '}
                <code>*spam</code> = Wortende, <code>*spam*</code> = irgendwo im Wort.
              </>
            }
            placeholder={'spam*\nböses wort'}
            values={rule.config.terms}
            max={AUTOMOD_LIMITS.terms}
            invalid={invalidTerms(rule.config.terms)}
            onChange={(terms) => onChange({ ...rule, config: { ...rule.config, terms } })}
          />
          <ListField
            label="Ausnahmen"
            hint={
              <>
                Ganze Wörter, die nie als Treffer zählen – z. B. „Klasse“ bei <code>*lass*</code>.
              </>
            }
            placeholder="klasse"
            rows={3}
            values={rule.config.allow}
            max={AUTOMOD_LIMITS.terms}
            invalid={invalidTerms(rule.config.allow, false)}
            onChange={(allow) => onChange({ ...rule, config: { ...rule.config, allow } })}
          />
        </div>
      )
    case 'links':
      return (
        <div className="automod-fields">
          <ListField
            label="Erlaubte Domains"
            hint="Eine Domain pro Zeile; Subdomains sind eingeschlossen (docs.example.org bei example.org). Leer = alle Links blockieren."
            placeholder={'example.org\nyoutube.com'}
            values={rule.config.allowed_domains}
            max={AUTOMOD_LIMITS.domains}
            onChange={(allowed_domains) => onChange({ ...rule, config: { ...rule.config, allowed_domains } })}
          />
          <SwitchRow
            title="Fremde Einladungslinks immer blockieren"
            description="Einladungen anderer Lykos-Instanzen und Discord-Einladungen – auch wenn ihre Domain erlaubt ist."
            checked={rule.config.block_foreign_invites}
            onChange={(block_foreign_invites) => onChange({ ...rule, config: { ...rule.config, block_foreign_invites } })}
          />
        </div>
      )
    case 'mentions':
      return (
        <div className="automod-fields">
          <label className="inline-field">
            Mehr als
            <input
              type="number"
              min={1}
              max={50}
              value={rule.config.max_mentions}
              onChange={(e) => onChange({ ...rule, config: { max_mentions: Number(e.target.value) } })}
            />
            erwähnte Mitglieder in einer Nachricht
          </label>
        </div>
      )
    case 'spam': {
      const c = rule.config
      const set = (patch: Partial<typeof c>) => onChange({ ...rule, config: { ...c, ...patch } })
      return (
        <div className="automod-fields">
          <SwitchRow
            title="Nachrichtenflut"
            description="Zu viele Nachrichten in kurzer Zeit (über alle Kanäle der Community)."
            checked={c.max_messages > 0}
            onChange={(on) => set({ max_messages: on ? 5 : 0 })}
          />
          {c.max_messages > 0 && (
            <div className="inline-field automod-inline">
              <label className="inline-field">
                Mehr als
                <input
                  type="number"
                  min={2}
                  max={50}
                  value={c.max_messages}
                  onChange={(e) => set({ max_messages: Number(e.target.value) })}
                  aria-label="Höchstzahl Nachrichten"
                />
                Nachrichten
              </label>
              <label className="inline-field">
                in
                <input
                  type="number"
                  min={1}
                  max={60}
                  value={c.per_seconds}
                  onChange={(e) => set({ per_seconds: Number(e.target.value) })}
                  aria-label="Zeitraum in Sekunden"
                />
                Sekunden
              </label>
            </div>
          )}
          <SwitchRow
            title="Wiederholungen"
            description="Derselbe Text mehrmals hintereinander."
            checked={c.max_duplicates > 0}
            onChange={(on) => set({ max_duplicates: on ? 3 : 0 })}
          />
          {c.max_duplicates > 0 && (
            <label className="inline-field">
              Mehr als
              <input
                type="number"
                min={2}
                max={20}
                value={c.max_duplicates}
                onChange={(e) => set({ max_duplicates: Number(e.target.value) })}
              />
              gleiche Nachrichten hintereinander
            </label>
          )}
        </div>
      )
    }
  }
}

/** Aktionen, Protokollkanal und eigene Blockiermeldung. */
function ActionFields({
  rule,
  onChange,
  textChannels,
  defaultMessage,
}: {
  rule: AutoModRule
  onChange: (rule: AutoModRule) => void
  textChannels: Channel[]
  defaultMessage: string
}) {
  const setAction = (patch: Partial<AutoModRule['actions']>) => onChange({ ...rule, actions: { ...rule.actions, ...patch } })
  const none = !rule.actions.block && !rule.actions.alert && !rule.actions.audit
  return (
    <fieldset className="automod-actions">
      <legend className="automod-subtitle">Wenn die Regel greift</legend>
      <label className="checkbox">
        <input type="checkbox" checked={rule.actions.block} onChange={(e) => setAction({ block: e.target.checked })} />
        <span>
          Nachricht blockieren
          <span className="automod-hint">Sie wird nicht gesendet; der Text bleibt im Eingabefeld.</span>
        </span>
      </label>
      {rule.actions.block && (
        <label className="field automod-indent">
          <span>Meldung an den Absender</span>
          <input
            value={rule.custom_message ?? ''}
            onChange={(e) => onChange({ ...rule, custom_message: e.target.value || null })}
            maxLength={AUTOMOD_LIMITS.customMessage}
            placeholder={defaultMessage}
          />
        </label>
      )}
      <label className="checkbox">
        <input type="checkbox" checked={rule.actions.alert} onChange={(e) => setAction({ alert: e.target.checked })} />
        <span>
          Alarm im Protokollkanal
          <span className="automod-hint">Mit Autor, Kanal, Regel und gekürztem Text.</span>
        </span>
      </label>
      {rule.actions.alert && (
        <label className="field automod-indent">
          <span>Protokollkanal</span>
          <select
            value={rule.alert_channel_id ?? ''}
            onChange={(e) => onChange({ ...rule, alert_channel_id: e.target.value || null })}
          >
            <option value="">Kanal wählen …</option>
            {textChannels.map((c) => (
              <option key={c.id} value={c.id}>
                #{c.name}
              </option>
            ))}
          </select>
          {!rule.alert_channel_id && <span className="automod-warning">Bitte einen Kanal wählen – am besten einen nur für Moderatoren.</span>}
        </label>
      )}
      <label className="checkbox">
        <input type="checkbox" checked={rule.actions.audit} onChange={(e) => setAction({ audit: e.target.checked })} />
        <span>
          Ins Audit-Log schreiben
          <span className="automod-hint">Ohne Nachrichteninhalt.</span>
        </span>
      </label>
      {none && <p className="automod-warning">Wähle mindestens eine Aktion.</p>}
    </fieldset>
  )
}

/**
 * Mehrzeiliges Textfeld für eine Liste. Hält den Rohtext selbst (Zeilenumbrüche beim Tippen bleiben) und meldet
 * die bereinigte Liste nach oben; bei Zurücksetzen/Speichern wird die Karte neu aufgebaut.
 */
function ListField({
  label,
  hint,
  placeholder,
  values,
  max,
  invalid = [],
  rows = 5,
  onChange,
}: {
  label: string
  hint: ReactNode
  placeholder: string
  values: string[]
  max: number
  invalid?: string[]
  rows?: number
  onChange: (values: string[]) => void
}) {
  const [text, setText] = useState(() => formatList(values))
  const over = values.length > max
  return (
    <label className="field automod-list">
      <span className="automod-list-label">
        {label}
        <span className={`automod-count${over ? ' over' : ''}`}>
          {values.length} / {max}
        </span>
      </span>
      <textarea
        value={text}
        rows={rows}
        placeholder={placeholder}
        spellCheck={false}
        onChange={(e) => {
          setText(e.target.value)
          onChange(parseList(e.target.value))
        }}
      />
      <span className="automod-hint">{hint}</span>
      {invalid.length > 0 && (
        <span className="automod-warning">
          Ungültig: {invalid.slice(0, 5).join(', ')}
          {invalid.length > 5 ? ' …' : ''}
        </span>
      )}
    </label>
  )
}

interface PickItem {
  id: string
  name: string
  /** Farbe des Rollenpunkts. */
  dot?: string
  channel?: boolean
}

/** Ausnahmen als Chips mit Entfernen-Knopf und Auswahlfeld zum Hinzufügen. */
function ExemptPicker({
  label,
  addLabel,
  items,
  selected,
  onChange,
}: {
  label: string
  addLabel: string
  items: PickItem[]
  selected: string[]
  onChange: (ids: string[]) => void
}) {
  const byId = new Map(items.map((i) => [i.id, i]))
  const chosen = selected.map((id) => byId.get(id)).filter((i): i is PickItem => !!i)
  const available = items.filter((i) => !selected.includes(i.id))
  return (
    <div className="field automod-exempt">
      <span>{label}</span>
      {chosen.length > 0 && (
        <ul className="automod-chips">
          {chosen.map((item) => (
            <li key={item.id} className="automod-chip">
              {item.channel ? (
                <Hash size={13} aria-hidden="true" />
              ) : (
                <span className="role-dot" style={{ background: item.dot }} aria-hidden="true" />
              )}
              <span className="truncate">{item.name}</span>
              <button
                type="button"
                className="automod-chip-remove"
                aria-label={`${item.name} entfernen`}
                title="Entfernen"
                onClick={() => onChange(selected.filter((id) => id !== item.id))}
              >
                <X size={13} />
              </button>
            </li>
          ))}
        </ul>
      )}
      {available.length > 0 ? (
        <select value="" aria-label={addLabel} onChange={(e) => e.target.value && onChange([...selected, e.target.value])}>
          <option value="">{addLabel}</option>
          {available.map((item) => (
            <option key={item.id} value={item.id}>
              {item.channel ? `#${item.name}` : item.name}
            </option>
          ))}
        </select>
      ) : (
        chosen.length === 0 && <span className="muted small">Keine vorhanden.</span>
      )}
    </div>
  )
}
