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
- Rich previews via optional tools (`bat`, `glow`, `jnv`, `csvlens`, `chafa`, `viu`, `sox`, `pdftotext`, `asciinema`)
- Side-by-side file compare with `delta`
- SSH/rclone/local-media mount picker
- Age file protection/decryption (`.age`) with `p`
- Clipboard full-path copy via `Ctrl+c` (`wl-copy`/`xclip`/`xsel`/`pbcopy`)
- Integration manager (`i`) to enable/disable optional integrations
- Tabbed Help/Bookmarks/Remote Mounts/Integrations overlays (`Tab` / `Shift+Tab`)
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

## Installation

### From Source

```bash
cargo install --path .
```

### From Releases

Prebuilt binaries and the auto-installer script are available in GitHub Releases.
Use the installer there if you want the fastest setup without building from source.

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
- `m`: move (cut+paste behavior) from internal clipboard
- `d`: delete (with confirmation)
- `x`: toggle executable bit on selected file(s)
- `p`: protect/unprotect file with `age` (`.age`)
- `F2` or `r`: rename (or bulk rename with `vidir` when multiple are marked)
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
- `Tab` (in browsing): edit current path inline
- `Tab` / `Shift+Tab` in Help/Bookmarks/Remote Mounts/Integrations: cycle tabs forward/backward
- `s`: toggle folder size calculation in listing
- `Ctrl+s`: open sort mode menu
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
- Viewers/previews: `bat`, `glow`, `jnv`, `csvlens`, `hexyl`, `chafa`, `viu`, `sox`, `pdftotext`, `asciinema`
- Diff/edit helpers: `delta`, `hexedit`, `vidir`
- Archives: `zip`/`unzip`, `tar`, `7z` family (`7z`/`7zz`/`7zr`), `rar`/`unrar`, `fuse-zip`, `archivemount`
- Security: `age`
- Remote mounts: `sshfs`, `rclone`
- Search: `rg`, `fzf`
- Clipboard backends: `wl-copy`, `xclip`, `xsel`, `pbcopy`

Remote picker (`S`) also lists existing local mounted folders discovered under:

- `/media/$USER`
- `/run/media/$USER`
- `/mnt`
- `/run/user/$UID/gvfs`

If an optional tool is not available, the feature is skipped or falls back gracefully.

## Environment Notes

- `NERD_FONT_ACTIVE=1`: enable Nerd Font icons
- `NO_COLOR=1`: disable file name colors (modifiers like bold/dim still apply)
- `TERMINAL_ICONS=0`: hide all file icons (Nerd Font glyphs and emoji)
- `EDITOR`: editor command used by `e`/`F4`
- `SB_BOOKMARK_0` ... `SB_BOOKMARK_9`: bookmark directories

## Shell Integration

To enable automatic directory change on exit, add the following function to your shell configuration file (e.g., `~/.bashrc`, `~/.zshrc`):

```bash
sb() {
    "$HOME/.cargo/bin/sbrs" "$@"
    if [ -f /tmp/sb_path ]
    then
        cd "$(cat /tmp/sb_path)"
        rm -i -f /tmp/sb_path
    fi
}
```

After adding the function, reload your shell configuration:

```bash
source ~/.bashrc  # or source ~/.zshrc
```

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
