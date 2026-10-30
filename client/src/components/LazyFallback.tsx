/**
 * Platzhalter, solange eine Vollbild-Ebene (Einstellungen, Instanz-Verwaltung) noch lädt: dieselbe
 * Hintergrundfläche, erst nach kurzer Verzögerung eingeblendet – bei schnellem (oder vorgeladenem)
 * Chunk sieht man ihn gar nicht, sonst ruhig statt eines kurz aufblitzenden Ladetexts.
 */
export function LazyLayerFallback() {
  return <div className="lazy-layer-fallback" aria-busy="true" aria-label="Wird geladen …" />
}
