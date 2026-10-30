import { useEffect } from 'react'
import type { MessageChannel, UserPublic } from '../lib/api'
import { displayName, useStore } from '../store'

const EMPTY: Record<string, number> = {}
const NO_USERS: UserPublic[] = []

/** „bob schreibt …“ unter dem Composer. Einträge verfallen im Store nach 8 s ohne neues Event. */
export function TypingIndicator({ channel }: { channel: MessageChannel }) {
  const typing = useStore((s) => s.typing[channel.id] ?? EMPTY)
  const communityId = channel.community_id
  const members = useStore((s) => (communityId ? s.members[communityId] : undefined))
  // In DMs kommen die Namen aus der Teilnehmerliste.
  const recipients = useStore((s) => (communityId ? NO_USERS : (s.dms[channel.id]?.recipients ?? NO_USERS)))
  const messages = useStore((s) => s.messages[channel.id]?.items)
  const loadMembers = useStore((s) => s.loadMembers)

  const userIds = Object.keys(typing)

  // Namen kommen aus der Mitgliederliste; ist sie noch nicht geladen, einmal nachladen.
  const needMembers = userIds.length > 0 && !members && communityId !== null
  useEffect(() => {
    if (needMembers && communityId) loadMembers(communityId).catch(() => {})
  }, [needMembers, communityId, loadMembers])

  function nameOf(userId: string): string {
    const recipient = recipients.find((u) => u.id === userId)
    if (recipient) return displayName(recipient)
    const member = members?.find((m) => m.user.id === userId)
    if (member) return member.nickname ?? displayName(member.user)
    const author = messages?.find((m) => m.author?.id === userId)?.author
    return author ? displayName(author) : 'Jemand'
  }

  const names = userIds.map(nameOf)
  let text = ''
  if (names.length === 1) text = `${names[0]} schreibt …`
  else if (names.length === 2) text = `${names[0]} und ${names[1]} schreiben …`
  else if (names.length === 3) text = `${names[0]}, ${names[1]} und ${names[2]} schreiben …`
  else if (names.length > 3) text = 'Mehrere Personen schreiben …'

  return (
    <div className="typing-indicator" aria-live="polite">
      {text && (
        <>
          <span className="typing-dots" aria-hidden="true">
            <span />
            <span />
            <span />
          </span>
          <span className="truncate">{text}</span>
        </>
      )}
    </div>
  )
}
