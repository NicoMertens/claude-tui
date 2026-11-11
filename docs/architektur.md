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
| `main.rs` | Terminal einrichten/zurücksetzen, Ereignisschleife (16 ms Takt) |
| `app.rs` | Zustand: Sessions, Listenauswahl, offenes Menü; verteilt Tasten und führt Menü-Aktionen aus |
| `windows.rs` | welche Session in welchem Fenster liegt (max. 4, Nachrücken, Tauschen) und wo der Fokus ist — ohne PTY testbar |
| `layout.rs` | Fensterrechtecke für 1–4 Fenster (2 wahlweise übereinander) und Nachbarsuche für `Ctrl+Pfeil` |
| `menu.rs` | Ctrl+A-Menü je Fokus und Fensterauswahl; Auswahl per Taste oder Pfeil + Enter |
| `dirpick.rs` | Verzeichnisauswahl vor einer neuen Session: Eingabe mit Tab-Ergänzung, zuletzt genutzte |
| `recent.rs` | zuletzt genutzte Startverzeichnisse laden, merken, speichern |
| `claude_store.rs` | liest `~/.claude/projects/*/*.jsonl` inkrementell: Arbeitsverzeichnis und Titel je Claude-Session |
| `list.rs` | baut die Sessionliste aus gespeicherten Sessions und laufenden Prozessen, gruppiert nach Verzeichnis |
| `state.rs` | Fensterlayout (Claude-IDs je Fenster, Fokus, Sessionliste ausgeblendet, 2 Fenster übereinander) als `state.json` für den Neustart |
| `paths.rs` | Konfigurationsordner von claude-tui und Datenordner von Claude Code |
| `session.rs` | eine Session: PTY starten, Lese-Thread, Statuserkennung (Hooks, sonst Heuristik), Schreiben, Resize |
| `hooks.rs` | Claude-Code-Hooks: `--settings`-Datei erzeugen, `claude-tui hook <Event>` (Event → Zustand → Statusdatei), Statusdateien lesen und aufräumen |
| `keys.rs` | Tastendruck → xterm-Bytefolge (inkl. Application-Cursor-Mode und AltGr), `Ctrl+Pfeil` als Fokuswechsel |
| `ui.rs` | Zeichnen: Sessionliste (Menühinweis im unteren Rahmen; ausblendbar, dann Fenster in voller Breite), Fenster mit Nummer, Zustand und Fokusrahmen, Menü als blaues Overlay mittig über dem Bereich mit dem Fokus (Fenster oder Sessionliste) |
| `selection.rs` | Mausmarkierung im Fenster, Wort per Doppelklick, Text für die Zwischenablage (`arboard`) |
| `install.rs` | `install` / `uninstall`: Binary an festen Ort, Benutzer-PATH (Windows über PowerShell/Registry) |
| `update.rs` | Update über GitHub-Releases: tägliche Prüfung im Hintergrund, SHA-256 (`sha2`), Entpacken (`zip` bzw. `tar`+`flate2`), Austausch der laufenden Datei (`self-replace`) |
| `http.rs` | HTTPS-Abrufe ohne eigenen TLS-Stack: `curl` des Systems, Rückfall auf `ureq` mit System-TLS (nur Windows/macOS) |
| `term_view.rs` | zeichnet einen `vt100`-Bildschirm (Farben, fett, *dim*, kursiv, unterstrichen, invers, Cursor) |
| `theme.rs` | Farben (Claude-Orange `#D97757` auf dunklem Grund) und Status-Beschriftungen |

## Datenfluss

1. `Session::spawn` öffnet ein PTY und startet den Befehl mit `--session-id <neue UUID>` bzw.
   `--resume <ID>`; so ist jeder Prozess einer Claude-Session zugeordnet. Dazu kommen
   `--settings <Konfigurationsordner>/hooks.json` (siehe [Hooks](#hooks)) und die
   Umgebungsvariable `CLAUDE_TUI_SESSION=<ID>`. `Windows::place` legt ihn
   in ein Fenster, `state.rs` speichert das Layout nach jeder Änderung.
2. Ein Lese-Thread je Session schreibt die Ausgabe in einen `vt100::Parser` (hinter `Mutex`) und
   merkt sich den Zeitpunkt der letzten Ausgabe.
3. Die Hauptschleife fragt bei jedem Takt den Status ab, zeichnet und leitet Tastendrücke als
   Bytes an das PTY der Session im Fenster mit Fokus weiter. Beendete Prozesse verlieren ihr Fenster; die Claude-Session bleibt in der Liste. Alle 2 s liest
   `claude_store` neue Protokollzeilen (nur das seit dem letzten Lesen Angehängte).
4. Nach jeder Layout-Änderung und bei Resize bekommt jede sichtbare Session die Innenmaße ihres
   Fensters. Sessions im Hintergrund behalten ihre letzte Größe; sie wird erst beim Anzeigen angepasst.

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

Die Datei `hooks.json` im Konfigurationsordner wird bei jedem Start einer Session neu geschrieben.
Jeder Eintrag nutzt die **Exec-Form** (`command` = Pfad der eigenen Binary aus
`std::env::current_exe()`, `args` = `["hook", "<Event>"]`): Claude startet die Binary dann direkt
ohne Shell, Pfade mit Leerzeichen brauchen kein Quoting (die Shell-Form liefe unter Windows über
Git Bash oder PowerShell). Die Hooks laufen synchron mit 5 s Timeout, damit die Meldungen in der
Reihenfolge der Events ankommen; ein Aufruf dauert Millisekunden.

`claude-tui hook <Event>` liest das Hook-JSON von stdin und schreibt `<Zustand> <Unix-ms>` nach
`<Konfigurationsordner>/status/<ID>` (erst temporäre Datei, dann umbenennen). Als `<ID>` dient
`CLAUDE_TUI_SESSION`, sonst `session_id` aus dem JSON — die Umgebungsvariable bleibt auch nach
`/clear` gleich, falls Claude intern eine neue Session-ID vergibt. Der Aufruf gibt nie etwas aus
(bei `SessionStart`/`UserPromptSubmit` landete Ausgabe in Claudes Kontext) und endet immer mit
Exit-Code 0.

| Event | Zustand |
|---|---|
| `SessionStart`, `Stop`, `StopFailure`, `Notification` mit `idle_prompt` | bereit |
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

## Grenzen / nächste Schritte

- Kein Scrollen in den Verlauf (der Parser hält 10 000 Zeilen, es fehlt nur die Bedienung).
- Keine Mausunterstützung, keine frei teilbaren Fenster (feste Layouts für 1–4).
- Sessions überleben das Beenden von claude-tui nicht (kein Detach wie bei herdr/tmux).
