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
| `windows.rs` | welche Session in welchem Fenster liegt (max. 4, Nachrücken, Tauschen), wo der Fokus ist und ob das Fenster mit Fokus gezoomt ist (endet bei Fokuswechsel, Öffnen, Schließen, Ersetzen) — ohne PTY testbar |
| `layout.rs` | Fensterrechtecke für 1–4 Fenster (2 wahlweise übereinander), sichtbare Fenster bei Zoom und Nachbarsuche für `Ctrl+Pfeil` |
| `menu.rs` | Ctrl+A-Menü je Fokus und Fensterauswahl; Auswahl per Taste oder Pfeil + Enter |
| `dirpick.rs` | Verzeichnisauswahl vor einer neuen Session: Eingabe mit Tab-Ergänzung, zuletzt genutzte |
| `recent.rs` | zuletzt genutzte Startverzeichnisse laden, merken, speichern |
| `claude_store.rs` | liest `~/.claude/projects/*/*.jsonl` inkrementell: Arbeitsverzeichnis und Titel je Claude-Session |
| `list.rs` | baut die Sessionliste aus gespeicherten Sessions und laufenden Prozessen, gruppiert nach Verzeichnis |
| `external.rs` | erkennt Sessions, die außerhalb von claude-tui laufen (`~/.claude/sessions`, Prozessliste, Rückfall-Heuristik) |
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
   `App::window_rects` liefert nur sichtbare Fenster als (Fensterindex, Session, Rechteck) — beim
   Zoom allein das gezoomte über die ganze Fensterfläche. Zeichnen, Maus und Resize gehen alle
   darüber; nur die Nachbarsuche für `Ctrl+Pfeil` nimmt das ungezoomte Layout.

## Statuserkennung

- **beendet:** Lese-Thread am Ende *oder* `child.try_wait()` liefert einen Exit-Status. Beides, weil
  unter ConPTY der Lesekanal nach Prozessende nicht zuverlässig schließt.
- **arbeitet:** letzte Ausgabe jünger als 1,5 s.
- **wartet auf dich:** sonst, wenn der sichtbare Bildschirm eine Rückfrage-Markierung enthält
  (`session.rs`, `waits_for_decision`).
- **bereit:** alles andere.

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

Grenzen: Eine PID, die nach einem Absturz an einen anderen Prozess vergeben wurde, hält eine
liegengebliebene Datei am Leben (nur eine überflüssige Rückfrage). Wechselt eine Session in
claude-tui per `/resume` intern auf eine andere ID, gilt diese als extern. Ohne Datei (alte
Versionen) wird eine untätige externe Session nicht erkannt.

## Grenzen / nächste Schritte

- Kein Scrollen in den Verlauf (der Parser hält 10 000 Zeilen, es fehlt nur die Bedienung).
- Keine Mausunterstützung, keine frei teilbaren Fenster (feste Layouts für 1–4).
- Sessions überleben das Beenden von claude-tui nicht (kein Detach wie bei herdr/tmux).
- Statuserkennung über Claude-Code-Hooks statt Bildschirmtext wäre robuster.
