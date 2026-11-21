# diskmap-rs — for agents and contributors

Same conventions as genealogy-rs.

## Layout

```
crates/scan   diskmap-scan: walk::scan → Tree (arena, children sorted by size), treemap::layout → Tiles;
              dev::find → build folders by project + home caches, dev::delete. No UI.
crates/i18n   diskmap-i18n: embedded translation catalogs, t!/tn!/tv! macros, i18n guard tests
crates/app    `diskmap` desktop app (GPUI + gpui-component); otherwise.rs = otherwise-link (links,
              About's other apps, low-space Today feed); storage.rs = `<data root>/diskmap/` +
              one-time migration from the old config folder; vcity.rs = Vcity invitation;
              dev.rs = Developer projects view (selection, two-step cleanup, Open in rama)
```

## Rules

- **i18n is mandatory.** No user-facing string literal in `app/src` or `scan/src`; use
  `i18n::t!` / `tn!` and add the key to **every** `crates/i18n/locales/<lang>/` catalog in the same
  change. Untranslatable literals (product name, symbols) end their line with `// i18n-ok`.
  Runtime-built keys need their prefix in `DYNAMIC_PREFIXES` (`app.kind.`, `app.dev.kind.`).
- Nothing destructive without confirmation: removal goes through the Trash (`trash` crate) after a
  native prompt. Never add a permanent delete without a second confirmation. The one permanent
  delete is Developer projects' cleanup of regenerable build folders / caches: an in-app list of
  every path and size first, then a native prompt; `scan::dev::delete` re-checks kind, symlinks,
  containment in the project (or the exact home cache path) and, in git repositories, that git
  ignores everything in it. Tests use temp dirs only; never run it on a real disk while developing.
- The scanner never follows symlinks and counts hard links once; keep it that way.
- Every scan/layout change ships with a test (`crates/scan/tests/scan.rs`, `tests/dev.rs`).
- rama: "Open in rama" uses `otherwise://rama/repo?path=…`; rama's worktrees come from
  `<data root>/rama/worktrees.txt` (read only). The low-space Today alert links to
  `otherwise://diskmap/dev` when the saved `dev-summary.json` is ≥ 10 % of the volume's used space.
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
