import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import { Activity, Database, HardDrive, Headphones, MessagesSquare, RefreshCw, Server, Users } from 'lucide-react'
import { adminApi } from '../../lib/adminApi'
import type { AdminOverview } from '../../lib/adminApi'
import { errorText } from '../../lib/errors'
import { formatBytes } from '../../lib/uploads'
import { fmtCount, fmtNumber, fmtUptime } from './format'

function Tile({ icon, label, value, children }: { icon: ReactNode; label: string; value: string; children?: ReactNode }) {
  return (
    <section className="ia-tile">
      <header className="ia-tile-head">
        {icon}
        <span>{label}</span>
      </header>
      <strong className="ia-tile-value">{value}</strong>
      {children && <div className="ia-tile-detail">{children}</div>}
    </section>
  )
}

export function OverviewTab() {
  const [data, setData] = useState<AdminOverview | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [reload, setReload] = useState(0)

  useEffect(() => {
    let cancelled = false
    adminApi
      .overview()
      .then((d) => {
        if (cancelled) return
        setData(d)
        setError(null)
      })
      .catch((e) => !cancelled && setError(errorText(e)))
    return () => {
      cancelled = true
    }
  }, [reload])

  return (
    <div className="ia-section">
      <div className="ia-section-head">
        <h3>Übersicht</h3>
        <button type="button" className="button small-button" onClick={() => setReload((n) => n + 1)}>
          <RefreshCw size={14} aria-hidden="true" />
          Aktualisieren
        </button>
      </div>
      {error && <p className="error">{error}</p>}
      {!data ? (
        !error && <p className="muted">Lade …</p>
      ) : (
        <div className="ia-tiles">
          <Tile icon={<Users size={16} />} label="Nutzer" value={fmtNumber(data.users.total)}>
            <span>+{fmtNumber(data.users.new_7d)} in 7 Tagen</span>
            <span>{fmtNumber(data.users.disabled)} gesperrt</span>
            <span>{fmtCount(data.users.admins, 'Admin', 'Admins')}</span>
          </Tile>
          <Tile icon={<Activity size={16} />} label="Gerade online" value={fmtNumber(data.online)}>
            <span>Unsichtbare zählen nicht mit</span>
          </Tile>
          <Tile icon={<MessagesSquare size={16} />} label="Communities" value={fmtNumber(data.communities)}>
            <span>{fmtCount(data.messages, 'Nachricht', 'Nachrichten')}</span>
          </Tile>
          <Tile
            icon={<Headphones size={16} />}
            label="In Sprachkanälen"
            value={data.voice.enabled ? fmtNumber(data.voice.participants) : 'aus'}
          >
            {!data.voice.enabled ? (
              <span>LiveKit nicht konfiguriert</span>
            ) : data.voice.channels.length === 0 ? (
              <span>Niemand im Voice</span>
            ) : (
              data.voice.channels.map((c) => (
                <span key={c.channel_id} className="truncate" title={`${c.community_name} · ${c.channel_name}`}>
                  {c.participants} · {c.community_name} / {c.channel_name}
                </span>
              ))
            )}
          </Tile>
          <Tile icon={<HardDrive size={16} />} label="Speicher (Anhänge)" value={formatBytes(data.storage.total_bytes)}>
            <span>
              {fmtCount(data.storage.attachment_count, 'Datei', 'Dateien')} · {data.storage.backend === 's3' ? 'S3' : 'lokal'}
            </span>
            {data.storage.top_communities.map((c) => (
              <span key={c.id} className="truncate" title={c.name}>
                {formatBytes(c.bytes)} · {c.name}
              </span>
            ))}
          </Tile>
          <Tile icon={<Database size={16} />} label="Datenbank" value={formatBytes(data.database_bytes)} />
          <Tile icon={<Server size={16} />} label="Server" value={`v${data.version}`}>
            <span>Läuft seit {fmtUptime(data.uptime_seconds)}</span>
          </Tile>
        </div>
      )}
    </div>
  )
}
