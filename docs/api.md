# Lykos API (Phase 1)

Basis: `/api`. Alle Bodies sind JSON. IDs sind UUIDv7-Strings (zeitlich sortierbar).
Zeitstempel sind RFC 3339 (`2026-10-29T13:00:00Z`).

## Authentifizierung

Serverseitige Sessions. Ein Session-Token wird angenommen als

- `Authorization: Bearer <token>` (Desktop-Client), oder
- Cookie `lykos_session` (Web-Client; `HttpOnly`, `SameSite=Lax`, `Path=/api`).

Login und Registrierung (auch 2FA-Schritt und OIDC-Registrierung) setzen immer das Cookie. Das Token steht
zusätzlich im Body (`token`) – außer für den eigenen Web-Client (`Origin` = `public_url`): Dort trägt allein das
`HttpOnly`-Cookie die Sitzung, damit Skripte (z. B. bei einer XSS-Lücke) es nicht lesen können. Desktop-App,
Dev-Client und Werkzeuge ohne `Origin` erhalten es weiter.
Sessions laufen 30 Tage nach der letzten Nutzung ab und sind jederzeit widerrufbar.

## Fehler

Fehler antworten mit passendem Status und `{ "error": "<code>" }`.

| Status | Codes |
|---|---|
| 400 | `invalid_username`, `invalid_password`, `invalid_name`, `invalid_content`, `invalid_kind`, `invalid_request`, `not_a_text_channel`, `too_many_attachments`, `invalid_attachments`, `not_a_voice_channel`, `invalid_reply`, `invalid_emoji`, `too_many_reactions`, `too_many_pins`, `invalid_image`, `invalid_bio`, `invalid_status_text`, `too_many_recipients`, `not_a_group_dm`, `cannot_leave_dm`, `invalid_email`, `email_unavailable` (kein Mailversand bzw. keine bestätigte Adresse), `email_unchanged`, `invalid_token` (Link ungültig/abgelaufen/verbraucht), `invalid_code`, `totp_not_started`, `invalid_title`, `invalid_description`, `invalid_tech_info`, `invalid_status`, `not_a_suggestion`, `invalid_pronouns`, `invalid_color`, `invalid_channel`, `too_many_emojis`, `invalid_bitrate`, `invalid_user_limit`, `not_in_call` |
| 401 | `unauthorized`, `invalid_credentials`, `invalid_code` (2FA-Code falsch), `invalid_mfa_token` (zweiter Anmeldeschritt abgelaufen/verbraucht) |
| 403 | `forbidden`, `registration_closed`, `invite_required`, `wrong_password` (Passwortbestätigung falsch), `dm_not_allowed`, `friend_request_not_allowed`, `mfa_code_required`, `invalid_code` (Re-Authentifizierung), `reauth_required` (Konto ohne Passwort: Sitzung älter als 10 min), `account_disabled` (Konto gesperrt), `channel_full` (Sprachkanal hat sein Nutzerlimit erreicht), `call_not_allowed` (1:1-DM mit Blockade: kein Anruf) |
| 404 | `not_found`, `invalid_invite`, `invalid_reset_token` |
| 409 | `username_taken`, `already_member`, `owner_cannot_leave`, `owns_communities`, `last_instance_admin`, `email_taken`, `email_mfa_active`, `email_mfa_already_enabled`, `email_mfa_not_enabled`, `totp_already_enabled`, `totp_not_enabled`, `mfa_not_enabled`, `identity_in_use`, `last_login_method`, `already_voted`, `screenshot_exists`, `issue_exists`, `emoji_name_taken`, `already_friends`, `user_blocked`, `too_many_pending_requests` |
| 429 | `rate_limited` – mit Header `Retry-After: <Sekunden>` |
| 413 | `file_too_large`, `quota_exceeded` (Speicher-Kontingent von Nutzer oder Community erschöpft) |
| 503 | `voice_unavailable` (LiveKit nicht konfiguriert), `oidc_unavailable` (Anmeldeanbieter nicht erreichbar), `gitea_not_configured`, `gitea_unavailable` (Feedback-Weiterleitung) |

## Objekte

```ts
User      { id, username, display_name: string | null, is_instance_admin: boolean, created_at,
            avatar_url: string | null, bio: string | null, status_text: string | null,
            pronouns: string | null, banner_url: string | null, banner_color: number | null,
            accent_color: number | null }
Session   { id, label: string | null, created_at, last_used_at, expires_at, current: boolean }
Community { id, name, owner_id, message_retention_days: number | null, upload_quota_mb: number | null, created_at,
            icon_url: string | null, banner_url: string | null, description: string | null,
            accent_color: number | null, system_channel_id: string | null }
Channel   { id, community_id: string | null, name, kind: 'text' | 'voice' | 'dm' | 'group_dm', topic: string | null, position: number,
            audio_bitrate: number /* kbit/s, 32–256, Standard 64 */, audio_stereo: boolean, audio_dtx: boolean, audio_red: boolean,
            user_limit: number | null /* 1–99, null = unbegrenzt */ }   // audio_*/user_limit nur bei Sprachkanälen wirksam
Role      { id, community_id, name, permissions: number, position: number, color: number | null, is_default: boolean }
Member    { user: UserPublic, nickname: string | null, role_ids: string[], joined_at }
UserPublic{ id, username, display_name: string | null, avatar_url: string | null }
UserProfile{ id, username, display_name: string | null, avatar_url: string | null, bio: string | null,
            status_text: string | null, pronouns: string | null, banner_url: string | null,
            banner_color: number | null, accent_color: number | null, created_at }
Message   { id, channel_id, community_id: string | null /* null in DMs */, author: UserPublic | null, content, created_at,
            edited_at: string | null, attachments: Attachment[], reply_to_id: string | null,
            referenced_message: MessageReference | null, reactions: Reaction[], mentions: string[],
            mention_everyone: boolean, pinned_at: string | null, kind: 'default' | 'member_join' | 'call_missed' }
MessageReference { id, author: UserPublic | null, content /* max. 200 Zeichen */, has_attachments: boolean }
Reaction  { emoji /* Unicode oder `name:id` */, count: number, me: boolean, emoji_id: string | null, animated: boolean }
CustomEmoji { id, community_id, name, animated: boolean, url /* /api/emojis/{id}.{png|gif|webp} */,
            uploader: UserPublic | null, created_at }
Attachment{ id, channel_id, message_id: string | null, filename, content_type, size_bytes: number,
            width: number | null, height: number | null, created_at, url }
Invite    { code, community_id, max_uses: number | null, uses: number, expires_at: string | null, created_at }
AuditEntry{ id, actor: UserPublic | null, action: string, target_id: string | null, data: object, created_at }
ChannelOverride { role_id, allow: number, deny: number }
VoiceState{ community_id: string | null /* null in DM-Anrufen */, channel_id, user_id, self_mute: boolean, self_deaf: boolean, streaming: boolean, video: boolean, joined_at }
Relationship { id /* ID des anderen Nutzers */, type: 'friend' | 'pending_out' | 'pending_in' | 'blocked',
            user: UserPublic, since /* seit wann in dieser Form */ }
DmChannel { id, kind: 'dm' | 'group_dm', name: string | null, recipients: UserPublic[] /* inkl. man selbst */,
            last_message_id: string | null, last_read_message_id: string | null, mention_count: number }
Feedback  { id, number: number /* Meldungsnummer */, kind: 'bug' | 'suggestion' | 'other',
            status: 'neu' | 'in_arbeit' | 'erledigt' | 'abgelehnt' | 'duplikat', title, description /* Markdown */,
            vote_count: number, voted: boolean, mine: boolean, replies: FeedbackReply[], created_at, updated_at,
            // nur Melder und Admins:
            tech_info?: TechInfo | null, screenshot_url?: string | null,
            // nur Admins:
            reporter?: UserPublic | null, issue_url?: string | null }
FeedbackReply { id, author: UserPublic | null, body, internal: boolean /* nur Admins sehen interne Notizen */, created_at }
TechInfo  { app_version: string | null, platform: 'web' | 'desktop' | null, os: string | null,
            window: { width, height } | null, view: string | null, errors: { at: string | null, message }[] }
```

Audit-Aktionen (`action`, `target_id`, `data`): `community.create` (–, `{name}`), `community.update`
(–, `{name, message_retention_days, upload_quota_mb, description, accent_color, system_channel_id}`),
`community.icon_set|icon_remove|banner_set|banner_remove` (–, `{}`), `channel.create|update|delete` (Kanal, `{name, kind}`/geänderte Ton-Felder `{audio_bitrate?, audio_stereo?, audio_dtx?, audio_red?, user_limit?}` (sonst `{}`)/`{name}`),
`channel.override_set` (Kanal, `{role_id, allow, deny}`), `channel.override_delete` (Kanal, `{role_id}`),
`role.create|update|delete` (Rolle, `{name}`/`{name, permissions}`/`{name}`), `role.reorder` (–, `{role_ids}`),
`member.join|leave|kick`
(Nutzer), `member.role_add|role_remove` (Nutzer, `{role_id}`), `invite.create|delete` (–, `{code}`),
`message.delete` (Nachricht, `{channel_id, author_id}` – nur Moderation, ohne Inhalt),
`message.pin|unpin` (Nachricht, `{channel_id, author_id}`), `member.nickname` (Nutzer,
`{nickname}` – nur fremde Änderungen), `community.transfer` (neuer Besitzer, `{from}`),
`emoji.create|update|delete` (Emote, `{name}`/`{name, old_name}`/`{name}`).

## Berechtigungen (Bitflags, `permissions: number`)

| Bit | Name | Bedeutung |
|---|---|---|
| 1 << 0 | `VIEW_CHANNEL` | Kanal sehen, Verlauf lesen |
| 1 << 1 | `SEND_MESSAGES` | Nachrichten senden |
| 1 << 2 | `MANAGE_MESSAGES` | Fremde Nachrichten löschen, Nachrichten anpinnen/lösen |
| 1 << 3 | `ATTACH_FILES` | Dateien anhängen |
| 1 << 4 | `CONNECT` | Voice-Kanal betreten (Phase 2) |
| 1 << 5 | `SPEAK` | In Voice sprechen (Phase 2) |
| 1 << 6 | `STREAM` | Bildschirm übertragen (Phase 4) |
| 1 << 7 | `CREATE_INVITES` | Einladungen erstellen |
| 1 << 8 | `MANAGE_CHANNELS` | Kanäle anlegen/ändern/löschen |
| 1 << 9 | `MANAGE_ROLES` | Rollen und Kanal-Overrides verwalten |
| 1 << 10 | `KICK_MEMBERS` | Mitglieder entfernen |
| 1 << 11 | `MANAGE_COMMUNITY` | Name, Aufbewahrung, Einladungen verwalten |
| 1 << 12 | `VIEW_AUDIT_LOG` | Audit-Log lesen |
| 1 << 13 | `VIDEO` | Kamera in Voice-Kanälen nutzen |
| 1 << 14 | `ADD_REACTIONS` | Emoji-Reaktionen hinzufügen (eigene entfernen geht immer) |
| 1 << 15 | `MENTION_EVERYONE` | `@everyone` benachrichtigt alle (ohne das Recht bleibt es normaler Text) |
| 1 << 16 | `MANAGE_EMOJIS` | Custom-Emotes der Community hochladen, umbenennen, löschen |
| 1 << 20 | `ADMINISTRATOR` | Alles |
| 1 << 21 | `CHANGE_NICKNAME` | Eigenen Nickname in der Community ändern |
| 1 << 22 | `MANAGE_NICKNAMES` | Nicknames anderer Mitglieder ändern (nicht den des Besitzers) |

Effektive Rechte: Besitzer hat alles. Sonst ODER über alle Rollen (inkl. Standardrolle);
`ADMINISTRATOR` ⇒ alles. Im Kanal: Override der Standardrolle (erst `deny`, dann `allow`),
danach die Overrides der übrigen Rollen des Mitglieds zusammengefasst (erst `deny`, dann `allow`).
Wer Rollen verwaltet, kann nur Rechte vergeben, die er selbst besitzt, und nur Rollen unterhalb seiner höchsten
Rolle verwalten (siehe „Rollen“ → Hierarchie).

Standardrolle neuer Communities: `VIEW_CHANNEL | SEND_MESSAGES | ATTACH_FILES | CONNECT | SPEAK | STREAM | VIDEO | ADD_REACTIONS | CREATE_INVITES | CHANGE_NICKNAME`.
Migration `0007` hat `VIDEO`, Migration `0008` `ADD_REACTIONS` und Migration `0009` `CHANGE_NICKNAME` auch allen bestehenden Standardrollen hinzugefügt.
`MENTION_EVERYONE` und `MANAGE_EMOJIS` (Migration `0014`) sind bewusst nicht in der Standardrolle.

