# claude-tui

Ein Terminal-Arbeitsplatz für mehrere Claude-Code-Sessions, geschrieben in Rust. Links eine
Seitenleiste mit allen Sessions und ihrem Zustand (working, waiting for you, ready),
rechts bis zu vier Sessions in echten PTYs. Die Idee stammt von
[herdr](https://github.com/ogulcancelik/herdr); Code ist nicht übernommen.

![claude-tui mit drei Sessions: working, waiting for you, ready](docs/screenshot.png)

## Features

Details zu allem stehen in der [Bedienung](docs/bedienung.md).

**Fenster und Layout**

- Bis zu **4 Sessions gleichzeitig** in echten PTYs, automatisch angeordnet (1, 2, 3 oder 4 Fenster).
- Zwei Fenster wahlweise **nebeneinander oder übereinander**.
- **Zoom** wie in tmux: ein Fenster bekommt die ganze Fläche, die übrigen Sessions laufen weiter.
- **Sessionliste ausblendbar**, damit die Fenster die volle Breite bekommen.
- Fokuswechsel mit `Ctrl+Pfeil`, Fokus erkennbar am blauen Rahmen.
- Der Hintergrund ist der des Terminals, also derselbe wie in Claude Code.

**Sessionliste**

- Alle gespeicherten Claude-Code-Sessions (`~/.claude/projects`), gruppiert nach Arbeitsverzeichnis,
  laufende zuerst.
- Je Session: Zustand, Titel (`/rename`, KI-Titel oder letzter Prompt), Fensternummer und Zeit seit
  der letzten Aktivität bzw. Wartezeit.
- Titel in der Farbe aus Claudes **`/color`**.
- **Eigene Gruppen** (📌) unabhängig vom Verzeichnis.
- **Schnellwahl:** `Alt+1`–`9` gibt einer Session eine Nummer, `Ctrl+1`–`9` springt zu ihr.
- Sessions **fortsetzen** (`--resume`), **löschen**, mehrere markiert oder nach Alter (älter als
  1 Tag / 1 Woche / 1 Monat) löschen.
- Neue Session mit **Verzeichnisauswahl**: Tab-Ergänzung und zuletzt genutzte Verzeichnisse.
- Startet sofort aus einem Cache, neue Sessions werden im Hintergrund nachgelesen.

**Zustände**

- **working** (rot, animiert), **waiting for you** (gelb), **ready** (grau), gespeichert, **läuft
  anderswo** (`◇ ext`), in der Liste und in der Fensterleiste.
- Zustand kommt über **Hooks** direkt von Claude Code. Die eigene `~/.claude/settings.json` bleibt
  dabei unverändert, und ohne Hook-Meldung greift eine Heuristik.
- **Hinweis auf wartende Sessions:** Terminal-Glocke und Meldung, wenn eine Session außerhalb des
  Fokus auf dich wartet. `Ctrl+A w` springt reihum zur nächsten wartenden.
- Erkennt Sessions, die **außerhalb von claude-tui** laufen, und fragt vor dem Öffnen nach.
  Hintergrund-Sessions (`claude --bg`) verbindet es per `claude attach`.
- Sessionwechsel in Claude (`/resume`, `/clear`, `/branch`) übernehmen Fenster und Liste.

**Bedienung**

- Alle Befehle im **`Ctrl+A`-Menü**, das über dem Bereich mit dem Fokus erscheint.
- **Hilfeseite** im Menü (`h`) mit allen Tasten, Zuständen und Menüpunkten.
- **Maus:** Klick setzt den Fokus, Doppelklick öffnet eine Session, das Mausrad scrollt den Verlauf
  (bis zu 10 000 Zeilen).
- **Markieren und Kopieren:** Claudes eigene Markierung im Fullscreen-Modus oder die von claude-tui
  (mit `Alt`), direkt in die Zwischenablage.
- `Shift+Enter` fügt in Claude einen Zeilenumbruch ein.

**Start und Wiederherstellen**

- Beim Start kommen die Sessions des letzten Laufs in ihre Fenster zurück, samt Fokus, Layout,
  Gruppen und Schnellwahl-Nummern.
- Die Claude-Prozesse starten **nacheinander**, damit sie sich nicht gegenseitig ausbremsen.
- Eigener Startbefehl möglich, z. B. `claude-tui "claude --model opus"`.
- Zeitmessung für die Fehlersuche mit `CLAUDE_TUI_TRACE=1`.

**Installation und Updates**

- Eine einzelne Datei für Windows, Linux und macOS. `install` / `uninstall` tragen sie in den PATH
  ein bzw. entfernen sie, ohne Adminrechte.
- **Tägliche Update-Prüfung** auf GitHub. Updates werden nie ungefragt installiert (`Ctrl+A u`), die
  heruntergeladene Datei wird per SHA-256 geprüft.

## Aufbau

| Ordner | Inhalt |
|---|---|
| `src/` | Rust-Quellcode; `Cargo.toml` liegt im Wurzelordner |
| `docs/` | Dokumentation: [Bedienung](docs/bedienung.md), [Architektur](docs/architektur.md), [Build & Release](docs/release.md), [Code-Richtlinien](docs/code-richtlinien.md) |

## Installieren

Fertige Programme für Windows, Linux und macOS gibt es unter
[Releases](https://github.com/NicoMertens/claude-tui/releases) — je Plattform eine einzelne Datei
`claude-tui-v<Version>-<Ziel>[.exe]`. Herunterladen (unter Linux/macOS `chmod +x`), dann:

```
claude-tui install     # kopiert sich an einen festen Ort und trägt ihn in den PATH ein
```

Danach startet `claude-tui` in jedem neuen Terminal. Unter Windows landet es in
`%LOCALAPPDATA%\Programs\claude-tui` (Benutzer-PATH, keine Adminrechte), unter Linux/macOS in
`~/.local/bin`. Windows in einem Schritt (PowerShell):

```powershell
$exe = "$env:TEMP\claude-tui.exe"
$url = (irm https://api.github.com/repos/NicoMertens/claude-tui/releases/latest).assets |
  ? name -like '*x86_64-pc-windows-msvc.exe' | % browser_download_url
iwr $url -OutFile $exe; & $exe install
```

**Updates:** claude-tui schaut beim Start höchstens einmal am Tag auf GitHub nach einer neuen
Version und zeigt sie in der Menübox an (`Ctrl+A u` installiert sie). Von Hand: `claude-tui update`.
Abschalten: Umgebungsvariable `CLAUDE_TUI_NO_UPDATE_CHECK=1`. Entfernen: `claude-tui uninstall`.

`install`, `update` und `uninstall` gibt es ab v0.2.0.

## Selbst bauen

```
cargo run --release    # bauen und starten, jede Session startet `claude`
```

Die gebaute App liegt danach unter `target/release/claude-tui`.

## Lizenz

[MIT](LICENSE)
