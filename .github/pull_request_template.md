## Was ändert sich?

<!-- Kurze Beschreibung. Bezug zu einem Issue: "Closes #123". -->

## Warum?

## Wie getestet?

<!-- Neue/angepasste Tests, manuelle Prüfung (Browser, Desktop, Voice), Screenshots bei UI-Änderungen. -->

## Checkliste

- [ ] `cargo fmt --all --check` und `cargo clippy --workspace --all-targets -- -D warnings` sauber
- [ ] `cargo test --workspace` grün (bei Server-Änderungen)
- [ ] `npm run lint`, `npm test`, `npm run build` in `client/` grün (bei Client-Änderungen)
- [ ] Neue Migration statt Änderung bestehender (bei Schemaänderungen)
- [ ] `docs/api.md` aktualisiert (bei API-/Gateway-Änderungen)
- [ ] `CHANGELOG.md` unter „Unveröffentlicht“ ergänzt (wenn für Nutzer/Betreiber sichtbar)
- [ ] Keine neuen Datenerhebungen oder Drittanbieter ohne Begründung
- [ ] Commits mit `Signed-off-by` (DCO); Beitrag unter AGPL-3.0-or-later