## Endpunkte

### Instanz
| Methode | Pfad | Auth | Beschreibung |
|---|---|---|---|
| GET | `/health` | – | `{ status, version, database }` |
| GET | `/instance` | – | `{ name, public_url, registration_mode: 'open'\|'invite_only'\|'closed', bootstrap: boolean, max_upload_bytes: number, voice: boolean, allow_community_creation: boolean, announcement: string\|null, email: boolean, oidc_providers: { id, label }[], source_url: string, legal: { imprint: boolean, privacy: boolean } }` – `bootstrap = true`, solange es noch keinen Nutzer gibt. Name, Registrierungsmodus, Upload-Größe, Community-Erstellung und Ankündigung sind die **effektiven** Werte (Laufzeit-Einstellung der Instanz-Verwaltung vor Umgebungsvariable). `email` = Mailversand eingerichtet (E-Mail-Adresse, Passwort vergessen, 2FA per E-Mail); `oidc_providers` = Anmeldeanbieter („Mit … anmelden“); `source_url` = Quellcode der laufenden Fassung (AGPL-3.0, `LYKOS_SOURCE_URL`); `legal` = welche Rechtstexte hinterlegt sind (siehe unten). |
| GET | `/legal/imprint` | – | `{ content: string }` – Impressum der Instanz als Markdown (`LYKOS_IMPRINT_FILE`); 404 `not_found`, wenn keins hinterlegt ist. |
| GET | `/legal/privacy` | – | `{ content: string }` – Datenschutzerklärung als Markdown (`LYKOS_PRIVACY_FILE`); sonst 404 `not_found`. |

### Desktop-App (öffentlich)
| Methode | Pfad | Auth | Beschreibung |
|---|---|---|---|
| GET | `/desktop/latest` | – | Neuester Windows-Installer: `{ version, notes, published_at: string\|null, release_url, file_name, download_url, size, sha256: string\|null }`; `404 no_release` ohne Konfiguration oder passendes Release, `503 desktop_unavailable`, wenn die Release-API nicht erreichbar ist und noch nichts gecacht war |
| GET | `/desktop/latest.json` | – | Statisches Tauri-Update-Manifest `{ version, notes, pub_date, platforms: { "windows-x86_64": { signature, url } } }`; `204` = kein Update (keine Konfiguration, kein Release oder keine `.sig`-Datei) |

Quelle ist die Gitea-Release-Liste aus `LYKOS_DESKTOP_RELEASES_URL` (z. B.
`https://api.github.com/repos/ZeasySoftware/Lykos/releases`; Gitea-Release-APIs funktionieren genauso). Verwendet wird das neueste
Release, das kein Entwurf ist (Vorabversionen zählen) und ein Asset `*-setup.exe` hat; `version` ist der
Tag ohne `v`. Daneben werden `<installer>.sha256` (`sha256sum`-Format) und `<installer>.sig`
(Tauri-Updater-Signatur) gelesen – nur vom selben Host wie die API. Das Ergebnis wird 10 Minuten
gecacht; fällt die API aus, bleibt der letzte Stand gültig (neuer Versuch nach 1 Minute). Beide Antworten
tragen `Cache-Control: public, max-age=300`. Die Desktop-App fragt `https://lykos.zeasy.dev/api/desktop/latest.json`
(fest in `tauri.conf.json`) und prüft die Signatur gegen den eingebauten Public Key.

### Auth
| Methode | Pfad | Body | Antwort |
|---|---|---|---|
| POST | `/auth/register` | `{ username, password, display_name?, invite?, session_label?, email? }` (`email` optional, wird erst nach Bestätigung aktiv) | `{ token, user }` |
| POST | `/auth/login` | `{ username, password, session_label? }` | `{ token, user }` oder bei aktiver 2FA `{ mfa_required: true, mfa_token, methods: ('totp'\|'email'\|'recovery')[] }` |
| POST | `/auth/logout` | – | 204 |
| GET | `/auth/me` | – | `User` |
| PATCH | `/auth/me` | `{ display_name?, bio?, status_text?, pronouns?, banner_color?, accent_color? }` (`null`/leer = entfernen) | `User` |
| DELETE | `/auth/me` | `{ password?, mfa_code?, mfa_method?, delete_messages?: boolean }` (Re-Auth) | 204 – Konto löschen, siehe unten |
| GET | `/auth/me/export` | – | JSON-Download (Datenexport), siehe unten |
| POST | `/auth/me/avatar` | `multipart/form-data`, Feld `file` | `User` |
| DELETE | `/auth/me/avatar` | – | `User` |
| POST | `/auth/me/banner` | `multipart/form-data`, Feld `file` | `User` – Profilbanner, siehe „Gestaltung“ |
| DELETE | `/auth/me/banner` | – | `User` |
| POST | `/auth/password` | `{ current_password?, new_password, logout_other_sessions?: boolean = true, mfa_code?, mfa_method? }` (Re-Auth; ohne bisheriges Passwort setzt es ein erstes) | 204 |
| GET | `/auth/me/usage` | – | `{ used_bytes, quota_bytes: number \| null }` – eigene Belegung durch Anhänge (`null` = unbegrenzt) |
| GET | `/auth/sessions` | – | `Session[]` |
| DELETE | `/auth/sessions` | – | 204 – alle **anderen** Sessions beenden |
| DELETE | `/auth/sessions/{id}` | – | 204 |
| POST | `/auth/password-reset/verify` | `{ token }` | `{ username, expires_at }` – Reset-Link prüfen (Admin-Link 24 h oder E-Mail-Link 30 min, gemeinsame Tabelle `password_reset_tokens`); `404 invalid_reset_token`, `403 account_disabled` |
| POST | `/auth/password-reset/complete` | `{ token, new_password }` | 204 – neues Passwort setzen, Link verbraucht, **alle** Sessions und offene 2FA-Anmeldungen beendet (2FA bleibt aktiv); `404 invalid_reset_token`, `403 account_disabled` |

Gesperrte Konten (Instanz-Verwaltung): `POST /auth/login`, `/auth/login/mfa`, `/auth/login/mfa/email`, OIDC-Callback (`#oidc_error=account_disabled`) und Passwort-Reset → `403 account_disabled` (Login nur bei richtigem Passwort;
sonst wie gewohnt `401`). Bestehende Tokens gesperrter Konten gelten nicht mehr (`401`).
Registrierung: `invite` darf eine Community- **oder** eine Instanz-Einladung sein (siehe „Instanz-Verwaltung“).
Bei `invite_only` ist eine gültige Einladung Pflicht, bei `closed` geht nichts (`403 registration_closed`), auch
nicht mit Einladung. Ungültige/abgelaufene/aufgebrauchte Einladung → `404 invalid_invite`, kein Konto wird angelegt.

Rate-Limits (pro Server-Instanz im Speicher; IPs werden weder gespeichert noch geloggt):

| Endpunkt | Limit | Zeitfenster |
|---|---|---|
| `POST /auth/login` | 10 Fehlversuche pro Benutzername (ohne Groß/Klein-Unterschied) **und** Client-IP – Fremde sperren ein Konto so nur für ihre eigene IP | 15 min |
| `POST /auth/login` | 30 Fehlversuche pro Client-IP | 15 min |
| `POST /auth/register` | 5 Versuche pro Client-IP (formal ungültige Anfragen zählen nicht) | 1 h |
| `POST /auth/login/mfa`, Re-Auth | Fehlversuche zählen wie Anmelde-Fehlversuche (Benutzername + IP); je `mfa_token` höchstens 5 Versuche | 15 min |
| `POST /auth/password/forgot` | 5 Anfragen pro Client-IP (→ 429); 3 Reset-Mails pro Konto (darüber still keine Mail) | 1 h |
| Bestätigungsmails, E-Mail-Codes | 5 pro Konto (→ 429); je E-Mail-Code höchstens 5 Versuche | 1 h |
| `POST /relationships`, `PUT`/`DELETE /relationships/{user_id}` | 30 pro Nutzer (zusammen) | 1 min |
| `POST /dms` | 10 pro Nutzer | 1 min |
| `POST /channels/{id}/messages` | 120 pro Nutzer über alle Kanäle (kurze Schübe sind möglich) | 1 min |

Ist ein Limit erreicht, antwortet der Server mit 429 `rate_limited` – bei der Anmeldung auch für
das richtige Passwort. Eine erfolgreiche Anmeldung setzt den Zähler des Benutzernamens für diese IP zurück.
Client-IP: Socket-Adresse; kommt die Verbindung von einem Proxy aus `LYKOS_TRUSTED_PROXIES` (ohne Liste: von
jeder Verbindung, wenn `LYKOS_TRUST_PROXY=true`), stattdessen der letzte Eintrag von `X-Forwarded-For` (den der
vorgeschaltete Reverse Proxy anhängt). IPv6 wird pro /64 gezählt.

Regeln: Benutzername 2–32 Zeichen `[A-Za-z0-9_.-]`, eindeutig ohne Groß/Klein-Unterschied.
Passwort 10–256 Zeichen. Der allererste Nutzer darf sich ohne Einladung registrieren und wird
Instanz-Admin. `bio` („Über mich“): max. 190 Zeichen, `status_text`: max. 128 Zeichen (getrimmt, ohne
Steuerzeichen; zu lang → `invalid_bio` bzw. `invalid_status_text`, dann wird nichts geändert). `pronouns`: max. 40 Zeichen,
eine Zeile (`invalid_pronouns`). Farben (`banner_color`, `accent_color`) sind 24-Bit-RGB als Zahl `0xRRGGBB`
(0–16 777 215, sonst `invalid_color`).

**Passwortbestätigung** (`POST /auth/password`, `DELETE /auth/me`): falsches Passwort → `403 wrong_password`.
Zählt wie ein Anmeldeversuch für die Rate-Limits (Benutzername + IP), damit ein gestohlenes Token nicht zum
Durchprobieren taugt. Beim Passwortwechsel werden standardmäßig alle anderen Sessions beendet (Gateway-Close 4001).

### Konto-Sicherheit: E-Mail, Passwort vergessen, 2FA, externe Anmeldung

Alles freiwillig. Antworten verraten nie, ob ein Konto oder eine Adresse existiert; Tokens und Codes werden nur
als Hash gespeichert. **Re-Auth** (`Reauth = { password?, mfa_code?, mfa_method?: 'totp'|'email'|'recovery' }`):
Passwort, falls das Konto eines hat (sonst muss die Sitzung jünger als 10 min sein → `403 reauth_required`), plus
2FA-Code, falls 2FA aktiv ist (`403 mfa_code_required` / `invalid_code`; Standardmethode `totp`, sonst `email`; nur
eingerichtete Methoden zählen).

