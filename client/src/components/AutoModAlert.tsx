import type { Message } from '../lib/api'
import { alertSummary, parseAlert } from '../lib/automod'
import { buildSystemMessageMenu, buildUserMenu, deleteMessage } from '../lib/contextMenus'
import { useContextMenu } from '../lib/menuStore'
import { useMemberName, useStore } from '../store'
import { Hash, ShieldAlert, Trash2 } from 'lucide-react'

const timeFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })

/**
 * AutoMod-Alarm im Protokollkanal (`kind = automod_alert`): Autor, Regel, betroffener Kanal, Begründung und der
 * gekürzte Text als einfacher Text (kein Markdown – Links und Erwähnungen bleiben unwirksam).
 */
export function AutoModAlert({ message, canDelete }: { message: Message; canDelete: boolean }) {
  const openProfile = useStore((s) => s.openProfile)
  const selectChannel = useStore((s) => s.selectChannel)
  const alert = parseAlert(message.content)
  const channel = useStore((s) =>
    alert && message.community_id
      ? s.details[message.community_id]?.channels.find((c) => c.id === alert.channel_id)
      : undefined,
  )
  const memberName = useMemberName(message.community_id, message.author)
  // Webhooks haben keinen Autor, nur einen Namen.
  const name = !message.author && message.webhook ? message.webhook.name : memberName
  const menu = useContextMenu(() => buildSystemMessageMenu(message, canDelete), 'AutoMod-Alarm')
  const authorMenu = useContextMenu(
    (ctx) =>
      message.author
        ? buildUserMenu({ userId: message.author.id, communityId: message.community_id, target: ctx.target })
        : [],
    name,
  )

  return (
    <div
      className={`message system-message automod-alert${alert?.blocked ? ' blocked' : ''}`}
      data-message-id={message.id}
      {...menu}
    >
      <ShieldAlert size={16} className="system-message-icon" aria-hidden="true" />
      <div className="automod-alert-body">
        <div className="automod-alert-head">
          <strong className="automod-alert-title">AutoMod · {alert ? alertSummary(alert) : 'Alarm'}</strong>
          <span className="automod-alert-meta">
            {message.author ? (
              <button
                type="button"
                className="message-author"
                data-menu-anchor
                {...authorMenu}
                onClick={(e) =>
                  openProfile(message.author!.id, message.community_id, e.currentTarget.getBoundingClientRect())
                }
              >
                {name}
              </button>
            ) : (
              <span className={`message-author${message.webhook ? '' : ' deleted'}`}>{name}</span>
            )}
            {alert && (
              <>
                {' in '}
                {channel ? (
                  <button type="button" className="automod-alert-channel" onClick={() => selectChannel(channel.id)}>
                    <Hash size={13} aria-hidden="true" />
                    {channel.name}
                  </button>
                ) : (
                  <span className="automod-alert-channel">
                    <Hash size={13} aria-hidden="true" />
                    unbekannt
                  </span>
                )}
                {' · '}
                {alert.rule_name}
                {alert.detail && ` · ${alert.detail}`}
              </>
            )}
          </span>
        </div>
        {alert?.excerpt && <p className="automod-alert-excerpt">{alert.excerpt}</p>}
      </div>
      <time className="message-time" dateTime={message.created_at}>
        {timeFmt.format(new Date(message.created_at))}
      </time>
      {canDelete && (
        <div className="message-actions">
          <button
            type="button"
            className="icon-button small danger"
            title="Meldung entfernen"
            aria-label="Meldung entfernen"
            onClick={(e) => deleteMessage(message, e.shiftKey)}
          >
            <Trash2 size={15} />
          </button>
        </div>
      )}
    </div>
  )
}
