import type { ReactNode } from 'react'

/** Schalter (an/aus) wie bei Discord – als `role="switch"`, per Leertaste/Enter bedienbar. */
export function Switch({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean
  onChange: (checked: boolean) => void
  disabled?: boolean
  /** Barrierefreier Name, falls der Schalter ohne sichtbare Beschriftung steht. */
  label?: string
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={`switch${checked ? ' on' : ''}`}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span className="switch-thumb" />
    </button>
  )
}

/** Einstellungszeile: Titel + Beschreibung links, Schalter rechts, Trennlinie darunter. */
export function SwitchRow({
  title,
  description,
  checked,
  onChange,
  disabled,
  disabledReason,
}: {
  title: ReactNode
  description?: ReactNode
  checked: boolean
  onChange: (checked: boolean) => void
  disabled?: boolean
  /** Erklärung, warum der Schalter gesperrt ist (Tooltip). */
  disabledReason?: string
}) {
  return (
    <div className={`switch-row${disabled ? ' disabled' : ''}`} title={disabled ? disabledReason : undefined}>
      <div className="switch-row-text">
        <span className="switch-row-title">{title}</span>
        {description && <span className="switch-row-desc">{description}</span>}
      </div>
      <Switch checked={checked} onChange={onChange} disabled={disabled} label={typeof title === 'string' ? title : undefined} />
    </div>
  )
}