| Methode | Pfad | Body | Antwort |
|---|---|---|---|
| POST | `/auth/login/mfa` | `{ mfa_token, method, code }` | `{ token, user }` (+ Cookie) |
| POST | `/auth/login/mfa/email` | `{ mfa_token }` | 202 – Code per E-Mail (nur bei aktiver 2FA per E-Mail) |
| GET | `/auth/security` | – | `{ email: string\|null, pending_email: string\|null, has_password, mfa: { totp, email, recovery_codes_left }, identities: { provider, label: string\|null, created_at, last_used_at }[] }` |
| PUT | `/auth/email` | `{ email, ...Reauth }` | 202 `{ pending_email }` – Bestätigungslink `/verify-email#token=…` (24 h) an die neue Adresse |
| DELETE | `/auth/email` | `Reauth` | 204 – bestätigte und ausstehende Adresse entfernen (`409 email_mfa_active`, solange 2FA per E-Mail an ist) |
| POST | `/auth/email/verify` | `{ token }` | `{ email }` – ohne Anmeldung; `409 email_taken`, wenn inzwischen anderweitig bestätigt |
| POST | `/auth/password/forgot` | `{ login }` (Benutzername oder bestätigte E-Mail) | immer 202, sofort (Suche und Versand laufen danach im Hintergrund – die Antwortzeit verrät kein Konto) – Mail mit Link `/reset-password#token=…` (Token im Fragment; ältere `?token=`-Links liest der Client weiter; 30 min; ersetzt ältere E-Mail-Links, gesperrte Konten bekommen keinen), einzulösen über `/auth/password-reset/verify` und `…/complete` |
| POST | `/auth/mfa/totp/setup` | `Reauth` | `{ secret, otpauth_uri, qr_svg }` – noch nicht aktiv |
| POST | `/auth/mfa/totp/enable` | `{ code }` | `{ recovery_codes: string[] \| null }` (Codes nur bei der ersten 2FA-Methode) |
| DELETE | `/auth/mfa/totp` | `Reauth` | 204 |
| POST | `/auth/mfa/email` | `Reauth` | `{ recovery_codes \| null }` – 2FA per E-Mail an (nur mit bestätigter Adresse) |
| DELETE | `/auth/mfa/email` | `Reauth` | 204 |
| POST | `/auth/mfa/recovery-codes` | `Reauth` | `{ recovery_codes }` – 10 neue, alte ungültig |
| POST | `/auth/mfa/email-code` | – | 202 – Code für eine Re-Auth per E-Mail |
| POST | `/auth/oidc/{provider}/start` | `{ mode: 'login'\|'link', session_label? }`, bei `link` zusätzlich `Reauth` | `{ url }` + Cookie `lykos_oidc` (Ablauf an Browser gebunden); `link` nur angemeldet und mit Bestätigung wie beim Trennen (`403 wrong_password`/`reauth_required`/`mfa_code_required`/`invalid_code`) |
| GET | `/auth/oidc/{provider}/callback` | Query `code`, `state` | 303 in den Web-Client, Ergebnis im Fragment (siehe unten) |
| POST | `/auth/oidc/signup` | `{ signup_token }` | `{ provider, provider_label, email: string\|null }` |
| POST | `/auth/oidc/register` | `{ signup_token, username, display_name?, invite?, use_email?: boolean }` | 201 `{ token, user }` – gleiche Regeln wie `/auth/register` |
| DELETE | `/auth/identities/{provider}` | `Reauth` | 204; `409 last_login_method`, wenn danach weder Passwort noch andere Identität bliebe |

**2FA:** TOTP nach RFC 6238 (SHA1, 6 Stellen, 30 s, ±1 Schritt), jeder Zeitschritt nur einmal (Replay-Schutz über
Sitzungen hinweg). Wiederherstellungscodes `xxxxx-xxxxx` (10 Stück, einmalig, Schreibweise egal). E-Mail-Codes
6-stellig, 10 min. `mfa_token` gilt 5 min. Das TOTP-Geheimnis gibt der Server nur bei `setup` heraus.

**Externe Anmeldung** (Anbieter in `/instance` → `oidc_providers`): OIDC (Discovery, ID-Token mit Prüfung von
Signatur, `iss`, `aud`/`azp`, `exp`, `nonce`) oder reines OAuth2 (Discord, GitHub; Identität über den
Userinfo-Endpunkt). Immer `state` + PKCE S256. Gespeichert werden nur `(provider, subject)`. Rückkehr per
`303` auf `/` (angemeldet, Session-Cookie), `/#mfa_token=…&mfa_methods=totp,recovery`, `/#oidc_signup=…`
(Benutzername wählen), `/#oidc_linked=<provider>` oder `/#oidc_error=<code>` mit `cancelled`, `invalid_state`,
`oidc_failed`, `unknown_provider`, `registration_closed`, `identity_in_use`, `already_linked`.

### Profile & Avatare
| Methode | Pfad | Recht | Antwort |
|---|---|---|---|
| GET | `/users/{id}` | selbst, gemeinsame Community, gemeinsame DM, Freund oder eingehende Anfrage (der andere hat mich angefragt) | `UserProfile & { mutual_community_ids: string[] }` |
| GET | `/users/{id}/avatar/{avatar_id}` | selbst oder gemeinsame Community | Bild |
| GET | `/users/{id}/banner/{banner_id}` | selbst oder gemeinsame Community | Bild (Profilbanner) |

Sonst jeweils `404` (Existenz nicht verraten). `avatar_url` ist `/api/users/{id}/avatar/{avatar_id}` oder `null`.
Jeder Upload bekommt eine neue `avatar_id` → die URL ist unveränderlich (`Cache-Control: private, max-age=31536000,
immutable`); alte URLs liefern `404`. Auth wie bei Anhängen (Cookie oder Bearer, im Desktop-Client per `fetch`).

**Upload** (`POST /auth/me/avatar`): PNG, JPEG, WebP oder GIF (Magic Bytes, nicht Endung), höchstens 8 MB und
4096 px Kantenlänge, sonst `400 invalid_image` bzw. `413 file_too_large`. Der Server dekodiert keine Bilder; der
Client schneidet vorher quadratisch zu, verkleinert auf 256 px und kodiert neu (Canvas → WebP/PNG) – das entfernt
nebenbei Metadaten wie EXIF/GPS. Das alte Bild wird beim Ersetzen/Entfernen sofort gelöscht.

### Datenschutz: Export & Konto löschen

**`GET /auth/me/export`** liefert `Content-Disposition: attachment; filename="lykos-export-{username}-{datum}.json"`:

```ts
{ format: 'lykos-export-1', instance, exported_at, profile: User,
  account_security: { email, pending_email, has_password, two_factor: { totp, email, recovery_codes_left },
                     identities: { provider, subject, created_at, last_used_at }[] },
  sessions: Session[], feedback: Feedback[],
  relationships: Relationship[], privacy: { friend_requests: 'everyone' | 'mutual_communities' | 'nobody' },
  memberships: { community_id, community_name, is_owner, nickname, joined_at, roles: string[] }[],
  messages: { id, community_id, community_name, channel_id, channel_name, content, created_at, edited_at }[],
  attachments: { id, community_name, channel_name, message_id, filename, content_type, size_bytes, width, height, created_at }[] }
```

Anhänge erscheinen nur als Metadaten (eigene Uploads); keine Passwort-/Token-Hashes. `profile` enthält auch
Pronomen, Banner-URL und Farben; Avatar und Profilbanner sind über ihre URLs abrufbar.

**`DELETE /auth/me`** löscht das Konto endgültig:

- Besitzer einer Community → `409 owns_communities` (vorher übertragen: `POST /communities/{id}/transfer`, oder löschen).
- Letzter Instanz-Admin → `409 last_instance_admin`.
- `delete_messages: false` (Standard): Nachrichten bleiben, `author` wird `null` („Gelöschter Nutzer“).
  `delete_messages: true`: eigene Nachrichten samt Anhängen werden gelöscht (`MESSAGE_DELETE` je Nachricht).
- Immer gelöscht: Profil, Avatar, Profilbanner, E-Mail-Adresse, 2FA-Daten, verknüpfte Identitäten, Sessions (Gateway-Close 4001), Mitgliedschaften (`MEMBER_REMOVE`),
  Freunde, Anfragen und Blockaden in beide Richtungen (die Gegenseite bekommt `RELATIONSHIP_REMOVE`), Rollen,
  Lesestände, Voice-Status (auch ein laufender DM-Anruf: die anderen bekommen `VOICE_STATE_DELETE`), Klingeln bei
  ihm und von ihm (`CALL_UPDATE`), nicht gesendete Uploads. Einladungen und Audit-Einträge bleiben ohne Verweis.
- Der Benutzername wird wieder frei. Bei `invite_only` ist ein gültiger Einladungscode Pflicht; man tritt der
Community der Einladung direkt bei.

### Communities
| Methode | Pfad | Recht | Antwort |
|---|---|---|---|
| GET | `/communities` | – | `Community[]` (eigene) |
| POST | `/communities` | – | `CommunityDetail` – legt Standardrolle, `#allgemein` (Text) und `Sprachkanal` (Voice) an; ist `allow_community_creation` aus, nur für Instanz-Admins (sonst `403 community_creation_disabled`) |
| GET | `/communities/{id}` | Mitglied | `CommunityDetail` |
| PATCH | `/communities/{id}` | `MANAGE_COMMUNITY` | `{ name?, message_retention_days?, upload_quota_mb?, description?, accent_color?, system_channel_id? }` → `Community` (`message_retention_days: null` = unbegrenzt; `upload_quota_mb`: 1–10 000 000 oder `null` = kein Community-Kontingent; Gestaltung siehe unten) |
| POST | `/communities/{id}/icon` | `MANAGE_COMMUNITY` | `multipart/form-data`, Feld `file` → `Community` |
| DELETE | `/communities/{id}/icon` | `MANAGE_COMMUNITY` | `Community` |
| POST | `/communities/{id}/banner` | `MANAGE_COMMUNITY` | `multipart/form-data`, Feld `file` → `Community` |
| DELETE | `/communities/{id}/banner` | `MANAGE_COMMUNITY` | `Community` |
| GET | `/communities/{id}/icon/{icon_id}` | Mitglied | Bild |
| GET | `/communities/{id}/banner/{banner_id}` | Mitglied | Bild |
| GET | `/communities/{id}/usage` | `MANAGE_COMMUNITY` | `{ used_bytes, quota_bytes: number \| null }` – Belegung durch Anhänge der Community |
| DELETE | `/communities/{id}` | Besitzer | 204 |
| GET | `/communities/{id}/members` | Mitglied | `Member[]` |
| DELETE | `/communities/{id}/members/{user_id}` | selbst oder `KICK_MEMBERS` (fremde nur, wenn deren höchste Rolle echt unter der eigenen steht; Besitzer immer) | 204 |
| PATCH | `/communities/{id}/members/{user_id}` bzw. `/members/@me` | siehe unten | `{ nickname: string \| null }` → `Member` |
| POST | `/communities/{id}/transfer` | Besitzer | `{ user_id }` → `Community` – Besitz übertragen (Ziel muss Mitglied sein, sonst 404) |
| PUT | `/communities/{id}/members/{user_id}/roles/{role_id}` | `MANAGE_ROLES` | 204 |
| DELETE | `/communities/{id}/members/{user_id}/roles/{role_id}` | `MANAGE_ROLES` | 204 |
| GET | `/communities/{id}/audit-log?before=&limit=` | `VIEW_AUDIT_LOG` | `AuditEntry[]` |

**Nickname:** eigener mit `CHANGE_NICKNAME` (oder `MANAGE_NICKNAMES`), fremder mit `MANAGE_NICKNAMES`;
den Nickname des Besitzers ändert nur er selbst. Fremde Nicknames folgen der Rollenhierarchie: Wer nicht
Besitzer ist, ändert sie nur bei Mitgliedern, deren höchste Rolle echt unter der eigenen höchsten steht (Mitglieder
nur mit Standardrolle stehen unter jeder Rolle; ohne eigene Rolle → `403`). 1–32 Zeichen, eine Zeile; `null`/leer entfernt ihn.
Fremde Änderungen landen im Audit-Log (`member.nickname`). Anzeige im Client: Nickname > `display_name` > `username`.

### Gestaltung (Community & Profil)

**Community** (Recht `MANAGE_COMMUNITY`):

- `icon_url` (`/api/communities/{id}/icon/{icon_id}`): PNG, JPEG, WebP oder GIF, höchstens 2 MB und 1024 px
  Kantenlänge. Der Client schneidet quadratisch zu und skaliert auf 256 px (Canvas → WebP/PNG, entfernt
  EXIF/GPS); animierte GIFs bis 1 MB lädt er unverändert hoch.
- `banner_url` (`/api/communities/{id}/banner/{banner_id}`): höchstens 4 MB und 2048 px Kantenlänge; der Client
  schneidet auf 960×540 zu. Wird oben in der Kanalleiste gezeigt (Name auf dem Banner).
- `description`: max. 300 Zeichen (getrimmt, `null`/leer entfernt, sonst `invalid_description`).
- `accent_color`: `0xRRGGBB` oder `null` (`invalid_color`); Clients nutzen sie nur dezent (Banner-Verlauf ohne
  Bild, Auswahlindikator in der Community-Leiste).
- `system_channel_id`: Textkanal derselben Community oder `null` (sonst `invalid_channel`). Ist er gesetzt, schreibt der
  Server bei jedem Beitritt (Einladung annehmen, Registrierung mit Einladung) eine **Systemnachricht**
  `kind: 'member_join'` mit leerem `content` und dem neuen Mitglied als `author` (kommt als normales
  `MESSAGE_CREATE`). Systemnachrichten lassen sich nicht bearbeiten (`400 invalid_request`), aber wie andere
  Nachrichten löschen. Wird der Kanal gelöscht, wird `system_channel_id` wieder `null`.

