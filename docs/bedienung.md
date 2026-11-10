# Bedienung

## Start

```
claude-tui              # jede Session startet `claude`
claude-tui "claude -c"  # beliebiger Befehl, z. B. mit Fortsetzen der letzten Unterhaltung
claude-tui --version
```

Sessions starten im aktuellen Arbeitsverzeichnis. Unter Windows läuft der Befehl über `cmd.exe /C`,
unter Linux/macOS über `sh -c`, damit PATH-Auflösung und npm-Wrapper (`claude.cmd`) funktionieren.

## Tasten

Alle Eingaben gehen an die aktive Session. Befehle an claude-tui selbst laufen über den Präfix
**Ctrl+A**, danach eine Taste:

| Taste | Wirkung |
|---|---|
| `c` | neue Session |
| `x` | aktive Session schließen (die letzte schließt die App) |
| `n` / `j` / `↓` | nächste Session |
| `p` / `k` / `↑` | vorige Session |
| `1`–`9` | Session direkt wählen |
| `q` | claude-tui beenden (alle Sessions werden beendet) |
| `Ctrl+A` | sendet ein echtes Ctrl+A an die Session (Zeilenanfang) |

`Shift+Enter` sendet `ESC CR` — Claude Code fügt damit einen Zeilenumbruch ein.

## Zustände in der Seitenleiste

| Anzeige | Bedeutung |
|---|---|
| ● arbeitet | Die Session hat in den letzten 1,5 s etwas ausgegeben (Claudes Spinner). |
| ● wartet auf dich | Keine Ausgabe, und auf dem Bildschirm steht eine Rückfrage („Do you want to …“). |
| ● bereit | Keine Ausgabe, keine Rückfrage — Claude wartet auf den nächsten Prompt. |
| ● beendet | Der Prozess ist beendet; mit `Ctrl+A x` schließen. |

Die Erkennung ist eine Heuristik über Ausgabe-Aktivität und Bildschirmtext, siehe
[Architektur](architektur.md).
