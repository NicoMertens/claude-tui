# claude-tui

Ein Terminal-Arbeitsplatz für mehrere Claude-Code-Sessions, geschrieben in Rust. Links eine
Seitenleiste mit allen Sessions und ihrem Zustand (arbeitet, wartet auf dich, bereit, beendet),
rechts die aktive Session in einem echten PTY. Die Idee stammt von
[herdr](https://github.com/ogulcancelik/herdr); Code ist nicht übernommen.

```
╭ ✻ claude-tui ──────────╮╭ claude 1 ─────────────────────────────╮
│▌1 claude 1             ││                                       │
│   ● arbeitet           ││  (Claude Code läuft hier)             │
│ 2 claude 2             ││                                       │
│   ● wartet auf dich    ││                                       │
╰────────────────────────╯╰───────────────────────────────────────╯
 Ctrl+A dann: c neu · x schließen · n/p wechseln · 1–9 wählen · q beenden
```

## Aufbau

| Ordner | Inhalt |
|---|---|
| `source/` | Rust-Quellcode (Cargo-Projekt) |
| `docs/` | Dokumentation: [Bedienung](docs/bedienung.md), [Architektur](docs/architektur.md), [Build & Release](docs/release.md), [Code-Richtlinien](docs/code-richtlinien.md) |
| `build/` | Build-Skripte für lokale Builds |
| `output/` | Lokal gebaute App (`build/build.ps1` / `build/build.sh`), nicht im Git |

## Schnellstart

```powershell
./build/build.ps1      # baut und kopiert nach output\
./output/claude-tui    # startet mit `claude` in jeder Session
```

Fertige Binaries für Windows, Linux und macOS gibt es unter **Releases**.