Jeder Upload bekommt eine neue Bild-ID → URLs sind unveränderlich (`Cache-Control: private, max-age=31536000,
immutable`), das vorige Bild wird sofort gelöscht, alte URLs liefern `404`. Fehler wie beim Avatar
(`400 invalid_image`, `413 file_too_large`). Jede Änderung geht als `COMMUNITY_UPDATE` an alle Mitglieder und
landet im Audit-Log; beim Löschen der Community werden Icon und Banner sofort mitgelöscht.

**Sichtbarkeit:** Icon, Banner, Beschreibung und Akzentfarbe sind für Mitglieder sichtbar und – bewusst – auf der
öffentlichen Einladungsseite für alle mit einem gültigen Einladungscode (`GET /invites/{code}` liefert sie mit,
Bilder über `/api/invites/{code}/icon|banner/{bild_id}`, ohne Anmeldung, `Cache-Control: private, max-age=3600`).
Ohne gültige Einladung (abgelaufen, aufgebraucht, gelöscht) und für Nicht-Mitglieder über den Community-Pfad: `404`.
`system_channel_id` sehen nur Mitglieder.

**Profil** (eigenes Konto, `PATCH /auth/me`, `POST|DELETE /auth/me/banner`):

- `banner_url` (`/api/users/{id}/banner/{banner_id}`): Bild wie beim Community-Banner (max. 4 MB/2048 px), der
  Client schneidet auf 600×240 zu. Ohne Bild zeigt der Client `banner_color` (sonst einen neutralen Verlauf).
- `accent_color`: Rahmen/Verlauf der Profilkarte.
- `pronouns`: max. 40 Zeichen.

Profilbanner sehen dieselben Nutzer wie den Avatar (selbst, gemeinsame Community, gemeinsame DM). `GET /users/{id}`
liefert zusätzlich `mutual_community_ids` (gemeinsame Communities; beim eigenen Profil alle eigenen) – nur dort,
nicht in `USER_UPDATE`.

Der Aufräum-Job kennt Avatare, Profilbanner, Community-Icons und -Banner; Dateien ohne Verweis (z. B. nach
abgebrochenen Vorgängen) entfernt er nach der Schonfrist wie verwaiste Anhänge.

`CommunityDetail = { community, channels: Channel[] (nur sichtbare), roles: Role[], permissions: number (Community-Ebene), channel_permissions: { [channel_id]: number }, read_states: { [channel_id]: message_id }, last_message_ids: { [channel_id]: message_id }, mention_counts: { [channel_id]: number } }`

`read_states` (eigener Lesestand) und `last_message_ids` (neueste Nachricht) gibt es nur für sichtbare
Textkanäle; Kanäle ohne Lesestand bzw. ohne Nachrichten fehlen in der jeweiligen Map.
Ein Kanal ist ungelesen, wenn `last_message_ids[c] > read_states[c]` (UUIDv7 sind als Zeichenkette sortierbar)
oder kein Lesestand existiert.
`mention_counts` zählt je sichtbarem Textkanal die ungelesenen fremden Nachrichten (nach dem Lesestand), die den Nutzer
erwähnen oder `mention_everyone` haben; Kanäle ohne solche Nachrichten fehlen.

### Kanäle
| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| POST | `/communities/{id}/channels` | `MANAGE_CHANNELS` | `{ name, kind, topic? }` → `Channel` |
| PATCH | `/channels/{id}` | `MANAGE_CHANNELS` | `{ name?, topic?, position?, audio_bitrate?, audio_stereo?, audio_dtx?, audio_red?, user_limit? }` → `Channel` |
| DELETE | `/channels/{id}` | `MANAGE_CHANNELS` | 204 |
| GET | `/channels/{id}/overrides` | `MANAGE_ROLES` | `ChannelOverride[]` = `[{ role_id, allow, deny }]` |
| PUT | `/channels/{id}/overrides/{role_id}` | `MANAGE_ROLES` | `{ allow, deny }` → 204 |
| DELETE | `/channels/{id}/overrides/{role_id}` | `MANAGE_ROLES` | 204 |

**Ton-Einstellungen (nur Sprachkanäle, sonst `400 not_a_voice_channel`):** `audio_bitrate` = Ziel-Bitrate der
Mikrofonspur in kbit/s (32–256, sonst `400 invalid_bitrate`), `audio_stereo` = Opus in Stereo, `audio_dtx` =
Discontinuous Transmission (Sprechpausen kaum übertragen), `audio_red` = Redundant Audio Data (robuster bei
Paketverlust), `user_limit` = Höchstzahl gleichzeitiger Teilnehmer (1–99, `null` = unbegrenzt, sonst
`400 invalid_user_limit`). Neue Kanäle: 64 kbit/s, Mono, DTX und RED an, kein Limit. Die Werte gibt der Server beim
Voice-Join an den Client (`audio`), der sie beim Veröffentlichen des Mikrofons anwendet; wer schon im Kanal ist,
bekommt Änderungen erst beim nächsten Beitritt.

Kanalnamen: 1–64 Zeichen. Textkanäle werden kleingeschrieben, Leerzeichen → `-`.

### Nachrichten
| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| GET | `/channels/{id}/messages?before={message_id}&limit={1..100}` | `VIEW_CHANNEL` | `Message[]`, neueste zuerst |
| GET | `/channels/{id}/messages?around={message_id}&limit={1..100}` | `VIEW_CHANNEL` | `Message[]` um die Nachricht herum (bis `limit/2` neuere, Rest inkl. `around` und älter), neueste zuerst |
| GET | `/channels/{id}/messages?after={message_id}&limit={1..100}` | `VIEW_CHANNEL` | `Message[]`: die nächsten `limit` neueren Nachrichten, neueste zuerst |
| POST | `/channels/{id}/messages` | `SEND_MESSAGES` | `{ content, attachment_ids?, reply_to_id? }` → `Message` |
| PATCH | `/messages/{id}` | Autor mit `SEND_MESSAGES` im Kanal (blockierte DM: nein) | `{ content }` → `Message` |
| DELETE | `/messages/{id}` | Autor oder `MANAGE_MESSAGES` | 204 |

Inhalt: 1–4000 Zeichen (nach Trim nicht leer). Mit mindestens einem Anhang darf `content` leer sein.
`attachment_ids`: höchstens 10 (`too_many_attachments`), zusätzlich `ATTACH_FILES` nötig. Jede ID muss ein
eigener, noch nicht verknüpfter Upload aus demselben Kanal sein, sonst `invalid_attachments` (es entsteht keine
Nachricht). Löschen einer Nachricht löscht auch ihre Anhänge samt Dateien.

**Antworten:** `reply_to_id` muss eine Nachricht im selben Kanal sein, sonst `invalid_reply`. `referenced_message` ist eine
kompakte Vorschau des Originals (Inhalt auf 200 Zeichen gekürzt). Wird das Original gelöscht, bleibt die Antwort erhalten:
`reply_to_id` bleibt gesetzt, `referenced_message` ist dann `null` („Ursprüngliche Nachricht gelöscht“).

**Erwähnungen:** Beim Senden und Bearbeiten löst der Server `@benutzername` (Zeichen `[A-Za-z0-9_.-]`, ohne Groß/Klein-
Unterschied; Satzzeichen am Ende wie in `@bob.` werden zusätzlich abgeschnitten probiert) zu `mentions` (Nutzer-IDs) auf –
nur Mitglieder der Community. Ein `@` direkt nach einem Namenszeichen (E-Mail-Adressen) zählt nicht, ebenso wenig
Erwähnungen in Code (`` `…` `` oder ```` ```…``` ````). `@everyone` setzt `mention_everyone` nur mit `MENTION_EVERYONE`
(sonst wird die Nachricht trotzdem gesendet, aber ohne Erwähnung).

