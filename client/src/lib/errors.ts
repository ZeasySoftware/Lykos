import { ApiError } from './api'

const messages: Record<string, string> = {
  network: 'Server nicht erreichbar.',
  invalid_username: 'Benutzername: 2–32 Zeichen, nur Buchstaben, Ziffern, _ . -',
  invalid_password: 'Passwort: mindestens 10 Zeichen.',
  invalid_name: 'Ungültiger Name.',
  invalid_content: 'Nachricht ist leer oder zu lang (max. 4000 Zeichen).',
  invalid_kind: 'Ungültiger Kanaltyp.',
  invalid_request: 'Ungültige Anfrage.',
  not_a_text_channel: 'Das ist kein Textkanal.',
  unauthorized: 'Nicht angemeldet.',
  invalid_credentials: 'Benutzername oder Passwort falsch.',
  forbidden: 'Dafür fehlt dir die Berechtigung.',
  registration_closed: 'Registrierung ist auf dieser Instanz geschlossen.',
  invite_required: 'Für die Registrierung brauchst du eine Einladung.',
  not_found: 'Nicht gefunden.',
  invalid_invite: 'Einladung ungültig oder abgelaufen.',
  username_taken: 'Benutzername ist bereits vergeben.',
  already_member: 'Du bist bereits Mitglied.',
  owner_cannot_leave: 'Als Besitzer kannst du die Community nicht verlassen.',
  rate_limited: 'Zu viele Versuche – bitte später erneut versuchen.',
  file_too_large: 'Die Datei ist zu groß.',
  quota_exceeded:
    'Speicherplatz erschöpft: Die Datei passt nicht mehr in dein Kontingent oder das der Community. Lösche ältere Anhänge oder frag die Verwaltung.',
  too_many_attachments: 'Höchstens 10 Anhänge pro Nachricht.',
  invalid_attachments: 'Ein Anhang ist nicht mehr verfügbar. Bitte erneut hochladen.',
  aborted: 'Upload abgebrochen.',
  not_a_voice_channel: 'Das ist kein Sprachkanal.',
  invalid_bitrate: 'Die Bitrate muss zwischen 32 und 256 kbit/s liegen.',
  invalid_user_limit: 'Das Nutzerlimit muss zwischen 1 und 99 liegen.',
  channel_full: 'Dieser Sprachkanal ist voll.',
  voice_unavailable: 'Sprachkanäle sind auf dieser Instanz nicht eingerichtet.',
  invalid_image: 'Bitte ein Bild im Format PNG, JPEG, WebP oder GIF wählen.',
  invalid_bio: '„Über mich“ darf höchstens 190 Zeichen lang sein.',
  invalid_status_text: 'Der Status-Text darf höchstens 128 Zeichen lang sein.',
  wrong_password: 'Das Passwort ist falsch.',
  owns_communities: 'Du besitzt noch Communities. Übertrage oder lösche sie zuerst.',
  last_instance_admin: 'Es muss mindestens einen Instanz-Admin geben – ernenne zuerst einen weiteren.',
  dm_not_allowed: 'Dieser Person kannst du keine Direktnachricht schicken.',
  too_many_recipients: 'Gruppen haben höchstens 10 Personen.',
  // Freundesystem
  friend_request_not_allowed:
    'Das hat nicht geklappt. Prüfe den Benutzernamen – oder die Person nimmt gerade keine Freundschaftsanfragen an.',
  already_friends: 'Ihr seid bereits befreundet.',
  user_blocked: 'Du hast diese Person blockiert. Hebe die Blockade zuerst auf.',
  too_many_pending_requests: 'Du hast zu viele offene Anfragen. Warte, bis welche beantwortet sind, oder ziehe welche zurück.',
  not_a_group_dm: 'Das geht nur in Gruppen.',
  cannot_leave_dm: 'Direktnachrichten zu zweit kann man nicht verlassen.',
  invalid_emoji: 'Dieses Emoji gibt es nicht (mehr).',
  too_many_reactions: 'Höchstens 20 verschiedene Reaktionen pro Nachricht.',
  too_many_pins: 'In diesem Kanal sind schon 50 Nachrichten angepinnt – löse zuerst eine.',
  invalid_reply: 'Die Nachricht, auf die du antwortest, gibt es nicht mehr.',
  too_many_emojis: 'Diese Community hat bereits 100 Emojis.',
  emoji_name_taken: 'Ein Emoji mit diesem Namen gibt es schon.',
  invalid_title: 'Titel: 3–150 Zeichen.',
  invalid_description: 'Beschreibung fehlt oder ist zu lang.',
  invalid_tech_info: 'Die technischen Infos konnten nicht übernommen werden.',
  invalid_status: 'Ungültiger Status.',
  not_a_suggestion: 'Abstimmen geht nur bei Vorschlägen.',
  already_voted: 'Du hast bereits zugestimmt.',
  screenshot_exists: 'Zu dieser Meldung gibt es schon einen Screenshot.',
  issue_exists: 'Für diese Meldung gibt es schon ein Issue.',
  gitea_not_configured: 'Die Gitea-Weiterleitung ist auf dieser Instanz nicht eingerichtet.',
  gitea_unavailable: 'Gitea ist gerade nicht erreichbar oder hat das Issue abgelehnt.',
  invalid_pronouns: 'Pronomen: höchstens 40 Zeichen, eine Zeile.',
  invalid_color: 'Ungültige Farbe.',
  invalid_channel: 'Bitte einen Textkanal dieser Community wählen.',
  // Instanz-Verwaltung (Paket H)
  account_disabled: 'Dieses Konto ist gesperrt. Wende dich an die Verwaltung der Instanz.',
  community_creation_disabled: 'Auf dieser Instanz dürfen nur Admins Communities anlegen.',
  instance_invite: 'Diese Einladung gilt nur für die Registrierung – du hast bereits ein Konto.',
  invalid_reset_token: 'Der Link ist ungültig, abgelaufen oder wurde schon benutzt.',
  cannot_target_self: 'Das geht nicht mit dem eigenen Konto.',
  invalid_note: 'Die Notiz darf höchstens 200 Zeichen lang sein.',
  invalid_announcement: 'Die Ankündigung darf höchstens 500 Zeichen lang sein.',
  invalid_registration_mode: 'Ungültiger Registrierungsmodus.',
  community_not_found: 'Community nicht gefunden.',
  // Konto-Sicherheit
  invalid_email: 'Bitte eine gültige E-Mail-Adresse eingeben.',
  email_unavailable: 'E-Mail-Funktionen sind dafür nicht verfügbar.',
  email_unchanged: 'Diese Adresse ist bereits hinterlegt.',
  email_taken: 'Diese Adresse gehört bereits zu einem anderen Konto.',
  invalid_token: 'Der Link ist ungültig oder abgelaufen.',
  invalid_code: 'Der Code ist falsch oder abgelaufen.',
  invalid_mfa_token: 'Die Anmeldung ist abgelaufen – bitte erneut anmelden.',
  mfa_code_required: 'Bitte gib deinen Bestätigungscode ein.',
  reauth_required: 'Bitte melde dich erneut an, um das zu bestätigen.',
  totp_already_enabled: 'Die Authenticator-App ist bereits eingerichtet.',
  totp_not_started: 'Die Einrichtung ist abgelaufen – bitte neu beginnen.',
  totp_not_enabled: 'Die Authenticator-App ist nicht eingerichtet.',
  email_mfa_active: 'Schalte zuerst die Bestätigung per E-Mail aus.',
  email_mfa_already_enabled: 'Die Bestätigung per E-Mail ist bereits aktiv.',
  email_mfa_not_enabled: 'Die Bestätigung per E-Mail ist nicht aktiv.',
  mfa_not_enabled: 'Die Zwei-Faktor-Anmeldung ist nicht aktiv.',
  identity_in_use: 'Dieses Konto beim Anbieter ist bereits mit einem anderen Lykos-Konto verknüpft.',
  already_linked: 'Mit diesem Anbieter ist bereits ein Konto verknüpft.',
  last_login_method:
    'Das ist dein einziger Anmeldeweg. Lege zuerst ein Passwort fest oder verknüpfe einen anderen Anbieter.',
  oidc_unavailable: 'Der Anmeldedienst ist gerade nicht erreichbar.',
}

/** Fehlercodes aus dem URL-Fragment nach der Rückkehr vom OIDC-Anbieter. */
const oidcMessages: Record<string, string> = {
  cancelled: 'Die Anmeldung beim Anbieter wurde abgebrochen.',
  invalid_state: 'Die Anmeldung ist abgelaufen oder wurde in einem anderen Fenster gestartet. Bitte erneut versuchen.',
  oidc_failed: 'Die Anmeldung beim Anbieter ist fehlgeschlagen.',
  unknown_provider: 'Dieser Anmeldedienst ist nicht (mehr) eingerichtet.',
}

export function oidcErrorText(code: string): string {
  return oidcMessages[code] ?? messages[code] ?? `Anmeldung fehlgeschlagen (${code}).`
}

/** Fehler in eine deutsche Meldung für die Oberfläche übersetzen. */
export function errorText(e: unknown): string {
  if (e instanceof ApiError && e.status === 429) return messages.rate_limited
  if (e instanceof ApiError) return messages[e.code] ?? `Fehler (${e.status}: ${e.code})`
  return 'Unerwarteter Fehler.'
}
