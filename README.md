# diskmap

<img src="crates/app/assets/logo/diskmap-icon-512.png" width="128" alt="diskmap icon">

See what fills your disk. diskmap scans a folder in parallel and draws it as a nested,
squarified treemap: every rectangle is as big as the space it takes on disk, coloured by kind
(video, images, archives, code…). Walk into folders, find the biggest files, reveal them in the
file manager or move them to the Trash.

Native desktop app in Rust with [GPUI](https://www.gpui.rs/) and
[gpui-component](https://github.com/longbridge/gpui-component). macOS, Windows and Linux.
UI in Spanish, English, Portuguese and French.

## Features

- Parallel scan with live counters; cancel anytime.
- Real disk usage (allocated blocks), hard links counted once, symlinks never followed.
- macOS aware: skips firmlinked `/System/Volumes` (no double counting when scanning `/`),
  treats `.app`, `.photoslibrary` and other bundles as one item you can still open.
- Click to select, double-click / Enter to go in, Backspace to go up, breadcrumb to jump.
- Side panel: size, share of view and of the whole scan, contents, largest files, legend.
- Move to Trash (asks first, recoverable) and Reveal in Finder / Explorer.

## Download

Ready-made builds for macOS (Apple silicon and Intel), Linux and Windows are attached to each
[GitHub Release](https://github.com/otherwisetech/diskmap-rs/releases).

- **macOS:** unzip and drag `diskmap.app` to Applications. The app is not notarized yet, so the
  first time right-click it → *Open* (or run `xattr -dr com.apple.quarantine /Applications/diskmap.app`).
- **Linux:** extract the `.tar.gz` and run `./diskmap`; copy `diskmap.desktop` and the icon to
  `~/.local/share/applications` / `~/.local/share/icons` for a menu entry. Needs Vulkan.
- **Windows:** unzip and run `diskmap.exe` (SmartScreen may ask to confirm, since it is unsigned).

The downloads are built by GitHub Actions from the tagged source, but they are **not code-signed
or notarized** (that needs paid certificates), which is why macOS and Windows ask for confirmation
the first time. diskmap makes no network requests.

## Build and run

```sh
cargo run -p diskmap-app -- ~/Downloads     # or no argument for the start screen
script/bundle-macos.sh                       # macOS: target/release/diskmap.app with the icon
cargo run --release -p diskmap-scan --example du -- ~   # scanner only, prints top entries
```

On macOS, grant **Full Disk Access** (System Settings → Privacy & Security) to see everything.

## Layout

```
crates/scan   diskmap-scan: parallel walker, Tree arena, squarified treemap layout — no UI
crates/i18n   diskmap-i18n: embedded catalogs (es, en, pt, fr) and i18n guard tests
crates/app    `diskmap` desktop app
```

## License

MIT or Apache-2.0, at your option.