**Inhalt/Markdown:** Der Server speichert Rohtext. Clients rendern eine kleine Markdown-Teilmenge: `**fett**`, `*kursiv*`/
`_kursiv_`, `~~durchgestrichen~~`, `` `Code` ``, Codeblöcke mit ```` ``` ````, `> Zitat`, `||Spoiler||`, automatische Links
(nur `http`/`https`); `\` maskiert ein Steuerzeichen. Custom-Emotes (`<:name:id>`, siehe „Custom-Emotes“) erscheinen als
Bild (22 px); besteht eine Nachricht nur aus höchstens 27 Emojis (Unicode oder Custom), werden sie groß (48 px) gezeigt.

### Reaktionen
| Methode | Pfad | Recht | Antwort |
|---|---|---|---|
| PUT | `/messages/{id}/reactions/{emoji}/@me` | `VIEW_CHANNEL` + `ADD_REACTIONS` | 204 (idempotent) |
| DELETE | `/messages/{id}/reactions/{emoji}/@me` | `VIEW_CHANNEL` | 204 (idempotent) |

`{emoji}` ist ein prozentkodiertes Unicode-Emoji (höchstens 16 Codepunkte, nur Emoji-Zeichen samt Verbindern,
Varianten-Selektoren, Hauttönen, Flaggen und Tastenkappen wie `1️⃣`), sonst `invalid_emoji`. Pro Nachricht höchstens
20 verschiedene Emojis (`too_many_reactions`). `Message.reactions` ist nach der ersten Reaktion sortiert; `me` gilt für
den abfragenden Nutzer.

**Custom-Emotes** als Reaktion: `{emoji}` = `name:id` (z. B. `wolf:0192…`). Maßgeblich ist nur die ID, der Name im Pfad
wird ignoriert. Eine **neue** Reaktion mit einem Emote setzt nur, wer Mitglied der Community des Emotes ist (sonst
`403`); einer schon bestehenden Reaktion darf sich jeder anschließen, der reagieren darf. Unbekannte/gelöschte Emotes
ohne bestehende Reaktion → `invalid_emoji`. Im Aggregat ist `emoji` dann `name:id` mit dem aktuellen Namen (bei
gelöschten Emotes dem zuletzt bekannten), dazu `emoji_id` und `animated`; Unicode-Reaktionen haben `emoji_id: null`.

### Custom-Emotes

| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| GET | `/emojis` | angemeldet | `CustomEmoji[]` aller eigenen Communities (= überall verwendbar) |
| GET | `/emojis/{id}.{ext}` | angemeldet | Bild (Endung optional und ohne Bedeutung) |
| GET | `/communities/{id}/emojis` | Mitglied oder Instanz-Admin | `CustomEmoji[]`, alphabetisch |
| POST | `/communities/{id}/emojis` | `MANAGE_EMOJIS` oder Instanz-Admin | `multipart/form-data` mit `name` und `file` → 201 `CustomEmoji` |
| PATCH | `/communities/{id}/emojis/{emoji_id}` | `MANAGE_EMOJIS` oder Instanz-Admin | `{ name }` → `CustomEmoji` |
| DELETE | `/communities/{id}/emojis/{emoji_id}` | `MANAGE_EMOJIS` oder Instanz-Admin | 204 |

Nicht-Mitglieder bekommen `404`, Mitglieder ohne Recht `403`. Instanz-Admins dürfen auch ohne Mitgliedschaft verwalten.

- **Name:** 2–32 Zeichen `[A-Za-z0-9_]`, eindeutig pro Community ohne Groß/Klein-Unterschied (`409 emoji_name_taken`),
  sonst `400 invalid_name`.
- **Datei:** PNG, GIF oder WebP (Magic Bytes), höchstens 256 KB (`413 file_too_large`) und 128 × 128 px, sonst
  `400 invalid_image`. GIFs gelten als animiert (`animated: true`). Der Server dekodiert keine Bilder; der Client
  verkleinert statische Bilder vorher per Canvas, GIFs werden nie neu kodiert (Animation bliebe sonst nicht erhalten).
- **Limit:** 100 Emotes pro Community (`400 too_many_emojis`).
- **Abruf:** Die ID ändert sich nie (Umbenennen ändert nur den Namen) → `Cache-Control: private, max-age=31536000,
  immutable`, `X-Content-Type-Options: nosniff`. Auth wie bei Avataren (Cookie oder Bearer, Desktop per `fetch`).
- **Datenschutz-Abwägung:** Emote-Bilder sind für **alle angemeldeten Nutzer der Instanz** abrufbar. Emotes dürfen
  überall verwendet werden (auch in DMs und fremden Communities), eine exakte Prüfung „in einer für dich sichtbaren
  Nachricht/Reaktion verwendet“ wäre bei jedem Bildabruf eine teure Suche. Die ID ist eine zufällige UUID und nur aus
  Nachrichten/Reaktionen bekannt; der Bildabruf verrät weder Name noch Community, und die Emote-Liste einer Community
  sehen nur ihre Mitglieder. Clients zeigen die Herkunfts-Community im Tooltip daher nur, wenn man selbst Mitglied ist.
- **Löschen:** Datei sofort weg; beim Löschen der Community entfernt die Datenbank die Emotes, die Dateien räumt der
  Hintergrund-Job ab. Nachrichten behalten ihr Token, Clients zeigen dann den Platzhalter `:name:`.
- Jede Änderung geht als `EMOJIS_UPDATE` an die Community und landet im Audit-Log (`emoji.create|update|delete`).

**Im Nachrichtentext** wie bei Discord: `<:name:id>` bzw. `<a:name:id>` (animiert). Beim Senden und Bearbeiten prüft
der Server jedes Token (außerhalb von Code): Das Emote muss existieren und der Absender Mitglied seiner Community sein –
sonst wird das Token zu `:name:` degradiert. Gültige Tokens werden auf den aktuellen Namen und das richtige Präfix
normalisiert. Wer eine Community verlässt, verliert die Emotes erst beim nächsten Senden/Bearbeiten.

### Angepinnte Nachrichten
| Methode | Pfad | Recht | Antwort |
|---|---|---|---|
| GET | `/channels/{id}/pins` | `VIEW_CHANNEL` | `Message[]`, zuletzt angepinnte zuerst |
| PUT | `/channels/{id}/pins/{message_id}` | `MANAGE_MESSAGES` (DM: `SEND_MESSAGES`) | 204 (idempotent) |
| DELETE | `/channels/{id}/pins/{message_id}` | `MANAGE_MESSAGES` (DM: `SEND_MESSAGES`) | 204 (idempotent) |

Nur Textkanäle; die Nachricht muss im Kanal liegen (sonst 404). Höchstens 50 Pins pro Kanal (`too_many_pins`).
Jede Änderung landet im Audit-Log und geht als `MESSAGE_UPDATE` (mit `pinned_at`) an alle, die den Kanal sehen.

### Dateianhänge
| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| POST | `/channels/{id}/attachments` | `VIEW_CHANNEL` + `ATTACH_FILES`, Textkanal | `multipart/form-data` mit genau einem Feld `file` → 201 `Attachment` |
| GET | `/attachments/{id}/{filename}` | `VIEW_CHANNEL` (unverknüpft: nur Uploader) | Dateiinhalt |
| DELETE | `/attachments/{id}` | Uploader, nur unverknüpft | 204 – Entwurf verwerfen |

Ablauf: Datei hochladen → `Attachment` (noch `message_id: null`) → Nachricht mit `attachment_ids` senden.
Nicht verknüpfte Uploads werden nach 24 h gelöscht.

- **Größe:** höchstens die Instanz-Einstellung `max_upload_mb` bzw. `LYKOS_MAX_UPLOAD_MB` (Standard 100 MB, siehe `/instance` → `max_upload_bytes`), sonst 413 `file_too_large`.
- **Kontingente:** Belegt ist die Summe aller eigenen Anhänge (auch noch nicht gesendeter Uploads) – höchstens
  Instanz-Einstellung `user_quota_mb` bzw. `LYKOS_USER_QUOTA_MB` pro Nutzer (Standard 2048 MB, `0` = unbegrenzt) – und, falls gesetzt, die Summe aller Anhänge
  der Community bis `upload_quota_mb`. Passt eine Datei nicht mehr hinein: 413 `quota_exceeded` (vorab per
  `Content-Length`, sonst sobald der Restplatz überschritten ist; nichts bleibt gespeichert). Gelöschte Nachrichten,
  verworfene Entwürfe und abgelaufene Anhänge geben Platz frei. Nur Nachrichten-Anhänge zählen (keine Avatare o. Ä.).
- **Typ:** `content_type` ermittelt der Server aus den ersten Bytes (Magic Bytes); Dateiendung und Client-Angabe
  zählen nicht. Unbekannt → `application/octet-stream`.
- **Bilder** (`image/png`, `image/jpeg`, `image/gif`, `image/webp`): `width`/`height` aus dem Dateikopf, sonst `null`.
- **Dateiname:** nur der letzte Pfadteil, ohne Steuerzeichen, max. 200 Zeichen; leer → `datei`. Auf dem Server
  liegt die Datei nur unter ihrer ID.
- **`url`:** `/api/attachments/{id}/{filename}` (relativ zur Instanz; Dateiname prozentkodiert, wird beim Abruf ignoriert).
  Auth wie überall (Cookie oder Bearer) – im Desktop-Client also per `fetch` mit Token laden, nicht als `<img src>`.
- **Download-Header:** `Content-Type` (aktive Typen wie HTML/SVG/XML/Text immer als `application/octet-stream`),
  `Content-Disposition: inline` nur für die vier Bildtypen, sonst `attachment` plus `Content-Security-Policy: sandbox`;
  immer `X-Content-Type-Options: nosniff`, `Cache-Control: private`. Range-Requests (`Range: bytes=…`, ein Bereich)
  → 206; ungültig → 416.
- **Aufbewahrung:** Die Frist der Community (`message_retention_days`) gilt auch für Anhänge. Beim Löschen von
  Kanälen/Communities entfernt die Datenbank die Einträge, die Dateien räumt der Hintergrund-Job ab.

### Lesestand (Ungelesen)
| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| PUT | `/channels/{id}/read` | `VIEW_CHANNEL` | `{ message_id }` → 204 |

Setzt den eigenen Lesestand im Textkanal auf `message_id` – **nur vorwärts**: ein älterer Stand als der
gespeicherte wird ignoriert (trotzdem 204). Die Nachricht muss im Kanal existieren (`404` sonst,
`400 not_a_text_channel` bei Voice-Kanälen). Hat sich der Stand geändert, geht `READ_STATE_UPDATE` an alle
eigenen Verbindungen (Abgleich zwischen Geräten). Clients sollten gedrosselt melden (z. B. höchstens 1×/s).

### Rollen
| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| POST | `/communities/{id}/roles` | `MANAGE_ROLES` | `{ name, permissions?, color? }` → `Role` |
| PATCH | `/roles/{id}` | `MANAGE_ROLES` | `{ name?, permissions?, color?, position? }` → `Role` |
| DELETE | `/roles/{id}` | `MANAGE_ROLES` (nicht Standardrolle) | 204 |
| PATCH | `/communities/{id}/roles/order` | `MANAGE_ROLES` | `{ role_ids: string[] }` → `Role[]` (alle Rollen der Community) |

**Reihenfolge:** Kleinere `position` = höher in der Hierarchie; die Standardrolle steht unabhängig von ihrer
Position immer ganz unten und lässt sich nicht verschieben (`400 invalid_request`). Neue Rollen kommen ans Ende
(direkt über die Standardrolle). `PATCH …/roles/order` erwartet **alle** Rollen außer der Standardrolle genau einmal,
höchste zuerst, und vergibt die Positionen `1..n` (sonst `400 invalid_request`); für jede verschobene Rolle geht
`ROLE_UPDATE` raus, ins Audit-Log `role.reorder`. Einzelne Rollen lassen sich auch über `PATCH /roles/{id}` mit
`position` verschieben (ohne Neunummerierung der anderen).

**Hierarchie:** Wer nicht Besitzer ist, verwaltet nur Rollen **echt unterhalb** seiner höchsten eigenen Rolle
(Standardrolle zählt nicht): bearbeiten, löschen, verschieben, Mitgliedern zuweisen/entziehen – sonst
`403 forbidden`. Beim Verschieben darf keine Rolle auf oder über die eigene höchste Position gelangen; in
`…/roles/order` müssen die eigene höchste Rolle und alle darüber unverändert vorn stehen. Die Standardrolle darf
jeder mit `MANAGE_ROLES` bearbeiten. Zusätzlich gilt weiterhin: Rechte nur vergeben, die man selbst hat.
Rauswerfen und fremde Nicknames folgen derselben Hierarchie gegenüber Mitgliedern (Ziel nur mit Standardrolle steht
unter jeder anderen Rolle). Kanal-Overrides sind nicht an die Hierarchie gebunden.

### Einladungen
| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| POST | `/communities/{id}/invites` | `CREATE_INVITES` | `{ max_uses?, expires_in_hours? }` → `Invite` |
| GET | `/communities/{id}/invites` | `MANAGE_COMMUNITY` | `Invite[]` |
| DELETE | `/invites/{code}` | Ersteller oder `MANAGE_COMMUNITY` | 204 |
| GET | `/invites/{code}` | – | `{ code, kind: 'community'\|'instance', instance_name, community: { id, name, icon_url, banner_url, description, accent_color } \| null, member_count }` – Bild-URLs über die Einladung (siehe „Gestaltung“) |
| GET | `/invites/{code}/icon/{icon_id}`, `/invites/{code}/banner/{banner_id}` | – (gültige Einladung) | Bild – auch über eine Instanz-Einladung mit automatischem Beitritt zu dieser Community |
| POST | `/invites/{code}/accept` | angemeldet | `CommunityDetail`; bei Instanz-Einladungen `409 instance_invite` (gelten nur für die Registrierung) |

Einladungslink im Client: `{public_url}/invite/{code}` – für Community- und Instanz-Einladungen. Bei
`kind: 'instance'` ist `community` die Community, der neue Nutzer automatisch beitreten (sonst `null`).
Community-Einladungen haben 8 Zeichen, Instanz-Einladungen 12.

### Direktnachrichten (DMs)

DMs sind Kanäle ohne Community (`community_id: null`, `kind` = `dm` für 1:1, `group_dm` für Gruppen bis
10 Personen inkl. Ersteller). Nachrichten, Anhänge, Lesestand und Tipp-Anzeige laufen über die normalen
Kanal-Endpunkte (`/channels/{id}/messages`, `/channels/{id}/attachments`, `/channels/{id}/read`, `typing`-Op).

| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| GET | `/dms` | angemeldet | `DmChannel[]`, neueste Aktivität zuerst |
| POST | `/dms` | angemeldet | `{ recipient_ids: string[], name? }` → `DmChannel` (201 neu, 200 bestehende 1:1-DM) |
| PATCH | `/dms/{id}` | Teilnehmer, nur `group_dm` | `{ name: string \| null }` → `DmChannel` |
| DELETE | `/dms/{id}` | Teilnehmer, nur `group_dm` | 204 – Gruppe verlassen; der letzte Teilnehmer löscht sie samt Verlauf |
| GET | `/dms/contacts` | angemeldet | `UserPublic[]` – wem man eine neue DM schreiben kann (Freunde + Mitglieder gemeinsamer Communities mit `allow_dms`, ohne Blockaden) |
| GET | `/dms/settings` | angemeldet | `{ allow_dms: boolean }` |
| PATCH | `/dms/settings` | angemeldet | `{ allow_dms: boolean }` → `{ allow_dms }` |

- **Eröffnen:** `recipient_ids` ohne einen selbst, 1–9 Einträge (`too_many_recipients` darüber). Jeder Empfänger
  muss ein **Freund** sein oder mindestens eine Community mit einem teilen und `allow_dms` (Standard `true`) haben;
  hat einer den anderen **blockiert**, geht es nie. Sonst `403 dm_not_allowed` – unbekannte Nutzer sind davon nicht
  zu unterscheiden. Mit einem Empfänger wird eine
  bestehende 1:1-DM wiederverwendet (auch wenn die Bedingungen inzwischen nicht mehr gelten), `name` ist dann
  nicht erlaubt. Gruppen-DMs werden immer neu angelegt.
- **Einstellung `allow_dms`** gilt nur für neue DMs; bestehende Unterhaltungen bleiben nutzbar. Freunde dürfen
  trotzdem eine DM eröffnen.
- **Blockiert:** In einer bestehenden 1:1-DM, in der einer den anderen blockiert hat, haben beide nur noch
  `VIEW_CHANNEL` (Verlauf lesen; Schreiben, Bearbeiten, Anhänge, Reaktionen, Anpinnen → `403 forbidden`). `POST /dms` liefert die
  bestehende 1:1-DM weiterhin zurück.
- **Rechte im DM-Kanal:** Teilnehmer haben `VIEW_CHANNEL | SEND_MESSAGES | ATTACH_FILES | ADD_REACTIONS` und dürfen
  anpinnen (ohne Audit-Log), sonst nichts – keine fremden Nachrichten löschen, kein `@everyone` (wird ignoriert),
  keine Kanal-/Override-Verwaltung (403), kein Voice (`not_a_voice_channel`). `@name` erwähnt nur Teilnehmer.
  Nicht-Teilnehmer bekommen überall `404`.
- **Profile:** DM-Partner sehen gegenseitig Profil und Avatar (`GET /users/{id}`), auch ohne gemeinsame Community;
  `USER_UPDATE` geht auch an sie.
- **Konto löschen:** Man verlässt alle DMs; die übrigen Teilnehmer bekommen `DM_UPDATE`, DMs ohne Teilnehmer werden
  samt Verlauf gelöscht. Der Datenexport enthält auch eigene DM-Nachrichten (`community_id: null`).
- **1:1-DMs** kann man weder verlassen (`cannot_leave_dm`) noch benennen (`not_a_group_dm`).
- **Aufbewahrung:** DMs gehören zu keiner Community; Aufbewahrungsfristen gelten dort (noch) nicht.

### Freunde

Beziehungen stehen aus Sicht jedes Nutzers in einer eigenen Zeile: Freundschaften und Anfragen paarweise
(`friend`/`friend`, `pending_out`/`pending_in`), Blockieren nur beim Blockierenden.

| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| GET | `/relationships` | angemeldet | `Relationship[]`, nach Benutzername sortiert |
| POST | `/relationships` | angemeldet | `{ username }` (Groß-/Kleinschreibung egal) → `Relationship` (201 neue Anfrage, 200 schon angefragt bzw. Gegenanfrage angenommen) |
| PUT | `/relationships/{user_id}` | angemeldet | `{ type?: 'friend' \| 'blocked' }` (Standard `friend`) → `Relationship` – `friend`: Anfrage senden bzw. eingehende annehmen; `blocked`: blockieren |
| DELETE | `/relationships/{user_id}` | angemeldet | 204 – Freund entfernen, Anfrage ablehnen/zurückziehen, Blockade aufheben (ohne Beziehung: nichts) |
| GET | `/relationships/settings` | angemeldet | `{ friend_requests: 'everyone' \| 'mutual_communities' \| 'nobody' }` |
| PATCH | `/relationships/settings` | angemeldet | `{ friend_requests }` → `{ friend_requests }` |

- **Anfrage:** Hat der andere schon angefragt, macht eine Anfrage die Freundschaft perfekt. `friend_requests` des
  Empfängers (Standard `everyone`) entscheidet, wer anfragen darf: alle, nur Mitglieder gemeinsamer Communities
  oder niemand; gesperrte Konten nehmen keine Anfragen an. Unbekannte Nutzer, abgelehnte Einstellung und eine
  Blockade durch den Empfänger sind nicht unterscheidbar (`403 friend_request_not_allowed`). Bereits befreundet →
  `409 already_friends`; selbst blockiert → `409 user_blocked`; höchstens 100 offene ausgehende Anfragen
  (`409 too_many_pending_requests`). Sich selbst → `400 invalid_request`.
- **Blockieren** beendet Freundschaft bzw. Anfragen; der Blockierte bekommt nur `RELATIONSHIP_REMOVE` (keinen
  Hinweis auf die Blockade). Danach: keine Anfragen und keine neuen DMs in beide Richtungen, bestehende 1:1-DM nur
  noch lesbar (siehe DMs). Clients blenden Nachrichten Blockierter aus (mit „Anzeigen“) und spielen keine Töne
  bzw. Benachrichtigungen dafür. Blockieren beide sich gegenseitig, hebt `DELETE` nur die eigene Blockade auf.
  Ein laufender 1:1-Anruf endet sofort (beide fliegen raus, der Raum wird geschlossen); in Gruppen-DMs klingelt es
  zwischen den beiden nicht mehr (siehe Anrufe in DMs).
- **Freunde** dürfen sich DMs schreiben (auch ohne gemeinsame Community und mit `allow_dms: false`), sehen
  gegenseitig Profil, Avatar und Präsenz und bekommen `USER_UPDATE`. Wer mich anfragt, gibt mir damit Sicht auf
  sein Profil (inkl. `USER_UPDATE`); eine eigene Anfrage und eine eigene Blockade öffnen das Profil eines Fremden
  dagegen nicht – die Freundesliste zeigt dafür nur `UserPublic` (Name, Avatar-URL).

### Suche

| Methode | Pfad | Recht | Antwort |
|---|---|---|---|
| GET | `/communities/{id}/search?q=&author_id=&channel_id=&before=&limit=` | Mitglied | `Message[]`, neueste zuerst |
| GET | `/dms/search?q=&author_id=&channel_id=&before=&limit=` | angemeldet | `Message[]` aus eigenen DMs |

Postgres-Volltextsuche: Jedes Wort (Buchstaben/Ziffern; alles andere trennt) wird als Präfix gesucht, alle Wörter
müssen vorkommen. Gesucht wird mit deutscher Stammformbildung (`haus` findet „Häuser“) **oder** ohne (`simple`,
für Stoppwörter, Namen, Fremdwörter). `q`: 1–200 Zeichen mit mindestens einem Wort, sonst `invalid_request`;
höchstens 10 Wörter zählen. `limit` 1–50 (Standard 25), `before` blättert (Nachrichten-ID).
Ergebnisse kommen nur aus Kanälen mit `VIEW_CHANNEL` (inkl. Overrides) bzw. aus eigenen DMs; ein unsichtbarer
`channel_id` liefert einfach keine Treffer. Zum Springen an einen Treffer: `GET /channels/{id}/messages?around={id}`.

### Voice (LiveKit)

Medien laufen über einen selbst gehosteten LiveKit-SFU. Der Server stellt nur Tokens aus und
führt den Voice-Status; ohne LiveKit-Konfiguration antworten die Endpunkte `503 voice_unavailable`.

| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| POST | `/channels/{id}/voice/join` | `VIEW_CHANNEL` + `CONNECT` | – → `{ url, token, audio: { bitrate, stereo, dtx, red } }` |
| POST | `/voice/state` | angemeldet | `{ self_mute, self_deaf }` → 204 (ohne aktiven Voice-Status wirkungslos) |
| GET | `/communities/{id}/voice-states` | Mitglied | `VoiceState[]` (nur sichtbare Kanäle) |
| POST | `/livekit/webhook` | LiveKit-Signatur | LiveKit-Webhook-Event → 200 |

**Join:** Nur Voice-Kanäle (sonst `400 not_a_voice_channel`). `url` ist die öffentliche
LiveKit-URL (`LYKOS_LIVEKIT_URL`), `token` ein LiveKit-Access-Token (JWT HS256, 10 min gültig,
LiveKit verlängert laufende Sitzungen selbst): `iss` = API-Key, `sub` = Nutzer-ID (Identity),
`name` = Anzeigename, `video = { room: <channel_id>, roomJoin, canSubscribe: true,
canPublish, canPublishData: false, canPublishSources }`. Quellen: `microphone` mit `SPEAK`,
`screen_share` + `screen_share_audio` mit `STREAM`, `camera` mit `VIEW_CHANNEL` + `CONNECT` + `VIDEO`;
ohne jede Quelle `canPublish: false`. `audio` sind die Ton-Einstellungen des Kanals (`bitrate` in kbit/s); der Client
setzt sie als `audioPreset.maxBitrate`, `forceStereo`, `dtx` und `red` der Mikrofonspur.

**Nutzerlimit:** Hat der Kanal ein `user_limit` und sitzen (laut Voice-Status) schon so viele *andere* Nutzer darin,
antwortet der Join mit `403 channel_full`. Wer selbst schon drin ist (Neuverbindung), zählt nicht mit; Nutzer mit
`MANAGE_CHANNELS` kommen immer hinein. Dasselbe prüft der Server noch einmal beim tatsächlichen Beitritt
(Webhook `participant_joined`): Ist der Kanal inzwischen voll (z. B. mit einem vorab geholten Token), wird der
Teilnehmer nicht übernommen, sondern aus dem LiveKit-Raum entfernt.

**Voice-Status:** Quelle der Wahrheit sind die LiveKit-Webhooks (`participant_joined`,
`participant_left`, `participant_connection_aborted`, `track_published`/`track_unpublished` für
`SCREEN_SHARE` → `streaming` und `CAMERA` → `video`, `room_finished`). Der Client schaltet die Kamera
durch Unveröffentlichen ab (nicht nur Muten), damit `video` stimmt. Der `Authorization`-Header ist ein mit dem
API-Secret signiertes JWT (`iss` = API-Key), dessen Claim `sha256` = Base64(SHA-256(Body)) ist;
alles andere → `401`. Ein Nutzer ist höchstens in einem Voice-Kanal. Mute/Deaf meldet der
Client selbst (`/voice/state`), sie dienen nur der Anzeige.

**Rechteentzug:** Verlassen/Kick der Community, entzogenes `VIEW_CHANNEL`/`CONNECT` (Overrides,
Rollen) und Beitritte ohne Recht werfen den Teilnehmer per LiveKit-RoomService
(`RemoveParticipant`) hinaus; Kanal-/Community-Löschung schließt den Raum (`DeleteRoom`).
Beim Serverstart wird der Voice-Status mit LiveKit abgeglichen.

**Entzug von `SPEAK`/`STREAM`/`VIDEO` im laufenden Call:** Bei jeder Rechteänderung in der Community
(Rolle geändert/gelöscht, Mitgliedsrolle hinzu/weg, Kanal-Override) prüft der Server alle verbundenen
Teilnehmer, die bleiben dürfen: `GetParticipant` liefert ihre aktuellen LiveKit-Rechte und Spuren;
weichen die Rechte ab, setzt `UpdateParticipant` die komplette `permission` neu (`canSubscribe: true`,
`canPublish`, `canPublishData: false`, `canPublishSources` wie im Token, als `TrackSource`-Namen).
Veröffentlichte, nicht stumme Spuren entzogener Quellen werden per `MutePublishedTrack` stummgeschaltet;
`streaming`/`video` im Voice-Status gehen sofort auf `false`. Ein Beitritt mit einem älteren Token
(mehr Rechte als aktuell) wird anhand der `permission` im Webhook direkt angeglichen. Der Client reagiert
auf LiveKits `ParticipantPermissionsChanged`: Buttons werden deaktiviert, eine laufende Bildschirm- oder
Kamerafreigabe wird beendet. Erneut erteilte Rechte gelten ebenso sofort.

### Anrufe in DMs

1:1- und Gruppen-DMs können einen Anruf haben: Der DM-Kanal ist der LiveKit-Raum (Raumname = Kanal-ID).
Beitreten, Voice-Status, Mute/Deaf, Webhooks und Rechteentzug laufen wie bei Sprachkanälen; Voice-States haben
`community_id: null` und gehen (wie `CALL_UPDATE`) nur an die Teilnehmer der DM. Ein Nutzer ist weiterhin in höchstens
einem Sprachkanal *oder* Anruf – der Beitritt woanders beendet den alten (`VOICE_STATE_DELETE`).

| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| POST | `/channels/{id}/voice/join` | Teilnehmer der DM | – → `{ url, token, audio }` wie oben; startet bzw. nimmt den Anruf an |
| POST | `/channels/{id}/call/decline` | Teilnehmer der DM | – → 204: Klingeln für sich beenden; als Anrufer ohne verbundenes Gegenüber: Anruf abbrechen |
| POST | `/channels/{id}/call/ring` | im Anruf | – → 204: bei allen, die noch nicht drin sind, erneut klingeln lassen (sonst `400 not_in_call`) |

- **Token:** Alle Teilnehmer dürfen sprechen, Bildschirm (+Ton) teilen und die Kamera nutzen (`canPublishSources` wie
  bei vollen Rechten); Standard-Ton `{ bitrate: 64, stereo: false, dtx: true, red: true }`, kein Nutzerlimit.
  Nicht-Teilnehmer → `404`; 1:1-DM mit Blockade (egal in welche Richtung) → `403 call_not_allowed`, ein noch gültiges
  Token wird beim Beitritt (Webhook) direkt wieder entfernt. In Gruppen-DMs dürfen auch Blockierende teilnehmen.
- **Anrufen:** Holt sich jemand ein Token, während niemand anderes im Raum ist und es nicht schon klingelt, klingelt es
  bei allen anderen Teilnehmern – außer bei gesperrten Konten, bei denen, die schon drin sind, und bei Blockaden
  zwischen Anrufer und Angerufenem. Anklingeln (auch `…/call/ring`) ist auf 8 pro Minute und Nutzer begrenzt
  (`429 rate_limited`, der Beitritt schlägt dann fehl). Wer selbst angeklingelt wird, nimmt mit dem Token an.
- **Klingeln endet** für einen Nutzer mit Annehmen (Token oder Webhook `participant_joined`), Ablehnen, Verlassen
  der Gruppe oder Konto-Löschung; für alle nach 40 s, wenn der Raum wieder leer ist (Anrufer legt allein auf) bzw.
  der Anrufer über `…/call/decline` abbricht, und bei einer Blockade zwischen Anrufer und Angerufenem.
- **Anruf verpasst:** Endet das Klingeln in einer 1:1-DM durch Ablauf oder weil der Anrufer aufgelegt hat (nicht
  durch Ablehnen), schreibt der Server die Systemnachricht `kind: 'call_missed'` (Autor = Anrufer, `content` leer).
- **Gruppe verlassen** wirft aus dem Anruf der Gruppe. Laufende Anrufe der eigenen DMs kommen mit `READY` und
  `RESYNC` (`dm_calls`), Klingeln überlebt keinen Server-Neustart.

`CALL_UPDATE` (an alle Teilnehmer der DM, nach jeder Änderung des Klingelns):
`{ channel_id, ringing: string[] /* bei wem es klingelt */, caller_id: string | null /* null, wenn es nirgends klingelt */ }`

### Instanz-Verwaltung

Nur für Instanz-Admins (`users.is_instance_admin`, nicht gesperrt). Nicht angemeldet → `401`, kein Admin →
`403 forbidden` (zentral über den Extractor `InstanceAdmin`). Jede ändernde Aktion landet im instanzweiten
Audit-Log. Admins sehen **keine** Nachrichteninhalte und **keine** IP-Adressen (werden ohnehin nicht gespeichert);
„zuletzt aktiv“ ist nur der Tag der jüngsten Session-Nutzung.

Listen mit Suche/Paging nehmen `?q=&page=&per_page=` (`page` ab 0, `per_page` 1–100, Standard 50) und liefern
`{ total, items }`. Die Suche ignoriert Groß/Klein und behandelt `%`/`_` wörtlich.
Die Feedback-Verwaltung (`/admin/feedback*`, siehe „Feedback“) nutzt denselben Guard.

**Übersicht**

| Methode | Pfad | Antwort |
|---|---|---|
| GET | `/admin/overview` | `{ users: { total, new_7d, disabled, admins }, communities, messages, storage: { total_bytes, attachment_count, backend: 'local'\|'s3', top_communities: { id, name, bytes }[] }, online, voice: { enabled, participants, channels: { community_id, community_name, channel_id, channel_name, participants }[] }, version, database_bytes, uptime_seconds }` |

`online` zählt Nutzer, die für andere sichtbar online sind (Unsichtbare nicht); `messages` nur die Anzahl.

**Instanz-Einladungen** – Registrierungs-Einladungen ohne Community-Bindung

| Methode | Pfad | Body / Antwort |
|---|---|---|
| GET | `/admin/invites` | `InstanceInvite[]` – alle (auch abgelaufene/widerrufene), neueste zuerst, max. 500 |
| POST | `/admin/invites` | `{ max_uses?: 1–10000, expires_in_hours?: 1–8760, note?: string (≤ 200), community_id? }` → 201 `InstanceInvite` |
| DELETE | `/admin/invites/{code}` | 204 – widerrufen (bleibt in der Liste, `status: 'revoked'`) |

```
InstanceInvite = { code, url, note, community: { id, name } | null, max_uses, uses, expires_at, revoked_at,
                   created_at, created_by: UserPublic | null, status: 'active'|'expired'|'used_up'|'revoked',
                   used_by: { user: UserPublic, used_at }[] }
