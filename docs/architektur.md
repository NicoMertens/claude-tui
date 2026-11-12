# Architektur

## Bausteine

| Crate | Zweck |
|---|---|
| `ratatui` 0.30 + `crossterm` 0.29 | Oberfläche, Tastatur-, Maus- und Resize-Ereignisse |
| `portable-pty` | echtes Pseudo-Terminal je Session (ConPTY unter Windows) |
| `vt100` 0.18 | Terminal-Emulation: wertet die Ausgabe der Session zu einem Bildschirm aus |
| `serde` + `serde_json`, `uuid` | Claude-Protokolle und `state.json` lesen/schreiben, Session-IDs |

**Größe der Binary:** Das Release-Profil optimiert auf Größe (`opt-level = "z"`, `panic = "abort"`,
LTO, gestrippt). Für HTTPS steckt bewusst kein eigener TLS-Stack in der Datei (`rustls` + `ring` +
Mozilla-Zertifikate hätten rund 1,3 MB gekostet): Updates laden über `curl` bzw. das TLS des
Betriebssystems. Ergebnis unter Windows x64: rund 1,06 MB statt 3,17 MB. Ein Panic-Hook in
`main.rs` setzt das Terminal auch beim Abbruch zurück.

Den `vt100`-Bildschirm zeichnet das eigene Widget `term_view.rs`, nicht `tui-term`: dessen Version
für ratatui 0.29 hing an `vt100` 0.15, und das verwirft das Attribut *dim* (SGR 2), mit dem Claude
Code z. B. Prompt-Vorschläge grau darstellt. Neuere `vt100` brauchen `unicode-width` ≥ 0.2.1, was
ratatui 0.29 (exakt 0.2.0) ausschließt — daher ratatui 0.30.

## Module (`src/`)

| Datei | Aufgabe |
|---|---|
| `main.rs` | Terminal einrichten/zurücksetzen, Ereignisschleife (16 ms Takt; bis zu 64 anstehende Ereignisse je Bild) |
| `app.rs` | Zustand: Sessions, Listenauswahl, offenes Menü; verteilt Tasten und führt Menü-Aktionen aus |
| `windows.rs` | welche Session in welchem Fenster liegt (max. 4, Nachrücken, Tauschen), wo der Fokus ist und ob das Fenster mit Fokus gezoomt ist (endet bei Fokuswechsel, Öffnen, Schließen, Ersetzen) — ohne PTY testbar |
| `layout.rs` | Fensterrechtecke für 1–4 Fenster (2 wahlweise übereinander), sichtbare Fenster bei Zoom und Nachbarsuche für `Ctrl+Pfeil` |
| `menu.rs` | Ctrl+A-Menü je Fokus und Fensterauswahl; Auswahl per Taste oder Pfeil + Enter |
| `dirpick.rs` | Verzeichnisauswahl vor einer neuen Session: Eingabe mit Tab-Ergänzung, zuletzt genutzte |
| `recent.rs` | zuletzt genutzte Startverzeichnisse laden, merken, speichern |
| `claude_store.rs` | liest `~/.claude/projects/*/*.jsonl` inkrementell: Arbeitsverzeichnis und Titel je Claude-Session; Lesestand als `sessions-cache.json`, beim Start im Hintergrund nachgelesen |
| `list.rs` | baut die Sessionliste aus gespeicherten Sessions und laufenden Prozessen, gruppiert nach Verzeichnis |
| `external.rs` | erkennt Sessions, die außerhalb von claude-tui laufen (`~/.claude/sessions`, Prozessliste, Rückfall-Heuristik) |
| `state.rs` | Fensterlayout (Claude-IDs je Fenster, Fokus, Sessionliste ausgeblendet, 2 Fenster übereinander) als `state.json` für den Neustart |
| `paths.rs` | Konfigurationsordner von claude-tui und Datenordner von Claude Code |
| `session.rs` | eine Session: PTY öffnen, Prozess sofort oder später starten (Startreihe beim Wiederherstellen), Lese-Thread, Statuserkennung (Hooks, sonst Heuristik), Schreiben, Resize |
| `trace.rs` | Zeitmessung bei `CLAUDE_TUI_TRACE=1`: Zeilen mit Unix-ms und PID nach `trace.log` (App und Hook-Prozesse) |
| `hooks.rs` | Claude-Code-Hooks: `--settings`-Datei erzeugen (HTTP-Hooks, sonst Befehls-Hooks), Event → Zustand → Statusdatei, Statusdateien lesen und aufräumen |
| `hook_server.rs` | HTTP-Server auf `127.0.0.1` für die Hook-Meldungen (eine Anfrage je Event, Token im Pfad) |
| `waiting.rs` | Hinweis auf wartende Sessions: Wechsel auf „wartet“ erkennen (außer im Fenster mit Fokus), nächste wartende Session reihum für Menüpunkt `w`, `CLAUDE_TUI_NO_BELL` |
| `keys.rs` | Tastendruck → xterm-Bytefolge (inkl. Application-Cursor-Mode und AltGr), `Ctrl+Pfeil` als Fokuswechsel |
| `ui.rs` | Zeichnen: Sessionliste (Menühinweis im unteren Rahmen; ausblendbar, dann Fenster in voller Breite), Fenster mit Nummer, Zustand und Fokusrahmen, Menü als blaues Overlay mittig über dem Bereich mit dem Fokus (Fenster oder Sessionliste) |
| `selection.rs` | Mausmarkierung im Fenster, Wort per Doppelklick, Text für die Zwischenablage (`arboard`) |
| `install.rs` | `install` / `uninstall`: Binary an festen Ort, Benutzer-PATH (Windows über PowerShell/Registry) |
| `update.rs` | Update über GitHub-Releases: tägliche Prüfung im Hintergrund, SHA-256 (`sha2`), Austausch der laufenden Datei (`self-replace`) |
| `http.rs` | HTTPS-Abrufe ohne eigenen TLS-Stack: `curl` des Systems, Rückfall auf `ureq` mit System-TLS (nur Windows/macOS) |
| `term_view.rs` | zeichnet einen `vt100`-Bildschirm (Farben, fett, *dim*, kursiv, unterstrichen, invers, Cursor) |
| `theme.rs` | Farben (Claude-Orange `#D97757` auf dunklem Grund) und Status-Beschriftungen |

