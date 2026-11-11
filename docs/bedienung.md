# Bedienung

## Start

```
claude-tui                      # Claude Code über `claude` starten
claude-tui "claude --model opus" # anderer Befehl bzw. zusätzliche Optionen
claude-tui --version
```

claude-tui hängt an den Befehl selbst `--session-id <neue ID>` (neue Session) bzw. `--resume <ID>`
(gespeicherte Session fortsetzen) an — Optionen wie `-c` oder `--resume` gehören deshalb nicht in den
Befehl. Unter Windows läuft er über `cmd.exe /C`, unter Linux/macOS über `sh -c`, damit
PATH-Auflösung und npm-Wrapper (`claude.cmd`) funktionieren.

**Wiederherstellen:** claude-tui merkt sich, welche Session in welchem Fenster lag und wo der Fokus
war (`state.json` im Konfigurationsordner). Beim nächsten Start werden genau diese Sessions mit
`--resume` fortgesetzt und in ihre Fenster gelegt. Gibt es nichts wiederherzustellen, startet eine
neue Session im aktuellen Verzeichnis.

## Installieren, aktualisieren, entfernen

```
claude-tui install     # an festen Ort kopieren und in den PATH eintragen
claude-tui update      # neuestes Release von GitHub installieren
claude-tui uninstall   # Programm und PATH-Eintrag entfernen (Einstellungen bleiben)
```

| | Windows | Linux / macOS |
|---|---|---|
| Ort | `%LOCALAPPDATA%\Programs\claude-tui\claude-tui.exe` | `~/.local/bin/claude-tui` |
| PATH | Eintrag im **Benutzer**-PATH, ohne Adminrechte; gilt für neu geöffnete Terminals | nicht verändert — steht `~/.local/bin` nicht im PATH, nennt `install` die Zeile fürs Shell-Profil |

**Update-Prüfung:** Beim Start fragt claude-tui höchstens einmal am Tag im Hintergrund die GitHub-API
nach dem neuesten Release (Zeitpunkt in `update-check.json` im Konfigurationsordner). Ist es neuer,
zeigt der untere Rahmen der Sessionliste `vX.Y.Z available (u)`, und im Ctrl+A-Menü erscheint **`u` Update to vX.Y.Z**.
Das Update lädt das Archiv für die eigene Plattform, vergleicht es mit `sha256.sum` und ersetzt die
laufende Datei; wirksam ab dem nächsten Start (die Fenster kommen über `state.json` zurück).
Installiert wird nie ungefragt. `CLAUDE_TUI_NO_UPDATE_CHECK=1` schaltet die Prüfung ab.

Heruntergeladen wird mit dem `curl` des Systems (unter Windows fest
`C:\Windows\System32\curl.exe`, vorhanden seit Windows 10 1803) — mit dessen Zertifikaten und
Proxy-Einstellungen. Fehlt `curl`, nimmt claude-tui unter Windows und macOS das TLS des
Betriebssystems; unter Linux muss `curl` installiert sein.

## Aufbau

Links die **Sessionliste**: alle gespeicherten Claude-Code-Sessions (`~/.claude/projects`),
gruppiert nach Arbeitsverzeichnis — Gruppen mit der jüngsten Aktivität oben, darin laufende Sessions
zuerst. Je Session eine Zeile: Zustandssymbol, Titel (eigener Name aus `/rename`, sonst der KI-Titel,
sonst der letzte Prompt), Fensternummer (`[1]`, falls sichtbar) und die Zeit seit der letzten
Aktivität (`now`, `5m`, `3h`, `2d`); bei einer wartenden Session ist das die Wartezeit. Laufende Sessions ohne Fenster
arbeiten im Hintergrund weiter. Die Liste aktualisiert sich alle 2 Sekunden.

Hat eine Session in Claude per **`/color`** eine Farbe bekommen (red, orange, yellow, green, cyan,
blue, purple, pink), erscheint ihr Titel in dieser Farbe; `/color default` hebt das wieder auf.
claude-tui liest dafür die letzte `agent-color`-Zeile aus dem Protokoll der Session.

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
oder die Sessionliste) hat einen blauen Rahmen (`#009fda`, wie das Menü), alle anderen einen
dunkelorangen.

Die Fensterleiste zeigt Nummer, Titel und den Zustand als Text: **working** (rot), **waiting for
you** (gelb), **ready** (grau) — z. B. `1 · claude-tui-rust · ✻ working` — mit demselben Symbol wie in der Sessionliste. Die Sessionliste nutzt dieselben Farben.

## Tasten

### Überall

| Taste | Wirkung |
|---|---|
| `Alt+←/→/↑/↓` | Fokus zum Nachbarfenster in Pfeilrichtung; `Alt+←` aus einem linken Fenster geht in die Sessionliste, `Alt+→` aus der Liste ins Fenster der gewählten Session (sonst Fenster 1) |
| `Ctrl+A` | öffnet das Menü mittig über dem Bereich mit dem Fokus (Fenster oder Sessionliste) |

`Alt+Pfeil` erreicht Claude dadurch nie; Wortsprünge im Eingabefeld gehen über `Ctrl+←/→`.

### Fokus auf einem Fenster

Alle anderen Tasten gehen an die Session. `Shift+Enter` sendet `ESC CR` — Claude Code fügt damit
einen Zeilenumbruch ein.

### Fokus auf der Sessionliste