```

`url` = `{public_url}/invite/{code}`. Eingelöst wird bei der Registrierung (`POST /auth/register`, Feld `invite`)
über `invites::redeem_for_registration` – dieselbe Funktion prüft auch Community-Einladungen und ist für weitere
Registrierungswege (z. B. OIDC) gedacht. Sie zählt die Nutzung, vermerkt den Nutzer in `instance_invite_uses`
und lässt ihn ggf. der Community beitreten (`MEMBER_ADD` wie gewohnt).

**Nutzer**

| Methode | Pfad | Body / Antwort |
|---|---|---|
| GET | `/admin/users` | `?q=&page=&per_page=&filter=admins\|disabled` → `{ total, items: AdminUser[] }`, neueste zuerst |
| GET | `/admin/users/{id}` | `AdminUser & { communities: { id, name, is_owner, joined_at }[], session_count, message_count, storage_bytes, registered_with: { code, note } \| null }` |
| POST | `/admin/users/{id}/disable` | 204 – sperren: Login verweigert, alle Sessions gelöscht (Gateway-Close 4001), aus Voice geworfen. Eigenes Konto → `409 cannot_target_self` |
| POST | `/admin/users/{id}/enable` | 204 – Sperre aufheben |
| DELETE | `/admin/users/{id}/sessions` | 204 – alle Sessions beenden (beim eigenen Konto außer der aktuellen) |
| PUT | `/admin/users/{id}/admin` | 204 – zum Instanz-Admin machen |
| DELETE | `/admin/users/{id}/admin` | 204 – Admin-Recht entziehen; nie dem letzten aktiven Admin (`409 last_instance_admin`), auch nicht sich selbst |
| POST | `/admin/users/{id}/password-reset` | 201 `{ url, expires_at }` – einmaliger Reset-Link (24 h), siehe unten |
| DELETE | `/admin/users/{id}` | `{ delete_messages?: boolean, owned_communities?: 'transfer'\|'delete' }` → 204, siehe unten |

```
AdminUser = { id, username, display_name, avatar_url, created_at, last_active_on: 'YYYY-MM-DD' | null,
              community_count, is_instance_admin, disabled_at: string | null }
