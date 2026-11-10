# Architektur

## Bausteine

| Crate | Zweck |
|---|---|
| `ratatui` + `crossterm` | Oberfläche und Tastatur-/Resize-Ereignisse |
| `portable-pty` | echtes Pseudo-Terminal je Session (ConPTY unter Windows) |
| `vt100` | Terminal-Emulation: wertet die Ausgabe der Session zu einem Bildschirm aus |
| `tui-term` | zeichnet einen `vt100`-Bildschirm als ratatui-Widget |

## Module (`source/src/`)

| Datei | Aufgabe |
|---|---|
| `main.rs` | Terminal einrichten/zurücksetzen, Ereignisschleife (16 ms Takt) |
| `app.rs` | Zustand: Liste der Sessions, aktive Session, Präfix-Modus, Befehle |
| `session.rs` | eine Session: PTY starten, Lese-Thread, Statuserkennung, Schreiben, Resize |
| `keys.rs` | Tastendruck → xterm-Bytefolge (inkl. Application-Cursor-Mode und AltGr) |
| `ui.rs` | Layout (Seitenleiste, Hauptfenster, Fußzeile) und Zeichnen |
| `theme.rs` | Farben (Claude-Orange `#D97757` auf dunklem Grund) und Status-Beschriftungen |

## Datenfluss

1. `Session::spawn` öffnet ein PTY in der Größe des Hauptfensters und startet den Befehl.
2. Ein Lese-Thread je Session schreibt die Ausgabe in einen `vt100::Parser` (hinter `Mutex`) und
   merkt sich den Zeitpunkt der letzten Ausgabe.
3. Die Hauptschleife fragt bei jedem Takt den Status ab, zeichnet und leitet Tastendrücke als
   Bytes an das PTY der aktiven Session weiter.
4. Bei Resize bekommen **alle** Sessions die neue Größe, damit ein Wechsel nicht neu umbrechen muss.

## Statuserkennung

- **beendet:** Lese-Thread am Ende *oder* `child.try_wait()` liefert einen Exit-Status. Beides, weil
  unter ConPTY der Lesekanal nach Prozessende nicht zuverlässig schließt.
- **arbeitet:** letzte Ausgabe jünger als 1,5 s.
- **wartet auf dich:** sonst, wenn der sichtbare Bildschirm eine Rückfrage-Markierung enthält
  (`session.rs`, `waits_for_decision`).
- **bereit:** alles andere.

## Grenzen / nächste Schritte

- Kein Scrollen in den Verlauf (der Parser hält 10 000 Zeilen, es fehlt nur die Bedienung).
- Keine Mausunterstützung, keine geteilten Fenster.
- Sessions überleben das Beenden von claude-tui nicht (kein Detach wie bei herdr/tmux).
- Statuserkennung über Claude-Code-Hooks statt Bildschirmtext wäre robuster.
