# Bedienung

## Start

```
claude-tui              # jede Session startet `claude`
claude-tui "claude -c"  # beliebiger Befehl, z. B. mit Fortsetzen der letzten Unterhaltung
claude-tui --version
```

Die erste Session startet im aktuellen Arbeitsverzeichnis, jede weitere im Verzeichnis aus der
Verzeichnisauswahl. Unter Windows läuft der Befehl über `cmd.exe /C`,
unter Linux/macOS über `sh -c`, damit PATH-Auflösung und npm-Wrapper (`claude.cmd`) funktionieren.

## Aufbau

Links die **Sessionliste**: alle Sessions mit Zustand; ist eine Session in einem Fenster zu sehen,
steht dessen Nummer hinter dem Namen (`claude 2 [1]`). Sessions ohne Fenster laufen im Hintergrund
weiter.

Rechts bis zu **4 Fenster**, nach Position nummeriert:

```
1 Fenster    2 Fenster    3 Fenster    4 Fenster
┌───────┐    ┌───┬───┐    ┌───┬───┐    ┌───┬───┐
│   1   │    │ 1 │ 2 │    │   │ 2 │    │ 1 │ 2 │
│       │    │   │   │    │ 1 ├───┤    ├───┼───┤
│       │    │   │   │    │   │ 3 │    │ 3 │ 4 │
└───────┘    └───┴───┘    └───┴───┘    └───┴───┘
```

Wird ein Fenster geschlossen, rücken die übrigen nach. Der Bereich mit dem **Fokus** (ein Fenster
oder die Sessionliste) hat einen hellorangen Rahmen, alle anderen einen dunkelorangen.

## Tasten

### Überall

| Taste | Wirkung |
|---|---|
| `Alt+←/→/↑/↓` | Fokus zum Nachbarfenster in Pfeilrichtung; `Alt+←` aus einem linken Fenster geht in die Sessionliste, `Alt+→` aus der Liste ins Fenster der gewählten Session (sonst Fenster 1) |
| `Ctrl+A` | öffnet das Menü unten in der Sessionliste |

`Alt+Pfeil` erreicht Claude dadurch nie; Wortsprünge im Eingabefeld gehen über `Ctrl+←/→`.

### Fokus auf einem Fenster

Alle anderen Tasten gehen an die Session. `Shift+Enter` sendet `ESC CR` — Claude Code fügt damit
einen Zeilenumbruch ein.

### Fokus auf der Sessionliste

| Taste | Wirkung |
|---|---|
| `↑` / `↓` | Session auswählen |
| `Enter` | hat die Session ein Fenster: Fokus dorthin; sonst Fensterauswahl |
| `1`–`4` | Session in dieses Fenster legen (eine sichtbare Session tauscht den Platz) |

### Menü (`Ctrl+A`)

Punkte per Taste oder mit `↑`/`↓` und `Enter` wählen, `Esc` schließt. Welche Punkte erscheinen, hängt
vom Fokus ab:

| Taste | Fokus auf der Liste | Fokus auf einem Fenster |
|---|---|---|
| `n` | neue Session: Verzeichnis, dann Fensterauswahl | neue Session: Verzeichnis, dann in diesem Fenster |
| `c` | Fenster der gewählten Session schließen | Fenster schließen |
| `x` | gewählte Session beenden | Session dieses Fensters beenden |
| `1`–`4` / `0` | Fokus auf Fenster / Sessionliste | Fokus auf Fenster / Sessionliste |
| `Ctrl+A` | – | sendet ein echtes Ctrl+A an Claude |
| `q` | claude-tui beenden (alle Sessions enden) | claude-tui beenden |

`c` lässt die Session im Hintergrund weiterlaufen, `x` beendet sie und entfernt sie aus der Liste.

**Verzeichnisauswahl:** Vor jeder neuen Session (`n`) fragt claude-tui unten in der Sessionliste, in
welchem Verzeichnis sie starten soll — erst danach kommt, falls nötig, die Fensterauswahl.

| Taste | Wirkung |
|---|---|
| Tippen / Einfügen | Pfad im Eingabefeld bearbeiten (vorbelegt mit dem zuletzt genutzten Verzeichnis) |
| `Tab` | Ordnernamen ergänzen; bei mehreren Treffern erst den gemeinsamen Anfang, dann reihum |
| `↑` / `↓` | zuletzt genutztes Verzeichnis ins Eingabefeld übernehmen |
| `Enter` | Verzeichnis übernehmen (muss existieren) |
| `Esc` | abbrechen |

Die zuletzt genutzten Verzeichnisse (höchstens 10) stehen in `%APPDATA%\claude-tui\recent-dirs.txt`
bzw. `~/.config/claude-tui/recent-dirs.txt`. Die erste Session beim Programmstart läuft ohne Auswahl
im aktuellen Verzeichnis.

**Fensterauswahl:** listet die belegten Fenster („claude 1 ersetzen“) und, solange weniger als 4 offen
sind, „neues Fenster“. Eine ersetzte Session läuft im Hintergrund weiter. Ist noch kein Fenster offen,
entfällt die Auswahl und Fenster 1 wird genommen.

## Zustände in der Seitenleiste

| Anzeige | Bedeutung |
|---|---|
| ● arbeitet | Die Session hat in den letzten 1,5 s etwas ausgegeben (Claudes Spinner). |
| ● wartet auf dich | Keine Ausgabe, und auf dem Bildschirm steht eine Rückfrage („Do you want to …“). |
| ● bereit | Keine Ausgabe, keine Rückfrage — Claude wartet auf den nächsten Prompt. |
| ● beendet | Der Prozess ist beendet; sein Fenster schließt sich, mit `x` aus der Liste entfernen. |

Die Erkennung ist eine Heuristik über Ausgabe-Aktivität und Bildschirmtext, siehe
[Architektur](architektur.md).