```

*Reset-Link:* Der Admin schickt `url` (`{public_url}/reset-password#token=…`) selbst an den Nutzer – ohne
E-Mail-Versand. Gespeichert wird nur der SHA-256-Hash des Tokens (`password_reset_tokens`, `kind = 'admin_reset'`).
Ein neuer Link macht ältere, unbenutzte Admin-Links desselben Kontos ungültig. Eingelöst wird über
`/auth/password-reset/verify` und `/complete` (siehe Auth); danach sind alle Sessions beendet.

*Konto löschen:* wie die Selbstlöschung (`DELETE /auth/me`), ohne Passwort. Eigenes Konto → `409 cannot_target_self`.
Besitzt der Nutzer Communities und fehlt `owned_communities` → `409 owns_communities`. `transfer` überträgt jede
an das am längsten beigetretene andere Mitglied (Communities ohne weitere Mitglieder werden gelöscht),
`delete` löscht alle.

**Communities** – ohne Mitgliedschaft, ohne Einsicht in Kanäle oder Nachrichten

| Methode | Pfad | Body / Antwort |
|---|---|---|
| GET | `/admin/communities` | `?q=&page=&per_page=` → `{ total, items: { id, name, owner: UserPublic, member_count, channel_count, storage_bytes, created_at }[] }` |
| GET | `/admin/communities/{id}/members` | `UserPublic[]` – zur Auswahl eines neuen Besitzers |
| POST | `/admin/communities/{id}/transfer` | `{ user_id }` → `Community` – Ziel muss Mitglied sein (`404`), Events wie beim Besitzerwechsel |
| DELETE | `/admin/communities/{id}` | 204 – samt Inhalt löschen (`COMMUNITY_DELETE`, Voice-Räume werden geschlossen) |

**Einstellungen** – zur Laufzeit, in `instance_settings`; Umgebungsvariablen bleiben die Standardwerte

| Methode | Pfad | Body / Antwort |
|---|---|---|
| GET | `/admin/settings` | `AdminSettings` |
| PATCH | `/admin/settings` | Felder optional, `null` = auf Standard zurücksetzen → `AdminSettings`; sendet `INSTANCE_UPDATE` |

```
AdminSettings = { instance_name, registration_mode, max_upload_mb, user_quota_mb: number | null,
                  allow_community_creation, announcement: string | null,
                  defaults: { instance_name, max_upload_mb, user_quota_mb }, overridden: string[] }
```

| Feld | Werte | Standard (Umgebung) |
|---|---|---|
| `instance_name` | 1–64 Zeichen | `LYKOS_INSTANCE_NAME` |
| `registration_mode` | `open`, `invite_only`, `closed` (`400 invalid_registration_mode`) | `invite_only` |
| `max_upload_mb` | 1–102400 | `LYKOS_MAX_UPLOAD_MB` |
| `user_quota_mb` | 0–10000000, `0` = unbegrenzt (Antwort dann `null`) | `LYKOS_USER_QUOTA_MB` |
| `allow_community_creation` | `true`/`false` – sonst dürfen nur Instanz-Admins Communities anlegen (`403 community_creation_disabled`) | `true` |
| `announcement` | ≤ 500 Zeichen, leer/`null` = keine (`400 invalid_announcement`) | keine |

**Audit-Log**

| Methode | Pfad | Antwort |
|---|---|---|
| GET | `/admin/audit-log` | `?before=<id>&limit=1–100` → `AuditEntry[]`, neueste zuerst |

Aktionen: `instance_invite.create`, `instance_invite.revoke`, `user.disable`, `user.enable`,
`user.sessions_revoke`, `user.admin_grant`, `user.admin_revoke`, `user.password_reset_link`, `user.delete`,
`community.delete`, `community.transfer`, `settings.update`, `feedback.status`, `feedback.reply`
(`{ internal }`), `feedback.delete`, `feedback.issue` (Feedback-Einträge ohne Meldungsinhalte). `data` enthält die nötigen Namen im Klartext
(z. B. `username`), damit Einträge das Löschen des Ziels überleben. Eigene Tabelle `instance_audit_log`
(das Community-Audit-Log verschwindet mit der Community); ein Besitzerwechsel durch Admins erscheint zusätzlich
im Audit-Log der Community.

### Feedback (Fehler & Vorschläge)

Meldungen aus der App. **Fehlerberichte** (`bug`) und **Sonstiges** (`other`) sehen nur der Melder und
Instanz-Admins; für alle anderen sind sie `404`. **Vorschläge** (`suggestion`) sind für alle angemeldeten Nutzer
der Instanz sichtbar und können unterstützt werden – dann aber ohne `reporter`, `tech_info`, `screenshot_url`,
`issue_url` und ohne interne Notizen (die Felder fehlen in der Antwort ganz).

| Methode | Pfad | Recht | Body / Antwort |
|---|---|---|---|
| POST | `/feedback` | angemeldet | `{ kind, title, description, tech_info?: TechInfo \| null }` → 201 `Feedback` |
| POST | `/feedback/{id}/screenshot` | Melder | `multipart/form-data`, Feld `file` → `Feedback` |
| GET | `/feedback/{id}/screenshot` | Melder oder Admin | Bild |
| GET | `/feedback/mine?before=&limit=` | angemeldet | `Feedback[]` – eigene Meldungen, neueste zuerst |
| GET | `/feedback/suggestions?sort=votes\|new&status=&offset=&limit=` | angemeldet | `Feedback[]` – alle Vorschläge (Standard: meiste Stimmen zuerst) |
| GET | `/feedback/{id}` | Melder, Admin oder Vorschlag | `Feedback` |
| DELETE | `/feedback/{id}` | Melder | 204 – eigene Meldung zurückziehen (samt Screenshot, Stimmen, Antworten) |
| PUT | `/feedback/{id}/vote` | angemeldet, nur Vorschläge | `{ vote_count, voted }`; zweite Stimme → `409 already_voted` |
| DELETE | `/feedback/{id}/vote` | angemeldet, nur Vorschläge | `{ vote_count, voted }` (idempotent) |

- **Eingaben:** `kind` = `bug` \| `suggestion` \| `other` (`invalid_kind`), `title` 3–150 Zeichen in einer Zeile
  (`invalid_title`), `description` 1–10 000 Zeichen Markdown (`invalid_description`), ohne Steuerzeichen.
  Abstimmen auf etwas anderes als einen Vorschlag → `400 not_a_suggestion`.
- **Rate-Limit:** höchstens 10 Meldungen pro Nutzer und Stunde (gezählt in der Datenbank), sonst `429 rate_limited`
  mit `Retry-After`.
- **Screenshot:** optional, nach dem Absenden in einem zweiten Schritt: nur der Melder, nur einmal (`409
  screenshot_exists`), nur in der ersten Stunde (`403`). PNG, JPEG, WebP oder GIF (Magic Bytes), höchstens 5 MB
  (`413 file_too_large`) und 8192 px Kantenlänge (`400 invalid_image`). Der Client kodiert das Bild vorher per
  Canvas neu (entfernt EXIF/GPS). Liegt im Speicher-Backend unter einer eigenen ID; Löschen der Meldung löscht die
  Datei, der Speicher-Abgleich behandelt Screenshots als bekannt. Abruf mit `Cache-Control: private, no-store`.
