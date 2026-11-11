# claude-tui

Ein Terminal-Arbeitsplatz für mehrere Claude-Code-Sessions, geschrieben in Rust. Links eine
Seitenleiste mit allen Sessions und ihrem Zustand (working, waiting for you, ready),
rechts bis zu vier Sessions in echten PTYs. Die Idee stammt von
[herdr](https://github.com/ogulcancelik/herdr); Code ist nicht übernommen.

```
╭ ✻ claude-tui ───────╮╭ 1 · claude 1 ──────╮╭ 2 · claude 3 ──────╮
│▌claude 1 [1]        ││                    ││                    │
│  ● working          ││                    │╰────────────────────╯
│ claude 2            ││                    │╭ 3 · claude 4 ──────╮
│  ● waiting for you  ││                    ││                    │
│ claude 3 [2]        ││                    ││                    │
│  ● ready            ││                    ││                    │
│ Ctrl+A Menu         ││                    ││                    │
╰─────────────────────╯╰────────────────────╯╰────────────────────╯
```

Bis zu 4 Fenster nebeneinander, Fokuswechsel mit `Alt+Pfeil`, alle Befehle im `Ctrl+A`-Menü unten in
der Sessionliste.

## Aufbau

| Ordner | Inhalt |
|---|---|
| `src/` | Rust-Quellcode; `Cargo.toml` liegt im Wurzelordner |
| `docs/` | Dokumentation: [Bedienung](docs/bedienung.md), [Architektur](docs/architektur.md), [Build & Release](docs/release.md), [Code-Richtlinien](docs/code-richtlinien.md) |

## Schnellstart

```
cargo run --release    # bauen und starten, jede Session startet `claude`
```

Die gebaute App liegt danach unter `target/release/claude-tui`.

Fertige Archive für Windows, Linux und macOS gibt es unter **Releases**.
