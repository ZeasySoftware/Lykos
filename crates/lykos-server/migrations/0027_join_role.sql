-- Beitrittsrolle: Rolle, die jedes neue Mitglied beim Beitritt automatisch bekommt (z. B. „Mitglied“, „Azubi“).
-- Wird beim Anlegen einer Community festgelegt und ist in den Einstellungen änderbar. Die Rolle gehört zur
-- Community (prüft die Anwendung); wird sie gelöscht, gibt es keine Beitrittsrolle mehr.
-- Idempotent formuliert, damit Tests die Migration erneut ausführen können.

ALTER TABLE communities ADD COLUMN IF NOT EXISTS join_role_id UUID REFERENCES roles (id) ON DELETE SET NULL;
