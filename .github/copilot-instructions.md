# sbrs (Shell Buddy / sb) — Copilot Instructions

A terminal-based file manager TUI written in Rust.

## Current Code Layout (Modular)

The codebase is no longer single-file. `src/main.rs` is now the orchestrator and core runtime host, with focused modules for integrations, UI helpers, formatting, and `App` helper impl blocks.

### Source Tree Ownership

- `src/main.rs`
	- Owns core state structs (`App`, runtime state), event loop, navigation/file operations, and top-level flow.
	- Keep this file focused on orchestration and behavior wiring.
- `src/app_input.rs`
	- Input-edit helpers (`input_buffer`, cursor movement/editing, selection delta helpers).
- `src/app_meta.rs`
	- Metadata and identity helpers (permissions/owner/group parsing, UID/GID caches, size format adapter).
- `src/app_render_cache.rs`
	- Entry render-cache generation and metadata-column width refresh helpers.
- `src/app_search.rs`
	- Internal search subsystem (candidate scan, fuzzy/regex matching, content search async pipeline, search scope and limits state transitions).
- `src/app_files.rs`
	- File/archive classification helpers and extension/signature checks (archive kind detection, media/document type checks, binary detection, age temp path helpers).
- `src/app_sizes.rs`
	- Folder-size/background scan helpers and aggregate size computation (folder scan workers, selected/current-dir total-size pipelines, free-space probe, recursive size walkers).
- `src/app_git.rs`
	- Git-status/background cache helpers (async git info request/pump, cached git header metadata resolution for branch/dirty/tag-ahead display).
- `src/app_archive.rs`
	- Archive mount/preview lifecycle helpers (mount path creation, mount/unmount flows, leave/cleanup mounted archive state, archive listing preview command flow).
- `src/integration/`
	- `app.rs`: `impl App` integration control flow (integration enable/active state, install prompt/confirm, brew guidance, integration row cache refresh, archive extraction gating by available tools).
	- `catalog.rs`: integration definitions and package mapping.
	- `probe.rs`: runtime tool detection/probing.
	- `rows.rs`: integration row model and row-building logic.
- `src/ui/`
	- `cli.rs`: `-l`/`-la` list-mode rendering and CLI help/version output.
	- `icons.rs`: directory icon mapping.
	- `panels.rs`: panel/tab bar line builders.
	- `search.rs`: highlighted search span generation.
	- `status.rs`: status message icon classification and footer decoration.
- `src/util/`
	- `format.rs`: shared formatting helpers (`format_size`, `format_eta`).

## Build & Test

```bash
cargo build            # debug build
cargo build --release  # optimized (size: opt-level=z, lto, strip)
cargo run              # run directly
cargo test             # run tests (none exist yet)
```

The release binary lands at `target/release/sbrs`.

## Architecture

Modular design with `App`-centric impl blocks:

- **`App` struct** — central state: `current_dir`, `entries`, `selected_index`, `marked_indices`, `clipboard`, `mode`, `show_hidden`, `input_buffer`
- **`AppMode` enum** — primary app mode switch for browse/edit/help/search flows
- **`refresh_entries()`** — re-reads directory; respects `show_hidden` toggle
- **Event loop** — single-threaded; `crossterm` raw-mode + ratatui render cycle
- **External commands** — `git` subprocess for branch/dirty status; `less` for file view; `$EDITOR` (fallback `nano`) for edit
- **Exit** — writes final directory to `/tmp/sb_path` for shell integration

## Copilot Routing Rules (Required)

When adding or modifying behavior, place new code in the most specific module instead of growing `src/main.rs`.

- New input editing behavior:
	- Extend `src/app_input.rs`.
- New permission/owner/group or identity-cache logic:
	- Extend `src/app_meta.rs`.
- New entry row rendering/cache fields and derivation:
	- Extend `src/app_render_cache.rs`.
- New internal search behavior (filename/content search, regex/literal matching, search limit handling):
	- Extend `src/app_search.rs`.
- New file/archive classification behavior (archive kind checks, binary/type detection, age temp path helpers):
	- Extend `src/app_files.rs`.
- New folder-size/background scan behavior or size-walk calculations:
	- Extend `src/app_sizes.rs`.
- New git header/status cache behavior or git-info background refresh logic:
	- Extend `src/app_git.rs`.
- New archive mount/preview/unmount behavior:
	- Extend `src/app_archive.rs`.
- New integration definitions or package mappings:
	- Extend `src/integration/catalog.rs`.
- New command/tool detection logic:
	- Extend `src/integration/probe.rs`.
- New integration list state-row composition:
	- Extend `src/integration/rows.rs`.
- New integration state/installation flow or integration-aware archive/tool gating behavior:
	- Extend `src/integration/app.rs`.
- New non-TUI CLI output/list-mode behavior:
	- Extend `src/ui/cli.rs`.
- New icon dictionaries:
	- Extend `src/ui/icons.rs`.
- New panel/tab line builders:
	- Extend `src/ui/panels.rs`.
- New search highlighting span logic:
	- Extend `src/ui/search.rs`.
- New status-message icon/decorating policy:
	- Extend `src/ui/status.rs`.
- New generic formatting helpers:
	- Extend `src/util/format.rs`.

Only keep code in `src/main.rs` if it is orchestration glue, central state wiring, or logic that is still awaiting extraction.

## Refactor Direction

- Continue shrinking `src/main.rs` in coherent chunks.
- Prefer extracting complete `impl App` method groups by concern (input, metadata, search, rendering, actions).
- Preserve behavior while refactoring; compile after each extraction step.

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
- **Module boundaries**: avoid cross-module duplication; add shared logic in `src/util` or `src/integration`/`src/ui` submodules
- **Visibility discipline**: default to `pub(crate)` for extracted `impl App` methods unless external/public API is required

## Pitfalls

- **Raw terminal mode** must always be restored; wrap any new panic paths with `crossterm::terminal::disable_raw_mode()` + `LeaveAlternateScreen`
- **Git subprocess** runs on every status refresh — avoid calling it in tight loops or hot paths
- **Hardcoded column widths** (40 % / 12 / 8 / ≥20) can overflow on narrow terminals
- **Clipboard holds absolute `PathBuf`s** — paste always targets the current directory
