# sbrs (Shell Buddy / sb) — Copilot Instructions

A terminal-based file manager TUI written in Rust. All source code lives in `src/main.rs`.

## Build & Test

```bash
cargo build            # debug build
cargo build --release  # optimized (size: opt-level=z, lto, strip)
cargo run              # run directly
cargo test             # run tests (none exist yet)
```

The release binary lands at `target/release/sbrs`.

## Architecture

Single-file design — `src/main.rs` contains everything:

- **`App` struct** — central state: `current_dir`, `entries`, `selected_index`, `marked_indices`, `clipboard`, `mode`, `show_hidden`, `input_buffer`
- **`Mode` enum** — `Browsing` | `Renaming` | `Help`
- **`refresh_entries()`** — re-reads directory; respects `show_hidden` toggle
- **Event loop** — single-threaded; `crossterm` raw-mode + ratatui render cycle
- **External commands** — `git` subprocess for branch/dirty status; `less` for file view; `$EDITOR` (fallback `nano`) for edit
- **Exit** — writes final directory to `/tmp/sb_path` for shell integration

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `ratatui` 0.26 | TUI layout and widgets |
| `crossterm` 0.27 | Raw mode, alternate screen, key events |
| `clap` 4 | CLI argument parsing (derive macros) |
| `devicons` | File-type Unicode icons (Dark theme) |
| `chrono` | Modification timestamp formatting |
| `hostname` | Prompt display |

## Conventions

- **Platform guards**: use `#[cfg(unix)]` / `#[cfg(not(unix))]` for OS-specific code (permissions parsing uses Unix inode mode bits)
- **Colors**: use `ratatui::style::Color::Rgb(r, g, b)` — never raw ANSI escape literals; the terminal must support 24-bit color
- **Silent failures**: current pattern is to ignore errors on file operations (`fs::copy`, `fs::rename`) — match this style unless adding explicit error feedback to the UI
- **No async**: all I/O is blocking; keep it that way unless refactoring the event loop

## Pitfalls

- **Raw terminal mode** must always be restored; wrap any new panic paths with `crossterm::terminal::disable_raw_mode()` + `LeaveAlternateScreen`
- **Git subprocess** runs on every status refresh — avoid calling it in tight loops or hot paths
- **Hardcoded column widths** (40 % / 12 / 8 / ≥20) can overflow on narrow terminals
- **Clipboard holds absolute `PathBuf`s** — paste always targets the current directory
