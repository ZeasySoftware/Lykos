/**
 * Bausteine der Gestaltung (Paket J): Community-Icon, Banner, Profilbanner, Farbauswahl und Bildfeld.
 */
import { useRef, useState } from 'react'
import type { CSSProperties, ReactNode } from 'react'
import type { Community } from '../../lib/api'
import { useAvatarSrc } from '../../lib/avatar'
import {
  COLOR_SUGGESTIONS,
  IMAGE_MAX_INPUT_BYTES,
  bannerFallback,
  colorToHex,
  communityInitials,
  hexToColor,
} from '../../lib/customize'
import { Ban, Check, ImagePlus, Pipette, Trash2 } from 'lucide-react'

/**
 * Bild über eine API-URL (Cookie im Web, Token-Fetch im Desktop-Bundle; gecacht wie Avatare).
 * Rendert nichts, solange das Bild nicht geladen ist oder fehlschlägt.
 */
export function RemoteImage({ url, className }: { url: string | null | undefined; className?: string }) {
  const src = useAvatarSrc(url)
  const [failed, setFailed] = useState<string | null>(null)
  if (!src || failed === src) return null
  return <img className={className} src={src} alt="" draggable={false} onError={() => setFailed(src)} />
}

/** Community-Icon (Bild, sonst Initialen). Größe und Form bestimmt der Aufrufer per CSS. */
export function CommunityIcon({
  community,
  className = '',
  style,
}: {
  community: Pick<Community, 'name'> & { icon_url?: string | null }
  className?: string
  style?: CSSProperties
}) {
  const src = useAvatarSrc(community.icon_url)
  const [failed, setFailed] = useState<string | null>(null)
  return (
    <span className={`community-icon ${className}`.trim()} style={style} aria-hidden="true">
      {src && failed !== src ? (
        <img src={src} alt="" draggable={false} onError={() => setFailed(src)} />
      ) : (
        communityInitials(community.name)
      )}
    </span>
  )
}

/**
 * Banner-Fläche: Bild, sonst Farbe/Verlauf. `color` = feste Bannerfarbe (Profil), `accent` = Akzentfarbe
 * für den dezenten Verlauf; ohne beides greift der Standard-Verlauf (`.banner-fill`).
 */
export function BannerFill({
  url,
  color,
  accent,
  className = '',
  children,
}: {
  url?: string | null
  color?: number | null
  accent?: number | null
  className?: string
  children?: ReactNode
}) {
  return (
    <div className={`banner-fill ${className}`.trim()} style={{ background: bannerFallback(color, accent) }}>
      <RemoteImage url={url} className="banner-fill-img" />
      {children}
    </div>
  )
}

/** Farbauswahl: Vorschläge, freie Farbe (Systemdialog) und optional „Keine“. */
export function ColorPicker({
  value,
  onChange,
  noneLabel = 'Standard',
  label,
}: {
  value: number | null
  onChange: (color: number | null) => void
  noneLabel?: string
  label: string
}) {
  const custom = value != null && !COLOR_SUGGESTIONS.includes(value)
  return (
    <div className="color-picker" role="radiogroup" aria-label={label}>
      <button
        type="button"
        role="radio"
        aria-checked={value == null}
        className={`color-swatch none${value == null ? ' selected' : ''}`}
        title={noneLabel}
        onClick={() => onChange(null)}
      >
        <Ban size={16} aria-hidden="true" />
      </button>
      {COLOR_SUGGESTIONS.map((c) => (
        <button
          key={c}
          type="button"
          role="radio"
          aria-checked={value === c}
          aria-label={colorToHex(c)}
          className={`color-swatch${value === c ? ' selected' : ''}`}
          style={{ background: colorToHex(c) }}
          onClick={() => onChange(c)}
        >
          {value === c && <Check size={16} aria-hidden="true" />}
        </button>
      ))}
      <label
        className={`color-swatch custom${custom ? ' selected' : ''}`}
        title="Eigene Farbe"
        style={custom && value != null ? { background: colorToHex(value) } : undefined}
      >
        <Pipette size={16} aria-hidden="true" />
        <input
          type="color"
          aria-label="Eigene Farbe"
          value={value != null ? colorToHex(value) : '#6d5bd0'}
          onChange={(e) => {
            const c = hexToColor(e.target.value)
            if (c != null) onChange(c)
          }}
        />
      </label>
    </div>
  )
}

/** Bild wählen/entfernen mit Hinweistext. `onFile` bekommt nur Dateien bis `IMAGE_MAX_INPUT_BYTES`. */
export function ImagePickerRow({
  label,
  hint,
  hasImage,
  busy,
  onFile,
  onRemove,
  onTooLarge,
}: {
  label: string
  hint: string
  hasImage: boolean
  busy: boolean
  onFile: (file: File) => void
  onRemove: () => void
  onTooLarge: () => void
}) {
  const input = useRef<HTMLInputElement>(null)
  return (
    <div className="image-picker-row">
      <div className="form-actions">
        <button type="button" className="button" disabled={busy} onClick={() => input.current?.click()}>
          <ImagePlus size={16} aria-hidden="true" /> {hasImage ? `${label} ändern` : `${label} hochladen`}
        </button>
        {hasImage && (
          <button type="button" className="button danger" disabled={busy} onClick={onRemove}>
            <Trash2 size={16} aria-hidden="true" /> Entfernen
          </button>
        )}
      </div>
      <span className="muted small">{hint}</span>
      <input
        ref={input}
        type="file"
        accept="image/png,image/jpeg,image/webp,image/gif"
        hidden
        onChange={(e) => {
          const file = e.target.files?.[0]
          e.target.value = ''
          if (!file) return
          if (file.size > IMAGE_MAX_INPUT_BYTES) onTooLarge()
          else onFile(file)
        }}
      />
    </div>
  )
}
