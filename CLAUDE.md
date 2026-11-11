# claude-tui

Rust-TUI, die mehrere Claude-Code-Sessions in PTYs mit Status-Seitenleiste verwaltet (Idee: herdr).

- Cargo-Projekt im Wurzelordner (Code in `src/`), Benutzer-/Entwickler-Doku in `docs/`, lokale Builds in `output/`
  (ignoriert).
- Branches: `develop` ist der Hauptbranch; ein Push auf `release` baut über
  `.github/workflows/release.yml` das Release `v<Version aus Cargo.toml>`.
- CI verlangt `cargo fmt --check` und
  `cargo clippy --all-targets -- -D warnings` sauber.
- **Code-Richtlinien:** [docs/code-richtlinien.md](docs/code-richtlinien.md) — vor jeder Code-Änderung
  beachten.
- `docs/` beschreibt das Projekt für Nutzer und Mitwirkende (liegt mit auf GitHub). Hintergrundwissen
  und Entscheidungen gehören ins Brain: `S:\.brain\wiki\private\architektur\claude-tui.md`.
