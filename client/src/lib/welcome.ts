/**
 * Eigener Willkommenstext einer Community (Systemkanal): Platzhalter einsetzen wie der Server
 * (`routes/customization.rs`, `render_welcome`) – für die Vorschau in den Einstellungen.
 */

/** Höchstlänge des Willkommenstexts (vor dem Einsetzen der Platzhalter). */
export const WELCOME_MAX = 500

/** Verfügbare Platzhalter mit Erklärung. */
export const WELCOME_PLACEHOLDERS: { token: string; label: string }[] = [
  { token: '{user}', label: 'Erwähnung des neuen Mitglieds' },
  { token: '{community}', label: 'Name der Community' },
]

/** `{user}` → `@name` (Erwähnung), `{community}` → Name der Community; sonst bleibt der Text, wie er ist. */
export function renderWelcome(template: string, username: string, communityName: string): string {
  return template.split('{user}').join(`@${username}`).split('{community}').join(communityName)
}
