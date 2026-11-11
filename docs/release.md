# Build & Release

## Lokal

Voraussetzung: Rust über [rustup](https://rustup.rs) (unter Windows zusätzlich die
„Desktopentwicklung mit C++“-Build-Tools von Visual Studio).

```
cargo run --release        # bauen und starten
cargo build --release      # nur bauen → target/release/claude-tui(.exe)
```

Die fertige Binary bleibt, wie in Rust üblich, in `target/release/`. `target/` ist der Build-Cache
von Cargo, im Git ignoriert und jederzeit mit `cargo clean` löschbar.

## Branches

| Branch | Zweck |
|---|---|
| `develop` | Hauptbranch, hier wird entwickelt |
| `release` | Jeder Push hierauf erzeugt ein Release |

## GitHub Actions

| Workflow | Auslöser | Was passiert |
|---|---|---|
| `ci.yml` | Push auf `develop`, Pull Request | `cargo fmt --check`, `clippy -D warnings`, `cargo test` auf Windows, Linux, macOS |
| `release.yml` | Push auf `release` (oder manuell) | Version aus `Cargo.toml` lesen, Build für 5 Ziele, GitHub-Release mit Tag `v<Version>`, Archiven und Prüfsummen |

Ziele: `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`,
`aarch64-apple-darwin`, `x86_64-apple-darwin`.

## Release-Dateien

Je Ziel ein Archiv nach dem Schema `claude-tui-v<Version>-<Ziel>`, darin die Binary und die
`README.md`:

| Plattform | Format |
|---|---|
| Windows | `.zip` |
| Linux, macOS | `.tar.gz` |

Dazu `sha256.sum` mit den SHA-256-Prüfsummen aller Archive (prüfen mit `sha256sum -c sha256.sum`).

## Release erstellen

1. Auf `develop` die Version in `Cargo.toml` erhöhen und committen.
2. `develop` in `release` mergen und pushen:

   ```
   git checkout release
   git merge --ff-only develop
   git push origin release
   ```

3. Der Workflow legt Tag und Release mit automatisch erzeugten Release-Notes an. Gibt es den Tag
   schon (Version vergessen zu erhöhen), bricht er vor dem Build mit einer Fehlermeldung ab.

`Cargo.lock` ist eingecheckt, und alle Workflows bauen mit `--locked` — Releases enthalten
damit exakt die getesteten Abhängigkeitsversionen. Abhängigkeiten aktualisieren: `cargo update`,
danach den geänderten `Cargo.lock` mit committen.
