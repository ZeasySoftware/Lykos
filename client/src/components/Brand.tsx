/**
 * Lykos-Marke: facettierter Wolfskopf (wie `public/favicon.svg`, aber ohne Kachel-Hintergrund),
 * damit er auf jeder dunklen Fläche frei steht – in der Rail, neben der Wortmarke, in leeren Zuständen.
 */
export function LykosMark({ className, title }: { className?: string; title?: string }) {
  return (
    <svg
      className={className}
      viewBox="88 82 336 370"
      role={title ? 'img' : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      focusable="false"
    >
      <path
        fill="#ece9ff"
        d="M256 150 L210 156 L152 90 L136 200 L98 254 L130 264 L96 312 L140 318 L122 360 L178 354 L214 410 L234 444 L278 444 L298 410 L334 354 L390 360 L372 318 L416 312 L382 264 L414 254 L376 200 L360 90 L302 156 Z"
      />
      <path fill="#c9c2ff" d="M256 150 L302 156 L330 228 L286 300 L270 396 L242 396 L226 300 L182 228 L210 156 Z" />
      <path fill="#6d5bd0" d="M162 120 L198 160 L150 190 Z M350 120 L314 160 L362 190 Z" />
      <path fill="#1f2348" d="M176 256 L236 280 L204 296 Z M336 256 L276 280 L308 296 Z M230 400 L282 400 L256 426 Z" />
    </svg>
  )
}

/** Logo + Schriftzug „Lykos“ (Anmeldung, Instanzwahl, Download-Seite). */
export function Wordmark() {
  return (
    <span className="wordmark">
      <LykosMark className="wordmark-mark" />
      <span className="wordmark-text">Lykos</span>
    </span>
  )
}
