# Drittkomponenten und Lizenzen

Lykos selbst steht unter der **GNU Affero General Public License v3.0 oder später** (siehe `LICENSE`, `NOTICE`).
Dieses Dokument fasst die Lizenzen der eingebundenen Drittkomponenten zusammen und hält die Prüfung auf
Verträglichkeit mit der AGPL fest. Die maßgeblichen Lizenztexte liegen jeweils dem Paket bei
(`~/.cargo/registry/src/…`, `client/node_modules/…`); verteilte Binärdateien (Docker-Image, Desktop-Installer)
müssen diese Hinweise mitliefern.

Stand der Prüfung: 2026-10-30, Version 0.4.1. Quelle: `cargo metadata --locked` (beide Cargo-Projekte, alle
Zielplattformen) und die `license`-Felder in `client/package-lock.json`.

## Ergebnis

**Keine Inkompatibilität mit AGPL-3.0-or-later gefunden.** Alle Abhängigkeiten stehen unter permissiven Lizenzen
(MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, 0BSD, CDLA-Permissive-2.0, BSL-1.0, Unlicense) bzw. unter der
dateibezogenen Copyleft-Lizenz MPL-2.0, die ausdrücklich mit der (A)GPL kombiniert werden darf (MPL-2.0 §3.3).
Apache-2.0 ist mit GPLv3/AGPLv3 verträglich (nicht mit GPLv2 – für Lykos ohne Belang). Keine Abhängigkeit steht
unter GPL-2.0-only, SSPL, BUSL, Commons Clause oder einer „non-commercial“-Lizenz.

Zu beachten:

- **Apache-2.0-Komponenten** (u. a. `livekit-client`, `@livekit/protocol`, `rxjs`, `tao`): Bringt ein Paket eine
  eigene `NOTICE`-Datei mit, muss deren Inhalt bei der Weitergabe von Binärdateien erhalten bleiben (Apache-2.0 §4d).
- **MPL-2.0** (Desktop-App: `cssparser`, `cssparser-macros`, `selectors`, `dtoa-short`, `option-ext`; Build-Werkzeuge
  im Client): Änderungen an diesen Dateien selbst müssten unter MPL-2.0 offengelegt werden; Lykos ändert sie nicht.
- **Mehrfachlizenzen mit LGPL-Option** (`r-efi`: MIT OR Apache-2.0 OR LGPL-2.1-or-later): Lykos nutzt die
  MIT/Apache-Option.
- **CDLA-Permissive-2.0** (`webpki-roots`, `webpki-root-certs`): Mozilla-Wurzelzertifikate als Daten; permissiv.
- **Unicode-3.0** (ICU4X-Crates über `url`/`idna`, `unicode-ident`): permissiv, Hinweis beibehalten.

## Rust – Server (`crates/lykos-server`, 344 Fremd-Crates)

| Anzahl | Lizenz (SPDX, wie angegeben) |
|---:|---|
| 232 | MIT OR Apache-2.0 (inkl. Schreibweisen `MIT/Apache-2.0`, `Apache-2.0 OR MIT`, `Apache-2.0 / MIT`) |
| 57 | MIT |
| 18 | Unicode-3.0 |
| 6 | Unlicense OR MIT (`aho-corasick`, `byteorder`, `memchr`, `winapi-util`, `same-file`, `walkdir`) |
| 5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT (`rustix`, `linux-raw-sys`, `wasi`, `wasip2`, `wit-bindgen`) |
| 3 | BSD-3-Clause (`curve25519-dalek`, `ed25519-dalek`, `subtle`) |
| 3 | Apache-2.0 OR ISC OR MIT (`rustls`, `hyper-rustls`, `rustls-native-certs`) |
| 2 | ISC (`rustls-webpki`, `untrusted`) |
| 2 | CDLA-Permissive-2.0 (`webpki-roots`, `webpki-root-certs`) |
| 2 | BSD-2-Clause OR Apache-2.0 OR MIT (`zerocopy`, `zerocopy-derive`) |
| 2 | (MIT OR Apache-2.0) AND Unicode-3.0 (`unicode-ident`, `unicode-normalization`) |
| 2 | MIT OR Apache-2.0 OR LGPL-2.1-or-later (`r-efi`) |
| 1 | Apache-2.0 AND ISC (`ring`) |
| 1 | (Apache-2.0 OR MIT) AND BSD-3-Clause (`encoding_rs`) |
| 1 | MIT AND BSD-3-Clause (`matchit`) |
| 1 | MIT OR Apache-2.0 OR BSD-1-Clause (`fiat-crypto`) |
| 1 | Zlib (`foldhash`) |
| 1 | Zlib OR Apache-2.0 OR MIT (`tinyvec`) |
| 1 | 0BSD (`quoted_printable`) |
| 1 | Apache-2.0 OR BSL-1.0 (`ryu`) |
| 1 | Apache-2.0 OR BSL-1.0 OR MIT (`whoami`) |
| 1 | Apache-2.0 (`sync_wrapper`) |

Wichtige direkte Abhängigkeiten: axum, tokio, tower-http, sqlx, serde, reqwest, rustls/ring, object_store, lettre,
jsonwebtoken, argon2, qrcode, uuid, time, tracing (alle MIT und/oder Apache-2.0).

## Rust – Desktop-App (`client/src-tauri`, 561 Fremd-Crates)

Überwiegend wie beim Server (MIT/Apache-2.0: 342, MIT: 125, Unicode-3.0: 18, Zlib-Varianten: 25). Zusätzlich:

