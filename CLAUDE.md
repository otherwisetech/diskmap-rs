# diskmap-rs — for agents and contributors

Same conventions as genealogy-rs.

## Layout

```
crates/scan   diskmap-scan: walk::scan → Tree (arena, children sorted by size), treemap::layout → Tiles. No UI.
crates/i18n   diskmap-i18n: embedded translation catalogs, t!/tn!/tv! macros, i18n guard tests
crates/app    `diskmap` desktop app (GPUI + gpui-component); otherwise.rs = otherwise-link (links,
              About's other apps, low-space Today feed)
```

## Rules

- **i18n is mandatory.** No user-facing string literal in `app/src` or `scan/src`; use
  `i18n::t!` / `tn!` and add the key to **every** `crates/i18n/locales/<lang>/` catalog in the same
  change. Untranslatable literals (product name, symbols) end their line with `// i18n-ok`.
  Runtime-built keys need their prefix in `DYNAMIC_PREFIXES` (`app.kind.`).
- Nothing destructive without confirmation: removal goes through the Trash (`trash` crate) after a
  native prompt. Never add a permanent delete without a second confirmation.
- The scanner never follows symlinks and counts hard links once; keep it that way.
- Every scan/layout change ships with a test (`crates/scan/tests/scan.rs`).
- otherwise-link is pinned by git tag; adding / bumping it must only add its own crates to
  Cargo.lock. Tests use temp dirs (`Root::at`, explicit `Locations`), never the real shared folder.
- Comments: sparse, only non-obvious why.
- GPUI pins: `Cargo.lock` pins the same zed / gpui-component revs as genealogy-rs. Don't
  `cargo update` them casually: APIs move.

## Validate

```
cargo test -p diskmap-i18n -p diskmap-scan -p diskmap-app
cargo clippy --workspace --all-targets
```
