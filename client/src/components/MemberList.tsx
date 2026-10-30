import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useShallow } from 'zustand/react/shallow'
import type { CommunityDetail, Member, Role } from '../lib/api'
import { buildUserMenu } from '../lib/contextMenus'
import { errorText } from '../lib/errors'
import { useContextMenu } from '../lib/menuStore'
import { statusOf } from '../lib/presence'
import { memberName, useStore } from '../store'
import { Avatar } from './Presence'
import { BotTag } from './BotTag'
import { Crown, MessageCircle, X } from 'lucide-react'

function roleColor(color: number | null): string | undefined {
  return color == null ? undefined : `#${color.toString(16).padStart(6, '0')}`
}

/** Einmal angelegt statt `localeCompare` je Vergleich (das baut intern jedes Mal einen Collator). */
const collator = new Intl.Collator('de')

/** `onClose`: im Mobil-Layout liegt die Liste als Vollbild-Ebene über dem Chat und braucht einen Schließen-Knopf. */
export function MemberList({ detail, onClose }: { detail: CommunityDetail; onClose?: () => void }) {
  const communityId = detail.community.id
  const members = useStore((s) => s.members[communityId])
  const loadMembers = useStore((s) => s.loadMembers)
  const me = useStore((s) => s.user)
  const openDm = useStore((s) => s.openDm)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    loadMembers(communityId).catch((e) => setError(errorText(e)))
  }, [communityId, loadMembers])

  const roles = useMemo(() => new Map(detail.roles.map((r) => [r.id, r])), [detail.roles])

  // `onClose` kommt im Mobil-Layout als neue Funktion je Render – über eine Ref, damit `message` (und damit
  // die memoisierten Zeilen) stabil bleibt.
  const onCloseRef = useRef(onClose)
  useEffect(() => {
    onCloseRef.current = onClose
  })
  const message = useCallback(
    async (m: Member) => {
      setError(null)
      try {
        await openDm([m.user.id])
        onCloseRef.current?.()
      } catch (e) {
        setError(errorText(e))
      }
    },
    [openDm],
  )

  // Sortieren nur, wenn sich die Mitgliederliste ändert – nicht bei jeder Präsenz- oder Store-Änderung.
  const sorted = useMemo(
    () => (members ? [...members].sort((a, b) => collator.compare(memberName(a), memberName(b))) : []),
    [members],
  )

  // Präsenz: in Online (inkl. abwesend/nicht stören) und Offline gruppieren. Der Selektor liefert nur die
  // IDs der Online-Mitglieder (flach verglichen) – die Liste rendert also nur neu, wenn jemand die Gruppe
  // wechselt. Wechsel zwischen online/abwesend/bitte nicht stören übernimmt die Zeile selbst.
  const onlineIds = useStore(
    useShallow((s) => sorted.filter((m) => statusOf(s, m.user.id) !== 'offline').map((m) => m.user.id)),
  )
  const groups = useMemo(() => {
    const online = new Set(onlineIds)
    // Bots stehen unabhängig vom Status in einer eigenen Gruppe.
    const humans = sorted.filter((m) => !m.user.bot)
    return [
      { title: 'Online', items: humans.filter((m) => online.has(m.user.id)) },
      { title: 'Offline', items: humans.filter((m) => !online.has(m.user.id)) },
      { title: 'Bots', items: sorted.filter((m) => m.user.bot) },
    ].filter((g) => g.items.length > 0)
  }, [sorted, onlineIds])

  return (
    <aside className={`members${onClose ? ' members-overlay' : ''}`} aria-label="Mitglieder">
      <header className="members-header">
        <span className="truncate">Mitglieder{members ? ` — ${members.length}` : ''}</span>
        {onClose && (
          <button type="button" className="icon-button members-close" onClick={onClose} aria-label="Mitglieder schließen">
            <X size={20} />
          </button>
        )}
      </header>
      {error && <p className="error small">{error}</p>}
      <div className="members-scroll">
        {groups.map((group) => (
          <section key={group.title} className="member-group">
            <h3 className="member-group-title">
              {group.title} — {group.items.length}
            </h3>
            <ul>
              {group.items.map((m) => (
                <MemberRow
                  key={m.user.id}
                  member={m}
                  detail={detail}
                  isMe={m.user.id === me?.id}
                  color={roleColor(topRole(m, roles)?.color ?? null)}
                  onMessage={message}
                />
              ))}
            </ul>
          </section>
        ))}
      </div>
    </aside>
  )
}

// Höchste (sichtbare) Rolle bestimmt die Farbe – kleinere Position = höher (siehe lib/roles.ts).
function topRole(m: Member, roles: Map<string, Role>) {
  return m.role_ids
    .map((id) => roles.get(id))
    .filter((r) => r && !r.is_default)
    .sort((a, b) => a!.position - b!.position)[0]
}

/** Eine Zeile; abonniert nur den Status ihres Mitglieds. */
const MemberRow = memo(function MemberRow({
  member: m,
  detail,
  isMe,
  color,
  onMessage,
}: {
  member: Member
  detail: CommunityDetail
  isMe: boolean
  color: string | undefined
  onMessage: (m: Member) => Promise<void>
}) {
  const communityId = detail.community.id
  const status = useStore((s) => statusOf(s, m.user.id))
  const openProfile = useStore((s) => s.openProfile)
  const isOwner = m.user.id === detail.community.owner_id
  const name = memberName(m)
  const menu = useContextMenu(
    (ctx) => buildUserMenu({ userId: m.user.id, communityId, target: ctx.target }),
    name,
  )
  const roleNames = m.role_ids
    .map((id) => detail.roles.find((r) => r.id === id))
    .filter((r) => r && !r.is_default)
    .map((r) => r!.name)
  return (
    <li
      className={`member${status === 'offline' ? ' member-offline' : ''}`}
      title={roleNames.join(', ') || undefined}
      onContextMenu={menu.onContextMenu}
    >
      <button
        type="button"
        className="member-open"
        data-menu-anchor
        onClick={(e) => openProfile(m.user.id, communityId, e.currentTarget.getBoundingClientRect())}
        onKeyDown={menu.onKeyDown}
      >
        <Avatar user={m.user} name={name} status={status} />
        <span className="member-name truncate" style={{ color }}>
          {name}
        </span>
        {m.user.bot && <BotTag />}
      </button>
      {isOwner && (
        <span className="member-owner" title="Besitzer">
          <Crown size={14} />
        </span>
      )}
      {!isMe && !m.user.bot && (
        <button
          type="button"
          className="icon-button small member-message"
          title="Nachricht senden"
          aria-label={`Nachricht an ${name} senden`}
          onClick={() => void onMessage(m)}
        >
          <MessageCircle size={15} />
        </button>
      )}
    </li>
  )
})