## Datenfluss

1. `Session::spawn` öffnet ein PTY und startet den Befehl mit `--session-id <neue UUID>` bzw.
   `--resume <ID>`; so ist jeder Prozess einer Claude-Session zugeordnet. Dazu kommen
   `--settings <Konfigurationsordner>/hooks-<PID>.json` (siehe [Hooks](#hooks)) und die
   Umgebungsvariable `CLAUDE_TUI_SESSION=<ID>`. `Windows::place` legt ihn
   in ein Fenster, `state.rs` speichert das Layout nach jeder Änderung.
2. Ein Lese-Thread je Session schreibt die Ausgabe in einen `vt100::Parser` (hinter `Mutex`) und
   merkt sich den Zeitpunkt der letzten Ausgabe.
3. Die Hauptschleife fragt bei jedem Takt den Status ab, zeichnet und leitet Tastendrücke als
   Bytes an das PTY der Session im Fenster mit Fokus weiter. Beendete Prozesse verlieren ihr Fenster; die Claude-Session bleibt in der Liste. Alle 2 s liest
   `claude_store` neue Protokollzeilen (nur das seit dem letzten Lesen Angehängte). Wechselt eine
   Session dabei auf „wartet“, setzt `App::tick` einen Hinweis und `ring_bell`; `main.rs` schreibt
   das BEL nach dem Zeichnen direkt über das Terminal-Backend.
4. Nach jeder Layout-Änderung und bei Resize bekommt jede sichtbare Session die Innenmaße ihres
   Fensters. Sessions im Hintergrund behalten ihre letzte Größe; sie wird erst beim Anzeigen angepasst.
   `App::window_rects` liefert nur sichtbare Fenster als (Fensterindex, Session, Rechteck) — beim
   Zoom allein das gezoomte über die ganze Fensterfläche. Zeichnen, Maus und Resize gehen alle
   darüber; nur die Nachbarsuche für `Ctrl+Pfeil` nimmt das ungezoomte Layout.

## Statuserkennung

- **beendet:** Lese-Thread am Ende *oder* `child.try_wait()` liefert einen Exit-Status. Beides, weil
  unter ConPTY der Lesekanal nach Prozessende nicht zuverlässig schließt.
- Sonst gilt die letzte **Hook-Meldung** (siehe unten). Liegt (noch) keine vor, z. B. bei einem
  eigenen Befehl ohne Hook-Unterstützung, entscheidet die **Heuristik**:
  - **arbeitet:** letzte Ausgabe jünger als 1,5 s.
  - **wartet auf dich:** sonst, wenn der sichtbare Bildschirm eine Rückfrage-Markierung enthält
    (`session.rs`, `waits_for_decision`).
  - **bereit:** alles andere.
- Hooks melden nicht jeden Übergang: Nach einer beantworteten Rückfrage kommt die nächste Meldung
  erst mit dem Ende des Tools, ein per `Esc` abgebrochener Lauf meldet kein `Stop`. Gab es nach
  „wartet“ oder „arbeitet“ eine Eingabe in die Session, entscheidet deshalb bis zur nächsten
  Meldung wieder die Heuristik (`session.rs`, `resolve`). Nach „bereit“ nicht — das Echo beim
  Tippen soll nicht als „arbeitet“ gelten.

## Hooks

claude-tui gibt jedem gestarteten Claude-Prozess per `--settings <Datei>` eigene Hooks mit. Laut
[CLI-Referenz](https://code.claude.com/docs/en/cli-reference) überschreibt `--settings` nur die
gesetzten Schlüssel für diese Session; die `~/.claude/settings.json` des Nutzers wird nicht
angefasst. Hook-Einträge mehrerer Ebenen werden zusammengeführt
([Hooks](https://code.claude.com/docs/en/hooks)), eigene Hooks des Nutzers laufen also weiter.

Die Datei `hooks-<PID>.json` im Konfigurationsordner gehört zu genau einem laufenden claude-tui (sie
enthält die URL seines Hook-Servers). Sie wird beim ersten Session-Start geschrieben — über eine
temporäre Datei und Umbenennen, damit ein startendes Claude nie eine halbe Datei liest — und beim
Beenden gelöscht. Reste abgebrochener Läufe und die gemeinsame `hooks.json` älterer Versionen räumt
`hooks::purge_stale` nach sieben Tagen ohne Änderung weg.

**HTTP-Hooks (Normalfall).** Beim Start öffnet claude-tui einen kleinen HTTP-Server auf
`127.0.0.1:<freier Port>` (`hook_server.rs`). Jeder Eintrag ist ein Hook vom Typ `http`:
Claude schickt das Event-JSON per `POST` an `http://127.0.0.1:<Port>/hook/<Token>`. Das Token ist
je Lauf zufällig, damit andere Programme keine Zustände unterschieben. Die Session-ID kommt im
Header `X-Claude-Tui-Session` aus `${CLAUDE_TUI_SESSION}` (per `allowedEnvVars` freigegeben). Der
Server antwortet sofort mit leerem `200` — für Claude „Erfolg, keine Anweisungen“. HTTP-Hooks
laufen synchron, aber eine Anfrage an localhost dauert Millisekunden. Läuft claude-tui nicht mehr,
scheitert die Verbindung sofort, und Claude macht weiter. Damit ein Firmen-Proxy die Anfragen nicht
abfängt, ergänzt claude-tui `NO_PROXY` der Claude-Prozesse um `127.0.0.1,localhost`.

Der Grund: Befehls-Hooks starten je Event einen Prozess. Auf einem Rechner, dessen Virenscanner
die unsignierte `claude-tui.exe` bei **jedem** Start prüft, kostete das 3–4 s pro Event — beim
Absenden jeder Nachricht und vor und nach jedem Werkzeug. Selbst `async` half nicht, weil schon das
Anlegen des Prozesses Claude aufhält.

`SessionStart` ruft HTTP-Hooks nicht auf (geprüft mit Claude Code 2.1.322: ein Befehls-Hook für
dasselbe Event lief, der HTTP-Hook nicht). Für dieses Event trägt claude-tui deshalb nichts ein;
bis zur ersten Meldung entscheidet die Heuristik.

**Befehls-Hooks (Rückfall),** wenn sich kein Port öffnen lässt: **Exec-Form** (`command` = Pfad der
eigenen Binary aus `std::env::current_exe()`, `args` = `["hook", "<Event>"]`), Claude startet die
Binary direkt ohne Shell. Sie laufen **asynchron** (`"async": true`) mit 30 s Timeout; eine
verspätete Meldung kann dabei eine neuere überschreiben. `claude-tui hook <Event>` liest das
Hook-JSON von stdin, gibt nie etwas aus (bei `SessionStart`/`UserPromptSubmit` landete Ausgabe in
Claudes Kontext) und endet immer mit Exit-Code 0.

Beide Wege schreiben `<Zustand> <Unix-ms>` nach `<Konfigurationsordner>/status/<ID>` (erst
temporäre Datei, dann umbenennen). Als `<ID>` dient `CLAUDE_TUI_SESSION`, sonst `session_id` aus
dem JSON — die Umgebungsvariable bleibt auch nach `/clear` gleich, falls Claude intern eine neue
Session-ID vergibt.

| Event | Zustand |
|---|---|
| `SessionStart` (nur Befehls-Hooks), `Stop`, `StopFailure`, `Notification` mit `idle_prompt` | bereit |
| `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PreCompact` | arbeitet |
| `PermissionRequest` (sofort beim Freigabedialog), `Notification` mit `permission_prompt` / `elicitation_dialog` (erst nach ca. 6 s ohne Eingabe) | wartet auf dich |
| `SessionEnd` | Statusdatei löschen |

Die App liest die Statusdatei je Session alle 250 ms. Vor dem Start einer Session und beim Beenden
ihres Prozesses wird die Statusdatei gelöscht; Dateien, die älter als 7 Tage sind, räumt der
Programmstart weg.

Unter Windows übergibt claude-tui den Befehl wortweise an `cmd.exe /C`: `portable-pty` quotet
Argumente nach den C-Runtime-Regeln (`\"`), die `cmd.exe` nicht versteht. So bekommt nur der
`--settings`-Pfad (bei Leerzeichen) äußere Anführungszeichen. Unter Linux/macOS wird er für
`sh -c` in einfache Anführungszeichen gesetzt.

## Extern laufende Sessions

Wird eine Session, die schon in einem anderen Terminal läuft, mit `--resume` ein zweites Mal
gestartet, schreiben zwei Prozesse in dasselbe Protokoll. `external.rs` erkennt solche Sessions:

1. **`~/.claude/sessions/<PID>.json`** (bzw. unter `$CLAUDE_CONFIG_DIR`): Claude Code legt die
   Datei je laufendem Prozess an — auch ohne `--resume` in der Kommandozeile — mit `pid` und
   `sessionId`, und entfernt sie beim Beenden. Weil ein abgestürzter Prozess sie liegen lassen kann,
   zählt sie nur, solange die PID noch läuft: Windows über `%SystemRoot%\System32\tasklist.exe`
   (fester Pfad wie bei `curl`), Linux über `/proc/<PID>`, sonst `ps -A -o pid=`. Keine neue
   Abhängigkeit (`sysinfo` o. Ä. hätte die Binary deutlich vergrößert; Windows-API direkt ginge nur
   mit `unsafe`).
   Hintergrund-Sessions (`"kind":"bg"`) haben zusätzlich eine `jobId`; `external::Claimed` hält
   sie je Session-ID fest.
2. **Rückfall** für Versionen ohne diese Datei: Protokoll seit dem Start von claude-tui und in den
   letzten 10 s geändert (`RECENT_WRITE`).

In beiden Fällen zählen Sessions nicht, die in claude-tui laufen oder dort vor weniger als 30 s
endeten (`ENDED_GRACE` in `app.rs`) — deren Datei und letzte Schreibzugriffe stammen von claude-tui
selbst. Ein Hintergrund-Thread liest das Verzeichnis alle 2 s; `tasklist` (unter Windows knapp 1 s)
läuft nur, wenn sich die PIDs geändert haben, sonst alle 20 s. Die Hauptschleife holt das Ergebnis
per Kanal ab und verrechnet es beim Neuaufbau der Liste (`list::Entry::external`). Öffnen einer
solchen Session (`Enter`, `1`–`4`) geht erst über `Menu::confirm_open` (Vorauswahl „No“); beim
Wiederherstellen aus `state.json` prüft `external::claimed_now` einmal synchron (die Prozessliste nur,
wenn eine der Sessions in einer Datei steht) und überspringt solche Sessions.

Ausnahme Hintergrund-Sessions: `claude --resume` lehnt sie ab, solange sie laufen. Mit bekannter
`jobId` startet `App::spawn` deshalb `claude attach <jobId>` (`Launch::Attach`, ohne `--settings`,
denn der Hintergrundprozess läuft mit seinen eigenen Einstellungen; Hook-Status gibt es dafür nicht,
es gilt die Heuristik). Das ist keine zweite Instanz — die Rückfrage entfällt, und beim
Wiederherstellen wird verbunden statt übersprungen.

Grenzen: Eine PID, die nach einem Absturz an einen anderen Prozess vergeben wurde, hält eine
liegengebliebene Datei am Leben (nur eine überflüssige Rückfrage). Wechselt eine Session in
claude-tui per `/resume` intern auf eine andere ID, gilt diese als extern. Ohne Datei (alte
Versionen) wird eine untätige externe Session nicht erkannt.

## Grenzen / nächste Schritte

- Kein Scrollen in den Verlauf (der Parser hält 10 000 Zeilen, es fehlt nur die Bedienung).
- Keine Mausunterstützung, keine frei teilbaren Fenster (feste Layouts für 1–4).
- Sessions überleben das Beenden von claude-tui nicht (kein Detach wie bei herdr/tmux).
