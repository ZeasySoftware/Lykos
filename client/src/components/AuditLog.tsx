import { useCallback, useEffect, useMemo, useState } from 'react'
import { endpoints } from '../lib/api'
import type { AuditEntry, CommunityDetail } from '../lib/api'
import { errorText } from '../lib/errors'
import { displayName, memberName, useStore } from '../store'
import { SettingsPage } from './SettingsLayer'

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })
const AUDIT_PAGE = 50


export function AuditTab({ detail }: { detail: CommunityDetail }) {
  const communityId = detail.community.id
  const members = useStore((s) => s.members[communityId])
  const loadMembers = useStore((s) => s.loadMembers)
  const [entries, setEntries] = useState<AuditEntry[]>([])
  const [hasMore, setHasMore] = useState(true)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  /** Lädt eine Seite; ohne `before` von vorn. Ladezustand setzt der Aufrufer. */
  const fetchPage = useCallback(
    (before?: string) =>
      endpoints
        .auditLog(communityId, before, AUDIT_PAGE)
        .then((page) => {
          setEntries((prev) => (before ? [...prev, ...page] : page))
          setHasMore(page.length === AUDIT_PAGE)
          setError(null)
        })
        .catch((e) => setError(errorText(e)))
        .finally(() => setLoading(false)),
    [communityId],
  )

  function load(before?: string) {
    setLoading(true)
    void fetchPage(before)
  }

  useEffect(() => {
    void fetchPage()
    if (!useStore.getState().members[communityId]) loadMembers(communityId).catch(() => {})
  }, [communityId, fetchPage, loadMembers])

  const names = useMemo(() => {
    const users = new Map((members ?? []).map((m) => [m.user.id, memberName(m)]))
    return {
      user: (id: unknown) => (typeof id === 'string' && users.get(id)) || 'ein ehemaliges Mitglied',
      role: (id: unknown) => {
        const r = detail.roles.find((x) => x.id === id)
        return r ? `„${r.name}“` : 'eine gelöschte Rolle'
      },
      channel: (id: unknown, fallback?: unknown) => {
        const c = detail.channels.find((x) => x.id === id)
        if (c) return c.kind === 'text' ? `#${c.name}` : `„${c.name}“`
        return typeof fallback === 'string' ? `„${fallback}“` : 'einem Kanal'
      },
    }
  }, [members, detail.roles, detail.channels])

  return (
    <SettingsPage title="Audit-Log" description="Wer hat wann was in der Community geändert? Nachrichteninhalte werden hier nie gespeichert.">
      <div className="form-actions form-actions-start">
        <button type="button" className="button small-button" onClick={() => load()} disabled={loading}>
          Aktualisieren
        </button>
      </div>
      {error && <p className="error">{error}</p>}
      {!loading && entries.length === 0 && !error && <p className="muted">Noch keine Einträge.</p>}
      <ul className="audit-list">
        {entries.map((e) => (
          <li key={e.id} className="audit-entry">
            <span>
              <strong>{displayName(e.actor)}</strong> {describeAudit(e, names)}
            </span>
            <time className="muted small" dateTime={e.created_at}>
              {dateFmt.format(new Date(e.created_at))}
            </time>
          </li>
        ))}
      </ul>
      {loading && <p className="muted small">Lade …</p>}
      {hasMore && entries.length > 0 && !loading && (
        <button type="button" className="button" onClick={() => load(entries[entries.length - 1].id)}>
          Ältere Einträge laden
        </button>
      )}
    </SettingsPage>
  )
}

interface Names {
  user: (id: unknown) => string
  role: (id: unknown) => string
  channel: (id: unknown, fallback?: unknown) => string
}

function quoted(v: unknown): string {
  return typeof v === 'string' ? `„${v}“` : ''
}