| Taste | Wirkung |
|---|---|
| `↑` / `↓` | Session auswählen |
| `Enter` | hat die Session ein Fenster: Fokus dorthin; sonst Fensterauswahl (eine gespeicherte Session wird dabei mit `--resume` fortgesetzt) |
| `1`–`4` | Session in dieses Fenster legen (eine sichtbare Session tauscht den Platz) |

### Menü (`Ctrl+A`)

Das Menü erscheint als Box mit blauem Rand mittig über dem Bereich mit dem Fokus: über dem
fokussierten Fenster bzw. über der Sessionliste. Ist das fokussierte Fenster zu klein für das Menü,
weicht es auf die ganze Fensterfläche aus; über der Sessionliste wird es auf deren Breite begrenzt.
Genauso erscheinen Fensterauswahl, Löschabfrage und Verzeichnisauswahl. Der untere Rahmen der
Sessionliste erinnert mit `Ctrl+A Menu` daran, wie sich das Menü öffnet. Punkte per Taste oder mit
`↑`/`↓` und `Enter` wählen, `Esc` schließt. Welche Punkte erscheinen, hängt
vom Fokus ab:

| Taste | Fokus auf der Liste | Fokus auf einem Fenster |
|---|---|---|
| `n` | neue Session: Verzeichnis, dann Fensterauswahl | neue Session: Verzeichnis, dann in diesem Fenster |
| `c` | Fenster der gewählten Session schließen | Fenster schließen |
| `x` | gewählte Session endgültig löschen | Session dieses Fensters endgültig löschen |
| `1`–`4` / `0` | Fokus auf Fenster / Sessionliste | Fokus auf Fenster / Sessionliste |
| `Ctrl+A` | – | sendet ein echtes Ctrl+A an Claude |
| `q` | claude-tui beenden (alle Sessions enden) | claude-tui beenden |

`c` lässt die Session im Hintergrund weiterlaufen. `x` fragt nach („Delete …?“, `y`/Enter bestätigt,
`n`/Esc bricht ab), schließt dann Fenster und Prozess und **löscht das Protokoll endgültig**
(`~/.claude/projects/<Ordner>/<id>.jsonl` samt gleichnamigem Unterordner) — die Session ist danach
auch für `claude --resume` weg.

**Verzeichnisauswahl:** Vor jeder neuen Session (`n`) fragt claude-tui im Overlay, in
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

**Fensterauswahl:** listet die belegten Fenster („Replace Window 1“, Nummer wie in der Fensterleiste)
und, solange weniger als 4 offen sind, „New window“. Eine ersetzte Session läuft im Hintergrund
weiter. Ist noch kein Fenster offen, entfällt die Auswahl und Fenster 1 wird genommen.

## Maus

Das **Mausrad** über einem Fenster scrollt dessen Verlauf (3 Zeilen je Raste, bis zu 10 000 Zeilen);
die Fensterleiste zeigt dann `↑ N lines`. Jede Eingabe in die Session springt zurück ans Ende.
Schaltet das Programm im Fenster selbst die Maus ein, bekommt es das Rad-Ereignis durchgereicht.

**Auswählen per Klick:** Ein Klick in ein Fenster (auch auf Rahmen oder Titel) gibt ihm den Fokus.
Ein Klick auf eine Session in der Sessionliste wählt sie aus und gibt der Liste den Fokus; ein
**Doppelklick** darauf wirkt wie `Enter` (ins Fenster der Session springen bzw. Fensterauswahl).

**Offenes Menü:** Ein Klick außerhalb des Menü-Overlays schließt es und wirkt wie oben beschrieben.
Klicks und Mausrad auf dem Overlay selbst tun nichts (das Menü wird per Tastatur bedient) und
erreichen das Fenster darunter nicht.

**Markieren und Kopieren:** Mit gedrückter linker Maustaste in einem Fenster ziehen markiert Text
(blau hinterlegt) — nur innerhalb dieses Fensters, auch wenn die Maus darüber hinaus gezogen wird.
Beim Loslassen landet der Text in der Zwischenablage, unten in der Sessionliste steht
`copied N characters`. **Doppelklick** markiert und kopiert ein Wort. Ein einfacher Klick setzt nur
den Fokus auf das Fenster. Die Markierung verschwindet mit der nächsten Taste, dem nächsten Klick
oder beim Scrollen; markiert wird der sichtbare Ausschnitt (zum Markieren älterer Ausgabe vorher
hochscrollen). `Ctrl+C` bleibt bei Claude.

claude-tui fängt dafür die Maus ab; die Markierung des Terminals selbst gibt es weiter mit
**Shift + Ziehen**, sie erfasst aber ganze Bildschirmzeilen samt Sessionliste.

## Zustände in der Seitenleiste

| Symbol | Farbe | Zustand | Bedeutung |
|---|---|---|---|
| ✻ (animiert) | rot | working | Die Session hat in den letzten 1,5 s etwas ausgegeben (Claudes Spinner). |
| ▲ | gelb | waiting for you | Keine Ausgabe, und auf dem Bildschirm steht eine Rückfrage („Do you want to …“). |
| ● | grau | ready | Keine Ausgabe, keine Rückfrage — Claude wartet auf den nächsten Prompt. |
| ○ | grau | gespeichert | Läuft nicht, lässt sich fortsetzen. Endet ein Prozess von selbst, schließt sich sein Fenster und die Session steht wieder so in der Liste. |

Die Formen unterscheiden sich auch ohne Farbe.

Die Erkennung ist eine Heuristik über Ausgabe-Aktivität und Bildschirmtext, siehe
[Architektur](architektur.md).
