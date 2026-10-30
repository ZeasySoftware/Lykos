/**
 * Reine Logik der Call-Bühne (ohne React/LiveKit, daher mit `node --test` prüfbar):
 * welche Kacheln es gibt, in welcher Reihenfolge, wie viele ins Raster passen und wie groß sie werden.
 */

/** `user` = Teilnehmer (Kamera oder Avatar), `screen` = Bildschirmübertragung. */
export type TileKind = 'user' | 'screen'

export interface CallTile {
  key: string
  kind: TileKind
  userId: string
  /** Kamera an (nur `user`). */
  camera: boolean
  /** Stream wird angesehen bzw. ist der eigene (nur `screen`). */
  watching: boolean
}

export interface TileParticipant {
  userId: string
  camera: boolean
  streaming: boolean
}

export function tileKey(kind: TileKind, userId: string): string {
  return `${kind}:${userId}`
}

/** Höchstens so viele Kacheln ohne Scrollen; darüber gibt es eine „+N“-Kachel. */
export const MAX_GRID_TILES = 12

/**
 * Kacheln in Anzeige-Reihenfolge: Bildschirmübertragungen zuerst (angesehene vor nicht angesehenen),
 * dann Teilnehmer mit Kamera, dann der Rest. Innerhalb einer Gruppe bleibt die Reihenfolge der
 * Teilnehmer (Beitritt) erhalten, damit Kacheln beim Sprechen nicht springen.
 */
export function buildTiles(participants: TileParticipant[], watching: readonly string[], me?: string): CallTile[] {
  const screens: CallTile[] = []
  const cams: CallTile[] = []
  const rest: CallTile[] = []
  for (const p of participants) {
    if (p.streaming) {
      const watched = p.userId === me || watching.includes(p.userId)
      screens.push({ key: tileKey('screen', p.userId), kind: 'screen', userId: p.userId, camera: false, watching: watched })
    }
    const tile: CallTile = { key: tileKey('user', p.userId), kind: 'user', userId: p.userId, camera: p.camera, watching: false }
    ;(p.camera ? cams : rest).push(tile)
  }
  // Stabil sortieren: angesehene Streams nach vorn.
  const watchedScreens = screens.filter((t) => t.watching)
  const otherScreens = screens.filter((t) => !t.watching)
  return [...watchedScreens, ...otherScreens, ...cams, ...rest]
}

/** Zeigt die Kachel gerade bewegtes Bild (und braucht dafür eine Video-Spur)? */
export function hasVideo(tile: CallTile): boolean {
  return tile.kind === 'user' ? tile.camera : tile.watching
}

/**
 * Höchstens `max` Kacheln sichtbar. Passen nicht alle hinein, bleibt ein Platz für die „+N“-Kachel;
 * Sprechende, die sonst verborgen wären, verdrängen die hintersten nicht sprechenden Avatar-Kacheln
 * (Sprecher zuerst sichtbar, ohne die übrige Reihenfolge umzuwerfen).
 */
export function pickVisible(
  tiles: CallTile[],
  max: number,
  speaking: ReadonlySet<string>,
): { visible: CallTile[]; hidden: CallTile[] } {
  if (tiles.length <= max) return { visible: tiles, hidden: [] }
  const slots = Math.max(1, max - 1)
  const visible = tiles.slice(0, slots)
  const hidden = tiles.slice(slots)
  for (const speaker of hidden.filter((t) => t.kind === 'user' && speaking.has(t.userId))) {
    // Hinterste verdrängbare Kachel: Avatar ohne Kamera, der gerade nicht spricht.
    let idx = -1
    for (let i = visible.length - 1; i >= 0; i--) {
      const t = visible[i]
      if (t.kind === 'user' && !t.camera && !speaking.has(t.userId)) {
        idx = i
        break
      }
    }
    if (idx < 0) break
    const [out] = visible.splice(idx, 1)
    visible.push(speaker)
    hidden.splice(hidden.indexOf(speaker), 1)
    hidden.unshift(out)
  }
  // Sichtbare Kacheln in Originalreihenfolge zurückgeben.
  const order = new Map(tiles.map((t, i) => [t.key, i]))
  visible.sort((a, b) => order.get(a.key)! - order.get(b.key)!)
  hidden.sort((a, b) => order.get(a.key)! - order.get(b.key)!)
  return { visible, hidden }
}

export interface GridLayout {
  cols: number
  rows: number
  tileWidth: number
  tileHeight: number
}

/**
 * Spaltenzahl, bei der `count` Kacheln im Seitenverhältnis `aspect` (Breite/Höhe) die Fläche
 * `width` × `height` am besten ausnutzen (größte Kachel gewinnt). Liefert ganze Pixel.
 */
export function gridLayout(count: number, width: number, height: number, gap = 8, aspect = 16 / 9): GridLayout {
  if (count <= 0 || width <= 0 || height <= 0) return { cols: 1, rows: 1, tileWidth: 0, tileHeight: 0 }
  let best: GridLayout = { cols: 1, rows: count, tileWidth: 0, tileHeight: 0 }
  for (let cols = 1; cols <= count; cols++) {
    const rows = Math.ceil(count / cols)
    // Mehr Spalten als nötig bringen nichts (leere Spalten).
    if (cols > 1 && Math.ceil(count / (cols - 1)) === rows) continue
    const maxW = (width - gap * (cols - 1)) / cols
    const maxH = (height - gap * (rows - 1)) / rows
    if (maxW <= 0 || maxH <= 0) continue
    const w = Math.min(maxW, maxH * aspect)
    if (w > best.tileWidth) best = { cols, rows, tileWidth: w, tileHeight: w / aspect }
  }
  return {
    ...best,
    tileWidth: Math.floor(best.tileWidth),
    tileHeight: Math.floor(best.tileHeight),
  }
}

/**
 * Was die Mini-Ansicht (Call läuft, Bühne nicht sichtbar) zeigt: die fokussierte Kachel, sonst die
 * Kamera der Person, die zuletzt gesprochen hat, sonst einen angesehenen Stream, sonst irgendeine
 * fremde Kamera. `null` = kein Bild (nur Avatare).
 */
export function featuredTile(
  tiles: CallTile[],
  focus: string | null,
  lastSpeaker: string | null,
  me?: string,
): CallTile | null {
  const withVideo = tiles.filter(hasVideo)
  const focused = withVideo.find((t) => t.key === focus)
  if (focused) return focused
  const speaker = withVideo.find((t) => t.kind === 'user' && t.userId === lastSpeaker && t.userId !== me)
  if (speaker) return speaker
  return (
    withVideo.find((t) => t.kind === 'screen' && t.userId !== me) ??
    withVideo.find((t) => t.kind === 'user' && t.userId !== me) ??
    null
  )
}
