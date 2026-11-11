# Code-Richtlinien

Es gelten die gängigen Rust-Richtlinien, ohne eigene Abweichungen.

## Formatierung

`rustfmt` mit Standardeinstellungen, keine `rustfmt.toml`. Vor jedem Commit `cargo fmt`; die CI
prüft mit `cargo fmt --check`.

## Lints

Clippy mit Standard-Lints, Warnungen sind Fehler:

```
cargo clippy --all-targets -- -D warnings
```

`#[allow(...)]` nur gezielt am betroffenen Element und mit Begründung in einer Zeile.

## Benennung

Nach den [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/naming.html):

| Element | Schreibweise |
|---|---|
| Funktionen, Methoden, Variablen, Module | `snake_case` |
| Typen, Traits, Enum-Varianten | `UpperCamelCase` |
| Konstanten, Statics | `SCREAMING_SNAKE_CASE` |

Bezeichner und Testnamen auf Englisch.

## Fehlerbehandlung

- `anyhow::Result` mit `.context(...)` an Systemgrenzen (PTY, Prozessstart, Terminal).
- Kein `unwrap()`/`expect()` außerhalb von Tests, außer bei echten Invarianten.

## Sicherheit

Kein `unsafe`.

## Tests

Unit-Tests als `#[cfg(test)] mod tests` im selben Modul. `cargo test` muss grün sein.

## Sprache

Kommentare und Doku auf Deutsch, **Oberflächentexte der App auf Englisch** (Menü, Zustände,
Hinweise, Fehlermeldungen, `--help`). Kommentiert wird nur, was am Code nicht ablesbar
ist: eine fachliche Regel, eine bewusste Abweichung, ein Workaround um fremdes Verhalten.