/** Deutsche Beschreibung eines Audit-Eintrags (Satz ohne den Akteur). */
function describeAudit(e: AuditEntry, n: Names): string {
  const d = e.data
  switch (e.action) {
    case 'community.create':
      return `hat die Community ${quoted(d.name)} angelegt.`
    case 'community.update': {
      const parts: string[] = []
      if (typeof d.name === 'string') parts.push(`Name: ${quoted(d.name)}`)
      // `null` heißt hier „unbegrenzt“ oder „nicht geändert“ – nur konkrete Werte nennen.
      if (typeof d.message_retention_days === 'number') parts.push(`Aufbewahrung: ${d.message_retention_days} Tage`)
      if (typeof d.description === 'string') parts.push('Beschreibung')
      return `hat die Community-Einstellungen geändert${parts.length ? ` (${parts.join(', ')})` : ''}.`
    }
    case 'community.icon_set':
      return 'hat das Community-Icon geändert.'
    case 'community.icon_remove':
      return 'hat das Community-Icon entfernt.'
    case 'community.banner_set':
      return 'hat das Community-Banner geändert.'
    case 'community.banner_remove':
      return 'hat das Community-Banner entfernt.'
    case 'emoji.create':
      return `hat das Emoji :${String(d.name ?? '')}: hochgeladen.`
    case 'emoji.update':
      return `hat das Emoji :${String(d.old_name ?? '')}: in :${String(d.name ?? '')}: umbenannt.`
    case 'emoji.delete':
      return `hat das Emoji :${String(d.name ?? '')}: gelöscht.`
    case 'channel.create':
      return `hat den Kanal ${d.kind === 'text' ? `#${String(d.name)}` : quoted(d.name)} angelegt.`
    case 'channel.update':
      return `hat den Kanal ${n.channel(e.target_id)} bearbeitet.`
    case 'channel.delete':
      return `hat den Kanal ${quoted(d.name)} gelöscht.`
    case 'channel.override_set':
      return `hat die Berechtigungen von ${n.role(d.role_id)} in ${n.channel(e.target_id)} angepasst.`
    case 'channel.override_delete':
      return `hat die Sonderrechte von ${n.role(d.role_id)} in ${n.channel(e.target_id)} entfernt.`
    case 'role.create':
      return `hat die Rolle ${quoted(d.name)} angelegt.`
    case 'role.update':
      return `hat die Rolle ${n.role(e.target_id)} bearbeitet.`
    case 'role.delete':
      return `hat die Rolle ${quoted(d.name)} gelöscht.`
    case 'role.reorder':
      return 'hat die Reihenfolge der Rollen geändert.'
    case 'member.join':
      return 'ist der Community beigetreten.'
    case 'member.leave':
      return 'hat die Community verlassen.'
    case 'member.kick':
      return `hat ${n.user(e.target_id)} aus der Community entfernt.`
    case 'member.ban':
      return typeof d.reason === 'string'
        ? `hat ${n.user(e.target_id)} gebannt (Grund: ${quoted(d.reason)}).`
        : `hat ${n.user(e.target_id)} gebannt.`
    case 'member.unban':
      return `hat den Bann von ${n.user(e.target_id)} aufgehoben.`
    case 'member.role_add':
      return `hat ${n.user(e.target_id)} die Rolle ${n.role(d.role_id)} gegeben.`
    case 'member.role_remove':
      return `hat ${n.user(e.target_id)} die Rolle ${n.role(d.role_id)} entzogen.`
    case 'invite.create':
      return `hat eine Einladung erstellt (${String(d.code ?? '')}).`
    case 'invite.delete':
      return `hat die Einladung ${String(d.code ?? '')} widerrufen.`
    case 'message.delete':
      return d.webhook_id
        ?`hat eine Webhook-Nachricht in ${n.channel(d.channel_id)} gelöscht.`
        : `hat eine Nachricht von ${n.user(d.author_id)} in ${n.channel(d.channel_id)} gelöscht.`
    case 'webhook.create':
      return `hat den Webhook ${quoted(d.name)} in ${n.channel(d.channel_id)} angelegt.`
    case 'webhook.update':
      if (typeof d.channel_id === 'string') return `hat einen Webhook nach ${n.channel(d.channel_id)} verschoben.`
      if (typeof d.name === 'string') return `hat einen Webhook in ${quoted(d.name)} umbenannt.`
      return d.avatar ? 'hat den Avatar eines Webhooks geändert.' : 'hat den Avatar eines Webhooks entfernt.'
    case 'webhook.token_reset':
      return 'hat die URL eines Webhooks neu erzeugt.'
    case 'webhook.delete':
      return `hat den Webhook ${quoted(d.name)} aus ${n.channel(d.channel_id)} gelöscht.`
    case 'bot.add':
      return `hat den Bot ${quoted(d.username)} hinzugefügt.`
    case 'message.pin':
      return `hat eine Nachricht von ${n.user(d.author_id)} in ${n.channel(d.channel_id)} angepinnt.`
    case 'message.unpin':
      return `hat eine Nachricht von ${n.user(d.author_id)} in ${n.channel(d.channel_id)} gelöst.`
    case 'member.nickname':
      return typeof d.nickname === 'string'
        ? `hat den Nickname von ${n.user(e.target_id)} auf ${quoted(d.nickname)} gesetzt.`
        : `hat den Nickname von ${n.user(e.target_id)} entfernt.`
    case 'automod.update':
      return 'hat die AutoMod-Regeln geändert.'
    case 'automod.block':
      return `wurde vom AutoMod in ${n.channel(d.channel_id)} blockiert${d.detail ? ` (${String(d.detail)})` : ''}.`
    case 'automod.flag':
      return `wurde vom AutoMod in ${n.channel(d.channel_id)} gemeldet${d.detail ? ` (${String(d.detail)})` : ''}.`
    case 'community.transfer':
      return `hat den Besitz der Community an ${n.user(e.target_id)} übertragen.`
    default:
      return `hat eine Aktion ausgeführt (${e.action}).`
  }
}
