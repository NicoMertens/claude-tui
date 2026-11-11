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
| `session.rs` | eine Session: PTY starten, Lese-Thread, Statuserkennung, Schreiben, Resize |
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
   `--resume <ID>`; so ist jeder Prozess einer Claude-Session zugeordnet. `Windows::place` legt ihn
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
- **arbeitet:** letzte Ausgabe jünger als 1,5 s.
- **wartet auf dich:** sonst, wenn der sichtbare Bildschirm eine Rückfrage-Markierung enthält
  (`session.rs`, `waits_for_decision`).
- **bereit:** alles andere.

## Grenzen / nächste Schritte

- Kein Scrollen in den Verlauf (der Parser hält 10 000 Zeilen, es fehlt nur die Bedienung).
- Keine Mausunterstützung, keine frei teilbaren Fenster (feste Layouts für 1–4).
- Sessions überleben das Beenden von claude-tui nicht (kein Detach wie bei herdr/tmux).
- Statuserkennung über Claude-Code-Hooks statt Bildschirmtext wäre robuster.
