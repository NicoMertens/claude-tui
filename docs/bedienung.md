# Bedienung

## Start

```
claude-tui                      # Claude Code über `claude` starten
claude-tui "claude --model opus" # anderer Befehl bzw. zusätzliche Optionen
claude-tui --version
```

claude-tui hängt an den Befehl selbst `--session-id <neue ID>` (neue Session) bzw. `--resume <ID>`
(gespeicherte Session fortsetzen) an — Optionen wie `-c` oder `--resume` gehören deshalb nicht in den
Befehl. Dazu kommt `--settings <Konfigurationsordner>/hooks-<PID>.json`: Hooks, über die Claude Code
seinen Zustand meldet (siehe [Zustände](#zustände-in-der-seitenleiste)). Sie gelten nur für die von
claude-tui gestarteten Prozesse; die eigene `~/.claude/settings.json` bleibt unverändert. Unter Windows läuft er über `cmd.exe /C`, unter Linux/macOS über `sh -c`, damit
PATH-Auflösung und npm-Wrapper (`claude.cmd`) funktionieren.

**Wiederherstellen:** claude-tui merkt sich, welche Session in welchem Fenster lag und wo der Fokus
war (`state.json` im Konfigurationsordner). Beim nächsten Start werden genau diese Sessions mit
`--resume` fortgesetzt und in ihre Fenster gelegt. Gibt es nichts wiederherzustellen, startet keine
neue Session; der Fokus liegt auf der Sessionliste.

Die Fenster erscheinen sofort, die Claude-Prozesse starten aber **nacheinander**: zuerst das Fenster
mit Fokus, dann die übrigen in Fensterreihenfolge — jeweils sobald die vorige Session etwas anzeigt,
spätestens nach 3 s. Mehrere Claude-Prozesse, die gleichzeitig ihr Protokoll und ihre MCP-Server
laden, bremsen sich sonst gegenseitig aus. Ein wartendes Fenster zeigt „queued – starts after the
previous window“; wer hineintippt, startet es sofort. Bis eine Session die erste Ausgabe zeigt,
steht im Fenster „starting claude … Ns“ und in der Leiste der Zustand `starting`.

**Sessionliste beim Start:** Den Lesestand der Protokolle merkt sich claude-tui in
`sessions-cache.json` im Konfigurationsordner. Beim Start erscheint die Liste damit sofort; was seit
dem letzten Lauf dazukam, liest ein Hintergrund-Thread nach. Ohne Cache (erster Start) steht dort
kurz „loading sessions …“.

**Zeitmessung:** Mit `CLAUDE_TUI_TRACE=1` schreibt claude-tui Zeitstempel nach `trace.log` im
Konfigurationsordner: Start (Cache, Einlesen, Wiederherstellen, erstes Bild), je Session
Prozessstart, erste Ausgabe und erste Hook-Meldung sowie jeden Hook-Aufruf. Damit lässt sich
klären, ob Wartezeit in claude-tui oder in Claude Code liegt — ein großer Abstand zwischen „process
started“ und „first output“ ist Claude Code selbst.

**Virenscanner:** Die Hooks melden per HTTP an claude-tui, Claude startet dafür keine Prozesse.
Prüft ein Virenscanner die unsignierte `claude-tui.exe` bei jedem Start, trifft das daher nur noch
den Start von claude-tui selbst. Messen lässt sich das mit
`Measure-Command { '{}' | claude-tui hook Test }` (normal unter 100 ms). Dauert es Sekunden, hilft
eine Ausnahme für `%LOCALAPPDATA%\Programs\claude-tui`, soweit die eigene IT das zulässt.

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
Das Update lädt die Binary für die eigene Plattform, vergleicht sie mit `sha256.sum` und ersetzt die
laufende Datei; wirksam ab dem nächsten Start (die Fenster kommen über `state.json` zurück).
Installiert wird nie ungefragt. `CLAUDE_TUI_NO_UPDATE_CHECK=1` schaltet die Prüfung ab.

Heruntergeladen wird mit dem `curl` des Systems (unter Windows fest
`C:\Windows\System32\curl.exe`, vorhanden seit Windows 10 1803) — mit dessen Zertifikaten und
Proxy-Einstellungen. Fehlt `curl`, nimmt claude-tui unter Windows und macOS das TLS des
Betriebssystems; unter Linux muss `curl` installiert sein.

## Aufbau

Links die **Sessionliste**: alle gespeicherten Claude-Code-Sessions (`~/.claude/projects`). Ganz
oben steht der [Notizblock](#notizblock) (`✎ Notepad (N)`), darunter die [eigenen Gruppen](#gruppen) (⚑, alphabetisch), darunter die übrigen Sessions gruppiert
nach Arbeitsverzeichnis (▾, alphabetisch). Jede Überschrift zeigt
die Anzahl ihrer Sessions, z. B. `▾ C:\dev\webshop (3) ────`. In jedem Abschnitt
stehen laufende Sessions zuerst (in der Reihenfolge, in der sie gestartet wurden, damit sie
nicht bei jeder Ausgabe springen), danach die übrigen nach letzter Aktivität. Je Session eine Zeile: Zustandssymbol, Titel (eigener Name aus `/rename`, sonst der KI-Titel,
sonst der letzte Prompt), Schnellwahl-Nummer (`#3`, falls vergeben, siehe
[Schnellwahl](#schnellwahl-altzahl--ctrlzahl)), Fensternummer (`[1]`, falls sichtbar) und die Zeit seit der letzten
Aktivität (`now`, `5m`, `3h`, `2d`); bei einer wartenden Session ist das die Wartezeit. Laufende Sessions ohne Fenster
arbeiten im Hintergrund weiter. Die Liste aktualisiert sich alle 2 Sekunden.

**Einklappen:** Jeder Abschnitt lässt sich einklappen; dann steht nur noch seine Überschrift da
(Verzeichnisse zeigen `▸` statt `▾`). Läuft darin eine Session oder ist eine ungelesen, steht ihr Zustandssymbol vor der
Überschrift — bei mehreren das dringlichste (waiting vor working vor ready). Eingeklappte Abschnitte
bleiben über einen Neustart erhalten. Springt man per [Schnellwahl](#schnellwahl-altzahl--ctrlzahl)
zu einer Session darin, klappt ihr Abschnitt auf.

Hat eine Session in Claude per **`/color`** eine Farbe bekommen (red, orange, yellow, green, cyan,
blue, purple, pink), erscheint ihr Titel in dieser Farbe; `/color default` hebt das wieder auf.
claude-tui liest dafür die letzte `agent-color`-Zeile aus dem Protokoll der Session.

Rechts bis zu **4 Fenster**, jedes mit einer Session oder dem [Notizblock](#notizblock). Jede
Nummer hat einen **festen Platz**: 1 oben links, 2 oben rechts,
3 unten links, 4 unten rechts. Jede Nummer lässt sich öffnen, auch wenn kleinere fehlen. Fehlende
Fenster lassen keine Lücke: Die linke Spalte gehört 1 und 3, die rechte 2 und 4. Ist eine Spalte
leer, nimmt die andere die ganze Breite; steht ein Fenster allein in seiner Spalte, bekommt es die
ganze Höhe.

```
nur 2        1, 2         1, 3         1, 2, 3      1, 2, 4      1, 2, 3, 4
┌───────┐    ┌───┬───┐    ┌───────┐    ┌───┬───┐    ┌───┬───┐    ┌───┬───┐
│   2   │    │ 1 │ 2 │    │   1   │    │ 1 │   │    │   │ 2 │    │ 1 │ 2 │
│       │    │   │   │    ├───────┤    ├───┤ 2 │    │ 1 ├───┤    ├───┼───┤
│       │    │   │   │    │   3   │    │ 3 │   │    │   │ 4 │    │ 3 │ 4 │
└───────┘    └───┴───┘    └───────┘    └───┴───┘    └───┴───┘    └───┴───┘
```

Wird ein Fenster geschlossen, behalten die übrigen ihre Nummer und ihren Platz; die Fläche wird
nach derselben Regel neu aufgeteilt. Welche Session in welchem Fenster liegt, bleibt über einen
Neustart erhalten (`state.json`). Der Bereich mit dem **Fokus** (ein Fenster
oder die Sessionliste) hat einen blauen Rahmen (`#009fda`, wie das Menü), alle anderen einen
grauen.

Die Fensterleiste zeigt Nummer, Titel und den Zustand als Text: **working** (rot), **waiting for
you** (gelb), **ready** (grau) — z. B. `[1] claude-tui-rust #3 · ✻ working` (`#3` nur mit
Schnellwahl-Nummer) — mit demselben Symbol wie in der Sessionliste. Die Sessionliste nutzt dieselben Farben.
Rechts in der Fensterleiste steht die Claude-Code-Version der Session (z. B. `v2.1.326`), gelesen aus
ihrem Protokoll; sie erscheint, sobald Claude die erste Nachricht dort festgehalten hat.

## Tasten

### Überall

| Taste | Wirkung |
|---|---|
| `Ctrl+←/→/↑/↓` | Fokus zum Nachbarfenster in Pfeilrichtung; `Ctrl+←` aus einem linken Fenster geht in die Sessionliste, `Ctrl+→` aus der Liste ins Fenster der gewählten Session (sonst ins offene Fenster mit der kleinsten Nummer). In der Liste springen `Ctrl+↑`/`Ctrl+↓` zur Überschrift der vorigen bzw. nächsten Gruppe oder des Verzeichnisses (von einer Session aus führt `Ctrl+↑` zuerst auf ihre eigene Überschrift) |
| `Ctrl+A` | öffnet das Menü mittig über dem Bereich mit dem Fokus (Fenster oder Sessionliste) |
| `Alt+1`–`Alt+9` | Schnellwahl-Nummer für die Session mit Fokus vergeben bzw. mit derselben Zahl wieder entfernen |
| `Ctrl+1`–`Ctrl+9` | Session mit dieser Schnellwahl-Nummer zeigen |
| `Ctrl+Space` | fertige Antwort der Session mit Fokus (bzw. der in der Liste gewählten) als gelesen setzen (aus **waiting for you** wird **ready**) bzw. wieder als ungelesen (siehe [Zustände](#zustände-in-der-seitenleiste)) |

`Ctrl+Pfeil`, `Alt+Zahl`, `Ctrl+Zahl` und `Ctrl+Space` erreichen Claude dadurch nie; Wortsprünge im Eingabefeld
gehen über `Alt+←/→`.

### Schnellwahl (`Alt+Zahl` / `Ctrl+Zahl`)

`Alt+Zahl` gibt der Session mit Fokus — der im Fenster mit Fokus bzw. der in der Liste gewählten —
eine Nummer von 1 bis 9. Sie steht als `#3` hinter dem Titel in der Sessionliste und in der
Fensterleiste. Jede Nummer gehört höchstens einer Session: Vergibt man sie neu, verliert die
bisherige Session sie. Eine Session hat höchstens eine Nummer; eine andere Zahl ersetzt sie.
Dieselbe Zahl noch einmal entfernt die Nummer.

`Ctrl+Zahl` zeigt die Session mit dieser Nummer:

- Hat sie schon ein Fenster, bekommt es den Fokus.
- Sonst, bei Fokus auf einem Fenster: Sie ersetzt dort die bisherige Session, die im Hintergrund
  weiterläuft.
- Sonst, bei Fokus auf der Sessionliste: wie `Enter` (Fensterauswahl bzw. Fenster 1).

Eine gespeicherte Session wird dabei mit `--resume` fortgesetzt; läuft sie anderswo, kommt dieselbe
Rückfrage wie in der Liste. Die Nummern bleiben über einen Neustart erhalten (`state.json`).
Gelöschte Sessions verlieren ihre Nummer, auch wenn ihr Protokoll außerhalb von claude-tui
gelöscht oder von Claude Code aufgeräumt wurde.

Nur reines `Alt` bzw. `Ctrl` zählt: `Ctrl+Alt` ist unter Windows AltGr (`²`, `³`, `{`, `[` …) und
geht weiter an die Session. Ob `Ctrl+Zahl` ankommt, hängt vom Terminal ab; unter Linux/macOS kommt
es in vielen Terminals nicht an.

### Fokus auf einem Fenster

Alle anderen Tasten gehen an die Session. `Shift+Enter` sendet `ESC CR` — Claude Code fügt damit
einen Zeilenumbruch ein.

### Fokus auf der Sessionliste

| Taste | Wirkung |
|---|---|
| `↑` / `↓` | Session oder Überschrift auswählen |
| `PgUp` / `PgDn` | Auswahl seitenweise verschieben |
| `←` / `→` | Abschnitt der Auswahl einklappen (die Auswahl springt auf die Überschrift) / ausklappen |
| `Enter` auf einer Überschrift | Abschnitt ein- bzw. ausklappen |
| `Leertaste` auf einer Überschrift | alle Sessions des Abschnitts markieren; sind schon alle markiert, Markierung aufheben |
| `Enter` | hat die Session ein Fenster: Fokus dorthin; sonst Fensterauswahl (eine gespeicherte Session wird dabei mit `--resume` fortgesetzt) |
| `1`–`4` | Session in dieses Fenster legen, auch wenn es noch nicht offen ist (eine sichtbare Session tauscht den Platz bzw. wandert ins freie Fenster) |

Läuft die gewählte Session **außerhalb von claude-tui** (Kennzeichnung `⇄`, siehe
[Zustände](#zustände-in-der-seitenleiste)), fragen `Enter`, Doppelklick und `1`–`4` erst nach:
„<Titel> runs elsewhere – open anyway?“. `y` startet sie trotzdem ein zweites Mal (bei `Enter` folgt
danach die Fensterauswahl), `n`, `Esc` und `Enter` brechen ab — die Vorauswahl ist „No“, denn zwei
Prozesse schreiben sonst in dasselbe Protokoll.

Läuft sie als **Hintergrund-Session** von Claude Code (`claude --bg`), entfällt die Rückfrage:
claude-tui verbindet sich mit `claude attach <Job-ID>` mit dem laufenden Prozess, statt einen
zweiten zu starten (`--resume` lehnt laufende Hintergrund-Sessions ab). Schließt man das Fenster,
trennt sich nur claude-tui; die Hintergrund-Session läuft weiter.

### Menü (`Ctrl+A`)

Das Menü erscheint als Box mit blauem Rand mittig über dem Bereich mit dem Fokus: über dem
fokussierten Fenster bzw. über der Sessionliste. Ist das fokussierte Fenster zu klein für das Menü,
weicht es auf die ganze Fensterfläche aus; über der Sessionliste wird es auf deren Breite begrenzt.
Genauso erscheinen Fensterauswahl, Löschabfrage und Verzeichnisauswahl. Der untere Rahmen der
Sessionliste erinnert mit `Ctrl+A Menu` daran, wie sich das Menü öffnet. Punkte per Taste oder mit
`↑`/`↓` und `Enter` wählen, `Esc` oder erneut `Ctrl+A` schließt. Welche Punkte erscheinen, hängt
vom Fokus ab:

| Taste | Fokus auf der Liste | Fokus auf einem Fenster |
|---|---|---|
| `n` | neue Session: Verzeichnis, dann Fensterauswahl | neue Session: Verzeichnis, dann in diesem Fenster |
| `c` | Fenster der gewählten Session schließen | Fenster schließen |
| `x` | gewählte Session endgültig löschen; sind Sessions markiert, alle markierten | Session dieses Fensters endgültig löschen (nicht im Notizblock) |
| `g` | gewählte Session einer [Gruppe](#gruppen) zuordnen; sind Sessions markiert, alle markierten | Session dieses Fensters einer Gruppe zuordnen (nicht im Notizblock) |
| `t` | [Notiz](#notizen) der gewählten Session schreiben, bearbeiten oder entfernen | Notiz der Session dieses Fensters schreiben, bearbeiten oder entfernen (nicht im Notizblock) |
| `p` | [Notizblock](#notizblock) in einem Fenster zeigen: Fensterauswahl (ohne offenes Fenster gleich in Fenster 1) | wie links |
| `f` | Abschnitt der Auswahl ein- bzw. ausklappen („Collapse group“ / „Expand group“) | — |
| `a` | alle Abschnitte einklappen (solange einer ausgeklappt ist) | — |
| `e` | alle Abschnitte ausklappen (solange einer eingeklappt ist) | — |
| `o` | gespeicherte Sessions älter als 1 Tag / 1 Woche / 1 Monat löschen | — |
| `1`–`4` / `0` | Fokus auf Fenster / Sessionliste | Fokus auf Fenster / Sessionliste |
| `s` | Sessionliste aus- bzw. einblenden | Sessionliste aus- bzw. einblenden |
| `z` | — | ab 2 Fenstern: Fenster zoomen; gezoomt: Zoom aufheben |
| `h` | [Hilfe](#hilfe) anzeigen | Hilfe anzeigen |
| `q` | claude-tui beenden (alle Sessions enden; laufen noch welche, fragt es nach, Enter = Nein) | claude-tui beenden |

`s` blendet die Sessionliste aus, damit die Fenster die volle Breite bekommen (nur mit
mindestens einem offenen Fenster; hatte die Liste den Fokus, geht er ins Fenster ihrer Session bzw.
ins offene Fenster mit der kleinsten Nummer). Bei ausgeblendeter Liste heißt der Punkt „Show session list“. Die Liste erscheint auch
von selbst wieder, sobald sie den Fokus bekommt (`Ctrl+←`, `0`) oder das letzte Fenster schließt.
Hinweise, die sonst unten in der Liste stehen, zeigt dann der untere Rahmen des Fensters mit Fokus.
Der Zustand bleibt über einen Neustart erhalten (`state.json`).

`z` zoomt wie in tmux: Das Fenster mit Fokus bekommt die ganze Fensterfläche, die übrigen Fenster
werden nicht gezeichnet, ihre Sessions laufen weiter. Die Fensterleiste zeigt dann `· zoomed`, der
Menüpunkt heißt „Unzoom“. Die Sessionliste bleibt, wie sie ist (`s` wirkt unabhängig). Der Zoom endet
von selbst, sobald sich das Layout ändert: Fokus auf ein anderes Fenster oder die Liste (`Ctrl+Pfeil`
— der Nachbar wird im normalen Layout gesucht —, `0`–`4`, Mausklick), Fenster öffnen, schließen
oder ersetzen, `l`. Er wird nicht in `state.json` gespeichert.

`c` lässt die Session im Hintergrund weiterlaufen. `x` fragt nach („Delete …?“, `y`/Enter bestätigt,
`n`/Esc bricht ab), schließt dann Fenster und Prozess und **löscht das Protokoll endgültig**
(`~/.claude/projects/<Ordner>/<id>.jsonl` samt gleichnamigem Unterordner) — die Session ist danach
auch für `claude --resume` weg.

**Mehrere Sessions löschen:**

- **Markieren:** In der Sessionliste markiert bzw. entmarkiert `Leertaste` die gewählte Session (✓ vor
  dem Titel) und springt zur nächsten; `Esc` hebt alle Markierungen auf. Der untere Rahmen zeigt
  `· N marked`. Solange Sessions markiert sind, heißt `x` bei Fokus auf der Liste „Delete N
  marked“ und löscht alle markierten — laufende samt Fenster und Prozess, wie beim Einzellöschen.
- **Nach Alter:** `o` („Delete sessions older than …“) bietet `d` 1 Tag, `w` 1 Woche, `m` 1 Monat.
  Maßgeblich ist die letzte Aktivität laut Protokoll (die Zeit in der Liste), über alle
  Verzeichnisse. Sessions, die in claude-tui oder anderswo laufen, bleiben immer stehen; die
  Rückfrage nennt, wie viele übersprungen werden.

Beide fragen vorher nach („Delete N sessions …?“). Anders als beim Einzellöschen wählt `Enter` hier
„No“ — bestätigt wird mit `y`.

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
bzw. `~/.config/claude-tui/recent-dirs.txt`. Ohne zuletzt genutztes Verzeichnis ist das Verzeichnis
vorgewählt, in dem claude-tui gestartet wurde.

**Fensterauswahl:** listet alle vier Fenster mit ihrer Nummer als Taste, belegte als „Replace
Window n“, freie als „New Window n“. Eine ersetzte Session läuft im Hintergrund weiter. Ist noch kein Fenster offen, entfällt die Auswahl und Fenster 1 wird genommen.

### Hilfe

`h` im Menü öffnet die Hilfe: ein Overlay mittig über allen Fenstern (ist die Fensterfläche
schmaler als 50 Spalten, über dem ganzen Bildschirm). Es erklärt den Aufbau, die Zustandssymbole,
alle Tasten, die Menüpunkte und die Maus — auf Englisch, wie die übrige Oberfläche. Die Menüpunkte
stammen aus derselben Tabelle wie das Menü selbst.

| Taste | Wirkung |
|---|---|
| `↑` / `↓` (`k` / `j`), Mausrad | zeilenweise scrollen |
| `PgUp` / `PgDn`, `Leertaste` | seitenweise scrollen |
| `Home` / `End` | Anfang / Ende |
| `Esc`, `q`, `Ctrl+A` | schließen (auch ein Klick daneben) |

Passt alles hinein, entfällt das Scrollen; sonst nennt die Fußzeile den sichtbaren Ausschnitt
(`12–40 of 85`).

### Gruppen

Sessions lassen sich in eigene Gruppen packen, unabhängig vom Verzeichnis. Eine gruppierte Session
steht nicht mehr unter ihrem Verzeichnis, sondern unter der Gruppe; die Gruppen stehen oben in der
Sessionliste, alphabetisch sortiert. Eine Session gehört höchstens einer Gruppe.

`g` im Menü öffnet den Gruppen-Dialog für die Session mit Fokus (im Fenster bzw. die in der Liste
gewählte) oder — mit Fokus auf der Liste — für alle markierten Sessions:

| Taste | Wirkung |
|---|---|
| Tippen / Einfügen | Namen einer neuen Gruppe eingeben; filtert zugleich die Liste darunter |
| `↑` / `↓` | Eintrag wählen: **None** (aus der Gruppe nehmen, nur wenn eine der Sessions in einer Gruppe ist), dann die zuletzt verwendeten und die übrigen vorhandenen Gruppen |
| `Enter` | gewählten Eintrag übernehmen, sonst den getippten Namen |
| `Esc` | abbrechen |

Groß-/Kleinschreibung trennt keine Gruppen: `backend` landet in einer vorhandenen Gruppe `Backend`.
„None“ ist als Name reserviert. Eine Gruppe besteht, solange ihr eine Session angehört. Zuordnung
und die 10 zuletzt verwendeten Gruppen bleiben über einen Neustart erhalten (`state.json`);
gelöschte Sessions verlieren ihre Gruppe, auch außerhalb von claude-tui gelöschte.

### Notizen

Jede Session kann eine Notiz tragen, z. B. worauf sie wartet oder was als Nächstes ansteht. Sie
steht orange unten im Fenster (`✎ …`), mehrzeilig bis zu 5 Zeilen und höchstens ein Drittel der
Fensterhöhe; was nicht passt, zählt `(+N)` am Ende. Das Terminal der Session wird dafür
entsprechend niedriger. In der Sessionliste
kennzeichnet ein `✎` hinter dem Titel Sessions mit Notiz, auch gespeicherte, die gerade nicht laufen.

`t` im Menü öffnet den Notiz-Editor für die Session mit Fokus (im Fenster bzw. die in der Liste
gewählte), mit der vorhandenen Notiz vorbelegt:

| Taste | Wirkung |
|---|---|
| Tippen / Einfügen | Text eingeben; Eingefügtes behält seine Zeilenumbrüche |
| `Shift+Enter` / `Ctrl+J` | neue Zeile |
| `←` `→` `↑` `↓` `Pos1` `Ende` | Cursor bewegen |
| `Enter` | speichern; leerer Text entfernt die Notiz |
| `Ctrl+D` | Notiz entfernen |
| `Esc` | abbrechen |

Notizen bleiben über einen Neustart erhalten (`state.json`) und gehen wie die Gruppe bei `/clear`
auf die neue Session über. Gelöschte Sessions verlieren ihre Notiz.

### Notizblock

Neben den Notizen je Session gibt es einen **Notizblock** für freien Text, der keiner Session
gehört. Er steht als erste Zeile in der Sessionliste, über allen Abschnitten: `✎ Notepad (12)` mit
der Anzahl seiner Textzeilen (umbrochene Zeilen zählen einfach) und, falls sichtbar, der
Fensternummer. Auf dieser Zeile zeigt `Enter` ihn (Fokus auf sein Fenster bzw. Fensterauswahl),
`1`–`4` legt ihn in dieses Fenster, ein Doppelklick wirkt wie `Enter` und `c` im Menü schließt sein
Fenster. Hat sein Fenster den Fokus, steht die Listenauswahl auf dieser Zeile.

`p` im Menü zeigt ihn in einem Fenster nach Wahl — wie eine Session: Er nimmt einen der
vier Plätze ein, tauscht beim Verschieben den Platz, lässt sich zoomen und mit `c` schließen. Die
Fensterleiste zeigt `[N] ✎ Notepad`. Es gibt nur einen Notizblock; ein zweites `p` verschiebt ihn.

Der Text wird an Wortgrenzen umbrochen (kein waagerechtes Scrollen, ein Wort länger als die Zeile
wird geteilt). Ist er länger als das Fenster, scrollt die Ansicht mit dem Cursor, und am rechten
Rahmen erscheint eine Scrollbar. Mit Fokus auf dem Notizblock:

| Taste | Wirkung |
|---|---|
| Tippen / Einfügen | Text eingeben; Eingefügtes behält seine Zeilenumbrüche |
| `Enter` / `Tab` | neue Zeile / vier Leerzeichen |
| `←` `→` | Cursor um ein Zeichen |
| `↑` `↓` `Pos1` `Ende` | Cursor nach Bildschirmzeilen (eine umbrochene Zeile zählt mehrfach) |
| `Ctrl+Pos1` / `Ctrl+Ende` | an den Anfang / das Ende des Texts |
| `PgUp` / `PgDn` | seitenweise |
| `Backspace` / `Entf` | Zeichen löschen |

Ein Klick setzt den Cursor, das Mausrad scrollt (der Cursor bleibt, wo er ist). `Ctrl+A`, `Ctrl+Pfeil`
und die Schnellwahl wirken wie in einem Session-Fenster.

Löschen gibt es nicht, der Notizblock ist ein reiner Editor: Der Text bleibt beim Schließen des
Fensters erhalten und wird spätestens eine Sekunde nach einer Änderung sowie beim Beenden als
`notepad.txt` im Konfigurationsordner gespeichert (`%APPDATA%\claude-tui` bzw.
`~/.config/claude-tui`). Das Fenster des Notizblocks kommt nach einem Neustart wie die Sessions
zurück (`state.json`). Laufen mehrere claude-tui, teilen sie sich den Notizblock: Ändert ein
anderes die Datei, liest dieses sie neu, solange es selbst nichts Ungespeichertes hat; sonst gewinnt,
wer zuletzt speichert. Lässt sich `notepad.txt` nicht lesen (z. B. nicht UTF-8), meldet claude-tui das
beim Start und überschreibt die Datei nicht.

## Maus

Das **Mausrad** über einem Fenster scrollt dessen Verlauf (3 Zeilen je Raste, bis zu 10 000 Zeilen;
über dem [Notizblock](#notizblock) dessen Text);
die Fensterleiste zeigt dann `↑ N lines`. Jede Eingabe in die Session springt zurück ans Ende.
Schaltet das Programm im Fenster selbst die Maus ein, bekommt es das Rad-Ereignis durchgereicht.

**Auswählen per Klick:** Ein Klick in ein Fenster (auch auf Rahmen oder Titel) gibt ihm den Fokus.
Ein Klick auf eine Session in der Sessionliste wählt sie aus und gibt der Liste den Fokus; ein
**Doppelklick** darauf wirkt wie `Enter` (ins Fenster der Session springen bzw. Fensterauswahl).
Ein Klick auf eine Überschrift wählt sie aus; ein weiterer Klick auf die gewählte Überschrift klappt
ihren Abschnitt ein bzw. aus.

**Offenes Menü:** Ein Klick außerhalb des Menü-Overlays schließt es und wirkt wie oben beschrieben.
Klicks und Mausrad auf dem Overlay selbst tun nichts (das Menü wird per Tastatur bedient) und
erreichen das Fenster darunter nicht. Einzige Ausnahme: Über der [Hilfe](#hilfe) scrollt das Mausrad
deren Text.

**Markieren und Kopieren in Claude:** Schaltet das Programm im Fenster selbst die Maus ein — Claude
Code tut das im Fullscreen-Modus, der bei neueren Installationen Standard ist (`/tui fullscreen`
bzw. `/tui default` schaltet um) —, gehen Klicks und Ziehen mit allen Tasten an das Programm.
Claude markiert dann selbst im Gesprächsverlauf: Ziehen markiert, Doppelklick ein Wort bzw. einen
ganzen Pfad oder eine URL, Dreifachklick eine Zeile, `Shift+Pfeil` erweitert per Tastatur (am
Rand scrollt die Ansicht mit). Kopiert wird von Claude selbst beim Loslassen (abschaltbar über
`/config` → *Copy on select*, dann `Ctrl+C` bei aktiver Markierung). Der Klick gibt dem Fenster
zugleich den Fokus. Reine Mausbewegungen (Hover) werden nicht weitergereicht.

**Eigene Markierung:** Hat das Programm die Maus nicht eingeschaltet, oder wird **Alt** beim
Klicken gehalten, markiert claude-tui selbst: Ziehen mit der linken Maustaste markiert Text (blau
hinterlegt) — nur innerhalb dieses Fensters, auch wenn die Maus darüber hinaus gezogen wird.
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
| ✻ (animiert) | rot | working | Claude bearbeitet einen Prompt oder nutzt ein Tool. |
| ▲ | gelb | waiting for you | Claude fragt nach einer Freigabe (Berechtigungsdialog) oder braucht eine Eingabe — oder hat eine Antwort fertig, die noch nicht als gelesen gesetzt ist. |
| ● | grau | ready | Claude ist fertig, die Antwort ist gelesen, es wartet auf den nächsten Prompt. |
| (keins) | grau (Titel) | gespeichert | Läuft nicht, lässt sich fortsetzen. Endet ein Prozess von selbst, schließt sich sein Fenster und die Session steht wieder so in der Liste. |
| ⇄ (statt Fensternummer) | orange | läuft anderswo | Ein Claude-Prozess außerhalb von claude-tui (z. B. in einem anderen Terminal) hat die Session offen. Öffnen fragt erst nach (Hintergrund-Sessions: `claude attach`, ohne Rückfrage). |

Die Formen unterscheiden sich auch ohne Farbe.

**Ungelesene Antworten:** Wird Claude mit einer Arbeit fertig (working → ready), steht die Session
auf **waiting for you**, bis man die Antwort mit `Ctrl+Space` als gelesen setzt oder Claude wieder
arbeitet — auch wenn die Session im Fenster mit Fokus liegt. `Ctrl+Space` auf einer Session im
Zustand **ready** setzt sie wieder als ungelesen. Eine echte Rückfrage bleibt dagegen **waiting for
you**, bis sie beantwortet ist; `Ctrl+Space` ändert daran nichts.

Ungelesene Sessions stehen in `state.json` und bleiben so über einen Neustart erhalten, auch über das
Schließen eines Fensters. Auch gespeicherte Sessions, die nicht laufen, zeigen dann ▲ in der Liste
(sonst haben sie kein Zustandssymbol); `Ctrl+Space` auf der gewählten Zeile schaltet die Markierung
um, ohne die Session zu öffnen. Gibt es das Protokoll einer Session nicht mehr (in claude-tui oder außerhalb gelöscht,
von Claude Code aufgeräumt), verschwindet auch ihr Eintrag.

Die Erkennung ist eine Heuristik über Ausgabe-Aktivität und Bildschirmtext, siehe
[Architektur](architektur.md).

**Läuft anderswo** erkennt claude-tui an der Datei, die Claude Code je laufendem Prozess unter
`~/.claude/sessions/<PID>.json` ablegt, solange der Prozess noch existiert; bei älteren
Claude-Code-Versionen ohne diese Datei daran, dass das Protokoll in den letzten 10 s geschrieben
wurde, ohne dass die Session in claude-tui läuft (dann nur, solange sie arbeitet). Die Kennzeichnung
folgt mit einigen Sekunden Verzögerung. Beim Start setzt claude-tui eine Session aus `state.json`, die
gerade anderswo läuft, nicht fort, sondern meldet „… runs elsewhere – not reopened“ — eine
Hintergrund-Session öffnet es dagegen mit `claude attach`.

**Sessionwechsel in Claude:** Lädt man in einem Fenster mit `/resume` eine andere Session oder
beginnt mit `/clear` eine neue, zeigen Fensterleiste und Sessionliste kurz danach die neue Session; die vorherige steht weiter als gespeicherte Session in
der Liste. Schnellwahl-Nummer, Gruppe und Notiz gehen bei `/clear` auf die neue Session über, bei
`/resume` bleiben sie bei der vorherigen. Dafür schaltet claude-tui in den Sessions, die es startet,
Claudes Agent View ab (`disableAgentView`): `/resume` wechselt dann im selben Prozess, statt die
Session in den Hintergrund zu verlegen, und `←` am leeren Prompt öffnet keine Agent View.

Eine Kopie der Unterhaltung, zu der man wechseln kann, legt **`/branch`** an: Das Fenster zeigt
danach die Kopie, das Original steht weiter in der Sessionliste. `/fork` erzeugt ohne Agent View
dagegen nur einen Subagenten innerhalb der Session — keine eigene Session, also auch kein Eintrag
in der Liste.

Endet ein Prozess in einem Fenster, nennt der Hinweis „… has exited: …“ die letzte Zeile, die er auf
den Bildschirm geschrieben hat (meist die Fehlermeldung).

**Hinweis auf wartende Sessions:** Wechselt eine Session, die nicht im Fenster mit Fokus liegt
(auch eine Hintergrund-Session ohne Fenster), auf **waiting for you**, läutet claude-tui einmal die
Terminal-Glocke (BEL) und zeigt unten in der Sessionliste `"<Titel>" is waiting for you`. Das
passiert nur beim Wechsel, nicht dauerhaft.
`CLAUDE_TUI_NO_BELL=1` schaltet die Glocke ab; der Hinweis bleibt. Ob und wie die Glocke zu hören
oder zu sehen ist, stellt das Terminal ein (Windows Terminal: `bellStyle`).

Den Zustand meldet Claude Code selbst über **Hooks**: claude-tui startet jede Session mit
`--settings` und eigenen Hook-Einträgen, die per HTTP an claude-tui melden (Prompt abgeschickt
oder Tool genutzt = working, Freigabedialog = waiting for you, Antwort fertig = ready). Liegt für
eine Session (noch) keine Meldung vor — etwa bei einem eigenen Befehl ohne Hook-Unterstützung —,
greift die frühere Heuristik: Ausgabe in den letzten 1,5 s = working, Rückfrage-Text auf dem
Bildschirm („Do you want to …“) = waiting for you, sonst ready. Nach einer Eingabe in eine Session,
die gerade wartet oder arbeitet, entscheidet bis zur nächsten Meldung ebenfalls die Heuristik
(z. B. nach einer abgelehnten Freigabe oder einem Abbruch mit `Esc`). Details in
[Architektur](architektur.md#hooks).