| Lizenz | Crates |
|---|---|
| MPL-2.0 | `cssparser`, `cssparser-macros`, `selectors`, `dtoa-short`, `option-ext` |
| Apache-2.0 | `tao`, `gethostname`, `sync_wrapper` |
| Apache-2.0 WITH LLVM-exception | `target-lexicon` (Build-Zeit) |
| BSD-3-Clause / BSD-3-Clause AND MIT | `alloc-no-stdlib`, `alloc-stdlib`, `subtle`, `brotli`, `brotli-decompressor` |
| ISC | `libloading`, `rustls-webpki`, `untrusted` |
| CC0-1.0 OR MIT-0 OR Apache-2.0 | `dunce` |
| Apache-2.0 AND MIT | `dpi` |

Wichtige direkte Abhängigkeiten: tauri und offizielle Tauri-Plugins (MIT OR Apache-2.0), keyring (MIT OR
Apache-2.0), webview2-com / windows-core (MIT OR Apache-2.0).

## JavaScript/TypeScript – Client (`client/`)

Laufzeit-Abhängigkeiten (landen im ausgelieferten Web-Client bzw. in der Desktop-App):

| Paket | Version | Lizenz |
|---|---|---|
| react, react-dom, scheduler | 19.3.1 / 0.28.0 | MIT |
| zustand | 5.0.16 | MIT |
| lucide-react | 1.64.0 | ISC |
| livekit-client, @livekit/protocol, @livekit/mutex | 2.23.0 / 1.52.1 / 1.1.1 | Apache-2.0 |
| @bufbuild/protobuf | 1.10.1 | Apache-2.0 AND BSD-3-Clause |
| rxjs | 7.8.2 | Apache-2.0 |
| webrtc-adapter | 9.0.6 | BSD-3-Clause |
| sdp, sdp-transform, events, jose, loglevel, typed-emitter, machina | – | MIT |
| tslib | 2.8.1 | 0BSD |
| @tauri-apps/api, @tauri-apps/plugin-* | 2.x | MIT OR Apache-2.0 |
| @fontsource-variable/inter, @fontsource-variable/bricolage-grotesque | 5.3.0 | OFL-1.1 (siehe Schriften) |

Entwicklungswerkzeuge (Vite, TypeScript, oxlint, Tauri-CLI, @types/*; 97 Pakete, nur im Build): MIT,
Apache-2.0, ISC, BSD-3-Clause, MPL-2.0 (u. a. `lightningcss`). Sie werden nicht ausgeliefert.

## Schriften

| Schrift | Quelle | Lizenz |
|---|---|---|
| Inter (Variable) | `@fontsource-variable/inter` | SIL Open Font License 1.1 |
| Bricolage Grotesque (Variable) | `@fontsource-variable/bricolage-grotesque` | SIL Open Font License 1.1 |

Die OFL erlaubt Bündeln und Weitergabe zusammen mit Software beliebiger Lizenz; die Schriften dürfen nur nicht
allein verkauft werden, und der Lizenztext muss beiliegen (liegt im npm-Paket; beim Build landen die `.woff2`
im Web-Client). Die Schriften werden selbst gehostet, kein Google-Fonts-CDN.

## Emoji-Daten

`client/src/data/emoji-data.json` wird von `client/scripts/gen-emoji-data.mjs` aus **emojibase-data 18.0.0**
erzeugt:

- emojibase-data: **MIT** (© Miles Johnson).
- Namen und Schlüsselwörter stammen aus **Unicode CLDR** – Unicode License v3 (`Unicode-3.0`), permissiv,
  Copyright-Hinweis „© Unicode, Inc.“ beibehalten.
- Shortcodes aus den iamcal- und GitHub-Sammlungen von emojibase (MIT).

Die Emoji-Glyphen selbst liefert die Systemschrift (z. B. Segoe UI Emoji); Lykos bündelt keine Emoji-Grafiken.

## Sounds, Icons, Logos

- **Sounds:** werden zur Laufzeit per Web Audio synthetisiert (`client/src/lib/soundSynth.ts`); keine
  Audiodateien, keine Fremdrechte.
- **Icons:** `lucide-react` (ISC).
- **Logos:** Wolfslogo/App-Icons (`client/public/favicon.svg`, `client/src-tauri/icons/`) und das Zeasy-Logo
  (`client/public/zeasy-logo.svg`) sind eigene Werke von Zeasy Software; Code-Lizenz AGPL, Markenrechte siehe
  `NOTICE`.

## Getrennt betriebene Dienste (nicht Teil des Lykos-Codes)

| Dienst | Lizenz | Hinweis |
|---|---|---|
| LiveKit Server (`livekit/livekit-server`) | Apache-2.0 | eigener Container, nur über API/WebRTC angebunden |
| PostgreSQL | PostgreSQL License | eigener Container |
| Basis-Images (`debian:trixie-slim`, `node`, `rust`) | jeweilige Debian-/Projektlizenzen | Build- bzw. Laufzeitbasis des Docker-Images |

## Prüfung wiederholen

```bash
cargo metadata --format-version 1 --locked            # Server; Feld "license" je Paket
(cd client/src-tauri && cargo metadata --format-version 1 --locked)
node -e "for (const [k,v] of Object.entries(require('./client/package-lock.json').packages)) if (k) console.log(k, v.license, v.dev ? 'dev' : '')"
```

Für eine dauerhafte Prüfung in der CI bietet sich `cargo deny check licenses` (mit Allowlist der obigen Lizenzen)
an.
