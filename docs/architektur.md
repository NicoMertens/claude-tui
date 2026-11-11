# Architektur

## Bausteine

| Crate | Zweck |
|---|---|
| `ratatui` + `crossterm` | Oberfläche und Tastatur-/Resize-Ereignisse |
| `portable-pty` | echtes Pseudo-Terminal je Session (ConPTY unter Windows) |
| `vt100` | Terminal-Emulation: wertet die Ausgabe der Session zu einem Bildschirm aus |
| `tui-term` | zeichnet einen `vt100`-Bildschirm als ratatui-Widget |

## Module (`src/`)

| Datei | Aufgabe |
|---|---|
| `main.rs` | Terminal einrichten/zurücksetzen, Ereignisschleife (16 ms Takt) |
| `app.rs` | Zustand: Sessions, Listenauswahl, offenes Menü; verteilt Tasten und führt Menü-Aktionen aus |
| `windows.rs` | welche Session in welchem Fenster liegt (max. 4, Nachrücken, Tauschen) und wo der Fokus ist — ohne PTY testbar |
| `layout.rs` | Fensterrechtecke für 1–4 Fenster und Nachbarsuche für `Alt+Pfeil` |
| `menu.rs` | Ctrl+A-Menü je Fokus und Fensterauswahl; Auswahl per Taste oder Pfeil + Enter |
| `session.rs` | eine Session: PTY starten, Lese-Thread, Statuserkennung, Schreiben, Resize |
| `keys.rs` | Tastendruck → xterm-Bytefolge (inkl. Application-Cursor-Mode und AltGr), `Alt+Pfeil` als Fokuswechsel |
| `ui.rs` | Zeichnen: Sessionliste mit Menü unten, Fenster mit Nummer und Fokusrahmen |
| `theme.rs` | Farben (Claude-Orange `#D97757` auf dunklem Grund) und Status-Beschriftungen |

## Datenfluss

1. `Session::spawn` öffnet ein PTY und startet den Befehl; `Windows::place` legt die Session in ein
   Fenster.
2. Ein Lese-Thread je Session schreibt die Ausgabe in einen `vt100::Parser` (hinter `Mutex`) und
   merkt sich den Zeitpunkt der letzten Ausgabe.
3. Die Hauptschleife fragt bei jedem Takt den Status ab, zeichnet und leitet Tastendrücke als
   Bytes an das PTY der Session im Fenster mit Fokus weiter. Beendete Sessions verlieren ihr Fenster.
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
