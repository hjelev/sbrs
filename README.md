# Shell Buddy (sbrs)

A terminal file manager (TUI) written in Rust using `ratatui` + `crossterm`.

`sbrs` (Shell Buddy, or `sb` for short) is a keyboard-driven explorer focused on fast local navigation with optional integrations for previews, archive handling, searching, and remote mounts.

## Highlights

- Single-binary terminal UI
- Directory navigation with marked multi-select
- Copy/paste with progress and status feedback
- Rename (single and bulk with `vidir`)
- Delete with confirmation dialog
- Archive create/extract workflows (`zip`, `tar`, `7z`, `rar` toolchain)
- Optional archive-as-folder mount for zip-based files (`fuse-zip`)
- Rich previews via optional tools (`bat`, `glow`, `jnv`, `csvlens`, `chafa`, `sox`)
- Side-by-side file compare with `delta`
- SSH/rclone remote mount picker
- Clipboard full-path copy via `Ctrl+c` (`wl-copy`/`xclip`/`xsel`/`pbcopy`)
- Integration manager (`i`) to enable/disable optional integrations
- Writes last directory to `/tmp/sb_path` on exit for shell integration

## Build and Run

```bash
cargo build
cargo run
```

Release build:

```bash
cargo build --release
```

Release binary path:

```text
target/release/sbrs
```

## Core Controls

- `q` / `Esc`: quit
- `Enter` / `Right`: open entry / preview file
- `Left` / `Backspace`: go to parent / leave mounted view
- `Up`/`Down`/`PageUp`/`PageDown`/`Home`/`End`: navigation
- `Space`: mark/unmark current entry
- `*`: toggle all marks
- `c` or `F5`: copy to internal clipboard
- `Ctrl+c`: copy selected full path(s) to system clipboard
- `v`: paste
- `d`: delete (with confirmation)
- `x`: toggle executable bit on selected file(s)
- `F2`: rename (or bulk rename with `vidir` when multiple are marked)
- `e` or `F4`: open in `$EDITOR` (or `hexedit` for binary if available)
- `n`: new file
- `N`: new folder
- `Z`: archive create/extract flow
- `C`: compare marked file vs cursor file with `delta`
- `o`: open with system GUI opener (`xdg-open`/`gio open`)
- `g`: content search (`rg`, optional `fzf` handoff)
- `f`: fuzzy file search (`fzf`)
- `S`: SSH/rclone remote picker
- `i`: integrations panel
- `b`: bookmarks panel
- `0-9`: jump to bookmark (`SB_BOOKMARK_0..9`)
- `.`: toggle hidden files
- `~`: jump to home
- `h`: help overlay

## Integrations

Required behavior:

- `less`: file viewing fallback
- `$EDITOR`: file editing command (defaults to `nano` if unset)

Optional integrations (auto-detected, toggle in `i` panel):

- VCS: `git`
- Viewers/previews: `bat`, `glow`, `jnv`, `csvlens`, `hexyl`, `chafa`, `sox`
- Diff/edit helpers: `delta`, `hexedit`, `vidir`
- Archives: `zip`/`unzip`, `tar`, `7z` family (`7z`/`7zz`/`7zr`), `rar`/`unrar`, `fuse-zip`
- Remote mounts: `sshfs`, `rclone`
- Search: `rg`, `fzf`
- Clipboard backends: `wl-copy`, `xclip`, `xsel`, `pbcopy`

If an optional tool is not available, the feature is skipped or falls back gracefully.

## Environment Notes

- `NERD_FONT_ACTIVE=1`: enable Nerd Font icons
- `EDITOR`: editor command used by `e`/`F4`
- `SB_BOOKMARK_0` ... `SB_BOOKMARK_9`: bookmark directories

## Shell Integration Idea

On exit, `sbrs` writes its final directory to:

```text
/tmp/sb_path
```

You can use that file in a wrapper function to `cd` your shell after quitting the app.

## Project Structure

Current code layout is intentionally simple:

- `src/main.rs`: all application logic, UI rendering, and event handling
- `Cargo.toml`: dependencies and release profile settings

## Dependencies

From `Cargo.toml`:

- `ratatui` (UI)
- `crossterm` (terminal events/raw mode)
- `chrono` (timestamps)
- `devicons` (file icons)
- `hostname` (header prompt)
- `users` (owner metadata)
- `clap` (present as dependency)
