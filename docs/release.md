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

Released wird die Version, die gerade in `develop` steht; erst danach wird sie für die nächste
Runde erhöht.

1. `develop` in `release` mergen und pushen:

   ```
   git checkout release
   git merge --ff-only develop
   git push origin release
   ```

   Der Workflow legt Tag `v<Version>` und Release mit automatisch erzeugten Release-Notes an.
2. Zurück auf `develop` die Version in `Cargo.toml` erhöhen (z. B. `0.1.0` → `0.2.0`), `cargo build`
   aktualisiert `Cargo.lock`, beides committen und pushen.

Wird die Erhöhung vergessen, bricht der nächste Release-Lauf vor dem Build ab, weil es den Tag
schon gibt. Weil `release` nur per Fast-Forward nachgezogen wird, enthält er nie eigene Commits.

`Cargo.lock` ist eingecheckt, und alle Workflows bauen mit `--locked` — Releases enthalten
damit exakt die getesteten Abhängigkeitsversionen. Abhängigkeiten aktualisieren: `cargo update`,
danach den geänderten `Cargo.lock` mit committen.
