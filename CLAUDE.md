# diskmap-rs — for agents and contributors

Same conventions as genealogy-rs.

## Layout

```
crates/scan   diskmap-scan: walk::scan → Tree (arena, children sorted by size), treemap::layout → Tiles;
              dev::find → build folders by project + home caches, dev::delete_with;
              safety::protected = paths never offered for deletion; snapshot + mcp = the AI
              panel's data and `diskmap mcp` server. No UI.
crates/i18n   diskmap-i18n: embedded translation catalogs, t!/tn!/tv! macros, i18n guard tests
crates/app    `diskmap` desktop app (GPUI + gpui-component); otherwise.rs = otherwise-link (links,
              About's other apps, low-space Today feed); storage.rs = `<data root>/diskmap/` +
              one-time migration from the old config folder; vcity.rs = Vcity invitation;
              dev.rs = Developer projects view (selection, two-step cleanup, Open in rama);
              agent.rs = AI panel (otherwise-agent-panel, ⌘J): snapshot, cards, Trash
```

## Rules

- **i18n is mandatory.** No user-facing string literal in `app/src` or `scan/src`; use
  `i18n::t!` / `tn!` and add the key to **every** `crates/i18n/locales/<lang>/` catalog in the same
  change. Untranslatable literals (product name, symbols) end their line with `// i18n-ok`.
  Runtime-built keys need their prefix in `DYNAMIC_PREFIXES` (`app.kind.`, `app.dev.kind.`).
- Nothing destructive without confirmation: removal goes through the Trash (`trash` crate) after a
  native prompt. No permanent delete anywhere. Developer projects' cleanup lists every path and
  size in-app, then a native prompt, then `scan::dev::delete_with` (the app passes
  `trash::delete`) re-checks protection, kind, symlinks, containment in the project (or the exact
  home cache path) and, in git repositories, that git ignores everything in it. Tests pass a
  plain remove on temp dirs; never the real home or Trash.
- **Never offer tool configs, installs or user data for deletion.** Anything that proposes a path
  to delete — the Dev view, and any AI assistant / MCP tool added later — must only propose
  `dev::find` results and must check `scan::safety::protected` (hidden home folders, `~/Library`,
  cloud folders, bundles, system, whole user folders, project roots). Nothing is pre-selected.
  New exceptions go in `safety::HOME_CACHE_ALLOW` with a row in `crates/scan/tests/safety.rs`.
- **AI assistant** (otherwise-agent-panel, ⌘J): `diskmap mcp --snapshot <file>` (the app binary,
  `scan/mcp.rs`) reads only the snapshot the window writes into its private session folder
  (`agent::make_session_dir`, 0700) once a session asked for it: the pruned scan and the Dev
  projects report with git state; it never writes or deletes anything. `--propose --inbox <dir>` adds
  `propose_trash`, which accepts only `cleanup_candidates` (report entries `safety::protected`
  allows); the window shows an Aplicar / Rechazar card listing every folder, re-checks with
  `dev::delete_with` and moves to the Trash. Agent-facing text is English with `// i18n-ok`;
  tests use synthetic snapshots in temp dirs (`crates/scan/tests/mcp.rs`).
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
