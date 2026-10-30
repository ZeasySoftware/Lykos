# Bot schreiben

Ein Bot ist ein eigenes Lykos-Konto, das einem Menschen gehört und sich mit einem Token statt mit Passwort anmeldet.
Er nutzt dieselbe API wie die Apps: REST unter `/api` (siehe [api.md](api.md)) und das WebSocket-Gateway für
Echtzeit-Events. Für reine Benachrichtigungen (CI, Monitoring) reicht oft ein **Webhook** – der braucht kein
Konto und keine Verbindung, siehe „Eingehende Webhooks“ in der API-Doku.

## 1. Bot anlegen

In Lykos: **Einstellungen → Entwickler → Neuer Bot**. Name eingeben, anlegen, Token kopieren. Das Token
(`lykb_…`) wird nur dieses eine Mal angezeigt; geht es verloren, unter „Token neu erzeugen“ ein neues holen
(das alte gilt dann sofort nicht mehr). Token gehören nicht in Repositories – das Präfix `lykb_` hilft
Secret-Scannern, sie zu finden.

Per API (mit einer normalen Sitzung):

```sh
curl -X POST https://chat.example.org/api/bots \
  -H "Authorization: Bearer $SESSION" -H "Content-Type: application/json" \
  -d '{ "username": "wetterbot", "description": "Sagt das Wetter an" }'
# → { "id": "…", "username": "wetterbot", …, "token": "lykb_…" }
```

Jeder Nutzer darf bis zu 10 Bots haben. Instanz-Admins können Bots in der Instanz-Verwaltung abschalten.

## 2. Bot zu einer Community hinzufügen

Wer in der Community **Community verwalten** darf, fügt den Bot unter **Community-Einstellungen → Mitglieder →
Bot hinzufügen** per Bot-ID hinzu. Eigene Bots gehen immer, fremde nur, wenn ihr Besitzer sie als „öffentlich“
markiert hat. Danach ist der Bot ein normales Mitglied: Er bekommt die Standardrolle, kann weitere Rollen bekommen
und wie jeder andere entfernt werden.

```sh
curl -X POST https://chat.example.org/api/communities/$COMMUNITY/bots \
  -H "Authorization: Bearer $SESSION" -H "Content-Type: application/json" \
  -d "{ \"bot_id\": \"$BOT_ID\" }"
```

## 3. REST mit dem Bot-Token

Alle Aufrufe tragen `Authorization: Bot <token>`:

```sh
TOKEN=lykb_…
API=https://chat.example.org/api

# Wer bin ich?
curl $API/auth/me -H "Authorization: Bot $TOKEN"

# Communities und Kanäle
curl $API/communities -H "Authorization: Bot $TOKEN"
curl $API/communities/$COMMUNITY -H "Authorization: Bot $TOKEN"     # enthält channels[]

# Nachricht senden (optional mit Embeds – Karten ohne Bilder)
curl -X POST $API/channels/$CHANNEL/messages \
  -H "Authorization: Bot $TOKEN" -H "Content-Type: application/json" \
  -d '{ "content": "Guten Morgen!", "embeds": [{ "title": "Wetter", "description": "Sonnig, 18 °C", "color": 16312092 }] }'

# Antworten und reagieren
curl -X POST $API/channels/$CHANNEL/messages -H "Authorization: Bot $TOKEN" \
  -H "Content-Type: application/json" -d "{ \"content\": \"pong\", \"reply_to_id\": \"$MESSAGE\" }"
curl -X PUT "$API/messages/$MESSAGE/reactions/%F0%9F%91%8D/@me" -H "Authorization: Bot $TOKEN"
```

Bots haben dieselben Rechte wie ein Mensch mit denselben Rollen und dieselben Rate-Limits (u. a. 120 Nachrichten
pro Minute; bei `429` den Header `Retry-After` abwarten). `@everyone` wirkt nur mit dem Recht dafür.

Nicht erlaubt (`403 bots_not_allowed`): Konto, Passwort, Sitzungen, 2FA, Datenexport, Freunde, DMs eröffnen,
Communities anlegen, Einladungen annehmen, Bots verwalten, Instanz-Verwaltung, Feedback und Sprachkanäle.
In bestehenden DMs (z. B. wenn ein Mensch dem Bot schreibt) darf ein Bot antworten.

## 4. Gateway: Events empfangen

1. WebSocket öffnen: `wss://chat.example.org/api/gateway` – entweder mit Header `Authorization: Bot <token>`
   oder ohne Header und dann binnen 10 Sekunden anmelden:

   ```json
   { "op": "identify", "token": "Bot lykb_…" }
   ```

2. Der Server antwortet mit `READY`:

   ```json
   { "t": "READY", "d": { "user": { "id": "…", "username": "wetterbot", "bot": true, … },
                          "community_ids": ["…"], "dm_channel_ids": [], "presences": [], "dm_calls": [] } }
   ```

3. Danach kommen Events als `{ "t": "<TYP>", "d": { … } }` – für Bots meist interessant: `MESSAGE_CREATE`,
   `MESSAGE_UPDATE`, `MESSAGE_DELETE`, `MESSAGE_REACTION_ADD`, `MEMBER_ADD`, `MEMBER_REMOVE`, `CHANNEL_CREATE`.
   Der Bot sieht nur Kanäle, die er sehen darf. Vollständige Liste in [api.md](api.md) unter „Gateway“.

4. Am Leben halten: Der Server schickt alle 30 s einen WebSocket-Ping (die meisten Bibliotheken antworten
   automatisch). Zusätzlich kann der Bot `{ "op": "ping" }` senden und bekommt `PONG`.

5. Nach `RESYNC` oder einem Neuverbinden den Zustand (Communities, Kanäle) per REST neu laden – verpasste Events
   werden nicht nachgeliefert. Schließt der Server mit Code **4001**, ist das Token ungültig (neu erzeugt,
   Bot gelöscht oder Bots abgeschaltet) – dann nicht endlos neu verbinden.

Nachrichten sendet der Bot immer per REST, nicht über das Gateway.

## Beispiel: Ping-Bot in Python

```python
# pip install websockets requests
import asyncio, json, os, requests, websockets

API = "https://chat.example.org/api"
TOKEN = os.environ["LYKOS_BOT_TOKEN"]
AUTH = {"Authorization": f"Bot {TOKEN}"}

async def main():
    me = requests.get(f"{API}/auth/me", headers=AUTH).json()
    async with websockets.connect(API.replace("https", "wss") + "/gateway") as ws:
        await ws.send(json.dumps({"op": "identify", "token": f"Bot {TOKEN}"}))
        async for raw in ws:
            event = json.loads(raw)
            if event["t"] != "MESSAGE_CREATE":
                continue
            msg = event["d"]
            author = msg.get("author") or {}
            if author.get("id") == me["id"] or msg["content"].strip() != "!ping":
                continue
            requests.post(f"{API}/channels/{msg['channel_id']}/messages", headers=AUTH,
                          json={"content": "pong", "reply_to_id": msg["id"]})

asyncio.run(main())
```

Tipp: Nachrichten anderer Bots (`author.bot`) und von Webhooks (`webhook_id`) ignorieren, sonst antworten sich
Bots gegenseitig endlos.
