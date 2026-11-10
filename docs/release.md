# Build & Release

## Lokal

Voraussetzung: Rust über [rustup](https://rustup.rs) (unter Windows zusätzlich die
„Desktopentwicklung mit C++“-Build-Tools von Visual Studio).

```powershell
./build.ps1     # Windows: source\target\release\claude-tui.exe → output\
./build.sh      # Linux/macOS
```

`output/` ist im Git ignoriert, nur `.gitkeep` liegt drin.

## Branches

| Branch | Zweck |
|---|---|
| `develop` | Hauptbranch, hier wird entwickelt |
| `release` | Jeder Push hierauf erzeugt ein Release |

## GitHub Actions

| Workflow | Auslöser | Was passiert |
|---|---|---|
| `ci.yml` | Push auf `develop`, Pull Request | `cargo fmt --check`, `clippy -D warnings`, `cargo test` auf Windows, Linux, macOS |
| `release.yml` | Push auf `release` (oder manuell) | Version aus `source/Cargo.toml` lesen, Build für 5 Ziele, GitHub-Release mit Tag `v<Version>` und den Binaries |

Ziele: `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`,
`aarch64-apple-darwin`, `x86_64-apple-darwin`.

## Release erstellen

1. Auf `develop` die Version in `source/Cargo.toml` erhöhen und committen.
2. `develop` in `release` mergen und pushen:

   ```
   git checkout release
   git merge --ff-only develop
   git push origin release
   ```

3. Der Workflow legt Tag und Release mit automatisch erzeugten Release-Notes an. Gibt es den Tag
   schon (Version vergessen zu erhöhen), bricht er vor dem Build mit einer Fehlermeldung ab.

`source/Cargo.lock` ist eingecheckt, und alle Workflows bauen mit `--locked` — Releases enthalten
damit exakt die getesteten Abhängigkeitsversionen. Abhängigkeiten aktualisieren: `cargo update` in
`source/`, danach den geänderten `Cargo.lock` mit committen.