- **Technische Infos** (`tech_info`) schickt der Client nur mit Opt-in des Melders (Standard: an, abwählbar, mit
  Aufklapper „Was wird gesendet?“). Der Server übernimmt nur die Felder von `TechInfo` (alles andere wird verworfen),
  kürzt Texte (64 Zeichen, Fehlermeldungen 500, höchstens die letzten 50 Fehler) und schwärzt UUIDs (`<id>`), lange
  Zeichenketten mit Ziffern (`<redacted>`, z. B. Tokens) und alles nach `Bearer`. Inhalt: App-Version, Plattform
  (`web`/`desktop`), grobes Betriebssystem, Fenstergröße, aktuelle Ansicht ohne IDs und die letzten Client-Fehler
  (`console.error`, `window.onerror`, `unhandledrejection`) – keine Nachrichteninhalte, keine Tokens.
- **Konto löschen:** Meldungen bleiben ohne Melder (`reporter: null`) erhalten; Stimmen des Kontos entfallen.

**Verwaltung** (nur Instanz-Admins über den Guard `InstanceAdmin`, siehe „Instanz-Verwaltung“; sonst `403 forbidden`; Aktionen landen im instanzweiten Audit-Log):

| Methode | Pfad | Body / Antwort |
|---|---|---|
| GET | `/admin/feedback?kind=&status=&before=&limit=` | `Feedback[]` (Admin-Ansicht), neueste zuerst |
| GET | `/admin/feedback/settings` | `{ gitea: boolean }` – Gitea-Weiterleitung konfiguriert? |
| GET | `/admin/feedback/{id}` | `Feedback` inkl. Melder, technischer Infos und interner Notizen |
| PATCH | `/admin/feedback/{id}` | `{ status }` → `Feedback` (`invalid_status`) |
| POST | `/admin/feedback/{id}/replies` | `{ body (1–4000 Zeichen), internal?: boolean = false }` → 201 `Feedback` |
| DELETE | `/admin/feedback/{id}` | 204 – samt Screenshot, Stimmen und Antworten |
| POST | `/admin/feedback/{id}/issue` | → `Feedback` mit `issue_url` – als Gitea-Issue anlegen |

Statuswechsel und öffentliche Antworten schicken `FEEDBACK_UPDATE` an den Melder; interne Notizen nicht.

**Gitea-Weiterleitung** (optional, `LYKOS_FEEDBACK_GITEA_URL` = API-Basis des Repos, z. B.
`https://git.example.org/api/v1/repos/<besitzer>/<repo>`, plus `LYKOS_FEEDBACK_GITEA_TOKEN`): Nie automatisch, nur per
Klick eines Admins. Ohne Konfiguration `503 gitea_not_configured` (und `settings.gitea = false`, der Client blendet die
Funktion aus); ist Gitea nicht erreichbar oder lehnt ab, `503 gitea_unavailable`. Pro Meldung höchstens ein Issue
(`409 issue_exists`). Titel `[Feedback #<Nummer>] <Titel>` (`@` mit Null-Breiten-Leerzeichen entschärft), Text =
Beschreibung als Codeblock (keine Bilder, Links oder @-Erwähnungen), Typ und Meldungsnummer, ggf. die
technischen Infos als JSON – **kein Nutzername, kein Screenshot**. Labels `feedback` sowie `bug` (Fehler) bzw.
`enhancement` (Vorschlag), sofern es sie im Repo gibt (Abgleich ohne Groß/Klein-Unterschied über `GET …/labels`).
Der Server folgt keinen Weiterleitungen, damit das Token nur an die konfigurierte Adresse geht.

## Gateway (WebSocket)

`GET /api/gateway` (Upgrade). Ist ein Cookie/Bearer-Token dabei, ist die Verbindung sofort
authentifiziert. Sonst muss der Client binnen 10 s senden:

```json
{ "op": "identify", "token": "<token>" }
```

Server → Client: `{ "t": "<TYP>", "d": { ... } }`

| `t` | `d` |
|---|---|
| `READY` | `{ user: User, community_ids: string[], dm_channel_ids: string[], presences: { user_id, status: PresenceStatus }[], dm_calls: DmCall[] }` – `presences` für alle Mitglieder der eigenen Communities und alle Freunde, die **nicht offline** sind; wer fehlt, ist offline; `dm_calls` = laufende Anrufe der eigenen DMs: `{ channel_id, voice_states: VoiceState[], ringing: string[], caller_id: string \| null }` |
| `MESSAGE_CREATE` / `MESSAGE_UPDATE` | `Message` – auch bei Anpinnen/Lösen; `reactions[].me` ist hier immer `false` (Clients behalten ihren Reaktionsstand und pflegen ihn über die Reaktions-Events) |
| `MESSAGE_REACTION_ADD` / `MESSAGE_REACTION_REMOVE` | `{ message_id, channel_id, community_id, user_id, emoji, emoji_id: string \| null, animated: boolean }` – `emoji` bei Custom-Emotes `name:id` |
| `EMOJIS_UPDATE` | `{ community_id, emojis: CustomEmoji[] }` – vollständige neue Emote-Liste der Community (nach Upload, Umbenennen, Löschen) |
| `MESSAGE_DELETE` | `{ id, channel_id, community_id }` |
| `CHANNEL_CREATE` / `CHANNEL_UPDATE` | `Channel` |
| `CHANNEL_DELETE` | `{ id, community_id }` |
| `COMMUNITY_UPDATE` | `Community` – auch bei Icon/Banner/Beschreibung/Akzentfarbe/Systemkanal |
| `COMMUNITY_DELETE` | `{ id }` |
| `MEMBER_ADD` / `MEMBER_UPDATE` | `{ community_id, member: Member }` |
| `MEMBER_REMOVE` | `{ community_id, user_id }` |
| `USER_UPDATE` | `UserProfile` – Profil (Name, Avatar, Banner, Farben, Pronomen, Über mich, Status-Text) geändert; an alle Communities und DM-Kanäle des Nutzers, an Freunde und von ihm Angefragte (nicht an Blockierende und nicht an Nutzer, die ihn angefragt haben) und an seine eigenen Verbindungen – jede Verbindung bekommt es genau einmal, auch bei mehreren gemeinsamen Communities. Zusätzlich kommt je Community `MEMBER_UPDATE`. |
| `ROLE_CREATE` / `ROLE_UPDATE` | `Role` |
| `ROLE_DELETE` | `{ id, community_id }` |
| `PERMISSIONS_CHANGED` | `{ community_id }` – Rechte/Overrides geändert: `CommunityDetail` neu laden |
| `VOICE_STATE_UPDATE` | `VoiceState` – Nutzer ist (jetzt) in diesem Voice-Kanal; ersetzt einen Eintrag in einem anderen Kanal |
| `VOICE_STATE_DELETE` | `{ community_id, channel_id, user_id }` – Nutzer hat den Voice-Kanal verlassen (`community_id: null` bei DM-Anrufen) |
| `CALL_UPDATE` | `{ channel_id, ringing: string[], caller_id: string \| null }` – Klingelzustand eines DM-Anrufs, an alle Teilnehmer (siehe Anrufe in DMs) |
| `RESYNC` | `{ presences: { user_id, status: PresenceStatus }[], dm_calls: DmCall[] }` – Events verpasst: Zustand komplett neu laden (Communities, Kanäle, Lesestände, Voice-States, Mitglieder, offene Nachrichten); `presences` und `dm_calls` ersetzen den Stand wie bei `READY` (`presences` nur nicht-offline; fehlend = offline) |
| `PRESENCE_UPDATE` | `{ user_id, status: PresenceStatus }` – an alle Communities und Freunde des Nutzers; bei einer neuen Freundschaft sofort der aktuelle Status des anderen (falls nicht offline) |
| `PRESENCE_BULK` | `{ presences: { user_id, status: PresenceStatus }[] }` – nach dem eigenen Beitritt zu einer Community: Status ihrer nicht-offline Mitglieder, die der Client noch nicht kennt; in den bestehenden Stand **einmischen** (nicht ersetzen) |
| `TYPING_START` | `{ channel_id, community_id, user_id }` – an alle, die den Kanal sehen (auch an den Tippenden selbst) |
| `READ_STATE_UPDATE` | `{ channel_id, community_id, last_read_message_id }` – nur an die eigenen Verbindungen |
| `DM_CREATE` / `DM_UPDATE` | `DmChannel` (`last_read_message_id` immer `null`, `mention_count` immer `0` – Clients behalten ihren Stand) – an alle Teilnehmer |
| `DM_DELETE` | `{ id }` – nur an den Nutzer, der die Gruppe verlassen hat |
| `RELATIONSHIP_ADD` | `Relationship` – Beziehung neu oder geändert (Anfrage gesendet/erhalten, angenommen, blockiert); jede Seite bekommt ihre eigene Sicht |
| `RELATIONSHIP_REMOVE` | `{ id }` – Beziehung zu Nutzer `id` besteht nicht mehr (entfernt, abgelehnt, zurückgezogen, Blockade aufgehoben, vom anderen blockiert, Konto gelöscht) |
| `INSTANCE_UPDATE` | wie `GET /instance` – Instanz-Einstellungen/Ankündigung geändert; an **alle** Verbindungen |
| `INSTANCE_ADMIN_UPDATE` | `{ is_instance_admin }` – eigenes Instanz-Admin-Recht vergeben/entzogen; nur an die eigenen Verbindungen |
| `FEEDBACK_UPDATE` | `Feedback` (Melder-Ansicht) – Status geändert oder neue Antwort; nur an den Melder |
| `FEEDBACK_DELETE` | `{ id }` – Meldung gelöscht (Admin oder selbst zurückgezogen); nur an den Melder |

Kanalgebundene Events (Nachrichten, Kanäle, Voice-Status) gehen nur an Verbindungen, die den Kanal sehen dürfen.
In DM-Kanälen gehen `MESSAGE_*`, `TYPING_START` usw. an alle Teilnehmer; `community_id` ist dort `null`.
Wird die eigene Session widerrufen, schließt der Server die Verbindung (Close-Code 4001).
Nach einem Reconnect (neues `READY`) sollten Clients ebenso wie nach `RESYNC` alles neu laden – Events aus der
Unterbrechung werden nicht nachgeliefert.
Der Client sendet optional `{ "op": "ping" }` und erhält `{ "t": "PONG", "d": {} }`.

### Präsenz

`PresenceStatus = 'online' | 'idle' | 'dnd' | 'offline'`

Ein Nutzer ist online, solange er mindestens eine Gateway-Verbindung hat. Jede Verbindung meldet ihren Status:

```json
{ "op": "presence", "status": "online" | "idle" | "dnd" | "invisible" }
```

Den Anfangsstatus kann der Client schon beim Verbinden angeben – `GET /api/gateway?status=invisible` –,
damit ein unsichtbarer Nutzer nicht kurz als online erscheint (Standard: `online`).
Aus allen Verbindungen eines Nutzers ergibt sich der sichtbare Status: `invisible` (→ andere sehen `offline`)
vor `dnd` vor `online` vor `idle`. `idle` setzt der Client selbst (nach 10 min ohne Eingabe).
Schließt die letzte Verbindung, geht `PRESENCE_UPDATE` mit `offline` raus.
`PRESENCE_UPDATE` kommt nur bei echten Änderungen und nie über den eigenen Nutzer (der Client kennt seinen Status).
`READY`/`RESYNC` enthalten nur Nutzer, die nicht offline sind – ein fehlender Eintrag bedeutet `offline`.
Tritt man einer Community bei, schickt das Gateway die Status ihrer (nicht-offline) Mitglieder als ein `PRESENCE_BULK`.
Freunde sehen sich gegenseitig auch ohne gemeinsame Community (`READY` und `PRESENCE_UPDATE` an jeden Freund).

Datenschutz: Präsenz ist flüchtig und wird nicht in der Datenbank gespeichert – es gibt kein „zuletzt online“.

### Tipp-Anzeige

```json
{ "op": "typing", "channel_id": "<id>" }
```

Erzeugt `TYPING_START` an alle, die den Kanal sehen – nur in Textkanälen und nur mit `SEND_MESSAGES`,
sonst wird die Op still ignoriert. Serverseitig höchstens ein Event pro Nutzer und Kanal alle 3 s.
Clients senden die Op beim Tippen gedrosselt (etwa alle 3,5 s) und blenden die Anzeige 8 s nach dem
letzten `TYPING_START` aus – oder sofort, wenn vom selben Nutzer `MESSAGE_CREATE` im Kanal kommt.
