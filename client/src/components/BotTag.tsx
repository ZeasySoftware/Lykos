/** Kennzeichen neben dem Namen: Bot-Konto oder Nachricht eines Webhooks. */
export function BotTag({ kind = 'bot' }: { kind?: 'bot' | 'webhook' }) {
  return (
    <span
      className="bot-tag"
      title={kind === 'bot' ? 'Bot-Konto' : 'Nachricht eines Webhooks (Integration)'}
    >
      {kind === 'bot' ? 'BOT' : 'WEBHOOK'}
    </span>
  )
}
