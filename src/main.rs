use crossterm::{
    cursor::{Hide, MoveTo, SetCursorStyle, Show},
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear as TermClear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use chrono::Local;
use regex::Regex;
use ratatui::{prelude::*, widgets::*};
use ratatui::widgets::BorderType;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    fs,
    io::{self, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use unicode_width::UnicodeWidthStr;

mod integration;
mod app_archive;
mod app_git;
mod app_input;
mod app_files;
mod app_meta;
mod app_render_cache;
mod app_search;
mod app_sizes;
mod ui;
mod util;

use integration::rows::IntegrationRow;

struct ArchiveMount {
    archive_path: PathBuf,
    mount_path: PathBuf,
    return_dir: PathBuf,
    archive_name: String,
}

#[derive(Clone)]
struct SshHost {
    alias: String,
    hostname: String,
    user: Option<String>,
    port: Option<u16>,
    identity_file: Option<String>,
}

#[derive(Clone)]
enum RemoteEntry {
    Ssh(SshHost),
    Rclone { name: String, rtype: String },
    ArchiveMount { archive_name: String, mount_path: PathBuf },
    LocalMount { name: String, mount_path: PathBuf, source: String },
}

impl RemoteEntry {
    fn alias(&self) -> &str {
        match self {
            RemoteEntry::Ssh(h) => &h.alias,
            RemoteEntry::Rclone { name, .. } => name,
            RemoteEntry::ArchiveMount { archive_name, .. } => archive_name,
            RemoteEntry::LocalMount { name, .. } => name,
        }
    }
}

struct SshMount {
    _host_alias: String,
    mount_path: PathBuf,
    return_dir: PathBuf,
    remote_label: String,
    remote_root: String,
}

struct GitInfoCache {
    path: PathBuf,
    info: Option<(String, bool, Option<(String, u64)>)>,
}

pub(crate) use app_render_cache::{EntryRenderCache, EntryRenderConfig};

enum CopyProgressMsg {
    TotalBytes(u64),
    CopiedBytes(u64),
    Finished(Result<(), String>),
}

enum ArchiveProgressMsg {
    TotalBytes(u64),
    Progress(u64),
    Finished(Result<String, String>),
}

enum FolderSizeMsg {
    EntrySize(u64, PathBuf, u64),
    Finished(u64),
}

enum SelectedTotalSizeMsg {
    Finished(u64, u64),
}

enum CurrentDirTotalSizeMsg {
    Finished(u64, u64),
}

enum RecursiveMtimeMsg {
    EntryMtime(u64, PathBuf, u64),
    Finished(u64),
}

enum NotesLoadMsg {
    Finished(u64, PathBuf, HashMap<String, String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    Zip,
    Tar,
    SevenZip,
    Rar,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortMode {
    NameAsc,
    NameDesc,
    ExtensionAsc,
    SizeAsc,
    SizeDesc,
    ModifiedNewest,
    ModifiedOldest,
}

#[derive(Clone)]
pub(crate) enum PathFilterMode {
    Prefix,
    Suffix,
    Contains,
}

#[derive(Clone)]
pub(crate) struct PathInputFilter {
    pub(crate) mode: PathFilterMode,
    pub(crate) pattern: String,
}

impl SortMode {
    fn label(self) -> &'static str {
        match self {
            SortMode::NameAsc => "Name (A-Z)",
            SortMode::NameDesc => "Name (Z-A)",
            SortMode::ExtensionAsc => "Extension (A-Z)",
            SortMode::SizeAsc => "Size (Small-Large)",
            SortMode::SizeDesc => "Size (Large-Small)",
            SortMode::ModifiedNewest => "Modified (Newest)",
            SortMode::ModifiedOldest => "Modified (Oldest)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Browsing,
    PathEditing,
    DbPreview,
    CommandInput,
    GitCommitMessage,
    GitTagInput,
    InternalSearch,
    NoteEditing,
    Renaming,
    PasteRenaming,
    NewFile,
    NewFolder,
    ArchiveCreate,
    ConfirmExtract,
    ConfirmIntegrationInstall,
    Help,
    ConfirmDelete,
    Bookmarks,
    Integrations,
    SortMenu,
    SshPicker,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InternalSearchScope {
    Filename,
    Content,
}

enum InternalSearchResult {
    Filename {
        rel_path: PathBuf,
        match_ranges: Vec<(usize, usize)>,
    },
    Content {
        rel_path: PathBuf,
        line_number: usize,
        line_text: String,
        match_ranges: Vec<(usize, usize)>,
    },
}

#[derive(Clone, Copy)]
struct InternalSearchContentLimits {
    max_files: usize,
    max_hits: usize,
    max_file_bytes: usize,
}

enum InternalSearchPattern {
    Regex {
        pattern: String,
        case_insensitive: bool,
    },
    Literal(String),
}

enum InternalSearchContentMsg {
    Finished {
        request_id: u64,
        results: Vec<InternalSearchResult>,
        limit_note: Option<String>,
    },
}

enum InternalSearchCandidatesMsg {
    Finished {
        scan_id: u64,
        candidates: Vec<PathBuf>,
        truncated: bool,
    },
}

struct App {
    current_dir: PathBuf,
    entries: Vec<fs::DirEntry>,
    entry_render_cache: Vec<EntryRenderCache>,
    selected_index: usize,
    marked_indices: HashSet<usize>,
    directory_selection: HashMap<PathBuf, usize>,
    archive_mounts: Vec<ArchiveMount>,
    mode: AppMode,
    table_state: TableState,
    show_hidden: bool,
    clipboard: Vec<PathBuf>,
    paste_queue: VecDeque<PathBuf>,
    paste_current_src: Option<PathBuf>,
    paste_move_mode: bool,
    paste_target_dir: Option<PathBuf>,
    path_input_filter: Option<PathInputFilter>,
    input_buffer: String,
    input_cursor: usize,
    status_message: String,
    page_size: usize,
    ssh_mounts: Vec<SshMount>,
    remote_entries: Vec<RemoteEntry>,
    ssh_picker_selection: usize,
    copy_rx: Option<Receiver<CopyProgressMsg>>,
    copy_total_rx: Option<Receiver<u64>>,
    copy_total_bytes: u64,
    copy_done_bytes: u64,
    copy_job_total_bytes: u64,
    copy_done_before_job: u64,
    copy_started_at: Option<Instant>,
    copy_item_name: String,
    copy_current_src: Option<PathBuf>,
    copy_from_remote: bool,
    paste_total_items: usize,
    paste_ok_items: usize,
    paste_failed_items: usize,
    archive_create_targets: Vec<PathBuf>,
    archive_extract_targets: Vec<PathBuf>,
    archive_rx: Option<Receiver<ArchiveProgressMsg>>,
    archive_total_bytes: u64,
    archive_done_bytes: u64,
    archive_started_at: Option<Instant>,
    archive_name: String,
    nerd_font_active: bool,
    no_color: bool,
    show_icons: bool,
    integration_selected: usize,
    bookmark_selected: usize,
    integration_overrides: HashMap<String, bool>,
    integration_rows_cache: Vec<IntegrationRow>,
    integration_install_key: Option<String>,
    integration_install_package: Option<String>,
    integration_install_brew_path: Option<String>,
    help_scroll_offset: u16,
    help_max_offset: u16,
    confirm_delete_scroll_offset: u16,
    confirm_delete_max_offset: u16,
    confirm_delete_button_focus: u8,
    git_info_cache: Option<GitInfoCache>,
    git_info_rx: Option<Receiver<(PathBuf, Option<(String, bool, Option<(String, u64)>)>)>>,
    folder_size_enabled: bool,
    folder_size_cache: HashMap<PathBuf, u64>,
    folder_size_rx: Option<Receiver<FolderSizeMsg>>,
    folder_size_scan_id: u64,
    tree_expansion_levels: HashMap<PathBuf, usize>,
    tree_last_tap: Option<(char, Instant)>,
    tree_row_prefixes: Vec<String>,
    current_dir_total_size_rx: Option<Receiver<CurrentDirTotalSizeMsg>>,
    current_dir_total_size_scan_id: u64,
    current_dir_total_size_pending: bool,
    current_dir_total_size_bytes: Option<u64>,
    current_dir_total_space_bytes: Option<u64>,
    current_dir_free_bytes: Option<u64>,
    recursive_mtime_rx: Option<Receiver<RecursiveMtimeMsg>>,
    recursive_mtime_scan_id: u64,
    selected_total_size_rx: Option<Receiver<SelectedTotalSizeMsg>>,
    selected_total_size_scan_id: u64,
    selected_total_size_pending: bool,
    selected_total_size_bytes: Option<u64>,
    selected_total_size_items: usize,
    sort_mode: SortMode,
    sort_menu_selected: usize,
    panel_tab: u8,
    internal_search_candidates: Vec<PathBuf>,
    internal_search_results: Vec<InternalSearchResult>,
    internal_search_selected: usize,
    internal_search_scope: InternalSearchScope,
    internal_search_candidates_rx: Option<Receiver<InternalSearchCandidatesMsg>>,
    internal_search_candidates_scan_id: u64,
    internal_search_candidates_pending: bool,
    internal_search_candidates_truncated: bool,
    internal_search_content_rx: Option<Receiver<InternalSearchContentMsg>>,
    internal_search_content_request_id: u64,
    internal_search_content_pending: bool,
    internal_search_content_limit_note: Option<String>,
    internal_search_content_limits: InternalSearchContentLimits,
    internal_search_limits_menu_open: bool,
    internal_search_limits_selected: usize,
    internal_search_regex_mode: bool,
    internal_search_regex: Option<Regex>,
    internal_search_regex_error: Option<String>,
    notes_by_name: HashMap<String, String>,
    notes_rx: Option<Receiver<NotesLoadMsg>>,
    notes_scan_id: u64,
    notes_loaded_for: Option<PathBuf>,
    note_edit_targets: Vec<String>,
    meta_group_width: usize,
    meta_owner_width: usize,
    header_clock_minute_key: Option<i64>,
    header_clock_text: String,
    db_preview_path: Option<PathBuf>,
    db_preview_tables: Vec<String>,
    db_preview_selected: usize,
    db_preview_output_lines: Vec<String>,
    db_preview_row_limit: usize,
    db_preview_error: Option<String>,
}

const ZIP_BASED_EXTENSIONS: &[&str] = &[
    "zip", "jar", "war", "ear", "apk", "xpi", "crx", "cbz", "epub", "ipa",
    "odt", "ods", "odp", "odg", "odf", "ott", "ots", "otp", "sxw", "sxc",
    "sxi", "docx", "xlsx", "pptx", "vsix", "nupkg", "kmz", "whl",
];

fn env_flag_true(names: &[&str]) -> bool {
    for name in names {
        if let Ok(raw) = env::var(name) {
            let v = raw.trim();
            let is_true = v == "1" || v.eq_ignore_ascii_case("true");
            if !is_true && *name == "NO_COLOR" {
                // SAFETY: This runs during startup/list-mode initialization before any
                // worker threads are spawned, so mutating the process environment here
                // avoids races while ensuring falsey NO_COLOR values do not leak through
                // to downstream color handling.
                unsafe {
                    env::remove_var(name);
                }
            }
            return is_true;
        }
    }
    false
}

impl App {
    fn open_path_in_editor_cli(path: &PathBuf) -> io::Result<()> {
        // Check if file is binary and use appropriate editor
        if Self::is_binary_file(path) {
            // Try hexedit first (interactive binary editor)
            if Self::integration_probe("hexedit").0 {
                let _ = Command::new("hexedit").arg(path).status();
            }
            // Fall back to hexyl with less paging if hexedit is not available
            if Self::integration_probe("hexyl").0 {
                if let Ok(mut child) = Command::new("hexyl")
                    .arg(path)
                    .stdout(Stdio::piped())
                    .spawn()
                {
                    if let Some(hex_out) = child.stdout.take() {
                        let _ = Command::new("less").args(["-R"]).stdin(hex_out).status();
                    }
                    let _ = child.wait();
                }
                return Ok(());
            }
        }

        // For text files or if no binary editors available, use regular editor
        let editor = env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
        let _ = Command::new(editor).arg(path).status()?;
        Ok(())
    }

    fn new() -> io::Result<Self> {
        let current_dir = env::current_dir()?;
        let internal_search_content_limits = Self::internal_search_content_limits();
        let mut app = Self {
            current_dir,
            entries: Vec::new(),
            entry_render_cache: Vec::new(),
            selected_index: 0,
            marked_indices: HashSet::new(),
            directory_selection: HashMap::new(),
            archive_mounts: Vec::new(),
            mode: AppMode::Browsing,
            table_state: TableState::default(),
            show_hidden: false,
            clipboard: Vec::new(),
            paste_queue: VecDeque::new(),
            paste_current_src: None,
            paste_move_mode: false,
            paste_target_dir: None,
            path_input_filter: None,
            input_buffer: String::new(),
            input_cursor: 0,
            status_message: String::new(),
            page_size: 20,
            ssh_mounts: Vec::new(),
            remote_entries: Vec::new(),
            ssh_picker_selection: 0,
            copy_rx: None,
            copy_total_rx: None,
            copy_total_bytes: 0,
            copy_done_bytes: 0,
            copy_job_total_bytes: 0,
            copy_done_before_job: 0,
            copy_started_at: None,
            copy_item_name: String::new(),
            copy_current_src: None,
            copy_from_remote: false,
            paste_total_items: 0,
            paste_ok_items: 0,
            paste_failed_items: 0,
            archive_create_targets: Vec::new(),
            archive_extract_targets: Vec::new(),
            archive_rx: None,
            archive_total_bytes: 0,
            archive_done_bytes: 0,
            archive_started_at: None,
            archive_name: String::new(),
            nerd_font_active: env::var("NERD_FONT_ACTIVE").map(|v| v == "1").unwrap_or(false),
            no_color: env_flag_true(&["NO_COLOR"]),
            show_icons: env::var("TERMINAL_ICONS").map(|v| v != "0").unwrap_or(true),
            integration_selected: 0,
            bookmark_selected: 0,
            integration_overrides: HashMap::new(),
            integration_rows_cache: Vec::new(),
            integration_install_key: None,
            integration_install_package: None,
            integration_install_brew_path: None,
            help_scroll_offset: 0,
            help_max_offset: 0,
            confirm_delete_scroll_offset: 0,
            confirm_delete_max_offset: 0,
            confirm_delete_button_focus: 0,
            git_info_cache: None,
            git_info_rx: None,
            folder_size_enabled: false,
            folder_size_cache: HashMap::new(),
            folder_size_rx: None,
            folder_size_scan_id: 0,
            tree_expansion_levels: HashMap::new(),
            tree_last_tap: None,
            tree_row_prefixes: Vec::new(),
            current_dir_total_size_rx: None,
            current_dir_total_size_scan_id: 0,
            current_dir_total_size_pending: false,
            current_dir_total_size_bytes: None,
            current_dir_total_space_bytes: None,
            current_dir_free_bytes: None,
            recursive_mtime_rx: None,
            recursive_mtime_scan_id: 0,
            selected_total_size_rx: None,
            selected_total_size_scan_id: 0,
            selected_total_size_pending: false,
            selected_total_size_bytes: None,
            selected_total_size_items: 0,
            sort_mode: SortMode::NameAsc,
            sort_menu_selected: 0,
            panel_tab: 0,
            internal_search_candidates: Vec::new(),
            internal_search_results: Vec::new(),
            internal_search_selected: 0,
            internal_search_scope: InternalSearchScope::Filename,
            internal_search_candidates_rx: None,
            internal_search_candidates_scan_id: 0,
            internal_search_candidates_pending: false,
            internal_search_candidates_truncated: false,
            internal_search_content_rx: None,
            internal_search_content_request_id: 0,
            internal_search_content_pending: false,
            internal_search_content_limit_note: None,
            internal_search_content_limits,
            internal_search_limits_menu_open: false,
            internal_search_limits_selected: 0,
            internal_search_regex_mode: false,
            internal_search_regex: None,
            internal_search_regex_error: None,
            notes_by_name: HashMap::new(),
            notes_rx: None,
            notes_scan_id: 0,
            notes_loaded_for: None,
            note_edit_targets: Vec::new(),
            meta_group_width: 1,
            meta_owner_width: 1,
            header_clock_minute_key: None,
            header_clock_text: String::new(),
            db_preview_path: None,
            db_preview_tables: Vec::new(),
            db_preview_selected: 0,
            db_preview_output_lines: Vec::new(),
            db_preview_row_limit: 8,
            db_preview_error: None,
        };
        app.refresh_header_clock_if_needed();
        app.refresh_entries()?;
        app.request_notes_for_current_dir_once();
        app.request_git_info_for_current_dir_once();
        Ok(app)
    }

    fn refresh_header_clock_if_needed(&mut self) {
        let now = Local::now();
        let minute_key = now.timestamp().div_euclid(60);
        if self.header_clock_minute_key == Some(minute_key) {
            return;
        }
        self.header_clock_minute_key = Some(minute_key);
        self.header_clock_text = now.format("%Y-%m-%d %H:%M").to_string();
    }

    fn age_encrypt_file_interactive(input: &PathBuf, output: &PathBuf) -> Result<(), String> {
        let status = Command::new("age")
            .args(["-p", "-o"])
            .arg(output)
            .arg(input)
            .status()
            .map_err(|e| e.to_string())?;

        if status.success() {
            Ok(())
        } else {
            Err("age encryption failed".to_string())
        }
    }

    fn age_decrypt_file_interactive(input: &PathBuf, output: &PathBuf) -> Result<(), String> {
        let status = Command::new("age")
            .args(["-d", "-o"])
            .arg(output)
            .arg(input)
            .status()
            .map_err(|e| e.to_string())?;

        if status.success() {
            Ok(())
        } else {
            Err("age decryption failed".to_string())
        }
    }

    fn protect_file_with_age(&mut self, input: &PathBuf) -> io::Result<()> {
        let protected_path = Self::age_protected_output_path(input);
        if protected_path.exists() {
            self.set_status(format!(
                "protected target exists: {}",
                protected_path.file_name().and_then(|n| n.to_str()).unwrap_or("target")
            ));
            return Ok(());
        }

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        let result = Self::age_encrypt_file_interactive(input, &protected_path);
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;

        match result {
            Ok(()) => {
                let _ = fs::remove_file(input);
                self.set_status("file protected with age password");
                self.refresh_entries_or_status();
            }
            Err(e) => {
                let _ = fs::remove_file(&protected_path);
                self.set_status(format!("protect failed: {}", e));
            }
        }
        Ok(())
    }

    fn unprotect_file_with_age(&mut self, input: &PathBuf) -> io::Result<()> {
        let plain_path = Self::age_plain_output_path(input);
        if plain_path.exists() {
            self.set_status(format!(
                "unprotect target exists: {}",
                plain_path.file_name().and_then(|n| n.to_str()).unwrap_or("target")
            ));
            return Ok(());
        }

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        let result = Self::age_decrypt_file_interactive(input, &plain_path);
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;

        match result {
            Ok(()) => {
                let _ = fs::remove_file(input);
                self.set_status("password protection removed");
                self.refresh_entries_or_status();
            }
            Err(e) => {
                let _ = fs::remove_file(&plain_path);
                self.set_status(format!("unprotect failed: {}", e));
            }
        }

        Ok(())
    }

    fn preview_age_file(&mut self, input: &PathBuf) -> io::Result<bool> {
        let Ok((tmp_dir, tmp_path)) = Self::age_temp_decrypt_paths(input, "preview") else {
            self.set_status("failed to prepare temporary file");
            return Ok(false);
        };

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        let decrypted = Self::age_decrypt_file_interactive(input, &tmp_path);

        let mut shown = false;
        if decrypted.is_ok() {
            if Self::is_image_file(&tmp_path) && self.integration_active("viu") {
                shown = Self::preview_single_image_with_tool(&tmp_path, "viu");
            } else if Self::is_image_file(&tmp_path) && self.integration_active("chafa") {
                shown = Self::preview_single_image_with_tool(&tmp_path, "chafa");
            } else if Self::is_markdown_file(&tmp_path) && self.integration_active("glow") {
                shown = Command::new("glow")
                    .arg("-p")
                    .arg(&tmp_path)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            } else if Self::is_mermaid_file(&tmp_path) && self.integration_active("mmdflux") {
                if let Ok(mut child) = Command::new("mmdflux")
                    .arg(&tmp_path)
                    .stdout(Stdio::piped())
                    .spawn()
                {
                    if let Some(mmd_out) = child.stdout.take() {
                        shown = Command::new("less")
                            .args(["-R"])
                            .stdin(mmd_out)
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false);
                    }
                    let _ = child.wait();
                }
            } else if Self::is_html_file(&tmp_path) && self.integration_active("links") {
                shown = Command::new("links")
                    .arg(&tmp_path)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            } else if Self::is_json_file(&tmp_path) && self.integration_active("jnv") {
                shown = Self::preview_json_with_jnv(&tmp_path)?;
            } else if Self::is_delimited_text_file(&tmp_path) && self.integration_active("csvlens") {
                shown = Command::new("csvlens")
                    .arg(&tmp_path)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            } else if Self::is_audio_file(&tmp_path) && self.integration_active("sox") {
                let mut child = if Self::integration_probe("play").0 {
                    Command::new("play")
                        .arg(&tmp_path)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                } else {
                    Command::new("sox")
                        .arg(&tmp_path)
                        .arg("-d")
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                };

                if let Ok(ref mut proc) = child {
                    println!("Playing decrypted audio: {}", input.display());
                    println!("Press q, Esc, or Left to stop playback.");
                    enable_raw_mode()?;
                    loop {
                        if proc.try_wait()?.is_some() {
                            break;
                        }
                        if event::poll(Duration::from_millis(120))? {
                            if let Event::Key(k) = event::read()? {
                                if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc | KeyCode::Left) {
                                    let _ = proc.kill();
                                    let _ = proc.wait();
                                    break;
                                }
                            }
                        }
                    }
                    disable_raw_mode()?;
                    shown = true;
                }
            } else if Self::is_cast_file(&tmp_path) && self.integration_active("asciinema") {
                shown = Self::preview_cast_with_asciinema(&tmp_path)?;
            } else if Self::is_supported_archive(&tmp_path) {
                shown = self.preview_archive_contents(&tmp_path);
            } else if Self::is_pdf_file(&tmp_path) && self.integration_active("pdftotext") {
                if let Ok(mut child) = Command::new("pdftotext")
                    .args(["-layout", "-nopgbrk"])
                    .arg(&tmp_path)
                    .arg("-")
                    .stdout(Stdio::piped())
                    .spawn()
                {
                    if let Some(pdf_text) = child.stdout.take() {
                        shown = Command::new("less")
                            .args(["-R"])
                            .stdin(pdf_text)
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false);
                    }
                    let _ = child.wait();
                }
            } else if Self::is_binary_file(&tmp_path) && self.integration_active("hexyl") {
                let hexyl = Command::new("hexyl")
                    .arg(&tmp_path)
                    .stdout(Stdio::piped())
                    .spawn();
                if let Ok(child) = hexyl {
                    shown = Command::new("less")
                        .args(["-R"])
                        .stdin(child.stdout.unwrap())
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                }
            } else if self.integration_active("bat") {
                let bat_cmd = Self::bat_tool().unwrap_or_else(|| "bat".to_string());
                shown = Command::new(bat_cmd)
                    .args(["--paging=always", "--style=full", "--color=always"])
                    .arg(&tmp_path)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            } else {
                shown = Command::new("less")
                    .args(["-R", tmp_path.to_str().unwrap_or_default()])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            }
        }

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;
        let _ = fs::remove_file(&tmp_path);
        let _ = fs::remove_dir_all(&tmp_dir);

        if let Err(e) = decrypted {
            self.set_status(format!("decrypt failed: {}", e));
            return Ok(false);
        }
        Ok(shown)
    }

    fn preview_json_with_jnv(path: &PathBuf) -> io::Result<bool> {
        let mut child = Command::new("jnv").arg(path).spawn();
        if let Ok(ref mut proc) = child {
            println!("Viewing JSON: {}", path.display());
            println!("Press q, Esc, or Left to close preview.");
            enable_raw_mode()?;
            loop {
                if proc.try_wait()?.is_some() {
                    break;
                }
                if event::poll(Duration::from_millis(120))? {
                    if let Event::Key(k) = event::read()? {
                        if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc | KeyCode::Left) {
                            let _ = proc.kill();
                            let _ = proc.wait();
                            break;
                        }
                    }
                }
            }
            disable_raw_mode()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn preview_single_image_with_tool(path: &PathBuf, tool: &str) -> bool {
        let script = r#"
tool="$1"
img="$2"
clear
"$tool" -- "$img"
printf '\n[Press any key to return]\n'
IFS= read -rsn1 _
"#;

        Command::new("bash")
            .arg("-lc")
            .arg(script)
            .arg("--")
            .arg(tool)
            .arg(path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn preview_cast_with_asciinema(path: &PathBuf) -> io::Result<bool> {
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;

        let mut child = match Command::new("asciinema")
            .arg("play")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return Ok(false),
        };

        println!("Playing cast: {}", path.display());
        println!("Press q or Esc to stop playback.");

        enable_raw_mode()?;
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if event::poll(Duration::from_millis(120))? {
                if let Event::Key(k) = event::read()? {
                    if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }
        disable_raw_mode()?;
        Ok(true)
    }

    fn edit_age_file(&mut self, input: &PathBuf) -> io::Result<bool> {
        let Ok((tmp_dir, tmp_path)) = Self::age_temp_decrypt_paths(input, "edit") else {
            self.set_status("failed to prepare temporary file");
            return Ok(false);
        };

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        execute!(io::stdout(), Show)?;
        let decrypted = Self::age_decrypt_file_interactive(input, &tmp_path);
        if decrypted.is_err() {
            execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
            enable_raw_mode()?;
            execute!(io::stdout(), Hide)?;
            let _ = fs::remove_file(&tmp_path);
            let _ = fs::remove_dir_all(&tmp_dir);
            self.set_status(format!("decrypt failed: {}", decrypted.err().unwrap_or_default()));
            return Ok(false);
        }

        let _ = Command::new(env::var("EDITOR").unwrap_or_else(|_| "nano".to_string()))
            .arg(&tmp_path)
            .status();

        let result = Self::age_encrypt_file_interactive(&tmp_path, input);
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        enable_raw_mode()?;
        execute!(io::stdout(), Hide)?;

        let _ = fs::remove_file(&tmp_path);
        let _ = fs::remove_dir_all(&tmp_dir);
        match result {
            Ok(()) => self.set_status("protected file updated"),
            Err(e) => self.set_status(format!("re-protect failed: {}", e)),
        }
        self.refresh_entries_or_status();
        Ok(true)
    }

    fn sort_mode_options() -> [SortMode; 7] {
        [
            SortMode::NameAsc,
            SortMode::NameDesc,
            SortMode::ExtensionAsc,
            SortMode::SizeAsc,
            SortMode::SizeDesc,
            SortMode::ModifiedNewest,
            SortMode::ModifiedOldest,
        ]
    }

    fn sort_mode_index(mode: SortMode) -> usize {
        Self::sort_mode_options()
            .iter()
            .position(|m| *m == mode)
            .unwrap_or(0)
    }

    fn entry_name_key(entry: &fs::DirEntry) -> String {
        entry.file_name().to_string_lossy().to_ascii_lowercase()
    }

    fn entry_extension_key(entry: &fs::DirEntry) -> String {
        entry.path()
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    }

    pub(crate) fn sort_entries_by_mode(
        entries: &mut Vec<fs::DirEntry>,
        mode: SortMode,
        folder_size_cache: Option<&HashMap<PathBuf, u64>>,
    ) {
        if entries.len() < 2 {
            return;
        }
        // Pre-collect all sort keys in O(n) — eliminates O(n log n) stat() calls that
        // the previous sort_by comparator incurred by calling is_file()/metadata() per pair.
        let metas: Vec<Option<fs::Metadata>> = entries.iter().map(|e| e.metadata().ok()).collect();
        let is_dirs: Vec<bool> = metas.iter()
            .map(|m| m.as_ref().map(|m| m.is_dir()).unwrap_or(false))
            .collect();
        let names: Vec<String> = entries.iter().map(|e| Self::entry_name_key(e)).collect();
        let paths: Vec<PathBuf> = entries.iter().map(|e| e.path()).collect();
        let sizes: Vec<u64>    = metas.iter()
            .enumerate()
            .map(|(idx, m)| {
                let default_size = m.as_ref().map(|m| m.len()).unwrap_or(0);
                if !matches!(mode, SortMode::SizeAsc | SortMode::SizeDesc) {
                    return default_size;
                }

                if is_dirs[idx] {
                    folder_size_cache
                        .and_then(|cache| cache.get(&paths[idx]).copied())
                        .unwrap_or(0)
                } else {
                    default_size
                }
            })
            .collect();
        let times: Vec<u64>    = metas.iter().map(|m| {
            m.as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0)
        }).collect();
        let exts: Vec<String>  = entries.iter().map(|e| Self::entry_extension_key(e)).collect();

        let mut indices: Vec<usize> = (0..entries.len()).collect();
        indices.sort_by(|&a, &b| {
            // Directories always sort before files.
            let type_ord = is_dirs[b].cmp(&is_dirs[a]);
            if type_ord != std::cmp::Ordering::Equal {
                return type_ord;
            }
            match mode {
                SortMode::NameAsc        => names[a].cmp(&names[b]),
                SortMode::NameDesc       => names[b].cmp(&names[a]),
                SortMode::ExtensionAsc   => exts[a].cmp(&exts[b]).then_with(|| names[a].cmp(&names[b])),
                SortMode::SizeAsc        => sizes[a].cmp(&sizes[b]).then_with(|| names[a].cmp(&names[b])),
                SortMode::SizeDesc       => sizes[b].cmp(&sizes[a]).then_with(|| names[a].cmp(&names[b])),
                SortMode::ModifiedNewest => times[b].cmp(&times[a]).then_with(|| names[a].cmp(&names[b])),
                SortMode::ModifiedOldest => times[a].cmp(&times[b]).then_with(|| names[a].cmp(&names[b])),
            }
        });

        // Rearrange entries in-place to match the sorted index permutation.
        let mut tmp: Vec<Option<fs::DirEntry>> = entries.drain(..).map(Some).collect();
        *entries = indices.into_iter().map(|i| tmp[i].take().unwrap()).collect();
    }

    fn apply_sort_to_current_entries(&mut self) {
        if !self.tree_expansion_levels.is_empty() {
            let selected_path = self.entries.get(self.selected_index).map(|e| e.path());
            let _ = self.refresh_entries();
            if let Some(path) = selected_path {
                if let Some(idx) = self.entries.iter().position(|e| e.path() == path) {
                    self.selected_index = idx;
                    self.table_state.select(Some(idx));
                }
            }
            return;
        }
        let selected_path = self.entries.get(self.selected_index).map(|e| e.path());
        let marked_paths: HashSet<PathBuf> = self
            .marked_indices
            .iter()
            .filter_map(|idx| self.entries.get(*idx).map(|e| e.path()))
            .collect();

        let folder_size_cache = if self.folder_size_enabled {
            Some(&self.folder_size_cache)
        } else {
            None
        };
        Self::sort_entries_by_mode(&mut self.entries, self.sort_mode, folder_size_cache);

        let config = EntryRenderConfig { nerd_font_active: self.nerd_font_active, show_icons: self.show_icons };
        let uid_cache = App::build_uid_cache(&self.entries);
        let gid_cache = App::build_gid_cache(&self.entries);
            self.entry_render_cache = self.entries.iter()
            .map(|entry| App::build_entry_render_cache(entry, config, &uid_cache, &gid_cache))
            .collect();
        self.apply_cached_folder_size_columns();
        self.refresh_meta_identity_widths();

        self.marked_indices = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| marked_paths.contains(&entry.path()))
            .map(|(idx, _)| idx)
            .collect();

        if self.entries.is_empty() {
            self.selected_index = 0;
            self.table_state.select(None);
            return;
        }

        self.selected_index = selected_path
            .and_then(|p| self.entries.iter().position(|e| e.path() == p))
            .unwrap_or_else(|| self.selected_index.min(self.entries.len() - 1));
        self.table_state.select(Some(self.selected_index));
    }

    fn begin_sort_menu(&mut self) {
        self.panel_tab = 4;
        self.sort_menu_selected = Self::sort_mode_index(self.sort_mode);
        self.mode = AppMode::SortMenu;
    }

    fn commit_sort_menu_choice(&mut self) {
        let options = Self::sort_mode_options();
        if let Some(mode) = options.get(self.sort_menu_selected).copied() {
            self.sort_mode = mode;
            self.apply_sort_to_current_entries();
            self.set_status(format!("sort: {}", mode.label()));
        }
        self.mode = AppMode::Browsing;
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = msg.into();
    }

    fn decorate_footer_message(&self, msg: &str) -> String {
        ui::status::decorate_footer_message(msg, self.nerd_font_active)
    }

    fn search_spans_with_ranges(
        text: &str,
        ranges: &[(usize, usize)],
        base_style: Style,
        match_style: Style,
    ) -> Vec<Span<'static>> {
        ui::search::search_spans_with_ranges(text, ranges, base_style, match_style)
    }

    fn refresh_entries_or_status(&mut self) -> bool {
        match self.refresh_entries() {
            Ok(()) => {
                if self.copy_rx.is_none() && self.archive_rx.is_none() {
                    self.status_message.clear();
                }
                true
            }
            Err(e) => {
                self.set_status(format!("refresh failed: {}", e));
                false
            }
        }
    }

    fn try_enter_dir(&mut self, target: PathBuf) {
        let previous_dir = self.current_dir.clone();
        let previous_filter = self.path_input_filter.clone();
        let changed_dir = target != previous_dir;
        self.remember_current_selection();
        self.current_dir = target;
        if changed_dir {
            self.path_input_filter = None;
        }
        if !self.refresh_entries_or_status() {
            self.current_dir = previous_dir;
            self.path_input_filter = previous_filter;
        } else {
            self.restore_selection_for_current_dir();
            self.request_git_info_for_current_dir_once();
        }
    }

    fn preview_images_with_chafa(&mut self, start_path: PathBuf) -> io::Result<()> {
        let images: Vec<PathBuf> = self
            .entries
            .iter()
            .map(|e| e.path())
            .filter(Self::is_image_file)
            .collect();

        if images.is_empty() {
            return Ok(());
        }

        let mut idx = images.iter().position(|p| *p == start_path).unwrap_or(0);

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let result_file = env::temp_dir().join(format!(
            "sbrs_chafa_sel_{}_{}",
            std::process::id(),
            stamp
        ));

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;

        let script = r#"
idx="$1"
out_file="$2"
shift 2
paths=("$@")
count="${#paths[@]}"

if [[ "$count" -eq 0 ]]; then
  exit 1
fi

while true; do
  clear
  chafa -- "${paths[$idx]}"
  printf '\n[←/→ prev/next (exits at ends), q/Esc/Enter exit]\n'

  IFS= read -rsn1 key || break
  if [[ "$key" == $'\x1b' ]]; then
    IFS= read -rsn2 key2
    key+="$key2"
  fi

  case "$key" in
    $'\x1b[D')
      if (( idx == 0 )); then break; fi
      ((idx--))
      ;;
    $'\x1b[C')
      if (( idx + 1 >= count )); then break; fi
      ((idx++))
      ;;
    q|$'\x1b'|$'\n'|$'\r')
      break
      ;;
  esac
done

printf '%s\n' "${paths[$idx]}" > "$out_file"
"#;

        let mut cmd = Command::new("bash");
        cmd.arg("-lc")
            .arg(script)
            .arg("--")
            .arg(idx.to_string())
            .arg(result_file.to_string_lossy().to_string());
        for image in &images {
            cmd.arg(image);
        }
        let _ = cmd.status();

        if let Ok(selected_path) = fs::read_to_string(&result_file) {
            let selected = selected_path.trim();
            if !selected.is_empty() {
                let selected_buf = PathBuf::from(selected);
                if let Some(pos) = images.iter().position(|p| *p == selected_buf) {
                    idx = pos;
                }
            }
        }
        let _ = fs::remove_file(&result_file);

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        enable_raw_mode()?;

        if let Some(name) = images[idx].file_name() {
            self.select_entry_named(&name.to_string_lossy());
        }

        Ok(())
    }

    fn preview_images_with_viu(&mut self, start_path: PathBuf) -> io::Result<()> {
        let images: Vec<PathBuf> = self
            .entries
            .iter()
            .map(|e| e.path())
            .filter(Self::is_image_file)
            .collect();

        if images.is_empty() {
            return Ok(());
        }

        let mut idx = images.iter().position(|p| *p == start_path).unwrap_or(0);

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let result_file = env::temp_dir().join(format!(
            "sbrs_viu_sel_{}_{}",
            std::process::id(),
            stamp
        ));

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;

        let script = r#"
idx="$1"
out_file="$2"
shift 2
paths=("$@")
count="${#paths[@]}"

if [[ "$count" -eq 0 ]]; then
  exit 1
fi

while true; do
  clear
  viu -- "${paths[$idx]}"
  printf '\n[←/→ prev/next (exits at ends), q/Esc/Enter exit]\n'

  IFS= read -rsn1 key || break
  if [[ "$key" == $'\x1b' ]]; then
    IFS= read -rsn2 key2
    key+="$key2"
  fi

  case "$key" in
    $'\x1b[D')
      if (( idx == 0 )); then break; fi
      ((idx--))
      ;;
    $'\x1b[C')
      if (( idx + 1 >= count )); then break; fi
      ((idx++))
      ;;
    q|$'\x1b'|$'\n'|$'\r')
      break
      ;;
  esac
done

printf '%s\n' "${paths[$idx]}" > "$out_file"
"#;

        let mut cmd = Command::new("bash");
        cmd.arg("-lc")
            .arg(script)
            .arg("--")
            .arg(idx.to_string())
            .arg(result_file.to_string_lossy().to_string());
        for image in &images {
            cmd.arg(image);
        }
        let _ = cmd.status();

        if let Ok(selected_path) = fs::read_to_string(&result_file) {
            let selected = selected_path.trim();
            if !selected.is_empty() {
                let selected_buf = PathBuf::from(selected);
                if let Some(pos) = images.iter().position(|p| *p == selected_buf) {
                    idx = pos;
                }
            }
        }
        let _ = fs::remove_file(&result_file);

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        enable_raw_mode()?;

        if let Some(name) = images[idx].file_name() {
            self.select_entry_named(&name.to_string_lossy());
        }

        Ok(())
    }

    fn create_temp_selection_path(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        env::temp_dir().join(format!("{}_{}_{}.txt", prefix, std::process::id(), stamp))
    }

    fn parse_ssh_config() -> Vec<SshHost> {
        let config_path = match env::var("HOME") {
            Ok(h) => PathBuf::from(h).join(".ssh/config"),
            Err(_) => return Vec::new(),
        };
        let content = match fs::read_to_string(&config_path) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let mut hosts: Vec<SshHost> = Vec::new();
        let mut current: Option<SshHost> = None;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let sep = trimmed.find(|c: char| c.is_ascii_whitespace() || c == '=');
            let (raw_key, raw_val) = match sep {
                Some(pos) => (&trimmed[..pos], trimmed[pos + 1..].trim_start_matches(|c: char| c == '=' || c.is_ascii_whitespace())),
                None => (trimmed, ""),
            };
            let key = raw_key.to_lowercase();
            let val = raw_val.to_string();
            if key == "host" || key == "match" {
                if let Some(h) = current.take() {
                    if !h.alias.contains('*') && !h.alias.contains('?') {
                        hosts.push(h);
                    }
                }
                if key == "host" {
                    if let Some(alias) = val.split_whitespace().find(|s| !s.contains('*') && !s.contains('?')).map(|s| s.to_string()) {
                        current = Some(SshHost { hostname: alias.clone(), alias, user: None, port: None, identity_file: None });
                    }
                }
            } else if let Some(ref mut h) = current {
                match key.as_str() {
                    "hostname" => h.hostname = val,
                    "user" => h.user = Some(val),
                    "port" => h.port = val.parse().ok(),
                    "identityfile" => h.identity_file = Some(val),
                    _ => {}
                }
            }
        }
        if let Some(h) = current {
            if !h.alias.contains('*') && !h.alias.contains('?') {
                hosts.push(h);
            }
        }
        hosts
    }

    fn parse_rclone_remotes() -> Vec<RemoteEntry> {
        let out = match Command::new("rclone").args(["listremotes", "--long"]).output() {
            Ok(o) if o.status.success() => o,
            _ => return Vec::new(),
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                // format: "name:   type"
                let mut parts = line.splitn(2, ':');
                let name = parts.next()?.trim().to_string();
                let rtype = parts.next().unwrap_or("").trim().to_string();
                if name.is_empty() { return None; }
                Some(RemoteEntry::Rclone { name, rtype })
            })
            .collect()
    }

    fn parse_local_mount_dirs() -> Vec<RemoteEntry> {
        let user = env::var("USER").unwrap_or_default();
        let uid = users::get_current_uid();
        let candidates: Vec<(&str, PathBuf)> = vec![
            ("media", PathBuf::from(format!("/media/{}", user))),
            ("run-media", PathBuf::from(format!("/run/media/{}", user))),
            ("mnt", PathBuf::from("/mnt")),
            ("gvfs", PathBuf::from(format!("/run/user/{}/gvfs", uid))),
        ];

        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut mounts: Vec<RemoteEntry> = Vec::new();

        for (source, root) in candidates {
            if !root.is_dir() {
                continue;
            }

            let entries = match fs::read_dir(&root) {
                Ok(rd) => rd,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() || !seen.insert(path.clone()) {
                    continue;
                }

                let child_name = entry.file_name().to_string_lossy().into_owned();
                let name = format!("{}:{}", source, child_name);
                mounts.push(RemoteEntry::LocalMount {
                    name,
                    mount_path: path,
                    source: source.to_string(),
                });
            }
        }

        mounts.sort_by(|a, b| a.alias().cmp(b.alias()));
        mounts
    }

    fn wait_for_mount_ready(path: &PathBuf) {
        // Some backends (notably rclone --daemon) return before the mount is fully ready.
        // Poll briefly so the first directory read after enter is accurate.
        for _ in 0..20 {
            let ready = Command::new("mountpoint")
                .args(["-q", path.to_string_lossy().as_ref()])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ready {
                break;
            }
            thread::sleep(Duration::from_millis(120));
        }
    }

    fn refresh_remote_entries(&mut self) {
        let has_sshfs = self.integration_active("sshfs");
        let has_rclone = self.integration_active("rclone");
        let mut entries: Vec<RemoteEntry> = Vec::new();
        if has_sshfs {
            entries.extend(App::parse_ssh_config().into_iter().map(RemoteEntry::Ssh));
        }
        if has_rclone {
            entries.extend(App::parse_rclone_remotes());
        }
        entries.extend(self.archive_mounts.iter().map(|m| RemoteEntry::ArchiveMount {
            archive_name: m.archive_name.clone(),
            mount_path: m.mount_path.clone(),
        }));
        entries.extend(App::parse_local_mount_dirs());
        self.remote_entries = entries;
        if self.remote_entries.is_empty() {
            self.ssh_picker_selection = 0;
        } else {
            self.ssh_picker_selection = self.ssh_picker_selection.min(self.remote_entries.len() - 1);
        }
    }

    fn current_remote_mount(&self) -> Option<&SshMount> {
        self.ssh_mounts
            .iter()
            .filter(|mount| self.current_dir == mount.mount_path || self.current_dir.starts_with(&mount.mount_path))
            .max_by_key(|mount| mount.mount_path.components().count())
    }

    fn current_header_identity(&self, local_user: &str, local_host: &str) -> String {
        self.current_remote_mount()
            .map(|mount| mount.remote_label.clone())
            .unwrap_or_else(|| format!("{}@{}", local_user, local_host))
    }

    fn current_dir_display_path(&self) -> String {
        let Some(mount) = self.current_remote_mount() else {
            return self.current_dir.to_string_lossy().into_owned();
        };

        let rel = self
            .current_dir
            .strip_prefix(&mount.mount_path)
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();

        if rel.is_empty() {
            return mount.remote_root.clone();
        }

        if mount.remote_root == "/" {
            format!("/{}", rel)
        } else if mount.remote_root.ends_with('/') {
            format!("{}{}", mount.remote_root, rel)
        } else {
            format!("{}/{}", mount.remote_root, rel)
        }
    }

    fn path_filter_suffix_text(&self) -> Option<String> {
        let filter = self.path_input_filter.as_ref()?;
        let suffix = match filter.mode {
            PathFilterMode::Prefix => format!("^{}", filter.pattern),
            PathFilterMode::Suffix => format!("{}$", filter.pattern),
            PathFilterMode::Contains => format!("~{}", filter.pattern),
        };
        Some(suffix)
    }

    fn path_with_filter_suffix(base: String, suffix: Option<String>) -> String {
        let Some(suffix) = suffix else {
            return base;
        };

        if base == "/" {
            format!("/{}", suffix)
        } else {
            format!("{}/{}", base, suffix)
        }
    }

    fn current_dir_display_path_with_filter(&self) -> String {
        Self::path_with_filter_suffix(self.current_dir_display_path(), self.path_filter_suffix_text())
    }

    fn current_path_edit_value(&self) -> String {
        let base = self.current_dir.to_string_lossy().into_owned();
        Self::path_with_filter_suffix(base, self.path_filter_suffix_text())
    }

    fn mount_rclone_remote(&mut self, name: &str, rtype: &str) -> io::Result<()> {
        // If already mounted, just navigate there
        if let Some(existing) = self.ssh_mounts.iter_mut().find(|m| m._host_alias == name) {
            existing.return_dir = self.current_dir.clone();
            let mount_path = existing.mount_path.clone();
            self.mode = AppMode::Browsing;
            self.try_enter_dir(mount_path);
            return Ok(());
        }
        let _ = rtype; // informational only
        let return_dir = self.current_dir.clone();
        let mount_dir = PathBuf::from(format!("/tmp/sbrs_rclone_{}", name));
        if mount_dir.exists() {
            let _ = fs::remove_dir(&mount_dir);
        }
        fs::create_dir_all(&mount_dir)?;
        let remote_spec = format!("{}:", name);
        let status = Command::new("rclone")
            .args(["mount", &remote_spec, mount_dir.to_str().unwrap_or(""),
                   "--daemon", "--vfs-cache-mode", "writes"])
            .status()?;
        if status.success() {
            Self::wait_for_mount_ready(&mount_dir);
            self.ssh_mounts.push(SshMount {
                _host_alias: name.to_string(),
                mount_path: mount_dir.clone(),
                return_dir,
                remote_label: name.to_string(),
                remote_root: "/".to_string(),
            });
            self.mode = AppMode::Browsing;
            self.try_enter_dir(mount_dir);
            Ok(())
        } else {
            let _ = fs::remove_dir(&mount_dir);
            Err(io::Error::new(io::ErrorKind::Other, "rclone mount failed"))
        }
    }

    fn mount_ssh_host(&mut self, host: &SshHost) -> io::Result<()> {
        // If already mounted, just navigate there
        if let Some(existing) = self.ssh_mounts.iter_mut().find(|m| m._host_alias == host.alias) {
            existing.return_dir = self.current_dir.clone();
            let mount_path = existing.mount_path.clone();
            self.mode = AppMode::Browsing;
            self.try_enter_dir(mount_path);
            return Ok(());
        }
        let return_dir = self.current_dir.clone();
        let mount_dir = PathBuf::from(format!("/tmp/sbrs_sshfs_{}", host.alias));
        // Remove stale dir if it exists but isn't mounted
        if mount_dir.exists() {
            let _ = fs::remove_dir(&mount_dir);
        }
        fs::create_dir_all(&mount_dir)?;
        let remote_spec = match &host.user {
            Some(u) => format!("{}@{}:", u, host.hostname),
            None => format!("{}:", host.hostname),
        };
        let mut cmd = Command::new("sshfs");
        if let Some(port) = host.port {
            cmd.args(["-p", &port.to_string()]);
        }
        if let Some(idf) = &host.identity_file {
            let expanded = idf.replace('~', &env::var("HOME").unwrap_or_default());
            cmd.args(["-o", &format!("IdentityFile={}", expanded)]);
        }
        cmd.arg(&remote_spec).arg(&mount_dir);
        let status = cmd.status()?;
        if status.success() {
            Self::wait_for_mount_ready(&mount_dir);
            let remote_label = match &host.user {
                Some(user) => format!("{}@{}", user, host.hostname),
                None => host.hostname.clone(),
            };
            self.ssh_mounts.push(SshMount {
                _host_alias: host.alias.clone(),
                mount_path: mount_dir.clone(),
                return_dir,
                remote_label,
                remote_root: "~".to_string(),
            });
            self.mode = AppMode::Browsing;
            self.try_enter_dir(mount_dir);
            Ok(())
        } else {
            let _ = fs::remove_dir(&mount_dir);
            Err(io::Error::new(io::ErrorKind::Other, "sshfs mount failed"))
        }
    }

    fn try_leave_ssh_mount(&mut self) -> bool {
        // Check if we are at the mount root (not just a subdir) — only intercept at the boundary
        let mount_idx = self.ssh_mounts.iter().rposition(|m| {
            self.current_dir == m.mount_path
        });
        let Some(idx) = mount_idx else { return false };
        self.remember_current_selection();
        let return_dir = self.ssh_mounts[idx].return_dir.clone();
        // Navigate back without unmounting — mount stays active, shown as mounted in S picker
        self.current_dir = return_dir;
        self.refresh_entries_or_status();
        true
    }

    fn cleanup_ssh_mounts(&mut self) {
        // If current_dir is inside any ssh mount, set it to the return dir first
        // so the shell cd integration lands on a local path
        for mount in self.ssh_mounts.iter() {
            if self.current_dir == mount.mount_path || self.current_dir.starts_with(&mount.mount_path) {
                self.current_dir = mount.return_dir.clone();
                break;
            }
        }
        while let Some(mount) = self.ssh_mounts.pop() {
            let path_str = mount.mount_path.to_string_lossy().to_string();
            // Try fusermount -u, then fusermount3 -u, then lazy -z variants, then umount
            let ok = Command::new("fusermount").args(["-u", &path_str]).status().map(|s| s.success()).unwrap_or(false)
                || Command::new("fusermount3").args(["-u", &path_str]).status().map(|s| s.success()).unwrap_or(false)
                || Command::new("fusermount").args(["-uz", &path_str]).status().map(|s| s.success()).unwrap_or(false)
                || Command::new("fusermount3").args(["-uz", &path_str]).status().map(|s| s.success()).unwrap_or(false)
                || Command::new("umount").args([&path_str]).status().map(|s| s.success()).unwrap_or(false)
                || Command::new("umount").args(["-l", &path_str]).status().map(|s| s.success()).unwrap_or(false);
            let _ = ok; // best-effort; proceed regardless
            let _ = fs::remove_dir(&mount.mount_path);
        }
    }

    fn unmount_ssh_mount_by_alias(&mut self, alias: &str) -> bool {
        let Some(idx) = self.ssh_mounts.iter().rposition(|m| m._host_alias == alias) else {
            return false;
        };

        let mount = self.ssh_mounts.remove(idx);
        if self.current_dir == mount.mount_path || self.current_dir.starts_with(&mount.mount_path) {
            self.current_dir = mount.return_dir.clone();
            self.refresh_entries_or_status();
        }

        let path_str = mount.mount_path.to_string_lossy().to_string();
        let _ = Command::new("fusermount").args(["-u", &path_str]).status();
        let _ = Command::new("fusermount3").args(["-u", &path_str]).status();
        let _ = Command::new("fusermount").args(["-uz", &path_str]).status();
        let _ = Command::new("fusermount3").args(["-uz", &path_str]).status();
        let _ = Command::new("umount").args([&path_str]).status();
        let _ = Command::new("umount").args(["-l", &path_str]).status();
        let _ = fs::remove_dir(&mount.mount_path);
        true
    }

    fn remember_current_selection(&mut self) {
        self.directory_selection
            .insert(self.current_dir.clone(), self.selected_index);
    }

    fn restore_selection_for_current_dir(&mut self) {
        if self.entries.is_empty() {
            self.selected_index = 0;
            self.table_state.select(None);
            return;
        }

        let index = self
            .directory_selection
            .get(&self.current_dir)
            .copied()
            .unwrap_or(0)
            .min(self.entries.len() - 1);
        self.selected_index = index;
        self.table_state.select(Some(index));
    }

    fn select_entry_named(&mut self, name: &str) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.file_name().to_string_lossy() == name)
        {
            self.selected_index = index;
            self.table_state.select(Some(index));
        }
    }

    fn try_enter_parent_dir(&mut self) {
        let child_name = self
            .current_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());

        if let Some(parent) = self.current_dir.parent() {
            self.try_enter_dir(parent.to_path_buf());
            if let Some(name) = child_name {
                self.select_entry_named(&name);
            }
        }
    }

    fn resolve_input_path(&self, raw: &str) -> PathBuf {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("~/") {
            if let Ok(home) = env::var("HOME") {
                return PathBuf::from(home).join(rest);
            }
        }
        if trimmed == "~" {
            if let Ok(home) = env::var("HOME") {
                return PathBuf::from(home);
            }
        }

        let candidate = PathBuf::from(trimmed);
        if candidate.is_absolute() {
            candidate
        } else {
            self.current_dir.join(candidate)
        }
    }

    fn apply_path_input(&mut self) {
        let raw_input = self.input_buffer.trim().to_string();
        let target = self.resolve_input_path(&raw_input);
        if target.is_dir() {
            self.path_input_filter = None;
            self.try_enter_dir(target);
            self.mode = AppMode::Browsing;
            self.clear_input_edit();
            return;
        }

        let Some((base_raw, filter)) = Self::parse_path_filter_suffix(&raw_input) else {
            self.set_status("path is not a directory");
            return;
        };

        if let Err(err) = Self::build_path_filter_regex(&filter) {
            self.set_status(format!("invalid path filter regex: {}", err));
            return;
        }

        let base_target = self.resolve_input_path(&base_raw);
        if !base_target.is_dir() {
            self.set_status("path is not a directory");
            return;
        }

        self.try_enter_dir(base_target);
        self.path_input_filter = Some(filter);
        self.refresh_entries_or_status();
        self.mode = AppMode::Browsing;
        self.clear_input_edit();
    }

    fn input_cursor_line_col(&self) -> (usize, usize) {
        let mut line = 0usize;
        let mut col = 0usize;
        for ch in self.input_buffer.chars().take(self.input_cursor) {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn active_input_line_text(&self) -> String {
        let (line_idx, _) = self.input_cursor_line_col();
        self.input_buffer
            .split('\n')
            .nth(line_idx)
            .unwrap_or_default()
            .to_string()
    }

    fn create_entries_from_input(&mut self, default_is_dir: bool) {
        let mut specs: Vec<(String, bool)> = Vec::new();
        for raw_line in self.input_buffer.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            let (name, is_dir) = if let Some(rest) = line.strip_prefix('/') {
                (rest.trim().to_string(), true)
            } else {
                (line.to_string(), default_is_dir)
            };
            if !name.is_empty() {
                specs.push((name, is_dir));
            }
        }

        if specs.is_empty() {
            self.set_status("name cannot be empty");
            return;
        }

        let mut created: Vec<String> = Vec::new();
        let mut failed = 0usize;
        let mut first_error: Option<String> = None;

        for (name, is_dir) in specs {
            let target = self.current_dir.join(&name);
            if target.exists() {
                failed += 1;
                if first_error.is_none() {
                    first_error = Some("target already exists".to_string());
                }
                continue;
            }

            let result = if is_dir {
                fs::create_dir(&target)
            } else {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map(|_| ())
            };

            match result {
                Ok(()) => created.push(name),
                Err(e) => {
                    failed += 1;
                    if first_error.is_none() {
                        first_error = Some(format!("create failed: {}", e));
                    }
                }
            }
        }

        if created.is_empty() {
            self.set_status(first_error.unwrap_or_else(|| "create failed".to_string()));
            return;
        }

        let last_created = created.last().cloned();
        self.mode = AppMode::Browsing;
        self.clear_input_edit();
        self.refresh_entries_or_status();
        if let Some(name) = last_created {
            self.select_entry_named(&name);
        }

        if failed == 0 {
            self.set_status(format!("created {} item(s)", created.len()));
        } else {
            self.set_status(format!("created {} item(s), {} failed", created.len(), failed));
        }
    }

    fn refresh_entries(&mut self) -> io::Result<()> {
        let folder_size_cache = if self.folder_size_enabled {
            Some(&self.folder_size_cache)
        } else {
            None
        };
        let mut tree_row_prefixes = Vec::new();
        let mut entries: Vec<_> = if !self.tree_expansion_levels.is_empty() {
            let rows = ui::tree::collect_tree_rows_with_expansions(
                &self.current_dir,
                self.show_hidden,
                self.sort_mode,
                folder_size_cache,
                &self.tree_expansion_levels,
            )?;
            tree_row_prefixes = rows.iter().map(|row| row.prefix.clone()).collect();
            rows.into_iter().map(|row| row.entry).collect()
        } else {
            let mut direct_entries: Vec<_> = fs::read_dir(&self.current_dir)?
                .filter_map(|res| res.ok())
                .filter(|e| self.show_hidden || !e.file_name().to_string_lossy().starts_with('.'))
                .collect();
            Self::sort_entries_by_mode(&mut direct_entries, self.sort_mode, folder_size_cache);
            direct_entries
        };
        if let Some(filter) = self.path_input_filter.as_ref() {
            let filter_regex = Self::build_path_filter_regex(filter)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            if !self.tree_expansion_levels.is_empty() {
                let mut filtered_entries = Vec::new();
                let mut filtered_prefixes = Vec::new();
                for (entry, prefix) in entries.into_iter().zip(tree_row_prefixes.into_iter()) {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if Self::entry_name_matches_path_filter(&name, &filter_regex) {
                        filtered_entries.push(entry);
                        filtered_prefixes.push(prefix);
                    }
                }
                entries = filtered_entries;
                tree_row_prefixes = filtered_prefixes;
            } else {
                entries.retain(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    Self::entry_name_matches_path_filter(&name, &filter_regex)
                });
            }
        }
        self.entries = entries;
        self.tree_row_prefixes = if !self.tree_expansion_levels.is_empty() {
            tree_row_prefixes
        } else {
            vec![String::new(); self.entries.len()]
        };
        let config = EntryRenderConfig { nerd_font_active: self.nerd_font_active, show_icons: self.show_icons };
        let uid_cache = App::build_uid_cache(&self.entries);
        let gid_cache = App::build_gid_cache(&self.entries);
            self.entry_render_cache = self.entries.iter()
            .map(|entry| App::build_entry_render_cache(entry, config, &uid_cache, &gid_cache))
            .collect();
        self.apply_cached_folder_size_columns();
        self.refresh_meta_identity_widths();
        self.refresh_current_dir_free_space();
        self.folder_size_scan_id = self.folder_size_scan_id.wrapping_add(1);
        self.folder_size_rx = None;
        self.recursive_mtime_rx = None;
        self.clear_current_dir_total_size_state();
        self.clear_selected_total_size_state();
        self.marked_indices.clear();
        
        if self.entries.is_empty() {
            self.selected_index = 0;
            self.table_state.select(None);
        } else {
            self.selected_index = self.selected_index.min(self.entries.len() - 1);
            self.table_state.select(Some(self.selected_index));
        }

        if self.folder_size_enabled {
            self.start_folder_size_scan();
            self.start_current_dir_total_size_scan();
        }
        self.start_recursive_mtime_scan();
        self.request_notes_for_current_dir_once();
        Ok(())
    }

    fn notes_file_path(dir: &PathBuf) -> PathBuf {
        dir.join(".sb")
    }

    fn escape_note_field(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        for ch in input.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '\t' => out.push_str("\\t"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                _ => out.push(ch),
            }
        }
        out
    }

    fn unescape_note_field(input: &str) -> Option<String> {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars();
        while let Some(ch) = chars.next() {
            if ch != '\\' {
                out.push(ch);
                continue;
            }

            let esc = chars.next()?;
            match esc {
                '\\' => out.push('\\'),
                't' => out.push('\t'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                _ => return None,
            }
        }
        Some(out)
    }

    fn load_notes_map_for_dir(dir: &PathBuf) -> HashMap<String, String> {
        let path = Self::notes_file_path(dir);
        let Ok(raw) = fs::read_to_string(path) else {
            return HashMap::new();
        };

        let mut notes = HashMap::new();
        for line in raw.lines() {
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, '\t');
            let Some(name_raw) = parts.next() else {
                continue;
            };
            let Some(note_raw) = parts.next() else {
                continue;
            };
            let Some(name) = Self::unescape_note_field(name_raw) else {
                continue;
            };
            let Some(note) = Self::unescape_note_field(note_raw) else {
                continue;
            };
            if name.is_empty() || note.trim().is_empty() {
                continue;
            }
            notes.insert(name, note);
        }
        notes
    }

    fn request_notes_for_current_dir_once(&mut self) {
        if self.notes_rx.is_some() {
            return;
        }
        if self
            .notes_loaded_for
            .as_ref()
            .map(|p| p == &self.current_dir)
            .unwrap_or(false)
        {
            return;
        }

        self.notes_scan_id = self.notes_scan_id.wrapping_add(1);
        let scan_id = self.notes_scan_id;
        let dir = self.current_dir.clone();
        self.notes_by_name.clear();
        let (tx, rx) = mpsc::channel();
        self.notes_rx = Some(rx);

        thread::spawn(move || {
            let notes = App::load_notes_map_for_dir(&dir);
            let _ = tx.send(NotesLoadMsg::Finished(scan_id, dir, notes));
        });
    }

    fn pump_notes_progress(&mut self) {
        let Some(rx) = self.notes_rx.take() else {
            return;
        };

        let mut keep_rx = true;
        loop {
            match rx.try_recv() {
                Ok(NotesLoadMsg::Finished(scan_id, path, notes)) => {
                    if scan_id == self.notes_scan_id && path == self.current_dir {
                        self.notes_by_name = notes;
                        self.notes_loaded_for = Some(path);
                        keep_rx = false;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    keep_rx = false;
                    break;
                }
            }
        }

        if keep_rx {
            self.notes_rx = Some(rx);
        }
    }

    fn selected_note_targets(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if !self.marked_indices.is_empty() {
            for idx in &self.marked_indices {
                if let Some(entry) = self.entries.get(*idx) {
                    out.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        } else if let Some(entry) = self.entries.get(self.selected_index) {
            out.push(entry.file_name().to_string_lossy().into_owned());
        }
        out.sort();
        out.dedup();
        out
    }

    fn begin_note_edit(&mut self) {
        let targets = self.selected_note_targets();
        if targets.is_empty() {
            self.set_status("no selected item");
            return;
        }

        let initial = if targets.len() == 1 {
            self.notes_by_name
                .get(&targets[0])
                .cloned()
                .unwrap_or_default()
        } else {
            String::new()
        };

        self.note_edit_targets = targets;
        self.begin_input_edit(AppMode::NoteEditing, initial);
    }

    fn current_dir_entry_names_all(&self) -> HashSet<String> {
        let mut names = HashSet::new();
        let Ok(entries) = fs::read_dir(&self.current_dir) else {
            return names;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".sb" {
                continue;
            }
            names.insert(name);
        }
        names
    }

    fn save_notes_for_current_dir(&mut self) -> io::Result<()> {
        let existing = self.current_dir_entry_names_all();
        self.notes_by_name
            .retain(|name, note| existing.contains(name) && !note.trim().is_empty());

        let notes_path = Self::notes_file_path(&self.current_dir);
        if self.notes_by_name.is_empty() {
            match fs::remove_file(notes_path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            self.notes_loaded_for = Some(self.current_dir.clone());
            return Ok(());
        }

        let mut keys: Vec<String> = self.notes_by_name.keys().cloned().collect();
        keys.sort();
        let mut lines: Vec<String> = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(note) = self.notes_by_name.get(&key) {
                lines.push(format!(
                    "{}\t{}",
                    Self::escape_note_field(&key),
                    Self::escape_note_field(note)
                ));
            }
        }

        let mut payload = lines.join("\n");
        payload.push('\n');

        let tmp_path = self
            .current_dir
            .join(format!(".sb.tmp.{}", self.notes_scan_id));
        fs::write(&tmp_path, payload)?;
        fs::rename(&tmp_path, &notes_path)?;
        self.notes_loaded_for = Some(self.current_dir.clone());
        Ok(())
    }

    fn commit_note_edit(&mut self) {
        if self.note_edit_targets.is_empty() {
            self.clear_input_edit();
            self.mode = AppMode::Browsing;
            return;
        }

        let note = self.input_buffer.clone();
        let is_empty = note.trim().is_empty();
        for target in &self.note_edit_targets {
            if is_empty {
                self.notes_by_name.remove(target);
            } else {
                self.notes_by_name.insert(target.clone(), note.clone());
            }
        }

        let count = self.note_edit_targets.len();
        match self.save_notes_for_current_dir() {
            Ok(()) => {
                if is_empty {
                    self.set_status(format!("cleared note for {} item(s)", count));
                } else {
                    self.set_status(format!("saved note for {} item(s)", count));
                }
            }
            Err(e) => {
                self.set_status(format!("save note failed: {}", e));
            }
        }

        self.note_edit_targets.clear();
        self.clear_input_edit();
        self.mode = AppMode::Browsing;
    }

    fn delete_targets(&self) -> Vec<PathBuf> {
        if !self.marked_indices.is_empty() {
            self.entries
                .iter()
                .enumerate()
                .filter(|(i, _)| self.marked_indices.contains(i))
                .map(|(_, e)| e.path())
                .collect()
        } else {
            self.entries
                .get(self.selected_index)
                .map(|e| e.path())
                .into_iter()
                .collect()
        }
    }

    fn begin_confirm_delete(&mut self) {
        self.confirm_delete_scroll_offset = 0;
        self.confirm_delete_max_offset = 0;
        self.confirm_delete_button_focus = 0;
        self.mode = AppMode::ConfirmDelete;
    }

    fn confirm_delete_selected_targets(&mut self) {
        let to_delete = self.delete_targets();
        for path in to_delete {
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
        self.mode = AppMode::Browsing;
        self.refresh_entries_or_status();
    }

    fn archive_targets(&self) -> Vec<PathBuf> {
        self.delete_targets()
    }

    fn toggle_executable_permissions(&mut self) {
        #[cfg(not(unix))]
        {
            self.set_status("executable permission toggle is only supported on Unix");
            return;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let targets = self.delete_targets();
            if targets.is_empty() {
                self.set_status("no selected item");
                return;
            }

            let mut changed = 0usize;
            let mut skipped_dirs = 0usize;
            let mut failed = 0usize;

            for path in targets {
                let meta = match fs::metadata(&path) {
                    Ok(m) => m,
                    Err(_) => {
                        failed += 1;
                        continue;
                    }
                };

                if meta.is_dir() {
                    skipped_dirs += 1;
                    continue;
                }

                let mode = meta.permissions().mode();
                let new_mode = if mode & 0o111 != 0 {
                    mode & !0o111
                } else {
                    mode | 0o111
                };

                let mut perms = meta.permissions();
                perms.set_mode(new_mode);
                if fs::set_permissions(&path, perms).is_ok() {
                    changed += 1;
                } else {
                    failed += 1;
                }
            }

            if changed > 0 {
                self.refresh_entries_or_status();
            }

            if changed > 0 && failed == 0 && skipped_dirs == 0 {
                self.set_status(format!("toggled executable bit on {} file(s)", changed));
            } else if changed > 0 {
                self.set_status(format!(
                    "toggled {} file(s), skipped {} dir(s), {} failed",
                    changed, skipped_dirs, failed
                ));
            } else if skipped_dirs > 0 && failed == 0 {
                self.set_status("no files changed (directories skipped)");
            } else {
                self.set_status("failed to toggle executable permissions");
            }
        }
    }

    fn drop_to_shell(&mut self) -> io::Result<()> {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        execute!(io::stdout(), Show)?;
        let _ = Command::new(&shell)
            .current_dir(&self.current_dir)
            .status();
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;
        execute!(io::stdout(), Hide)?;
        self.set_status("returned from shell");
        self.refresh_entries_or_status();
        Ok(())
    }

    fn open_path_in_view_mode(path: &PathBuf, use_pager: bool) -> io::Result<()> {
        if Self::is_image_file(path) {
            if Self::integration_probe("viu").0 {
                let _ = Command::new("viu").arg(path).status();
                return Ok(());
            }
            if Self::integration_probe("chafa").0 {
                let _ = Command::new("chafa").arg(path).status();
                return Ok(());
            }
        }

        if Self::is_markdown_file(path) && Self::integration_probe("glow").0 {
            let mut cmd = Command::new("glow");
            if use_pager {
                cmd.arg("-p");
            }
            let _ = cmd.arg(path).status();
            return Ok(());
        }

        if Self::is_mermaid_file(path) && Self::integration_probe("mmdflux").0 {
            if use_pager {
                if let Ok(mut child) = Command::new("mmdflux")
                    .arg(path)
                    .stdout(Stdio::piped())
                    .spawn()
                {
                    if let Some(mmd_out) = child.stdout.take() {
                        let _ = Command::new("less").args(["-R"]).stdin(mmd_out).status();
                    }
                    let _ = child.wait();
                }
            } else {
                let _ = Command::new("mmdflux").arg(path).status();
            }
            return Ok(());
        }

        if Self::is_html_file(path) && Self::integration_probe("links").0 {
            let _ = Command::new("links").arg(path).status();
            return Ok(());
        }

        if Self::is_json_file(path) && Self::integration_probe("jnv").0 {
            let _ = Command::new("jnv").arg(path).status();
            return Ok(());
        }

        if Self::is_delimited_text_file(path) && Self::integration_probe("csvlens").0 {
            let _ = Command::new("csvlens").arg(path).status();
            return Ok(());
        }

        if Self::is_audio_file(path) && Self::integration_probe("sox").0 {
            if Self::integration_probe("play").0 {
                let _ = Command::new("play")
                    .arg(path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            } else {
                let _ = Command::new("sox")
                    .arg(path)
                    .arg("-d")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            return Ok(());
        }

        if Self::is_pdf_file(path) && Self::integration_probe("pdftotext").0 {
            if use_pager {
                let mut shown = false;
                if let Ok(mut child) = Command::new("pdftotext")
                    .args(["-layout", "-nopgbrk"])
                    .arg(path)
                    .arg("-")
                    .stdout(Stdio::piped())
                    .spawn()
                {
                    if let Some(pdf_text) = child.stdout.take() {
                        shown = Command::new("less")
                            .args(["-R"])
                            .stdin(pdf_text)
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false);
                    }
                    let _ = child.wait();
                }
                if !shown {
                    let _ = Command::new("less")
                        .args(["-R", path.to_str().unwrap_or_default()])
                        .status();
                }
            } else {
                let _ = Command::new("pdftotext")
                    .args(["-layout", "-nopgbrk"])
                    .arg(path)
                    .arg("-")
                    .status();
            }
            return Ok(());
        }

        if Self::is_cast_file(path) && Self::integration_probe("asciinema").0 {
            let _ = Command::new("asciinema").args(["play", "-i"]).arg(path).status();
            return Ok(());
        }

        if Self::is_binary_file(path) && Self::integration_probe("hexyl").0 {
            if use_pager {
                if let Ok(child) = Command::new("hexyl")
                    .arg(path)
                    .stdout(Stdio::piped())
                    .spawn()
                {
                    let _ = Command::new("less")
                        .args(["-R"])
                        .stdin(child.stdout.unwrap())
                        .status();
                    return Ok(());
                }
            } else {
                let _ = Command::new("hexyl").arg(path).status();
                return Ok(());
            }
        }

        if Self::integration_probe("bat").0 {
            let bat_cmd = Self::bat_tool().unwrap_or_else(|| "bat".to_string());
            let paging = if use_pager { "always" } else { "never" };
            let _ = Command::new(bat_cmd)
                .args([&format!("--paging={}", paging), "--style=full", "--color=always"])
                .arg(path)
                .status();
            return Ok(());
        }

        if use_pager {
            let _ = Command::new("less")
                .args(["-R", path.to_str().unwrap_or_default()])
                .status();
        } else {
            let _ = Command::new("cat")
                .arg(path)
                .status();
        }
        Ok(())
    }

    fn sqlite_quote_ident(name: &str) -> String {
        format!("\"{}\"", name.replace('"', "\"\""))
    }

    fn sqlite_query_rows(path: &PathBuf, sql: &str, with_header: bool) -> io::Result<Vec<Vec<String>>> {
        let mut cmd = Command::new("sqlite3");
        cmd.args(["-readonly", "-batch", "-separator", "\x1f", "-nullvalue", "NULL"]);
        if with_header {
            cmd.arg("-header");
        } else {
            cmd.arg("-noheader");
        }
        cmd.arg(path);
        cmd.arg(sql);
        let out = cmd.output()?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let msg = if stderr.is_empty() {
                "sqlite3 query failed".to_string()
            } else {
                format!("sqlite3 query failed: {}", stderr)
            };
            return Err(io::Error::other(msg));
        }

        let stdout = String::from_utf8_lossy(&out.stdout);
        let rows = stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.split('\x1f').map(|s| s.to_string()).collect::<Vec<String>>())
            .collect::<Vec<Vec<String>>>();
        Ok(rows)
    }

    fn sqlite_query_box_lines(path: &PathBuf, sql: &str) -> io::Result<Vec<String>> {
        let out = Command::new("sqlite3")
            .args(["-readonly", "-batch", "-header", "-box"])
            .arg(path)
            .arg(sql)
            .output()?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let msg = if stderr.is_empty() {
                "sqlite3 query failed".to_string()
            } else {
                format!("sqlite3 query failed: {}", stderr)
            };
            return Err(io::Error::other(msg));
        }

        let stdout = String::from_utf8_lossy(&out.stdout);
        Ok(stdout.lines().map(|line| line.to_string()).collect())
    }

    fn sqlite_list_tables(path: &PathBuf) -> io::Result<Vec<String>> {
        let rows = Self::sqlite_query_rows(
            path,
            "SELECT name FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name;",
            false,
        )?;
        let mut tables = rows
            .into_iter()
            .filter_map(|row| row.first().cloned())
            .filter(|name| !name.trim().is_empty())
            .collect::<Vec<String>>();
        tables.sort();
        tables.dedup();
        Ok(tables)
    }

    fn refresh_sqlite_preview_rows(&mut self) {
        self.db_preview_output_lines.clear();
        self.db_preview_error = None;

        let Some(path) = self.db_preview_path.clone() else {
            return;
        };
        let Some(table_name) = self.db_preview_tables.get(self.db_preview_selected).cloned() else {
            return;
        };

        let quoted_table = Self::sqlite_quote_ident(&table_name);
        let sql = format!("SELECT * FROM {} LIMIT {};", quoted_table, self.db_preview_row_limit);
        match Self::sqlite_query_box_lines(&path, &sql) {
            Ok(lines) => {
                self.db_preview_output_lines = lines;
            }
            Err(err) => {
                self.db_preview_error = Some(err.to_string());
            }
        }
    }

    fn begin_sqlite_preview(&mut self, db_path: PathBuf) {
        self.db_preview_path = Some(db_path.clone());
        self.db_preview_tables.clear();
        self.db_preview_selected = 0;
        self.db_preview_output_lines.clear();
        self.db_preview_error = None;

        match Self::sqlite_list_tables(&db_path) {
            Ok(tables) => {
                self.db_preview_tables = tables;
                if self.db_preview_tables.is_empty() {
                    self.db_preview_error = Some("No tables/views found in this database".to_string());
                } else {
                    self.refresh_sqlite_preview_rows();
                }
            }
            Err(err) => {
                self.db_preview_error = Some(err.to_string());
            }
        }

        self.mode = AppMode::DbPreview;
    }

    fn switch_sqlite_preview_table(&mut self, delta: isize) {
        if self.db_preview_tables.is_empty() {
            return;
        }
        let last = self.db_preview_tables.len().saturating_sub(1) as isize;
        let next = (self.db_preview_selected as isize + delta).clamp(0, last) as usize;
        if next != self.db_preview_selected {
            self.db_preview_selected = next;
            self.refresh_sqlite_preview_rows();
        }
    }

    fn shell_single_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }

    fn open_split_shell_with_less(&mut self) -> io::Result<()> {
        if !self.integration_active("tmux") {
            self.set_status("tmux not found in PATH");
            return Ok(());
        }

        let Some(entry) = self.entries.get(self.selected_index) else {
            self.set_status("no selected item");
            return Ok(());
        };

        let selected_path = entry.path();
        if selected_path.is_dir() {
            self.set_status("split shell preview works on files only");
            return Ok(());
        }

        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let current_dir = self.current_dir.to_string_lossy().into_owned();
        let selected_file = selected_path.to_string_lossy().into_owned();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let session_name = format!("sbrs_i_{}_{}", std::process::id(), stamp % 1_000_000_000);

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        execute!(io::stdout(), Show)?;

        let tmux_result = (|| -> io::Result<()> {
            let left_cmd = format!(
                "{} -i; tmux kill-session -t {} >/dev/null 2>&1",
                Self::shell_single_quote(&shell),
                Self::shell_single_quote(&session_name)
            );
            let right_cmd = format!(
                "less -R -- {}",
                Self::shell_single_quote(&selected_file)
            );
            let target_window = format!("{}:0", session_name);
            let target_left = format!("{}:0.0", session_name);

            let create_status = Command::new("tmux")
                .args(["new-session", "-d", "-s", &session_name, "-c", &current_dir, &left_cmd])
                .status()?;
            if !create_status.success() {
                return Err(io::Error::other("tmux new-session failed"));
            }

            let split_status = Command::new("tmux")
                .args(["split-window", "-h", "-p", "30", "-t", &target_window, "-c", &current_dir, &right_cmd])
                .status()?;
            if !split_status.success() {
                let _ = Command::new("tmux").args(["kill-session", "-t", &session_name]).status();
                return Err(io::Error::other("tmux split-window failed"));
            }

            let _ = Command::new("tmux")
                .args(["select-pane", "-t", &target_left])
                .status();

            let _ = Command::new("tmux")
                .args(["attach-session", "-t", &session_name])
                .status();

            let _ = Command::new("tmux")
                .args(["kill-session", "-t", &session_name])
                .status();

            Ok(())
        })();

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;
        execute!(io::stdout(), Hide)?;

        match tmux_result {
            Ok(()) => self.set_status("returned from split shell"),
            Err(e) => self.set_status(format!("split shell failed: {}", e)),
        }
        self.refresh_entries_or_status();
        Ok(())
    }

    fn open_split_shell_with_editor(&mut self) -> io::Result<()> {
        if !self.integration_active("tmux") {
            self.set_status("tmux not found in PATH");
            return Ok(());
        }

        let Some(entry) = self.entries.get(self.selected_index) else {
            self.set_status("no selected item");
            return Ok(());
        };

        let selected_path = entry.path();
        if selected_path.is_dir() {
            self.set_status("split shell edit works on files only");
            return Ok(());
        }

        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let editor = env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
        let current_dir = self.current_dir.to_string_lossy().into_owned();
        let selected_file = selected_path.to_string_lossy().into_owned();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let session_name = format!("sbrs_E_{}_{}", std::process::id(), stamp % 1_000_000_000);

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        execute!(io::stdout(), Show)?;

        let tmux_result = (|| -> io::Result<()> {
            let left_cmd = format!(
                "{} -i; tmux kill-session -t {} >/dev/null 2>&1",
                Self::shell_single_quote(&shell),
                Self::shell_single_quote(&session_name)
            );
            let right_cmd = format!(
                "{} -- {}",
                editor,
                Self::shell_single_quote(&selected_file)
            );
            let target_window = format!("{}:0", session_name);
            let target_left = format!("{}:0.0", session_name);

            let create_status = Command::new("tmux")
                .args(["new-session", "-d", "-s", &session_name, "-c", &current_dir, &left_cmd])
                .status()?;
            if !create_status.success() {
                return Err(io::Error::other("tmux new-session failed"));
            }

            let split_status = Command::new("tmux")
                .args(["split-window", "-h", "-p", "30", "-t", &target_window, "-c", &current_dir, &right_cmd])
                .status()?;
            if !split_status.success() {
                let _ = Command::new("tmux").args(["kill-session", "-t", &session_name]).status();
                return Err(io::Error::other("tmux split-window failed"));
            }

            let _ = Command::new("tmux")
                .args(["select-pane", "-t", &target_left])
                .status();

            let _ = Command::new("tmux")
                .args(["attach-session", "-t", &session_name])
                .status();

            let _ = Command::new("tmux")
                .args(["kill-session", "-t", &session_name])
                .status();

            Ok(())
        })();

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;
        execute!(io::stdout(), Hide)?;

        match tmux_result {
            Ok(()) => self.set_status("returned from split shell"),
            Err(e) => self.set_status(format!("split shell failed: {}", e)),
        }
        self.refresh_entries_or_status();
        Ok(())
    }

    fn run_shell_command_and_wait_key(&mut self, command: &str) -> io::Result<()> {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            self.set_status("command cancelled");
            return Ok(());
        }

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;

        println!("$ {}", trimmed);
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = Command::new(&shell);
        // Non-interactive mode avoids shell job-control side effects that can
        // suspend sbrs when returning from the command runner.
        cmd.args(["-c", trimmed]);

        let status = cmd.current_dir(&self.current_dir).status();

        match status {
            Ok(s) => {
                if let Some(code) = s.code() {
                    println!("\n[exit code: {}]", code);
                } else {
                    println!("\n[process terminated by signal]");
                }
            }
            Err(e) => {
                println!("\n[failed to execute command: {}]", e);
            }
        }

        println!("\nPress Enter to return to sbrs...");
        let _ = io::stdout().flush();
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;

        self.set_status(format!("ran command: {}", trimmed));
        self.refresh_entries_or_status();
        Ok(())
    }

    fn parse_git_commit_message(raw: &str) -> (String, bool) {
        let mut amend = false;
        let mut parts: Vec<&str> = Vec::new();
        for token in raw.split_whitespace() {
            if token == "--amend" {
                amend = true;
            } else {
                parts.push(token);
            }
        }
        (parts.join(" "), amend)
    }

    fn latest_git_tag(&self) -> Option<String> {
        let out = Command::new("git")
            .args(["describe", "--tags", "--abbrev=0"])
            .current_dir(&self.current_dir)
            .output()
            .ok()?;

        if !out.status.success() {
            return None;
        }

        let tag = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if tag.is_empty() {
            None
        } else {
            Some(tag)
        }
    }

    fn preview_git_diff_and_confirm_commit(&mut self) -> io::Result<bool> {
        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;

        let delta_available = self.integration_active("delta");
        if delta_available {
            println!("$ git -c core.pager=delta -c delta.side-by-side=true -c delta.features=side-by-side diff");
            let _ = Command::new("git")
                .args([
                    "-c",
                    "core.pager=delta",
                    "-c",
                    "delta.side-by-side=true",
                    "-c",
                    "delta.features=side-by-side",
                    "diff",
                ])
                .current_dir(&self.current_dir)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();
        } else {
            println!("$ git -c color.ui=always diff");
            let _ = Command::new("git")
                .args(["-c", "color.ui=always", "diff"])
                .current_dir(&self.current_dir)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();
            println!("\nTip: install delta for side-by-side colored diff preview.");
        }

        println!("\n$ git status");
        let _ = Command::new("git")
            .arg("status")
            .current_dir(&self.current_dir)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();

        print!("\nDo you really want to commit these changes? [y/N]: ");
        let _ = io::stdout().flush();
        let mut answer = String::new();
        let _ = io::stdin().read_line(&mut answer);
        let confirmed = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;

        Ok(confirmed)
    }

    fn run_git_commit_and_push(&mut self, commit_message: &str, amend: bool) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;

        let mut failed_step: Option<String> = None;
        let mut push_forced = false;
        let run_step = |args: &[&str], dir: &PathBuf| -> io::Result<bool> {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()?;
            Ok(status.success())
        };

        println!("$ git add --all");
        if !run_step(&["add", "--all"], &self.current_dir)? {
            failed_step = Some("git add --all failed".to_string());
        }

        if failed_step.is_none() {
            if amend {
                println!("$ git commit -m \"{}\" --amend", commit_message);
                if !run_step(&["commit", "-m", commit_message, "--amend"], &self.current_dir)? {
                    failed_step = Some("git commit --amend failed".to_string());
                }
            } else {
                println!("$ git commit -m \"{}\"", commit_message);
                if !run_step(&["commit", "-m", commit_message], &self.current_dir)? {
                    failed_step = Some("git commit failed".to_string());
                }
            }
        }

        if failed_step.is_none() {
            if amend {
                println!("$ git push origin HEAD -f");
                push_forced = true;
                if !run_step(&["push", "origin", "HEAD", "-f"], &self.current_dir)? {
                    failed_step = Some("git push -f failed".to_string());
                }
            } else {
                println!("$ git push origin HEAD");
                if !run_step(&["push", "origin", "HEAD"], &self.current_dir)? {
                    failed_step = Some("git push failed".to_string());
                }
            }
        }

        let mut tag_requested = false;
        if failed_step.is_none() {
            println!("\nPress any key to return to sbrs, or press 't' to create+push a tag...");
            let _ = io::stdout().flush();
            enable_raw_mode()?;
            loop {
                if let Event::Key(key) = event::read()? {
                    tag_requested = matches!(key.code, KeyCode::Char('t') | KeyCode::Char('T'));
                    break;
                }
            }
            disable_raw_mode()?;
        } else {
            println!("\nPress any key to return to sbrs...");
            let _ = io::stdout().flush();
            enable_raw_mode()?;
            loop {
                if let Event::Key(_) = event::read()? {
                    break;
                }
            }
            disable_raw_mode()?;
        }

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;

        if let Some(step) = failed_step {
            self.set_status(step);
        } else if push_forced {
            self.set_status("amend commit pushed with -f");
            if tag_requested {
                let prefill = self
                    .latest_git_tag()
                    .unwrap_or_else(|| "v0.1.0".to_string());
                self.begin_input_edit(AppMode::GitTagInput, prefill);
                self.set_status("edit tag and press Enter to create+push (Esc=cancel)");
            }
        } else {
            self.set_status("commit pushed");
            if tag_requested {
                let prefill = self
                    .latest_git_tag()
                    .unwrap_or_else(|| "v0.1.0".to_string());
                self.begin_input_edit(AppMode::GitTagInput, prefill);
                self.set_status("edit tag and press Enter to create+push (Esc=cancel)");
            }
        }

        self.refresh_entries_or_status();
        self.git_info_cache = None;
        self.request_git_info_for_current_dir_once();
        Ok(())
    }

    fn run_git_tag_and_push(&mut self, tag: &str) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;

        let run_step = |args: &[&str], dir: &PathBuf| -> io::Result<bool> {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()?;
            Ok(status.success())
        };

        let mut failed_step: Option<String> = None;

        println!("$ git tag {}", tag);
        if !run_step(&["tag", tag], &self.current_dir)? {
            failed_step = Some("git tag failed".to_string());
        }

        if failed_step.is_none() {
            println!("$ git push origin {}", tag);
            if !run_step(&["push", "origin", tag], &self.current_dir)? {
                failed_step = Some("git push tag failed".to_string());
            }
        }

        println!("\nPress Enter to return to sbrs...");
        let _ = io::stdout().flush();
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;

        if let Some(step) = failed_step {
            self.set_status(step);
        } else {
            self.set_status(format!("tag pushed: {}", tag));
        }

        self.refresh_entries_or_status();
        self.git_info_cache = None;
        self.request_git_info_for_current_dir_once();
        Ok(())
    }

    fn run_zip_action(&mut self) {
        if self.archive_rx.is_some() {
            self.set_status("archive creation already in progress");
            return;
        }

        let targets = self.archive_targets();
        if targets.is_empty() {
            self.set_status("no selected item");
            return;
        }

        let all_archives = targets.iter().all(Self::is_supported_archive);

        if all_archives {
            if targets.iter().any(|p| !self.can_extract_archive(p)) {
                self.set_status("missing extractor for one or more selected archives");
                return;
            }

            self.archive_extract_targets = targets;
            self.mode = AppMode::ConfirmExtract;
            self.set_status("confirm extraction: press y to continue");
            return;
        }

        if !self.integration_enabled("zip") || Self::integration_probe("zip").0 == false {
            self.set_status("zip not found in PATH");
            return;
        }

        let base_name = if targets.len() == 1 {
            targets[0]
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "archive".to_string())
        } else {
            "archive".to_string()
        };
        let mut archive_name = format!("{}.zip", base_name);
        let mut n = 2usize;
        while self.current_dir.join(&archive_name).exists() {
            archive_name = format!("{}-{}.zip", base_name, n);
            n += 1;
        }

        self.archive_create_targets = targets;
        self.begin_input_edit(AppMode::ArchiveCreate, archive_name);
        self.set_status("confirm archive name and press Enter");
    }

    fn run_delta_compare(&mut self) -> io::Result<()> {
        if !self.integration_active("delta") {
            self.set_status("delta not found in PATH");
            return Ok(());
        }

        if self.marked_indices.len() != 1 {
            self.set_status("mark exactly one file, then move cursor to another file and press C");
            return Ok(());
        }

        let marked_idx = *self.marked_indices.iter().next().unwrap_or(&self.selected_index);
        let Some(marked_entry) = self.entries.get(marked_idx) else {
            self.set_status("marked file not found");
            return Ok(());
        };
        let Some(cursor_entry) = self.entries.get(self.selected_index) else {
            self.set_status("cursor file not found");
            return Ok(());
        };

        let marked_path = marked_entry.path();
        let cursor_path = cursor_entry.path();

        if marked_path == cursor_path {
            self.set_status("choose a different cursor file to compare");
            return Ok(());
        }
        if marked_path.is_dir() || cursor_path.is_dir() {
            self.set_status("delta compare works on files only");
            return Ok(());
        }

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        let _ = Command::new("delta")
            .arg("--side-by-side")
            .arg("--paging=always")
            .arg(&marked_path)
            .arg(&cursor_path)
            .status();
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        enable_raw_mode()?;

        let left = marked_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| marked_path.to_string_lossy().into_owned());
        let right = cursor_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| cursor_path.to_string_lossy().into_owned());
        self.set_status(format!("delta compared: {} vs {}", left, right));
        Ok(())
    }

    fn open_selected_with_default_app(&mut self) -> io::Result<()> {
        let Some(entry) = self.entries.get(self.selected_index) else {
            self.set_status("no selected item");
            return Ok(());
        };

        let path = entry.path();
        let display_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        let opened = if Self::integration_probe("xdg-open").0 {
            Command::new("xdg-open")
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_ok()
        } else if Self::integration_probe("gio").0 {
            Command::new("gio")
                .arg("open")
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_ok()
        } else {
            false
        };

        if opened {
            self.set_status(format!("opened with default app: {}", display_name));
        } else {
            self.set_status("no default opener found (tried xdg-open, gio open)");
        }

        Ok(())
    }

    fn open_todo_file_in_editor(&mut self) -> io::Result<()> {
        let home = match env::var("HOME") {
            Ok(v) => v,
            Err(_) => {
                self.set_status("HOME is not set");
                return Ok(());
            }
        };

        let todo_path = PathBuf::from(home).join(".todo");
        if !todo_path.exists() {
            if let Err(e) = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&todo_path)
            {
                self.set_status(format!("failed to create ~/.todo: {}", e));
                return Ok(());
            }
        }

        let editor = env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        execute!(io::stdout(), Show)?;
        let _ = Command::new(editor).arg(&todo_path).status();
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), Hide)?;
        enable_raw_mode()?;
        self.refresh_entries_or_status();
        self.set_status("opened ~/.todo");
        Ok(())
    }

    fn create_archive_from_input(&mut self) {
        if self.archive_rx.is_some() {
            self.set_status("archive creation already in progress");
            return;
        }

        let mut archive_name = self.input_buffer.trim().to_string();
        if archive_name.is_empty() {
            self.set_status("archive name cannot be empty");
            return;
        }
        if !archive_name.to_lowercase().ends_with(".zip") {
            archive_name.push_str(".zip");
        }

        let targets = self.archive_create_targets.clone();
        if targets.is_empty() {
            self.mode = AppMode::Browsing;
            self.clear_input_edit();
            self.set_status("nothing to archive");
            return;
        }

        if self.current_dir.join(&archive_name).exists() {
            self.set_status("archive already exists: choose another name");
            return;
        }

        let mut item_names: Vec<String> = Vec::new();
        for t in &targets {
            if let Some(name) = t.file_name() {
                item_names.push(name.to_string_lossy().into_owned());
            }
        }
        if item_names.is_empty() {
            self.mode = AppMode::Browsing;
            self.archive_create_targets.clear();
            self.clear_input_edit();
            self.set_status("nothing to archive");
            return;
        }

        self.mode = AppMode::Browsing;
        let targets = std::mem::take(&mut self.archive_create_targets);
        self.clear_input_edit();
        self.start_archive_job(archive_name, targets);
    }

    fn extract_archives_confirmed(&mut self) {
        let targets = std::mem::take(&mut self.archive_extract_targets);
        if targets.is_empty() {
            self.set_status("no archives selected");
            return;
        }

        let mut ok_count = 0usize;
        let mut fail_count = 0usize;
        for archive in &targets {
            let base = archive
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "extracted".to_string());

            let mut out_dir = self.current_dir.join(&base);
            let mut n = 2usize;
            while out_dir.exists() {
                out_dir = self.current_dir.join(format!("{}-{}", base, n));
                n += 1;
            }

            let _ = fs::create_dir_all(&out_dir);
            let ok = match Self::archive_kind(archive) {
                Some(ArchiveKind::Zip) => Command::new("unzip")
                    .args(["-q"])
                    .arg(archive)
                    .args(["-d"])
                    .arg(&out_dir)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false),
                Some(ArchiveKind::Tar) => Command::new("tar")
                    .arg("-xf")
                    .arg(archive)
                    .arg("-C")
                    .arg(&out_dir)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false),
                Some(ArchiveKind::SevenZip) => {
                    if let Some(tool) = Self::seven_zip_tool() {
                        Command::new(tool)
                            .arg("x")
                            .arg("-y")
                            .arg(format!("-o{}", out_dir.to_string_lossy()))
                            .arg(archive)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false)
                    } else {
                        false
                    }
                }
                Some(ArchiveKind::Rar) => {
                    if let Some(tool) = Self::rar_tool() {
                        Command::new(tool)
                            .arg("x")
                            .arg("-o+")
                            .arg(archive)
                            .arg(&out_dir)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false)
                    } else {
                        false
                    }
                }
                None => false,
            };
            if ok {
                ok_count += 1;
            } else {
                fail_count += 1;
            }
        }

        self.refresh_entries_or_status();
        if fail_count == 0 {
            self.set_status(format!("extracted {} archive(s)", ok_count));
        } else {
            self.set_status(format!("extract finished: {} ok, {} failed", ok_count, fail_count));
        }
    }

    fn update_archive_status(&mut self) {
        if self.archive_name.is_empty() {
            return;
        }

        let total = self.archive_total_bytes;
        let done = self.archive_done_bytes;
        let scanning = total == 0 && done == 0;
        let display_total = total.max(done).max(1);
        let percent = if total == 0 {
            0.0
        } else {
            (done.min(display_total) as f64 * 100.0) / display_total as f64
        };

        let bar_len = 20usize;
        let filled = ((percent / 100.0) * bar_len as f64).round() as usize;
        let filled = filled.min(bar_len);
        let bar = format!("{}{}", "#".repeat(filled), "-".repeat(bar_len.saturating_sub(filled)));

        let elapsed = self
            .archive_started_at
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0)
            .max(0.001);
        let speed = done as f64 / elapsed;
        let speed_str = if speed > 0.0 {
            format!("{}/s", Self::format_size(speed as u64))
        } else {
            "-".to_string()
        };

        let eta = if speed > 0.0 && display_total > done {
            let eta_secs = ((display_total - done) as f64 / speed).round() as u64;
            Self::format_eta(eta_secs)
        } else {
            "-".to_string()
        };
        let total_label = if scanning {
            "?".to_string()
        } else {
            Self::format_size(display_total)
        };
        let scan_suffix = if scanning { " scanning size..." } else { "" };

        self.set_status(format!(
            "archive [{}] {:>3.0}% {}/{} {} eta {} {}{}",
            bar,
            percent,
            Self::format_size(done),
            total_label,
            speed_str,
            eta,
            self.archive_name,
            scan_suffix
        ));
    }

    fn start_archive_job(&mut self, archive_name: String, targets: Vec<PathBuf>) {
        let mut item_names: Vec<String> = Vec::new();
        for t in &targets {
            if let Some(name) = t.file_name() {
                item_names.push(name.to_string_lossy().into_owned());
            }
        }
        if item_names.is_empty() {
            self.set_status("nothing to archive");
            return;
        }

        let cwd = self.current_dir.clone();
        let archive_path = cwd.join(&archive_name);
        let (tx, rx) = mpsc::channel();
        self.archive_rx = Some(rx);
        self.archive_total_bytes = 0;
        self.archive_done_bytes = 0;
        self.archive_started_at = Some(Instant::now());
        self.archive_name = archive_name.clone();
        self.update_archive_status();

        thread::spawn(move || {
            let total_bytes = targets
                .iter()
                .filter_map(|p| Self::compute_total_bytes(p).ok())
                .fold(0u64, |acc, v| acc.saturating_add(v));
            let _ = tx.send(ArchiveProgressMsg::TotalBytes(total_bytes));

            let mut cmd = Command::new("zip");
            cmd.arg("-r")
                .arg(&archive_name)
                .args(&item_names)
                .current_dir(&cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            match cmd.spawn() {
                Ok(mut child) => loop {
                    let done = fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0);
                    let _ = tx.send(ArchiveProgressMsg::Progress(done));
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            if status.success() {
                                let _ = tx.send(ArchiveProgressMsg::Finished(Ok(archive_name.clone())));
                            } else {
                                let _ = tx.send(ArchiveProgressMsg::Finished(Err("zip command failed".to_string())));
                            }
                            break;
                        }
                        Ok(None) => {
                            thread::sleep(Duration::from_millis(120));
                        }
                        Err(e) => {
                            let _ = tx.send(ArchiveProgressMsg::Finished(Err(e.to_string())));
                            break;
                        }
                    }
                },
                Err(e) => {
                    let _ = tx.send(ArchiveProgressMsg::Finished(Err(e.to_string())));
                }
            }
        });
    }

    fn pump_archive_progress(&mut self) {
        let Some(rx) = self.archive_rx.take() else {
            return;
        };

        let mut finished: Option<Result<String, String>> = None;
        loop {
            match rx.try_recv() {
                Ok(ArchiveProgressMsg::TotalBytes(total)) => {
                    self.archive_total_bytes = total;
                }
                Ok(ArchiveProgressMsg::Progress(done)) => {
                    self.archive_done_bytes = done;
                }
                Ok(ArchiveProgressMsg::Finished(result)) => {
                    finished = Some(result);
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = Some(Err("archive worker disconnected".to_string()));
                    break;
                }
            }
        }

        if let Some(result) = finished {
            self.archive_started_at = None;
            self.archive_total_bytes = 0;
            self.archive_done_bytes = 0;
            self.archive_name.clear();
            match result {
                Ok(name) => {
                    self.refresh_entries_or_status();
                    self.select_entry_named(&name);
                    self.set_status(format!("archive created: {}", name));
                }
                Err(e) => {
                    self.set_status(format!("archive create failed: {}", e));
                }
            }
        } else {
            self.archive_rx = Some(rx);
            self.update_archive_status();
        }
    }

    fn is_path_inside_remote_mount(&self, path: &PathBuf) -> bool {
        self.ssh_mounts
            .iter()
            .any(|m| path == &m.mount_path || path.starts_with(&m.mount_path))
    }

    fn begin_transfer(&mut self, move_mode: bool) {
        if self.clipboard.is_empty() {
            self.set_status("clipboard is empty");
            return;
        }
        if self.archive_rx.is_some() {
            self.set_status("archive creation in progress");
            return;
        }
        if self.copy_rx.is_some() {
            self.set_status("copy already in progress");
            return;
        }
        self.paste_queue = self.clipboard.iter().cloned().collect();
        self.paste_current_src = None;
        self.paste_move_mode = move_mode;
        self.paste_target_dir = Some(self.current_dir.clone());
        self.paste_total_items = self.clipboard.len();
        self.paste_ok_items = 0;
        self.paste_failed_items = 0;
        let sources = self.clipboard.clone();
        let (tx_total, rx_total) = mpsc::channel();
        self.copy_total_rx = Some(rx_total);
        thread::spawn(move || {
            let total = sources
                .iter()
                .filter_map(|src| App::compute_total_bytes(src).ok())
                .fold(0u64, |acc, v| acc.saturating_add(v));
            let _ = tx_total.send(total);
        });
        self.copy_total_bytes = 0;
        self.copy_done_bytes = 0;
        self.copy_done_before_job = 0;
        self.copy_job_total_bytes = 0;
        self.copy_started_at = Some(Instant::now());
        self.copy_current_src = None;
        self.advance_paste_queue();
    }

    fn pump_copy_total_prescan(&mut self) {
        let Some(rx) = self.copy_total_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(total) => {
                self.copy_total_bytes = total;
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.copy_total_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.copy_total_rx = None;
            }
        }
    }

    fn begin_paste(&mut self) {
        self.begin_transfer(false);
    }

    fn begin_move(&mut self) {
        self.begin_transfer(true);
    }

    fn copy_full_paths_to_system_clipboard(&mut self) {
        let targets = self.delete_targets();
        if targets.is_empty() {
            self.set_status("no selected item");
            return;
        }

        let payload = targets
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");

        for backend in ["wl-copy", "xclip", "xsel", "pbcopy"] {
            if !self.integration_active(backend) {
                continue;
            }

            let mut cmd = match backend {
                "wl-copy" => Command::new("wl-copy"),
                "xclip" => {
                    let mut cmd = Command::new("xclip");
                    cmd.args(["-selection", "clipboard"]);
                    cmd
                }
                "xsel" => {
                    let mut cmd = Command::new("xsel");
                    cmd.args(["--clipboard", "--input"]);
                    cmd
                }
                "pbcopy" => Command::new("pbcopy"),
                _ => continue,
            };

            let mut child = match cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(_) => continue,
            };

            let write_ok = child
                .stdin
                .take()
                .map(|mut stdin| stdin.write_all(payload.as_bytes()).is_ok())
                .unwrap_or(false);
            if !write_ok {
                let _ = child.kill();
                let _ = child.wait();
                continue;
            }

            if child.wait().map(|s| s.success()).unwrap_or(false) {
                self.set_status(format!(
                    "copied {} full path(s) to system clipboard via {}",
                    targets.len(),
                    backend
                ));
                return;
            }
        }

        self.set_status("no clipboard backend available (wl-copy/xclip/xsel/pbcopy)");
    }

    fn read_system_clipboard_text(&self) -> Option<(String, &'static str)> {
        for backend in ["wl-copy", "xclip", "xsel", "pbcopy"] {
            if !self.integration_active(backend) {
                continue;
            }

            let output = match backend {
                "wl-copy" => {
                    if !Self::integration_probe("wl-paste").0 {
                        continue;
                    }
                    Command::new("wl-paste")
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .output()
                }
                "xclip" => Command::new("xclip")
                    .args(["-selection", "clipboard", "-out"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .output(),
                "xsel" => Command::new("xsel")
                    .args(["--clipboard", "--output"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .output(),
                "pbcopy" => {
                    if !Self::integration_probe("pbpaste").0 {
                        continue;
                    }
                    Command::new("pbpaste")
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .output()
                }
                _ => continue,
            };

            if let Ok(out) = output {
                if out.status.success() {
                    return Some((String::from_utf8_lossy(&out.stdout).into_owned(), backend));
                }
            }
        }

        None
    }

    fn write_system_clipboard_text(&self, payload: &str) -> Option<&'static str> {
        for backend in ["wl-copy", "xclip", "xsel", "pbcopy"] {
            if !self.integration_active(backend) {
                continue;
            }

            let mut cmd = match backend {
                "wl-copy" => Command::new("wl-copy"),
                "xclip" => {
                    let mut cmd = Command::new("xclip");
                    cmd.args(["-selection", "clipboard"]);
                    cmd
                }
                "xsel" => {
                    let mut cmd = Command::new("xsel");
                    cmd.args(["--clipboard", "--input"]);
                    cmd
                }
                "pbcopy" => Command::new("pbcopy"),
                _ => continue,
            };

            let mut child = match cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(_) => continue,
            };

            let write_ok = child
                .stdin
                .take()
                .map(|mut stdin| stdin.write_all(payload.as_bytes()).is_ok())
                .unwrap_or(false);
            if !write_ok {
                let _ = child.kill();
                let _ = child.wait();
                continue;
            }

            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return Some(backend);
            }
        }

        None
    }

    fn edit_system_clipboard_via_temp_file(&mut self) -> io::Result<()> {
        let Some((clipboard_text, read_backend)) = self.read_system_clipboard_text() else {
            self.set_status("no clipboard backend available (wl-copy/xclip/xsel/pbcopy)");
            return Ok(());
        };

        let tmp = Self::create_temp_selection_path("sbrs_clipboard_edit");
        if fs::write(&tmp, clipboard_text.as_bytes()).is_err() {
            self.set_status("failed to create temporary clipboard file");
            return Ok(());
        }

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        execute!(io::stdout(), Show)?;

        let edit_result = (|| -> io::Result<String> {
            let _ = Command::new(env::var("EDITOR").unwrap_or_else(|_| "nano".to_string()))
                .arg(&tmp)
                .status();
            fs::read_to_string(&tmp)
        })();

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        enable_raw_mode()?;
        execute!(io::stdout(), Hide)?;

        let _ = fs::remove_file(&tmp);

        match edit_result {
            Ok(updated_text) => {
                if let Some(write_backend) = self.write_system_clipboard_text(&updated_text) {
                    self.set_status(format!(
                        "clipboard updated via {} (read via {})",
                        write_backend, read_backend
                    ));
                } else {
                    self.set_status("failed to write updated clipboard content");
                }
            }
            Err(e) => {
                self.set_status(format!("clipboard edit failed: {}", e));
            }
        }

        Ok(())
    }

    fn copy_path_with_progress(
        src: &PathBuf,
        dest: &PathBuf,
        tx: &Sender<CopyProgressMsg>,
        copied_bytes: &mut u64,
    ) -> io::Result<()> {
        if src.is_dir() {
            fs::create_dir_all(dest)?;
            for child in fs::read_dir(src)? {
                let child = child?;
                let child_src = child.path();
                let child_dest = dest.join(child.file_name());
                Self::copy_path_with_progress(&child_src, &child_dest, tx, copied_bytes)?;
            }
            Ok(())
        } else {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut in_file = fs::File::open(src)?;
            let mut out_file = fs::File::create(dest)?;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = in_file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                out_file.write_all(&buffer[..read])?;
                *copied_bytes = copied_bytes.saturating_add(read as u64);
                let _ = tx.send(CopyProgressMsg::CopiedBytes(*copied_bytes));
            }
            Ok(())
        }
    }

    fn update_copy_status(&mut self) {
        if self.copy_item_name.is_empty() {
            return;
        }
        let total = self.copy_total_bytes;
        let scanning = total == 0 && self.copy_total_rx.is_some();
        let done = if total == 0 {
            self.copy_done_bytes
        } else {
            self.copy_done_bytes.min(total)
        };
        let effective_total = if total == 0 {
            done
                .saturating_add(self.copy_job_total_bytes)
                .max(1)
        } else {
            total.max(1)
        };
        let percent = if total == 0 {
            if self.copy_total_rx.is_some() { 0.0 } else { 100.0 }
        } else {
            (done as f64 * 100.0) / effective_total as f64
        };
        let elapsed_secs = self
            .copy_started_at
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0)
            .max(0.001);
        let bytes_per_sec = done as f64 / elapsed_secs;
        let remaining = if total == 0 { 0 } else { total.saturating_sub(done) };
        let eta_secs = if bytes_per_sec > 0.0 {
            (remaining as f64 / bytes_per_sec) as u64
        } else {
            0
        };
        let bar_width = 14usize;
        let filled = ((percent / 100.0) * bar_width as f64).round() as usize;
        let bar = format!(
            "{}{}",
            "#".repeat(filled.min(bar_width)),
            "-".repeat(bar_width.saturating_sub(filled.min(bar_width)))
        );
        let total_label = if total == 0 && self.copy_total_rx.is_some() {
            "?".to_string()
        } else {
            Self::format_size(effective_total)
        };
        let eta_label = if total == 0 { "-".to_string() } else { Self::format_eta(eta_secs) };
        let scan_suffix = if scanning { " scanning size..." } else { "" };
        let current_idx = (self.paste_ok_items + self.paste_failed_items + 1).min(self.paste_total_items.max(1));
        let scope = if self.copy_from_remote { "remote " } else { "" };
        self.set_status(format!(
            "{}copy [{}] {:>3.0}% {}/{} {}/s eta {} ({}/{}) {}{}",
            scope,
            bar,
            percent,
            Self::format_size(done),
            total_label,
            Self::format_size(bytes_per_sec as u64),
            eta_label,
            current_idx,
            self.paste_total_items,
            self.copy_item_name,
            scan_suffix
        ));
    }

    fn start_copy_job(&mut self, src: PathBuf, dest: PathBuf, display_name: String) {
        let (tx, rx) = mpsc::channel();
        self.copy_rx = Some(rx);
        self.copy_done_before_job = self.copy_done_bytes;
        self.copy_job_total_bytes = 0;
        self.copy_item_name = display_name;
        self.copy_current_src = Some(src.clone());
        self.copy_from_remote = self.is_path_inside_remote_mount(&src);
        self.update_copy_status();

        thread::spawn(move || {
            let total = Self::compute_total_bytes(&src).unwrap_or(0);
            let _ = tx.send(CopyProgressMsg::TotalBytes(total));
            let mut copied = 0u64;
            let result = Self::copy_path_with_progress(&src, &dest, &tx, &mut copied)
                .map_err(|e| e.to_string());
            let _ = tx.send(CopyProgressMsg::Finished(result));
        });
    }

    fn pump_copy_progress(&mut self) {
        let Some(rx) = self.copy_rx.take() else {
            return;
        };

        let mut done_result: Option<Result<(), String>> = None;
        loop {
            match rx.try_recv() {
                Ok(CopyProgressMsg::TotalBytes(total)) => {
                    self.copy_job_total_bytes = total;
                }
                Ok(CopyProgressMsg::CopiedBytes(done)) => {
                    self.copy_done_bytes = self.copy_done_before_job.saturating_add(done);
                }
                Ok(CopyProgressMsg::Finished(result)) => {
                    done_result = Some(result);
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    done_result = Some(Err("copy worker disconnected".to_string()));
                    break;
                }
            }
        }

        if let Some(result) = done_result {
            match result {
                Ok(()) => {
                    if self.paste_move_mode {
                        if let Some(src) = self.copy_current_src.take() {
                            let delete_res = if src.is_dir() {
                                fs::remove_dir_all(&src)
                            } else {
                                fs::remove_file(&src)
                            };
                            if let Err(e) = delete_res {
                                self.paste_failed_items += 1;
                                self.set_status(format!("move cleanup failed for {}: {}", self.copy_item_name, e));
                                self.copy_job_total_bytes = 0;
                                self.copy_done_before_job = self.copy_done_bytes;
                                self.copy_item_name.clear();
                                self.copy_from_remote = false;
                                let _ = self.refresh_entries();
                                self.advance_paste_queue();
                                return;
                            }
                        }
                    }
                    self.paste_ok_items += 1;
                    self.copy_done_bytes = self
                        .copy_done_before_job
                        .saturating_add(self.copy_job_total_bytes);
                }
                Err(e) => {
                    self.paste_failed_items += 1;
                    self.set_status(format!("paste failed for {}: {}", self.copy_item_name, e));
                }
            }
            self.copy_job_total_bytes = 0;
            self.copy_done_before_job = self.copy_done_bytes;
            self.copy_item_name.clear();
            self.copy_current_src = None;
            self.copy_from_remote = false;
            let _ = self.refresh_entries();
            self.advance_paste_queue();
        } else {
            self.copy_rx = Some(rx);
            self.update_copy_status();
        }
    }

    fn format_eta(total_seconds: u64) -> String {
        util::format::format_eta(total_seconds)
    }

    fn advance_paste_queue(&mut self) {
        if self.copy_rx.is_some() {
            return;
        }
        while let Some(src) = self.paste_queue.pop_front() {
            let name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "pasted_item".to_string());
            let target_dir = self
                .paste_target_dir
                .as_ref()
                .cloned()
                .unwrap_or_else(|| self.current_dir.clone());
            let dest = target_dir.join(&name);
            if dest.exists() {
                self.paste_current_src = Some(src);
                self.begin_input_edit(AppMode::PasteRenaming, name);
                self.set_status("target exists: edit name and press Enter");
                return;
            }

            if self.paste_move_mode {
                if fs::rename(&src, &dest).is_ok() {
                    self.paste_ok_items += 1;
                    let _ = self.refresh_entries();
                    continue;
                }
            }

            self.start_copy_job(src, dest, name);
            return;
        }

        self.paste_current_src = None;
        self.paste_move_mode = false;
        self.paste_target_dir = None;
        self.clear_input_edit();
        self.mode = AppMode::Browsing;
        self.copy_started_at = None;
        self.copy_total_rx = None;
        self.copy_current_src = None;
        self.refresh_entries_or_status();
        if self.paste_failed_items == 0 && self.paste_ok_items > 0 {
            self.set_status(format!("transfer complete: {} item", self.paste_ok_items));
        } else if self.paste_failed_items == 0 {
            self.set_status("nothing to transfer");
        } else {
            self.set_status(format!(
                "transfer finished: {} ok, {} failed ({} total)",
                self.paste_ok_items, self.paste_failed_items, self.paste_total_items
            ));
        }
    }

    fn panel_tab_bar_line(active: u8) -> Line<'static> {
        ui::panels::panel_tab_bar_line(active)
    }

    fn panel_tab_hit_test(relative_x: u16) -> Option<u8> {
        ui::panels::panel_tab_hit_test(relative_x)
    }

    fn tabbed_overlay_close_area(popup_area: Rect) -> Rect {
        Rect::new(
            popup_area.x + popup_area.width.saturating_sub(2),
            popup_area.y,
            1,
            1,
        )
    }

    fn primary_content_area(area: Rect) -> Rect {
        Layout::default()
            .constraints([Constraint::Min(3), Constraint::Length(2)])
            .split(area)[0]
    }

    fn tab_overlay_anchor(area: Rect) -> Rect {
        let area = Self::primary_content_area(area);
        let anchor_w = (area.width * 5 / 6).max(50).min(area.width);
        let anchor_h = (area.height * 5 / 6).max(12).min(area.height);
        Rect::new(
            area.x + (area.width.saturating_sub(anchor_w)) / 2,
            area.y + (area.height.saturating_sub(anchor_h)) / 2,
            anchor_w,
            anchor_h,
        )
    }

    fn open_panel_tab(&mut self, tab: u8) {
        if tab == self.panel_tab
            && matches!(
                (tab, self.mode),
                (0, AppMode::Help)
                    | (1, AppMode::InternalSearch)
                    | (2, AppMode::Bookmarks)
                    | (3, AppMode::SshPicker)
                    | (4, AppMode::SortMenu)
                    | (5, AppMode::Integrations)
            )
        {
            return;
        }

        match tab {
            0 => {
                self.panel_tab = 0;
                self.help_scroll_offset = 0;
                self.mode = AppMode::Help;
            }
            1 => {
                self.panel_tab = 1;
                self.start_internal_search();
            }
            2 => {
                self.panel_tab = 2;
                self.mode = AppMode::Bookmarks;
            }
            3 => {
                self.panel_tab = 3;
                self.refresh_remote_entries();
                self.mode = AppMode::SshPicker;
            }
            4 => {
                self.begin_sort_menu();
            }
            5 => {
                self.integration_selected = 0;
                self.refresh_integration_rows_cache();
                self.panel_tab = 5;
                self.mode = AppMode::Integrations;
            }
            _ => {}
        }
    }

    fn close_tabbed_overlay(&mut self) {
        match self.mode {
            AppMode::InternalSearch => {
                self.cancel_internal_search_candidate_scan();
                self.cancel_internal_search_content_request();
                self.clear_input_edit();
                self.mode = AppMode::Browsing;
            }
            AppMode::Help
            | AppMode::Bookmarks
            | AppMode::Integrations
            | AppMode::SortMenu
            | AppMode::SshPicker => {
                self.mode = AppMode::Browsing;
            }
            _ => {}
        }
    }

    fn handle_tab_close_click(&mut self, column: u16, row: u16, area: Rect) -> bool {
        if !matches!(
            self.mode,
            AppMode::InternalSearch
                | AppMode::Help
                | AppMode::Bookmarks
                | AppMode::Integrations
                | AppMode::SortMenu
                | AppMode::SshPicker
        ) {
            return false;
        }

        let popup_area = Self::tab_overlay_anchor(area);
        let close_area = Self::tabbed_overlay_close_area(popup_area);
        if row == close_area.y && column >= close_area.x && column < close_area.x + close_area.width {
            self.close_tabbed_overlay();
            return true;
        }

        false
    }

    fn handle_tab_click(&mut self, column: u16, row: u16, area: Rect) -> bool {
        if !matches!(
            self.mode,
            AppMode::InternalSearch
                | AppMode::Help
                | AppMode::Bookmarks
                | AppMode::Integrations
                | AppMode::SortMenu
                | AppMode::SshPicker
        ) {
            return false;
        }

        let popup_area = Self::tab_overlay_anchor(area);
        if row != popup_area.y || column <= popup_area.x || column >= popup_area.x + popup_area.width.saturating_sub(1) {
            return false;
        }

        let relative_x = column.saturating_sub(popup_area.x + 1);
        if let Some(tab) = Self::panel_tab_hit_test(relative_x) {
            self.open_panel_tab(tab);
            return true;
        }

        false
    }

    fn handle_confirm_delete_click(&mut self, column: u16, row: u16, area: Rect) -> bool {
        if self.mode != AppMode::ConfirmDelete {
            return false;
        }

        let to_delete = self.delete_targets();
        let (mut file_count, mut folder_count) = (0usize, 0usize);
        for path in &to_delete {
            if path.is_dir() {
                folder_count += 1;
            } else {
                file_count += 1;
            }
        }
        let plural = |count: usize, singular: &str, plural: &str| -> String {
            if count == 1 {
                singular.to_string()
            } else {
                plural.to_string()
            }
        };
        let title = if file_count > 0 && folder_count > 0 {
            format!(
                " Delete {} {} and {} {}? ",
                file_count,
                plural(file_count, "file", "files"),
                folder_count,
                plural(folder_count, "folder", "folders")
            )
        } else if folder_count > 0 {
            format!(
                " Delete {} {}? ",
                folder_count,
                plural(folder_count, "folder", "folders")
            )
        } else {
            format!(
                " Delete {} {}? ",
                file_count,
                plural(file_count, "file", "files")
            )
        };

        let content_w = title.chars().count().max(42) as u16;
        let content_h = area.height.saturating_sub(8).max(7);
        let max_w = area.width.saturating_sub(4).max(1);
        let max_h = area.height.saturating_sub(4).max(1);
        let dialog_w = (content_w + 2).max(48).min(max_w);
        let full_dialog_h = (content_h + 2).max(10).min(max_h);
        let dialog_h = (full_dialog_h / 2).max(8).min(max_h);
        let confirm_area = Rect::new(
            (area.width.saturating_sub(dialog_w)) / 2,
            (area.height.saturating_sub(dialog_h)) / 2,
            dialog_w,
            dialog_h,
        );

        let inner = Rect::new(
            confirm_area.x.saturating_add(1),
            confirm_area.y.saturating_add(1),
            confirm_area.width.saturating_sub(2),
            confirm_area.height.saturating_sub(2),
        );
        if inner.width == 0 || inner.height == 0 {
            return false;
        }

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        let button_area = sections[1];
        if row != button_area.y {
            return false;
        }

        let prefix_w = 2u16;
        let confirm_w = "  Confirm  ".chars().count() as u16;
        let gap_w = 4u16;
        let cancel_w = "  Cancel  ".chars().count() as u16;
        let total_w = prefix_w + confirm_w + gap_w + cancel_w;
        if button_area.width < total_w {
            return false;
        }

        let start_x = button_area.x + (button_area.width - total_w) / 2;
        let confirm_start = start_x + prefix_w;
        let cancel_start = confirm_start + confirm_w + gap_w;

        if column >= confirm_start && column < confirm_start + confirm_w {
            self.confirm_delete_button_focus = 0;
            self.confirm_delete_selected_targets();
            return true;
        }
        if column >= cancel_start && column < cancel_start + cancel_w {
            self.confirm_delete_button_focus = 1;
            self.mode = AppMode::Browsing;
            return true;
        }

        false
    }

    fn inner_with_borders(area: Rect) -> Rect {
        Rect::new(
            area.x.saturating_add(1),
            area.y.saturating_add(1),
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        )
    }

    fn internal_search_header_rows(&self) -> usize {
        let mut rows = 0usize;
        if self.internal_search_candidates_pending || self.internal_search_candidates_truncated {
            rows += 1;
        }

        if self.internal_search_scope == InternalSearchScope::Content {
            rows += 1; // limits summary
            if self.internal_search_limits_menu_open {
                rows += 4; // 3 editable rows + helper line
            } else {
                rows += 1; // open editor hint
            }
            if self.internal_search_content_pending {
                rows += 1;
            }
            if self.internal_search_content_limit_note.is_some() {
                rows += 1;
            }
        }

        rows
    }

    fn clickable_key_from_tabbed_row(
        &mut self,
        column: u16,
        row: u16,
        area: Rect,
    ) -> Option<KeyEvent> {
        match self.mode {
            AppMode::InternalSearch => {
                if self.internal_search_results.is_empty() {
                    return None;
                }

                let popup_area = Self::tab_overlay_anchor(area);
                let popup_inner = Self::inner_with_borders(popup_area);
                let search_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(1),
                        Constraint::Length(2),
                    ])
                    .split(popup_inner);
                let body_area = search_layout[1];

                if row < body_area.y || row >= body_area.y + body_area.height {
                    return None;
                }
                if column < body_area.x || column >= body_area.x + body_area.width {
                    return None;
                }

                let header_rows = self.internal_search_header_rows();
                let regex_rows = usize::from(self.internal_search_regex_error.is_some());
                let visible_rows = body_area.height as usize;
                let max_rows = visible_rows.saturating_sub(header_rows).max(1);
                let offset = if self.internal_search_selected >= max_rows {
                    self.internal_search_selected + 1 - max_rows
                } else {
                    0
                };

                let result_start_y = body_area
                    .y
                    .saturating_add((header_rows + regex_rows) as u16);
                if row < result_start_y {
                    return None;
                }

                let clicked_result_row = row.saturating_sub(result_start_y) as usize;
                let rendered_results = self
                    .internal_search_results
                    .len()
                    .saturating_sub(offset)
                    .min(max_rows);
                if clicked_result_row >= rendered_results {
                    return None;
                }

                self.internal_search_selected = offset + clicked_result_row;
                Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            }
            AppMode::Bookmarks => {
                let overlay = Self::tab_overlay_anchor(area);
                let bookmarks = Self::load_bookmarks();
                if bookmarks.is_empty() {
                    return None;
                }

                let bm_w = (area.width * 2 / 3).max(50).min(overlay.width);
                let mut line_count = 1usize + bookmarks.len();
                line_count += 4; // trailing helper lines
                let bm_h = (line_count as u16 + 4).max(17).min(overlay.height);
                let bm_area = Rect::new(overlay.x, overlay.y, bm_w, bm_h);
                let bm_inner = Self::inner_with_borders(bm_area);
                let bm_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(2)])
                    .split(bm_inner);
                let content = bm_chunks[0];

                if row < content.y || row >= content.y + content.height {
                    return None;
                }
                if column < content.x || column >= content.x + content.width {
                    return None;
                }

                let line_idx = row.saturating_sub(content.y) as usize;
                if line_idx >= 1 && line_idx <= bookmarks.len() {
                    self.bookmark_selected = line_idx - 1;
                    return Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }

                None
            }
            AppMode::Integrations => {
                let overlay = Self::tab_overlay_anchor(area);
                let integrations = self.integration_rows_cache.clone();
                if integrations.is_empty() {
                    return None;
                }

                let int_w = (area.width * 5 / 6).max(70).min(overlay.width);
                let int_h = (integrations.len() as u16 + 1 + 4).min(overlay.height);
                let int_area = Rect::new(overlay.x, overlay.y, int_w, int_h);
                let int_inner = Self::inner_with_borders(int_area);
                let int_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(2)])
                    .split(int_inner);
                let content = int_chunks[0];

                if row < content.y || row >= content.y + content.height {
                    return None;
                }
                if column < content.x || column >= content.x + content.width {
                    return None;
                }

                let visible_rows = content.height as usize;
                let selected_line = self.integration_selected + 1;
                let int_scroll = if selected_line + 1 <= visible_rows {
                    0usize
                } else {
                    selected_line + 1 - visible_rows
                };
                let line_idx = int_scroll + row.saturating_sub(content.y) as usize;
                if line_idx >= 1 && line_idx <= integrations.len() {
                    self.integration_selected = line_idx - 1;
                    return Some(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
                }

                None
            }
            AppMode::SshPicker => {
                if self.remote_entries.is_empty() {
                    return None;
                }

                let overlay = Self::tab_overlay_anchor(area);
                let ssh_w = (area.width * 2 / 3).max(60).min(area.width);
                let ssh_popup_w = ssh_w.min(overlay.width);
                let lines_len = 1usize + self.remote_entries.len();
                let ssh_h = (lines_len as u16 + 4).max(8).min(overlay.height);
                let ssh_area = Rect::new(overlay.x, overlay.y, ssh_popup_w, ssh_h);
                let ssh_inner = Self::inner_with_borders(ssh_area);
                let ssh_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(2)])
                    .split(ssh_inner);
                let content = ssh_chunks[0];

                if row < content.y || row >= content.y + content.height {
                    return None;
                }
                if column < content.x || column >= content.x + content.width {
                    return None;
                }

                let line_idx = row.saturating_sub(content.y) as usize;
                if line_idx >= 1 && line_idx <= self.remote_entries.len() {
                    self.ssh_picker_selection = line_idx - 1;
                    return Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }

                None
            }
            AppMode::SortMenu => {
                let overlay = Self::tab_overlay_anchor(area);
                let options = Self::sort_mode_options();
                if options.is_empty() {
                    return None;
                }

                let sort_w = overlay.width;
                let line_count = 1usize + options.len();
                let sort_h = (line_count as u16 + 4).max(10).min(overlay.height);
                let sort_area = Rect::new(overlay.x, overlay.y, sort_w, sort_h);
                let sort_inner = Self::inner_with_borders(sort_area);
                let sort_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(2)])
                    .split(sort_inner);
                let content = sort_chunks[0];

                if row < content.y || row >= content.y + content.height {
                    return None;
                }
                if column < content.x || column >= content.x + content.width {
                    return None;
                }

                let line_idx = row.saturating_sub(content.y) as usize;
                if line_idx >= 1 && line_idx <= options.len() {
                    self.sort_menu_selected = line_idx - 1;
                    return Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }

                None
            }
            _ => None,
        }
    }

    fn handle_mouse_scroll(&mut self, scroll_up: bool) {
        match self.mode {
            AppMode::Browsing => {
                let delta = if scroll_up { -3 } else { 3 };
                self.move_selection_delta(delta);
            }
            AppMode::Help => {
                if scroll_up {
                    self.help_scroll_offset = self.help_scroll_offset.saturating_sub(3);
                } else {
                    self.help_scroll_offset = (self.help_scroll_offset + 3).min(self.help_max_offset);
                }
            }
            AppMode::InternalSearch => {
                if self.internal_search_limits_menu_open {
                    if scroll_up {
                        self.internal_search_limits_selected = self.internal_search_limits_selected.saturating_sub(1);
                    } else {
                        self.internal_search_limits_selected = (self.internal_search_limits_selected + 1).min(2);
                    }
                } else if !self.internal_search_results.is_empty() {
                    if scroll_up {
                        self.internal_search_selected = self.internal_search_selected.saturating_sub(1);
                    } else {
                        self.internal_search_selected = (self.internal_search_selected + 1)
                            .min(self.internal_search_results.len().saturating_sub(1));
                    }
                }
            }
            AppMode::Bookmarks => {
                let max_idx = Self::load_bookmarks().len().saturating_sub(1);
                if scroll_up {
                    self.bookmark_selected = self.bookmark_selected.saturating_sub(1);
                } else {
                    self.bookmark_selected = (self.bookmark_selected + 1).min(max_idx);
                }
            }
            AppMode::Integrations => {
                let max_idx = self.integration_count().saturating_sub(1);
                if scroll_up {
                    self.integration_selected = self.integration_selected.saturating_sub(1);
                } else {
                    self.integration_selected = (self.integration_selected + 1).min(max_idx);
                }
            }
            AppMode::SortMenu => {
                let max_idx = Self::sort_mode_options().len().saturating_sub(1);
                if scroll_up {
                    self.sort_menu_selected = self.sort_menu_selected.saturating_sub(1);
                } else {
                    self.sort_menu_selected = (self.sort_menu_selected + 1).min(max_idx);
                }
            }
            AppMode::SshPicker => {
                let max_idx = self.remote_entries.len().saturating_sub(1);
                if scroll_up {
                    self.ssh_picker_selection = self.ssh_picker_selection.saturating_sub(1);
                } else {
                    self.ssh_picker_selection = (self.ssh_picker_selection + 1).min(max_idx);
                }
            }
            AppMode::ConfirmDelete => {
                if scroll_up {
                    self.confirm_delete_scroll_offset = self.confirm_delete_scroll_offset.saturating_sub(3);
                } else {
                    self.confirm_delete_scroll_offset =
                        (self.confirm_delete_scroll_offset + 3).min(self.confirm_delete_max_offset);
                }
            }
            _ => {}
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<KeyEvent> {
        match mouse.kind {
            MouseEventKind::ScrollUp => self.handle_mouse_scroll(true),
            MouseEventKind::ScrollDown => self.handle_mouse_scroll(false),
            MouseEventKind::Down(MouseButton::Left) => {
                if self.handle_tab_close_click(mouse.column, mouse.row, area) {
                    return None;
                }
                if self.handle_tab_click(mouse.column, mouse.row, area) {
                    return None;
                }
                if let Some(key) = self.clickable_key_from_tabbed_row(mouse.column, mouse.row, area) {
                    return Some(key);
                }
                let _ = self.handle_confirm_delete_click(mouse.column, mouse.row, area);
            }
            _ => {}
        }

        None
    }

    fn load_bookmarks() -> Vec<(usize, Option<PathBuf>)> {
        (0..=9).map(|i| {
            let path = env::var(format!("SB_BOOKMARK_{}", i))
                .ok()
                .map(PathBuf::from)
                .filter(|p| p.is_dir());
            (i, path)
        }).collect()
    }

}

/// Returns (glyph, (r, g, b)) for well-known directory names, or None for generic folders.
fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        ui::cli::print_help();
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        ui::cli::print_version();
        return Ok(());
    }
    if let Err(message) = ui::cli::validate_cli_args(&args) {
        eprintln!("Error: {}", message);
        eprintln!("Run with --help to see supported usage.");
        return Ok(());
    }
    if let Some(list_args) = ui::cli::parse_list_mode_args(&args) {
        if !list_args.include_hidden && list_args.tree_depth.is_none() {
            if let Some(path) = list_args.path {
                let target = PathBuf::from(path);
                if target.is_file() {
                    return App::open_path_in_view_mode(&target, true);
                }
            }
        }
        return ui::cli::list_current_directory(
            list_args.include_hidden,
            list_args.include_total_size,
            list_args.tree_depth,
            list_args.path,
        );
    }

    if let Some((mode, path)) = ui::cli::parse_direct_file_mode_args(&args) {
        let target = PathBuf::from(path);
        if target.is_file() {
            return match mode {
                ui::cli::DirectFileMode::ViewNoPager => App::open_path_in_view_mode(&target, false),
                ui::cli::DirectFileMode::ViewWithPager => App::open_path_in_view_mode(&target, true),
                ui::cli::DirectFileMode::Edit => App::open_path_in_editor_cli(&target),
            };
        } else if target.is_dir() && matches!(mode, ui::cli::DirectFileMode::Edit) {
            // If -e is used with a directory, open the TUI file manager in that directory
            let _ = env::set_current_dir(&target);
        }
    }

    // If a single argument is provided that is a directory, list it like -l
    if args.len() == 1 && !args[0].starts_with('-') {
        if let Ok(target) = PathBuf::from(&args[0]).canonicalize() {
            if target.is_dir() {
                return ui::cli::list_current_directory(false, false, None, Some(&args[0]));
            }
        }
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let mut app = App::new()?;
    let mut deferred_key: Option<KeyEvent> = None;
    let hostname = hostname::get().map(|h| h.to_string_lossy().into_owned()).unwrap_or_else(|_| "host".to_string());
    let user = env::var("USER").unwrap_or_else(|_| "user".to_string());

    loop {
        app.refresh_header_clock_if_needed();
        app.pump_archive_progress();
        app.pump_copy_total_prescan();
        app.pump_copy_progress();
        app.pump_folder_size_progress();
        app.pump_recursive_mtime_progress();
        app.pump_current_dir_total_size_progress();
        app.pump_selected_total_size_progress();
        app.pump_git_info();
        app.pump_notes_progress();
        app.pump_internal_search_candidates_progress();
        app.pump_internal_search_content_progress();
        let text_input_cursor = matches!(
            app.mode,
            AppMode::PathEditing
                | AppMode::Renaming
                | AppMode::PasteRenaming
                | AppMode::NewFile
                | AppMode::NewFolder
                | AppMode::ArchiveCreate
                | AppMode::NoteEditing
                | AppMode::CommandInput
                | AppMode::GitCommitMessage
                | AppMode::GitTagInput
                | AppMode::InternalSearch
        );
        if text_input_cursor {
            execute!(terminal.backend_mut(), SetCursorStyle::BlinkingBar)?;
        } else {
            execute!(terminal.backend_mut(), SetCursorStyle::DefaultUserShape)?;
        }
        terminal.draw(|f| {
            let chunks = Layout::default()
                .constraints([Constraint::Min(3), Constraint::Length(2)])
                .split(f.size());

            // --- Header ---
            let header_identity = app.current_header_identity(&user, &hostname);
            let current_display_path = if app.mode == AppMode::PathEditing {
                app.input_buffer.clone()
            } else {
                app.current_dir_display_path_with_filter()
            };
            let mut path_spans = vec![
                Span::styled(header_identity.as_str(), Style::default().fg(Color::Cyan)),
                Span::raw(" » "),
                if app.mode == AppMode::PathEditing {
                    Span::styled(current_display_path.as_str(), Style::default().fg(Color::Rgb(255, 220, 120)))
                } else {
                    Span::raw(current_display_path.as_str())
                },
            ];
            if app.integration_enabled("git") {
                if let Some((branch, is_dirty, tag_info)) = app.cached_git_info_for_current_dir() {
                    let branch_style = Style::default().fg(Color::Rgb(100, 150, 255));
                    path_spans.push(Span::styled(" (", branch_style));
                    path_spans.push(Span::styled(branch, branch_style));
                    if is_dirty {
                        path_spans.push(Span::styled("*", Style::default().fg(Color::White)));
                    }
                    if let Some((tag_name, ahead)) = tag_info {
                        let at_style = Style::default().fg(Color::Rgb(120, 120, 120));
                        let tag_style = Style::default().fg(Color::Rgb(80, 255, 120));
                        let tag_text = if ahead > 0 {
                            format!("{}+{}", tag_name, ahead)
                        } else {
                            tag_name.to_string()
                        };
                        path_spans.push(Span::styled(" ", branch_style));
                        path_spans.push(Span::styled("@", at_style));
                        path_spans.push(Span::styled(" ", branch_style));
                        path_spans.push(Span::styled(tag_text, tag_style));
                    }
                    path_spans.push(Span::styled(")", branch_style));
                }
            }
            let header_right = if let Some(total_suffix) = app.current_dir_total_size_header_suffix() {
                Some((
                    total_suffix,
                    Style::default().fg(Color::Rgb(150, 220, 150)),
                ))
            } else if !app.folder_size_enabled {
                Some((
                    app.header_clock_text.clone(),
                    Style::default().fg(Color::White),
                ))
            } else {
                None
            };

            if let Some((header_right_text, header_right_style)) = header_right {
                let suffix_width = UnicodeWidthStr::width(header_right_text.as_str()) as u16;
                let right_width = suffix_width.saturating_add(1).min(chunks[0].width.saturating_sub(1));
                if right_width > 0 {
                    let header_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Min(1), Constraint::Length(right_width)])
                        .split(chunks[0]);
                    f.render_widget(Paragraph::new(Line::from(path_spans)), header_chunks[0]);
                    f.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(header_right_text, header_right_style),
                        ]))
                        .alignment(Alignment::Right),
                        header_chunks[1],
                    );
                } else {
                    f.render_widget(Paragraph::new(Line::from(path_spans)), chunks[0]);
                }
            } else {
                f.render_widget(Paragraph::new(Line::from(path_spans)), chunks[0]);
            }
            if app.mode == AppMode::PathEditing {
                let prefix_len = format!("{} » ", header_identity).chars().count() as u16;
                app.clamp_input_cursor();
                let cursor_x = chunks[0].x + prefix_len + app.input_cursor as u16;
                let cursor_y = chunks[0].y;
                f.set_cursor(cursor_x, cursor_y);
            }
            f.render_widget(Block::default().borders(Borders::BOTTOM).border_type(BorderType::Rounded).border_style(Style::default().fg(Color::DarkGray)), 
                Rect::new(chunks[0].x, chunks[0].y + 1, chunks[0].width, 1));

            // --- Table ---
            let term_w = chunks[0].width;
            let show_date = term_w >= 90;
            let show_size = term_w >= 70;
            let show_meta = term_w >= 50;
            let show_pct = app.folder_size_enabled && show_size;
            let perms_width = 11usize;
            let group_width = app.meta_group_width.max(1);
            let owner_width = app.meta_owner_width.max(1);
            let size_width = if show_size {
                app.entry_render_cache
                    .iter()
                    .map(|entry| entry.size_col.trim().chars().count())
                    .max()
                    .unwrap_or(1)
                    .max(1)
            } else {
                1
            };
            let pct_width = 4usize;
            let date_width = 16usize;
            let reserved_width = (if show_meta { perms_width + group_width + owner_width } else { 0 })
                + (if show_size { size_width } else { 0 })
                + (if show_pct { pct_width } else { 0 })
                + (if show_date { date_width } else { 0 });
            let name_cell_width = (term_w as usize).saturating_sub(reserved_width);
            // Keep a small safety margin so truncation occurs before the table widget clips.
            let file_name_width = name_cell_width.saturating_sub(6).max(1);

            let truncate_with_ellipsis = |s: &str, max: usize| -> String {
                if s.chars().count() <= max {
                    return s.to_string();
                }
                if max <= 1 {
                    return "…".to_string();
                }
                let mut out = String::new();
                for ch in s.chars().take(max - 1) {
                    out.push(ch);
                }
                out.push('…');
                out
            };

            let note_style = Style::default().fg(Color::Rgb(150, 150, 150));
            let tree_style = Style::default().fg(Color::Rgb(140, 140, 140));

            // Keep selected-row background while preserving per-span foreground colors
            // (e.g. filename white, note text gray).
            let selection_style = Style::default().bg(Color::Rgb(50, 50, 50));
            let marker_width = if app.no_color { 3 } else { 0 };
            let name_text_width = file_name_width.saturating_sub(marker_width).max(1);
            let entry_styles = |mut icon_style: Style, mut name_style: Style, is_selected: bool| {
                if app.no_color && !is_selected {
                    icon_style.fg = None;
                    name_style.fg = None;
                }
                (icon_style, name_style)
            };

            let size_min_max = if show_size {
                ui::list_temperature::size_min_max_from_sizes(
                    app.entry_render_cache.iter().map(|entry| entry.size_bytes),
                )
            } else {
                None
            };

            let date_rank_by_ts = if show_date {
                ui::list_temperature::date_rank_map_from_unix(
                    app.entry_render_cache.iter().map(|entry| entry.modified_unix),
                )
            } else {
                HashMap::new()
            };

            let rows: Vec<Row> = app.entry_render_cache.iter().enumerate().map(|(idx, entry_cache)| {
                let is_marked = app.marked_indices.contains(&idx);
                let is_selected = idx == app.selected_index;
                let (icon_style, name_style) = entry_styles(entry_cache.icon_style, entry_cache.name_style, is_selected);

                let group_style = Style::default().fg(Color::Rgb(172, 136, 98));
                let owner_style = Style::default().fg(Color::Rgb(196, 172, 118));
                let size_style = Style::default().fg(ui::list_temperature::size_color_for(
                    entry_cache.size_bytes,
                    size_min_max,
                ));
                let pct_style = size_style;
                let date_style =
                    Style::default().fg(ui::list_temperature::date_color_for(
                        entry_cache.modified_unix,
                        &date_rank_by_ts,
                    ));
                let marker = if app.no_color {
                    format!(
                        "{}{} ",
                        if is_selected { '>' } else { ' ' },
                        if is_marked { '*' } else { ' ' }
                    )
                } else {
                    String::new()
                };
                let note_text = app
                    .notes_by_name
                    .get(&entry_cache.raw_name)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let tree_prefix = app.tree_row_prefixes.get(idx).map(|s| s.as_str()).unwrap_or("");
                let icon_prefix_width = if app.show_icons && !entry_cache.icon_glyph.is_empty() { 2usize } else { 0usize };
                let prefix_width = tree_prefix.chars().count();
                let available_name_width = name_text_width.saturating_sub(prefix_width + icon_prefix_width).max(1);
                let rendered_name = truncate_with_ellipsis(&entry_cache.raw_name, available_name_width);
                let mut rendered_note = String::new();
                if !note_text.is_empty() {
                    let used = prefix_width + icon_prefix_width + rendered_name.chars().count();
                    let sep = "  ";
                    let sep_len = sep.chars().count();
                    if used + sep_len < name_text_width {
                        let remaining = name_text_width - used - sep_len;
                        let clipped_note = truncate_with_ellipsis(note_text, remaining);
                        if !clipped_note.is_empty() {
                            rendered_note = format!("{}{}", sep, clipped_note);
                        }
                    }
                }

                let mut cells = vec![Cell::from(Line::from({
                    let mut spans = vec![];
                    if !marker.is_empty() {
                        spans.push(Span::raw(marker));
                    }
                    if !tree_prefix.is_empty() {
                        spans.push(Span::styled(tree_prefix.to_string(), tree_style));
                    }
                    if app.show_icons {
                        spans.push(Span::styled(format!("{} ", entry_cache.icon_glyph), icon_style));
                    }
                    spans.push(Span::styled(rendered_name, name_style));
                    if !rendered_note.is_empty() {
                        spans.push(Span::styled(rendered_note, note_style));
                    }
                    spans
                }))];
                if show_meta {
                    let perms_text = entry_cache.perms_col.trim();
                    let perms_spans: Vec<Span> = ui::list_render::permission_gradient_segments(
                        perms_text,
                        perms_width,
                    )
                    .into_iter()
                    .map(|(text, color)| match color {
                        Some(c) => Span::styled(text, Style::default().fg(c)),
                        None => Span::raw(text),
                    })
                    .collect();
                    cells.push(Cell::from(Line::from(perms_spans)));
                    cells.push(Cell::from(Span::styled(
                        format!("{:>width$}", entry_cache.group_name, width = group_width),
                        group_style,
                    )));
                    cells.push(Cell::from(Span::styled(
                        format!("{:<width$}", entry_cache.owner_name, width = owner_width),
                        owner_style,
                    )));
                }
                if show_size {
                    let size_col = format!("{:>width$}", entry_cache.size_col.trim(), width = size_width);
                    cells.push(Cell::from(Span::styled(size_col, size_style)));
                }
                if show_pct {
                    let pct_col = match (app.current_dir_total_size_bytes, entry_cache.size_bytes) {
                        (Some(total), Some(entry_bytes)) if total > 0 => {
                            let pct = (entry_bytes as f64 * 100.0) / (total as f64);
                            format!("{:>width$}", format!("{:.0}%", pct), width = pct_width)
                        }
                        _ => format!("{:>width$}", "-", width = pct_width),
                    };
                    cells.push(Cell::from(Span::styled(pct_col, pct_style)));
                }
                if show_date {
                    cells.push(Cell::from(Span::styled(entry_cache.date_col.as_str(), date_style)));
                }
                Row::new(cells).style(if is_marked { Style::default().bg(Color::Rgb(0, 100, 150)) } else { Style::default() })
            }).collect();

            let mut col_constraints: Vec<Constraint> = vec![Constraint::Min(0)];
            if show_meta {
                col_constraints.push(Constraint::Length(perms_width as u16));
                col_constraints.push(Constraint::Length(group_width as u16));
                col_constraints.push(Constraint::Length(owner_width as u16));
            }
            if show_size { col_constraints.push(Constraint::Length(size_width as u16)); }
            if show_pct { col_constraints.push(Constraint::Length(pct_width as u16)); }
            if show_date { col_constraints.push(Constraint::Length(date_width as u16)); }
            let table = Table::new(rows, col_constraints)
                .highlight_style(selection_style)
                .highlight_symbol(""); 

            let table_area = Rect::new(chunks[0].x, chunks[0].y + 2, chunks[0].width, chunks[0].height - 2);
            app.page_size = (table_area.height as usize).saturating_sub(1).max(1);
            f.render_stateful_widget(table, table_area, &mut app.table_state);

            if app.entries.is_empty() {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        "No files or folders yet. Use the 'n' or 'N' buttons to break the silence.",
                        Style::default()
                            .fg(Color::Rgb(140, 140, 140))
                            .add_modifier(Modifier::ITALIC),
                    )))
                    .alignment(Alignment::Left),
                    table_area,
                );
            }

            // If the selected item is truncated, temporarily hide its metadata and
            // render its full name across the whole row width.
            if let Some(selected_idx) = app.table_state.selected() {
                if let Some(entry_cache) = app.entry_render_cache.get(selected_idx) {
                    let tree_prefix = app.tree_row_prefixes.get(selected_idx).map(|s| s.as_str()).unwrap_or("");
                    let full_name = entry_cache.raw_name.as_str();
                    if tree_prefix.chars().count() + full_name.chars().count() > file_name_width {
                        let offset = app.table_state.offset();
                        if selected_idx >= offset {
                            let row_in_view = selected_idx - offset;
                            if row_in_view < table_area.height as usize {
                                let row_area = Rect::new(
                                    table_area.x,
                                    table_area.y + row_in_view as u16,
                                    table_area.width,
                                    1,
                                );
                                let is_marked = app.marked_indices.contains(&selected_idx);
                                let icon_style = entry_cache.icon_style.fg(Color::White);
                                let name_style = entry_cache.name_style.fg(Color::White);
                                let marker = if app.no_color {
                                    format!(">{} ", if is_marked { '*' } else { ' ' })
                                } else {
                                    String::new()
                                };
                                let note_text = app
                                    .notes_by_name
                                    .get(entry_cache.raw_name.as_str())
                                    .map(|s| s.as_str())
                                    .unwrap_or("");
                                let note_suffix = if note_text.is_empty() {
                                    String::new()
                                } else {
                                    format!("  {}", note_text)
                                };

                                f.render_widget(Clear, row_area);
                                f.render_widget(
                                    Block::default().style(selection_style),
                                    row_area,
                                );
                                f.render_widget(
                                    Paragraph::new(Line::from({
                                        let mut spans = vec![];
                                        if !marker.is_empty() {
                                            spans.push(Span::raw(marker));
                                        }
                                        if !tree_prefix.is_empty() {
                                            spans.push(Span::styled(tree_prefix.to_string(), tree_style));
                                        }
                                        if app.show_icons {
                                            spans.push(Span::styled(format!("{} ", entry_cache.icon_glyph), icon_style));
                                        }
                                        spans.push(Span::styled(full_name.to_string(), name_style));
                                        if !note_suffix.is_empty() {
                                            spans.push(Span::styled(note_suffix, note_style));
                                        }
                                        spans
                                    })),
                                    row_area,
                                );
                            }
                        }
                    }
                }
            }

            // --- Overlays ---
            let tab_overlay_anchor = {
                let area = chunks[0];
                let anchor_w = (area.width * 5 / 6).max(50).min(area.width);
                let anchor_h = (area.height * 5 / 6).max(12).min(area.height);
                Rect::new(
                    area.x + (area.width.saturating_sub(anchor_w)) / 2,
                    area.y + (area.height.saturating_sub(anchor_h)) / 2,
                    anchor_w,
                    anchor_h,
                )
            };
            if app.mode == AppMode::InternalSearch {
                let popup_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    tab_overlay_anchor.width,
                    tab_overlay_anchor.height,
                );

                f.render_widget(Clear, popup_area);
                let popup_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(App::panel_tab_bar_line(app.panel_tab))
                    .title_style(Style::default().fg(Color::White))
                    .border_style(Style::default().fg(Color::Rgb(80, 200, 180)));
                let popup_inner = popup_block.inner(popup_area);
                f.render_widget(popup_block, popup_area);
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "x",
                        Style::default().fg(Color::Rgb(170, 170, 170)),
                    )),
                    App::tabbed_overlay_close_area(popup_area),
                );

                let search_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(1),
                        Constraint::Length(2),
                    ])
                    .split(popup_inner);
                let query_box_area = search_layout[0];
                let body_area = search_layout[1];
                let footer_area = search_layout[2];

                let query_box_block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(95, 95, 95)));
                let query_inner = query_box_block.inner(query_box_area);
                f.render_widget(query_box_block, query_box_area);

                let (mode_text, mode_style) = if app.internal_search_scope == InternalSearchScope::Content {
                    (
                        "Scope: Content".to_string(),
                        Style::default().fg(Color::Rgb(120, 220, 180)),
                    )
                } else {
                    (
                        "Scope: Filename".to_string(),
                        Style::default().fg(Color::Rgb(120, 170, 255)),
                    )
                };
                let mode_width = UnicodeWidthStr::width(mode_text.as_str()) as u16;
                let query_row = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(1), Constraint::Length(mode_width + 1)])
                    .split(query_inner);
                let query_input_area = query_row[0];
                let query_mode_area = query_row[1];

                let query_icon = if app.show_icons && app.nerd_font_active { "\u{f002}" } else { "/" };
                let query_icon_prefix = format!(" {}  ", query_icon);
                let query_line = Line::from(vec![
                    Span::styled(query_icon_prefix.clone(), Style::default().fg(Color::Rgb(120, 180, 255))),
                    Span::styled(app.input_buffer.as_str(), Style::default().fg(Color::Rgb(255, 220, 120))),
                ]);
                f.render_widget(Paragraph::new(query_line), query_input_area);
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(mode_text.clone(), mode_style))).alignment(Alignment::Right),
                    query_mode_area,
                );

                let mut lines: Vec<Line> = Vec::new();

                if app.internal_search_candidates_pending {
                    lines.push(Line::from(Span::styled(
                        "Indexing files asynchronously...",
                        Style::default().fg(Color::Rgb(120, 200, 255)),
                    )));
                } else if app.internal_search_candidates_truncated {
                    lines.push(Line::from(Span::styled(
                        "Indexed first 20000 files (refine query to narrow results)",
                        Style::default().fg(Color::Rgb(160, 160, 160)),
                    )));
                }

                if app.internal_search_scope == InternalSearchScope::Content {
                    let limits = app.internal_search_content_limits;
                    lines.push(Line::from(Span::styled(
                        format!(
                            "Limits: files={}  hits={}  max-file={}",
                            limits.max_files,
                            limits.max_hits,
                            App::format_size(limits.max_file_bytes as u64)
                        ),
                        Style::default().fg(Color::Rgb(160, 160, 160)),
                    )));

                    if app.internal_search_limits_menu_open {
                        let selected_style = Style::default().fg(Color::Rgb(255, 220, 120)).add_modifier(Modifier::BOLD);
                        let normal_style = Style::default().fg(Color::Rgb(180, 180, 180));
                        let item_line = |idx: usize, label: &str, value: String| {
                            let marker = if idx == app.internal_search_limits_selected { ">" } else { " " };
                            let style = if idx == app.internal_search_limits_selected {
                                selected_style
                            } else {
                                normal_style
                            };
                            Line::from(Span::styled(format!("{} {}: {}", marker, label, value), style))
                        };
                        lines.push(item_line(0, "Max files", limits.max_files.to_string()));
                        lines.push(item_line(1, "Max hits", limits.max_hits.to_string()));
                        lines.push(item_line(2, "Max file size", App::format_size(limits.max_file_bytes as u64)));
                        lines.push(Line::from(Span::styled(
                            "Editor: Up/Down select  Left/Right or +/- adjust  Shift=10x  r reset  Ctrl+L close",
                            Style::default().fg(Color::DarkGray),
                        )));
                    } else {
                        lines.push(Line::from(Span::styled(
                            "Ctrl+L open limits editor (live, no restart)",
                            Style::default().fg(Color::DarkGray),
                        )));
                    }

                    if app.internal_search_content_pending {
                        lines.push(Line::from(Span::styled(
                            "Scanning content asynchronously...",
                            Style::default().fg(Color::Rgb(120, 200, 255)),
                        )));
                    }
                    if let Some(note) = &app.internal_search_content_limit_note {
                        lines.push(Line::from(Span::styled(
                            note.clone(),
                            Style::default().fg(Color::Rgb(160, 160, 160)),
                        )));
                    }
                }

                let selected = app.internal_search_selected;
                let body_content_w = body_area.width as usize;
                let visible_rows = body_area.height as usize;
                let header_rows = lines.len();
                let max_rows = visible_rows.saturating_sub(header_rows).max(1);
                let offset = if selected >= max_rows {
                    selected + 1 - max_rows
                } else {
                    0
                };

                if let Some(err) = &app.internal_search_regex_error {
                    lines.push(Line::from(Span::styled(
                        format!("Regex error: {}", err),
                        Style::default().fg(Color::Rgb(255, 120, 120)),
                    )));
                }

                if app.internal_search_results.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "No matches",
                        Style::default().fg(Color::Rgb(180, 90, 90)),
                    )));
                } else {
                    for (display_idx, result_idx) in app
                        .internal_search_results
                        .iter()
                        .skip(offset)
                        .take(max_rows)
                        .enumerate()
                    {
                        let absolute_idx = offset + display_idx;
                        let is_selected = absolute_idx == selected;
                        let base_style = if is_selected {
                            Style::default()
                                .fg(Color::White)
                                .bg(Color::Rgb(60, 60, 60))
                        } else {
                            Style::default().fg(Color::Rgb(200, 200, 200))
                        };
                        let match_style = if is_selected {
                            Style::default()
                                .fg(Color::Rgb(255, 240, 170))
                                .bg(Color::Rgb(60, 60, 60))
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                                .fg(Color::Rgb(255, 220, 120))
                                .add_modifier(Modifier::BOLD)
                        };
                        let marker = "  ";
                        let mut spans: Vec<Span> = vec![Span::styled(marker, base_style)];

                        let rel_path_for_icon = match result_idx {
                            InternalSearchResult::Filename { rel_path, .. } => rel_path,
                            InternalSearchResult::Content { rel_path, .. } => rel_path,
                        };
                        let abs_path = app.current_dir.join(rel_path_for_icon);
                        let is_symlink = abs_path
                            .symlink_metadata()
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false);
                        let is_dir = abs_path.is_dir();
                        let icon_name = rel_path_for_icon
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|name| name.to_string())
                            .unwrap_or_else(|| rel_path_for_icon.to_string_lossy().into_owned());
                        let (icon_glyph, icon_style) = App::icon_for_name(
                            icon_name.as_str(),
                            is_dir,
                            app.show_icons,
                            app.nerd_font_active,
                            is_symlink,
                        );
                        if app.show_icons && !icon_glyph.is_empty() {
                            let adjusted_icon_style = if is_selected {
                                icon_style.bg(Color::Rgb(60, 60, 60))
                            } else {
                                icon_style
                            };
                            spans.push(Span::styled(format!("{} ", icon_glyph), adjusted_icon_style));
                        }

                        match result_idx {
                            InternalSearchResult::Filename { rel_path, match_ranges } => {
                                let rel_str = rel_path.to_string_lossy().into_owned();
                                spans.extend(App::search_spans_with_ranges(
                                    &rel_str,
                                    match_ranges,
                                    base_style,
                                    match_style,
                                ));
                            }
                            InternalSearchResult::Content {
                                rel_path,
                                line_number,
                                line_text,
                                match_ranges,
                            } => {
                                let prefix = format!("{}:{}: ", rel_path.display(), line_number);
                                spans.push(Span::styled(
                                    prefix,
                                    base_style.fg(Color::Rgb(150, 190, 255)),
                                ));
                                spans.extend(App::search_spans_with_ranges(
                                    line_text,
                                    match_ranges,
                                    base_style,
                                    match_style,
                                ));
                            }
                        }

                        if is_selected {
                            let used_w: usize = spans
                                .iter()
                                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                                .sum();
                            if body_content_w > used_w {
                                spans.push(Span::styled(
                                    " ".repeat(body_content_w - used_w),
                                    base_style,
                                ));
                            }
                        }

                        lines.push(Line::from(spans));
                    }
                }

                f.render_widget(Paragraph::new(lines), body_area);
                f.render_widget(
                    Paragraph::new(vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            " ↑↓:navigate  Enter:open  Ctrl+T:toggle scope  Tab:switch tabs  Regex: re:pattern or /pattern/i",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ]),
                    footer_area,
                );

                app.clamp_input_cursor();
                let cursor_x = query_input_area.x
                    + UnicodeWidthStr::width(query_icon_prefix.as_str()) as u16
                    + app.input_cursor as u16;
                let cursor_y = query_input_area.y;
                f.set_cursor(
                    cursor_x.min(query_input_area.x + query_input_area.width.saturating_sub(1)),
                    cursor_y,
                );
            } else if app.mode == AppMode::DbPreview {
                let popup_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    tab_overlay_anchor.width,
                    tab_overlay_anchor.height,
                );

                let db_title = app
                    .db_preview_path
                    .as_ref()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| "SQLite Preview".to_string());

                let mut lines: Vec<Line> = vec![
                    Line::from(Span::styled(
                        "←→:switch table  Home/End:jump  Esc:close",
                        Style::default().fg(Color::DarkGray),
                    )),
                ];

                let mut table_spans: Vec<Span> = vec![Span::styled(
                    "Tables: ",
                    Style::default().fg(Color::Rgb(160, 160, 160)),
                )];
                if app.db_preview_tables.is_empty() {
                    table_spans.push(Span::styled(
                        "(none)",
                        Style::default().fg(Color::Rgb(180, 90, 90)),
                    ));
                } else {
                    for (idx, table_name) in app.db_preview_tables.iter().enumerate() {
                        if idx > 0 {
                            table_spans.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
                        }
                        let display = if table_name.chars().count() > 20 {
                            let mut t = table_name.chars().take(19).collect::<String>();
                            t.push('…');
                            t
                        } else {
                            table_name.clone()
                        };
                        let style = if idx == app.db_preview_selected {
                            Style::default()
                                .fg(Color::Rgb(20, 20, 20))
                                .bg(Color::Rgb(120, 220, 140))
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Rgb(170, 210, 255))
                        };
                        table_spans.push(Span::styled(display, style));
                    }
                }
                lines.push(Line::from(table_spans));

                if let Some(err) = &app.db_preview_error {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        err.clone(),
                        Style::default().fg(Color::Rgb(255, 120, 120)),
                    )));
                } else {
                    lines.push(Line::from(""));
                    if app.db_preview_output_lines.is_empty() {
                        lines.push(Line::from(Span::styled(
                            "(no rows)",
                            Style::default().fg(Color::Rgb(140, 140, 140)),
                        )));
                    } else {
                        let visible_w = popup_area.width.saturating_sub(4) as usize;
                        let clip_line = |text: &str| -> String {
                            if text.chars().count() <= visible_w {
                                return text.to_string();
                            }
                            if visible_w <= 1 {
                                return "…".to_string();
                            }
                            let mut out = text.chars().take(visible_w - 1).collect::<String>();
                            out.push('…');
                            out
                        };

                        for row in &app.db_preview_output_lines {
                            lines.push(Line::from(Span::styled(
                                clip_line(row),
                                Style::default().fg(Color::Rgb(210, 210, 210)),
                            )));
                        }
                    }
                }

                f.render_widget(Clear, popup_area);
                f.render_widget(
                    Paragraph::new(lines)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(format!(" SQLite: {} ", db_title))
                                .title_style(Style::default().fg(Color::White))
                                .border_style(Style::default().fg(Color::Rgb(120, 200, 150))),
                        )
                        .wrap(Wrap { trim: true }),
                    popup_area,
                );
            } else if app.mode == AppMode::Help {
                let help_w = tab_overlay_anchor.width;
                let inner_w = help_w.saturating_sub(4) as usize;
                let shortcut_w = inner_w.clamp(10, 18);
                let section_style = Style::default().fg(Color::Rgb(120, 200, 255)).add_modifier(Modifier::BOLD);
                let shortcut_style = Style::default().fg(Color::Rgb(255, 220, 140)).add_modifier(Modifier::BOLD);
                let desc_style = Style::default().fg(Color::Rgb(200, 200, 200));

                let mut lines: Vec<Line> = vec![
                    Line::from(""),
                    Line::from(vec![
                        Span::styled(
                            format!("{:<width$}", "Shortcut", width = shortcut_w),
                            Style::default().fg(Color::Rgb(190, 190, 190)).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Description", Style::default().fg(Color::Rgb(190, 190, 190)).add_modifier(Modifier::BOLD)),
                    ]),
                ];

                let sections: [(&str, [(&str, &str); 10]); 5] = [
                    (
                        "Navigation",
                        [
                            ("Up / Down", "Move selection"),
                            ("PageUp / PageDown", "Jump by visible page"),
                            ("Home / End", "Jump to first or last item"),
                            ("Enter / Right", "Open folder/file or preview"),
                            ("Left / Backspace", "Go to parent folder"),
                            ("Tab", "Edit current path"),
                            ("~", "Go to home folder"),
                            ("", ""),
                            ("", ""),
                            ("", ""),
                        ],
                    ),
                    (
                        "Selection And Clipboard",
                        [
                            ("Space / Insert", "Toggle mark for selected item"),
                            ("*", "Toggle all marks"),
                            ("c / F5", "Copy selected/marked item(s) to app clipboard"),
                            ("Ctrl+c", "Copy full path(s) to system clipboard"),
                            ("Ctrl+e", "Edit system clipboard content via temporary file"),
                            ("v", "Paste clipboard into current folder"),
                            ("m", "Move clipboard into current folder"),
                            ("", ""),
                            ("", ""),
                            ("", ""),
                        ],
                    ),
                    (
                        "Operations",
                        [
                            ("n", "Create item(s): name=file, /name=folder, Shift/Alt+Enter or Ctrl+J=new item"),
                            ("Ctrl+n", "Add/edit note for selected item(s)"),
                            ("Ctrl+z", "Drop to shell in current directory"),
                            ("F2 / r", "Rename or bulk rename"),
                            ("e / F4", "Edit file, or rename if selection is a folder"),
                            ("d / Del", "Delete selected/marked item(s)"),
                            ("x / p", "Toggle executable bit / protect/unprotect file"),
                            ("Z", "Create or extract archive"),
                            ("o", "Open with default GUI app"),
                            ("", ""),
                        ],
                    ),
                    (
                        "Search And Integrations",
                        [
                            ("s / Ctrl+s", "Toggle size calc / open sorting menu"),
                            ("f", "Fuzzy search with fzf"),
                            ("g", "Content search with ripgrep"),
                            ("G", "Commit+push if repo is dirty (--amend enables -f push)"),
                            ("H", "Pretty git log graph (git repos only)"),
                            ("C", "Delta compare (marked vs cursor)"),
                            ("S", "Open SSH/rclone mount picker"),
                            ("i / E", "Split shell (left) + less preview / editor (right 30%)"),
                            ("I", "Open integrations panel"),
                            ("b / 0-9", "Open bookmarks / jump to bookmark"),
                        ],
                    ),
                    (
                        "General",
                        [
                            ("h", "Open help"),
                            ("q / Esc", "Quit Shell Buddy"),
                            ("t", "Open ~/.todo in $EDITOR (creates if missing)"),
                            ("", ""),
                            ("", ""),
                            ("", ""),
                            ("", ""),
                            ("", ""),
                            ("", ""),
                            ("", ""),
                        ],
                    ),
                ];

                for (section_title, rows) in sections {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(section_title.to_string(), section_style)));
                    for (shortcut, description) in rows {
                        if shortcut.is_empty() && description.is_empty() {
                            continue;
                        }
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("{:<width$}", shortcut, width = shortcut_w),
                                shortcut_style,
                            ),
                            Span::styled(description.to_string(), desc_style),
                        ]));
                    }
                }

                let desired_h = (lines.len() as u16 + 4).max(18);
                let help_h = desired_h.min(tab_overlay_anchor.height);
                let help_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    help_w,
                    help_h,
                );
                f.render_widget(Clear, help_area);

                let help_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(App::panel_tab_bar_line(app.panel_tab))
                    .title_style(Style::default().fg(Color::White))
                    .border_style(Style::default().fg(Color::Rgb(80, 200, 180)));
                let help_inner = help_block.inner(help_area);
                f.render_widget(help_block, help_area);
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "x",
                        Style::default().fg(Color::Rgb(170, 170, 170)),
                    )),
                    App::tabbed_overlay_close_area(help_area),
                );
                let help_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(2)])
                    .split(help_inner);
                let help_content_area = help_chunks[0];
                let help_footer_area = help_chunks[1];

                let visible_lines = help_content_area.height as usize;
                let total_lines = lines.len();
                let max_scroll = total_lines.saturating_sub(visible_lines);
                app.help_max_offset = max_scroll as u16;
                let clamped_offset = (app.help_scroll_offset as usize).min(max_scroll) as u16;
                let indented_lines: Vec<Line> = lines
                    .iter()
                    .map(|line| {
                        let mut spans: Vec<Span> = Vec::with_capacity(line.spans.len() + 1);
                        spans.push(Span::raw(" "));
                        spans.extend(line.spans.iter().cloned());
                        Line::from(spans)
                    })
                    .collect();
                
                f.render_widget(
                    Paragraph::new(indented_lines)
                        .wrap(Wrap { trim: false })
                        .scroll((clamped_offset, 0)),
                    help_content_area,
                );
                f.render_widget(
                    Paragraph::new(vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            " ↑↓/PgUp/PgDn/Home/End:scroll  Tab:switch tabs  Esc:close",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ]),
                    help_footer_area,
                );
            } else if matches!(app.mode, AppMode::NewFile | AppMode::NewFolder) {
                let area = f.size();
                let title = " Create ";
                let dialog_w = (area.width * 2 / 3).max(40).min(area.width.saturating_sub(4).max(1));

                let lines: Vec<&str> = if app.input_buffer.is_empty() {
                    vec![""]
                } else {
                    app.input_buffer.split('\n').collect()
                };
                let (cursor_line, cursor_col) = app.input_cursor_line_col();
                let max_content_lines = area.height.saturating_sub(7).max(1) as usize;
                let content_lines = lines.len().max(1).min(max_content_lines);
                let window_start = cursor_line.saturating_sub(content_lines.saturating_sub(1));
                let window_end = (window_start + content_lines).min(lines.len().max(1));
                let shown_lines = &lines[window_start..window_end];

                let dialog_h = (shown_lines.len() as u16 + 3).max(4).min(area.height.saturating_sub(2).max(1));
                let create_area = Rect::new(
                    (area.width.saturating_sub(dialog_w)) / 2,
                    (area.height.saturating_sub(dialog_h)) / 2,
                    dialog_w,
                    dialog_h,
                );

                f.render_widget(Clear, create_area);
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(title)
                    .title_style(Style::default().fg(Color::White))
                    .border_style(Style::default().fg(Color::Rgb(120, 120, 120)));
                let input_area = block.inner(create_area);
                f.render_widget(block, create_area);

                let create_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(1),
                        Constraint::Length(1),
                    ])
                    .split(input_area);
                let list_area = create_chunks[0];
                let help_area = create_chunks[1];

                let mut rendered_lines: Vec<Line> = Vec::new();
                for line in shown_lines {
                    let is_dir = if app.mode == AppMode::NewFolder {
                        true
                    } else {
                        line.trim_start().starts_with('/')
                    };
                    let icon_name = if is_dir {
                        line.trim_start().trim_start_matches('/').trim()
                    } else {
                        line.trim()
                    };
                    let (icon_glyph, icon_style) = App::icon_for_name(
                        icon_name,
                        is_dir,
                        app.show_icons,
                        app.nerd_font_active,
                        false,
                    );
                    let mut spans = Vec::new();
                    if app.show_icons && !icon_glyph.is_empty() {
                        spans.push(Span::styled(format!("{} ", icon_glyph), icon_style));
                    }
                    spans.push(Span::styled(*line, Style::default().fg(Color::Rgb(230, 230, 230))));
                    rendered_lines.push(Line::from(spans));
                }
                f.render_widget(Paragraph::new(rendered_lines), list_area);
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        "(/name = folder, name = file)  Alt+Enter: new line",
                        Style::default().fg(Color::DarkGray),
                    ))),
                    help_area,
                );

                let active_line_text = app.active_input_line_text();
                let active_is_dir = if app.mode == AppMode::NewFolder {
                    true
                } else {
                    active_line_text.trim_start().starts_with('/')
                };
                let active_icon_name = if active_is_dir {
                    active_line_text.trim_start().trim_start_matches('/').trim()
                } else {
                    active_line_text.trim()
                };
                let (active_icon_glyph, _) = App::icon_for_name(
                    active_icon_name,
                    active_is_dir,
                    app.show_icons,
                    app.nerd_font_active,
                    false,
                );
                let icon_prefix_width = if app.show_icons && !active_icon_glyph.is_empty() {
                    UnicodeWidthStr::width(format!("{} ", active_icon_glyph).as_str()) as u16
                } else {
                    0
                };

                app.clamp_input_cursor();
                let visible_cursor_line = cursor_line.saturating_sub(window_start);
                let cursor_x = list_area.x + icon_prefix_width + cursor_col as u16;
                let cursor_y = list_area.y + visible_cursor_line as u16;
                f.set_cursor(
                    cursor_x.min(list_area.x + list_area.width.saturating_sub(1)),
                    cursor_y.min(list_area.y + list_area.height.saturating_sub(1)),
                );
            } else if app.mode == AppMode::Renaming {
                let area = f.size();
                let selected_entry = app.entries.get(app.selected_index);
                let old_name = selected_entry
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .unwrap_or_else(|| app.input_buffer.clone());
                let selected_path = selected_entry.map(|e| e.path());
                let selected_is_dir = selected_path.as_ref().map(|p| p.is_dir()).unwrap_or(false);
                let selected_is_symlink = selected_path
                    .as_ref()
                    .and_then(|p| p.symlink_metadata().ok())
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                let dialog_w = (area.width * 2 / 3).max(36).min(area.width.saturating_sub(4).max(1));
                let dialog_h = 3u16.min(area.height.saturating_sub(2).max(1));
                let rename_area = Rect::new(
                    (area.width.saturating_sub(dialog_w)) / 2,
                    (area.height.saturating_sub(dialog_h)) / 2,
                    dialog_w,
                    dialog_h,
                );
                let title = format!(" Rename \"{}\" ", old_name);
                f.render_widget(Clear, rename_area);
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(title)
                    .title_style(Style::default().fg(Color::White))
                    .border_style(Style::default().fg(Color::Rgb(120, 120, 120)));
                let input_area = block.inner(rename_area);
                f.render_widget(block, rename_area);

                let (icon_glyph, icon_style) = App::icon_for_name(
                    app.input_buffer.as_str(),
                    selected_is_dir,
                    app.show_icons,
                    app.nerd_font_active,
                    selected_is_symlink,
                );
                let icon_prefix = if app.show_icons && !icon_glyph.is_empty() {
                    format!("{} ", icon_glyph)
                } else {
                    String::new()
                };
                let mut spans = Vec::new();
                if !icon_prefix.is_empty() {
                    spans.push(Span::styled(icon_prefix.clone(), icon_style));
                }
                spans.push(Span::styled(
                    app.input_buffer.as_str(),
                    Style::default().fg(Color::Rgb(230, 230, 230)),
                ));
                f.render_widget(Paragraph::new(Line::from(spans)), input_area);

                app.clamp_input_cursor();
                let cursor_x = input_area.x
                    + UnicodeWidthStr::width(icon_prefix.as_str()) as u16
                    + app.input_cursor as u16;
                let cursor_y = input_area.y;
                f.set_cursor(cursor_x.min(input_area.x + input_area.width.saturating_sub(1)), cursor_y);
            } else if matches!(app.mode, AppMode::PasteRenaming | AppMode::ArchiveCreate | AppMode::NoteEditing | AppMode::CommandInput | AppMode::GitCommitMessage | AppMode::GitTagInput) {
                let area = f.size();
                let rename_area = Rect::new(area.width/4, area.height/2 - 1, area.width/2, 3);
                f.render_widget(Clear, rename_area);
                let title = match app.mode {
                    AppMode::PasteRenaming => " Paste As ",
                    AppMode::NewFile => " New File Name ",
                    AppMode::NewFolder => " New Folder Name ",
                    AppMode::ArchiveCreate => " Create Archive (Enter=Confirm, Esc=Cancel) ",
                    AppMode::NoteEditing => " Note (Enter=Save, Esc=Cancel) ",
                    AppMode::CommandInput => " Command (; Enter=Run, Esc=Cancel) ",
                    AppMode::GitCommitMessage => " Commit Message (Enter=Commit+Push, Esc=Cancel) ",
                    AppMode::GitTagInput => " Tag (Enter=Create+Push Tag, Esc=Cancel) ",
                    _ => " New Name ",
                };
                let prompt_value = app.input_buffer.clone();
                f.render_widget(Paragraph::new(prompt_value).block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(title).title_style(Style::default().fg(Color::White))), rename_area);
                app.clamp_input_cursor();
                let cursor_x = rename_area.x + 1 + app.input_cursor as u16;
                let cursor_y = rename_area.y + 1;
                f.set_cursor(cursor_x.min(rename_area.x + rename_area.width.saturating_sub(1)), cursor_y);
            } else if app.mode == AppMode::Bookmarks {
                let area = f.size();
                let bookmarks = App::load_bookmarks();
                if !bookmarks.is_empty() && app.bookmark_selected >= bookmarks.len() {
                    app.bookmark_selected = bookmarks.len() - 1;
                }
                let mut lines: Vec<Line> = vec![Line::from("")];
                let bm_w = (area.width * 2 / 3).max(50).min(tab_overlay_anchor.width);
                let bm_content_w = bm_w.saturating_sub(2) as usize;
                for (row_idx, (i, path)) in bookmarks.iter().enumerate() {
                    let is_selected = row_idx == app.bookmark_selected;
                    let base_style = if is_selected {
                        Style::default().bg(Color::Rgb(60, 60, 60)).fg(Color::White)
                    } else {
                        Style::default()
                    };

                    let (label, style) = match path {
                        Some(p) => (
                            format!(" [{}]  {}", i, p.display()),
                            Style::default().fg(Color::Rgb(100, 220, 120)).patch(base_style),
                        ),
                        None => (
                            format!(" [{}]  (not set)", i),
                            Style::default().fg(Color::Rgb(80, 80, 80)).patch(base_style),
                        ),
                    };

                    let padded_label = if is_selected {
                        let used_w = UnicodeWidthStr::width(label.as_str());
                        if bm_content_w > used_w {
                            format!("{}{}", label, " ".repeat(bm_content_w - used_w))
                        } else {
                            label
                        }
                    } else {
                        label
                    };

                    lines.push(Line::from(Span::styled(padded_label, style)));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(" Add to your shell config to set bookmarks:", Style::default().fg(Color::Rgb(200, 180, 80)))));
                lines.push(Line::from(Span::styled("  export SB_BOOKMARK_1=\"$HOME/.config\"", Style::default().fg(Color::DarkGray))));
                lines.push(Line::from(Span::styled("  export SB_BOOKMARK_2=\"/var/log\"", Style::default().fg(Color::DarkGray))));
                let bm_h = (lines.len() as u16 + 4).max(17).min(tab_overlay_anchor.height);
                let bm_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    bm_w,
                    bm_h,
                );
                f.render_widget(Clear, bm_area);
                let bm_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(App::panel_tab_bar_line(app.panel_tab))
                    .title_style(Style::default().fg(Color::White))
                    .border_style(Style::default().fg(Color::Rgb(80, 200, 180)));
                let bm_inner = bm_block.inner(bm_area);
                f.render_widget(bm_block, bm_area);
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "x",
                        Style::default().fg(Color::Rgb(170, 170, 170)),
                    )),
                    App::tabbed_overlay_close_area(bm_area),
                );
                let bm_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(2)])
                    .split(bm_inner);
                f.render_widget(Paragraph::new(lines), bm_chunks[0]);
                f.render_widget(
                    Paragraph::new(vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            " ↑↓:navigate  Enter/0-9:jump  Tab:switch tabs  Esc:close",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ]),
                    bm_chunks[1],
                );
            } else if app.mode == AppMode::Integrations {
                let area = f.size();
                let integrations = app.integration_rows_cache.clone();
                let int_w = (area.width * 5 / 6).max(70).min(tab_overlay_anchor.width);
                let int_content_w = int_w.saturating_sub(2) as usize;
                if !integrations.is_empty() && app.integration_selected >= integrations.len() {
                    app.integration_selected = integrations.len() - 1;
                }
                let mut lines: Vec<Line> = vec![Line::from("")];
                for (i, row) in integrations.iter().enumerate() {
                    let is_selected = i == app.integration_selected;
                    let status_text = if row.required || (app.integration_enabled(&row.key) && row.available) {
                        " ✓ ".to_string()
                    } else {
                        " ✕ ".to_string()
                    };
                    let status_style = if row.required || (app.integration_enabled(&row.key) && row.available) {
                        Style::default().fg(Color::Rgb(100, 220, 120))
                    } else {
                        Style::default().fg(Color::Rgb(220, 80, 80))
                    };
                    let base_style = if is_selected {
                        Style::default().bg(Color::Rgb(60, 60, 60)).fg(Color::White)
                    } else {
                        Style::default().fg(Color::Rgb(190, 190, 190))
                    };
                    let name_text = format!("  {:<12}", row.label);
                    let state_text = format!(" {:<10}", row.state);
                    let category_text = format!(" {:<9}", row.category);
                    let purpose_text = format!(" {}", row.description);

                    let name_span = Span::styled(name_text.clone(), base_style);
                    let state_span = Span::styled(
                        state_text.clone(),
                        if row.required {
                            base_style.fg(Color::Rgb(200, 200, 200))
                        } else if app.integration_enabled(&row.key) {
                            base_style.fg(Color::Rgb(255, 220, 140))
                        } else {
                            base_style.fg(Color::Rgb(150, 150, 150))
                        },
                    );
                    let category_span = Span::styled(category_text.clone(), base_style);
                    let purpose_span = Span::styled(purpose_text.clone(), base_style);
                    let mut spans = vec![
                        Span::styled(status_text.clone(), base_style.patch(status_style)),
                        name_span,
                        state_span,
                        category_span,
                        purpose_span,
                    ];

                    if is_selected {
                        let used_w = UnicodeWidthStr::width(status_text.as_str())
                            + UnicodeWidthStr::width(name_text.as_str())
                            + UnicodeWidthStr::width(state_text.as_str())
                            + UnicodeWidthStr::width(category_text.as_str())
                            + UnicodeWidthStr::width(purpose_text.as_str());
                        if int_content_w > used_w {
                            spans.push(Span::styled(
                                " ".repeat(int_content_w - used_w),
                                base_style,
                            ));
                        }
                    }

                    lines.push(Line::from(spans));
                }
                let int_h = (lines.len() as u16 + 4).min(tab_overlay_anchor.height);
                let int_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    int_w,
                    int_h,
                );
                f.render_widget(Clear, int_area);
                let int_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(App::panel_tab_bar_line(app.panel_tab))
                    .title_style(Style::default().fg(Color::White))
                    .border_style(Style::default().fg(Color::Rgb(80, 200, 180)));
                let int_inner = int_block.inner(int_area);
                f.render_widget(int_block, int_area);
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "x",
                        Style::default().fg(Color::Rgb(170, 170, 170)),
                    )),
                    App::tabbed_overlay_close_area(int_area),
                );
                let int_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(2)])
                    .split(int_inner);
                let visible_rows = int_chunks[0].height as usize;
                let selected_line = app.integration_selected + 1;
                let int_scroll = if selected_line + 1 <= visible_rows {
                    0u16
                } else {
                    (selected_line + 1 - visible_rows) as u16
                };
                f.render_widget(
                    Paragraph::new(lines).scroll((int_scroll, 0)),
                    int_chunks[0],
                );
                f.render_widget(
                    Paragraph::new(vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            " ↑↓:navigate  Space:toggle  Enter:install missing  Tab:switch tabs  Esc:close",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ]),
                    int_chunks[1],
                );
            } else if app.mode == AppMode::SortMenu {
                let options = App::sort_mode_options();
                let sort_w = tab_overlay_anchor.width;
                let sort_content_w = sort_w.saturating_sub(2) as usize;
                let mut lines: Vec<Line> = vec![Line::from("")];
                for (idx, mode) in options.iter().enumerate() {
                    let is_selected = idx == app.sort_menu_selected;
                    let is_current = *mode == app.sort_mode;
                    let (nerd_icon, fallback_icon) = match mode {
                        SortMode::NameAsc => ("\u{f15d}", "[A-Z]"),
                        SortMode::NameDesc => ("\u{f15e}", "[Z-A]"),
                        SortMode::ExtensionAsc => ("\u{f1c9}", "[EXT]"),
                        SortMode::SizeAsc => ("\u{f160}", "[SZ+]"),
                        SortMode::SizeDesc => ("\u{f161}", "[SZ-]"),
                        SortMode::ModifiedNewest => ("\u{f017}", "[NEW]"),
                        SortMode::ModifiedOldest => ("\u{f1da}", "[OLD]"),
                    };
                    let sort_icon = if app.nerd_font_active {
                        nerd_icon
                    } else {
                        fallback_icon
                    };
                    let row_text = format!(" {}  {}", sort_icon, mode.label());
                    let row_text = if is_selected {
                        let used_w = UnicodeWidthStr::width(row_text.as_str());
                        if sort_content_w > used_w {
                            format!("{}{}", row_text, " ".repeat(sort_content_w - used_w))
                        } else {
                            row_text
                        }
                    } else {
                        row_text
                    };
                    let style = if is_selected {
                        Style::default().bg(Color::Rgb(60, 60, 60)).fg(Color::White)
                    } else if is_current {
                        Style::default().fg(Color::Rgb(255, 220, 140))
                    } else {
                        Style::default().fg(Color::Rgb(190, 190, 190))
                    };
                    lines.push(Line::from(Span::styled(row_text, style)));
                }

                let sort_h = (lines.len() as u16 + 4).max(10).min(tab_overlay_anchor.height);
                let sort_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    sort_w,
                    sort_h,
                );
                f.render_widget(Clear, sort_area);
                let sort_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(App::panel_tab_bar_line(app.panel_tab))
                    .title_style(Style::default().fg(Color::White))
                    .border_style(Style::default().fg(Color::Rgb(80, 200, 180)));
                let sort_inner = sort_block.inner(sort_area);
                f.render_widget(sort_block, sort_area);
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "x",
                        Style::default().fg(Color::Rgb(170, 170, 170)),
                    )),
                    App::tabbed_overlay_close_area(sort_area),
                );
                let sort_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(2)])
                    .split(sort_inner);
                f.render_widget(Paragraph::new(lines), sort_chunks[0]);
                f.render_widget(
                    Paragraph::new(vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            " ↑↓:navigate  Enter:apply  Tab:switch tabs  Esc:close",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ]),
                    sort_chunks[1],
                );
            } else if app.mode == AppMode::SshPicker {
                let area = f.size();
                let ssh_w = (area.width * 2 / 3).max(60).min(area.width);
                let ssh_popup_w = ssh_w.min(tab_overlay_anchor.width);
                let ssh_content_w = ssh_popup_w.saturating_sub(2) as usize;
                let content_w = ssh_w.saturating_sub(4) as usize;
                let type_w = 6usize;
                let mounted_w = 10usize;
                let available_for_alias_and_detail = content_w.saturating_sub(type_w + mounted_w + 3);
                let alias_w = if available_for_alias_and_detail >= 12 {
                    available_for_alias_and_detail.min(22)
                } else {
                    available_for_alias_and_detail
                };
                let detail_w = available_for_alias_and_detail.saturating_sub(alias_w);
                let trunc = |s: &str, max: usize| -> String {
                    if max == 0 {
                        return String::new();
                    }
                    if s.chars().count() <= max {
                        return s.to_string();
                    }
                    if max == 1 {
                        return "…".to_string();
                    }
                    let mut out = String::new();
                    for ch in s.chars().take(max - 1) {
                        out.push(ch);
                    }
                    out.push('…');
                    out
                };

                let mut lines: Vec<Line> = vec![Line::from("")];
                if app.remote_entries.is_empty() {
                    lines.push(Line::from(Span::styled(" No SSH/rclone/media mounts or mounted archives found", Style::default().fg(Color::Rgb(180, 80, 80)))));
                } else {
                    let mounted_aliases: HashSet<String> = app.ssh_mounts
                        .iter()
                        .map(|m| m._host_alias.clone())
                        .collect();
                    for (i, entry) in app.remote_entries.iter().enumerate() {
                        let is_selected = i == app.ssh_picker_selection;
                        let is_mounted = match entry {
                            RemoteEntry::ArchiveMount { .. } | RemoteEntry::LocalMount { .. } => true,
                            _ => mounted_aliases.contains(entry.alias()),
                        };
                        let mount_tag = if is_mounted { "  \u{25cf} mounted" } else { "" };
                        let (type_tag, detail) = match entry {
                            RemoteEntry::Ssh(h) => {
                                let user_at_host = match &h.user {
                                    Some(u) => format!("{}@{}", u, h.hostname),
                                    None => h.hostname.clone(),
                                };
                                let port_str = h.port.map(|p| format!(":{}", p)).unwrap_or_default();
                                ("ssh", format!("{}{}", user_at_host, port_str))
                            }
                            RemoteEntry::Rclone { rtype, .. } => ("rclone", rtype.clone()),
                            RemoteEntry::ArchiveMount { mount_path, .. } => ("zip", mount_path.to_string_lossy().into_owned()),
                            RemoteEntry::LocalMount { mount_path, source, .. } => ("mount", format!("{}: {}", source, mount_path.to_string_lossy())),
                        };
                        let type_col = format!("{:<width$}", type_tag, width = type_w);
                        let alias_col = format!(
                            "{:<width$}",
                            trunc(entry.alias(), alias_w),
                            width = alias_w
                        );
                        let detail_col = trunc(&detail, detail_w);
                        let label = format!(" {} {} {}{}", type_col, alias_col, detail_col, mount_tag);
                        let label = if is_selected {
                            let used_w = UnicodeWidthStr::width(label.as_str());
                            if ssh_content_w > used_w {
                                format!("{}{}", label, " ".repeat(ssh_content_w - used_w))
                            } else {
                                label
                            }
                        } else {
                            label
                        };
                        let style = if is_selected {
                            Style::default().fg(Color::White).bg(Color::Rgb(60, 60, 60)).add_modifier(Modifier::BOLD)
                        } else if is_mounted {
                            Style::default().fg(Color::Rgb(80, 220, 160))
                        } else {
                            Style::default().fg(Color::Rgb(200, 200, 200))
                        };
                        lines.push(Line::from(Span::styled(label, style)));
                    }
                }
                let ssh_h = (lines.len() as u16 + 4).max(8).min(tab_overlay_anchor.height);
                let ssh_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    ssh_popup_w,
                    ssh_h,
                );
                f.render_widget(Clear, ssh_area);
                let ssh_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(App::panel_tab_bar_line(app.panel_tab))
                    .title_style(Style::default().fg(Color::White))
                    .border_style(Style::default().fg(Color::Rgb(80, 200, 180)));
                let ssh_inner = ssh_block.inner(ssh_area);
                f.render_widget(ssh_block, ssh_area);
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "x",
                        Style::default().fg(Color::Rgb(170, 170, 170)),
                    )),
                    App::tabbed_overlay_close_area(ssh_area),
                );
                let ssh_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(2)])
                    .split(ssh_inner);
                f.render_widget(Paragraph::new(lines), ssh_chunks[0]);
                f.render_widget(
                    Paragraph::new(vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            " ↑↓:navigate  Enter/→:open or mount  Tab:switch tabs  u/Delete:unmount  Esc:close",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ]),
                    ssh_chunks[1],
                );
            } else if app.mode == AppMode::ConfirmExtract {
                let area = f.size();
                let to_extract = &app.archive_extract_targets;
                let mut msg_lines: Vec<String> = vec!["Extract selected archives?".to_string(), String::new()];
                let max_list_rows = ((area.height.saturating_sub(10) as usize).min(14)).max(1);
                for (idx, path) in to_extract.iter().enumerate() {
                    if idx >= max_list_rows {
                        break;
                    }
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.to_string_lossy().into_owned());
                    msg_lines.push(format!(" - {}", name));
                }
                if to_extract.len() > max_list_rows {
                    let remaining = to_extract.len() - max_list_rows;
                    msg_lines.push(format!(" ... and {} more", remaining));
                }
                msg_lines.push(String::new());
                msg_lines.push("Each archive is extracted to its own folder".to_string());
                msg_lines.push("  y = confirm    n / Esc = cancel".to_string());
                let msg = msg_lines.join("\n");

                let content_w = msg_lines
                    .iter()
                    .map(|line| line.chars().count() as u16)
                    .max()
                    .unwrap_or(28);
                let content_h = msg_lines.len() as u16;
                let max_w = area.width.saturating_sub(4).max(1);
                let max_h = area.height.saturating_sub(4).max(1);
                let dialog_w = (content_w + 2)
                    .max(40)
                    .min(max_w);
                let dialog_h = (content_h + 2)
                    .max(7)
                    .min(max_h);
                let confirm_area = Rect::new(
                    (area.width.saturating_sub(dialog_w)) / 2,
                    (area.height.saturating_sub(dialog_h)) / 2,
                    dialog_w,
                    dialog_h,
                );
                f.render_widget(Clear, confirm_area);
                f.render_widget(
                    Paragraph::new(msg)
                        .wrap(Wrap { trim: true })
                        .style(Style::default().fg(Color::Rgb(140, 200, 255)))
                        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Confirm Extract ").title_style(Style::default().fg(Color::White))),
                    confirm_area,
                );
            } else if app.mode == AppMode::ConfirmIntegrationInstall {
                let area = f.size();
                let key = app.integration_install_key.clone().unwrap_or_else(|| "(unknown)".to_string());
                let package = app.integration_install_package.clone().unwrap_or_else(|| "(unknown)".to_string());
                let brew_display = app
                    .integration_install_brew_path
                    .clone()
                    .unwrap_or_else(|| "brew (not found)".to_string());

                let mut msg_lines: Vec<String> = vec![
                    "Install missing integration?".to_string(),
                    String::new(),
                    format!(" Integration: {}", key),
                    format!(" Package:     {}", package),
                    format!(" Command:     {} install {}", brew_display, package),
                    String::new(),
                ];

                if app.integration_install_brew_path.is_none() {
                    msg_lines.push("Homebrew is not installed; setup guidance will be shown first.".to_string());
                    msg_lines.push(String::new());
                }

                msg_lines.push("  y = install    n / Esc = cancel".to_string());

                let msg = msg_lines.join("\n");
                let content_w = msg_lines
                    .iter()
                    .map(|line| line.chars().count() as u16)
                    .max()
                    .unwrap_or(36);
                let content_h = msg_lines.len() as u16;
                let max_w = area.width.saturating_sub(4).max(1);
                let max_h = area.height.saturating_sub(4).max(1);
                let dialog_w = (content_w + 2)
                    .max(56)
                    .min(max_w);
                let dialog_h = (content_h + 2)
                    .max(8)
                    .min(max_h);
                let confirm_area = Rect::new(
                    (area.width.saturating_sub(dialog_w)) / 2,
                    (area.height.saturating_sub(dialog_h)) / 2,
                    dialog_w,
                    dialog_h,
                );
                f.render_widget(Clear, confirm_area);
                f.render_widget(
                    Paragraph::new(msg)
                        .wrap(Wrap { trim: true })
                        .style(Style::default().fg(Color::Rgb(140, 200, 255)))
                        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Install Integration ").title_style(Style::default().fg(Color::White))),
                    confirm_area,
                );
            } else if app.mode == AppMode::ConfirmDelete {
                let area = f.size();
                let to_delete = app.delete_targets();
                let (mut file_count, mut folder_count) = (0usize, 0usize);
                for path in &to_delete {
                    if path.is_dir() {
                        folder_count += 1;
                    } else {
                        file_count += 1;
                    }
                }

                let plural = |count: usize, singular: &str, plural: &str| -> String {
                    if count == 1 {
                        singular.to_string()
                    } else {
                        plural.to_string()
                    }
                };
                let title = if file_count > 0 && folder_count > 0 {
                    format!(
                        " Delete {} {} and {} {}? ",
                        file_count,
                        plural(file_count, "file", "files"),
                        folder_count,
                        plural(folder_count, "folder", "folders")
                    )
                } else if folder_count > 0 {
                    format!(
                        " Delete {} {}? ",
                        folder_count,
                        plural(folder_count, "folder", "folders")
                    )
                } else {
                    format!(
                        " Delete {} {}? ",
                        file_count,
                        plural(file_count, "file", "files")
                    )
                };

                let content_w = title.chars().count().max(42) as u16;
                let content_h = area.height.saturating_sub(8).max(7);
                let max_w = area.width.saturating_sub(4).max(1);
                let max_h = area.height.saturating_sub(4).max(1);
                let dialog_w = (content_w + 2)
                    .max(48)
                    .min(max_w);
                let full_dialog_h = (content_h + 2)
                    .max(10)
                    .min(max_h);
                let dialog_h = (full_dialog_h / 2)
                    .max(8)
                    .min(max_h);
                let confirm_area = Rect::new(
                    (area.width.saturating_sub(dialog_w)) / 2,
                    (area.height.saturating_sub(dialog_h)) / 2,
                    dialog_w,
                    dialog_h,
                );
                f.render_widget(Clear, confirm_area);

                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(title)
                    .title_style(Style::default().fg(Color::White))
                    .border_style(Style::default().fg(Color::Rgb(255, 100, 100)));
                let inner = block.inner(confirm_area);
                f.render_widget(block, confirm_area);

                if inner.width > 2 && inner.height > 2 {
                    let sections = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Min(1),
                            Constraint::Length(1),
                        ])
                        .split(inner);

                    let list_block = Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Rgb(90, 90, 90)));
                    let list_frame_area = sections[0];
                    let list_inner = list_block.inner(list_frame_area);
                    f.render_widget(list_block, list_frame_area);

                    let needs_scroll = to_delete.len() > list_inner.height as usize;
                    let can_draw_scrollbar = list_inner.width > 4 && needs_scroll;
                    let list_area = if can_draw_scrollbar {
                        Rect::new(list_inner.x, list_inner.y, list_inner.width.saturating_sub(1), list_inner.height)
                    } else {
                        list_inner
                    };
                    let visible_rows = list_area.height.max(1) as usize;
                    let max_scroll = to_delete.len().saturating_sub(visible_rows);
                    app.confirm_delete_max_offset = max_scroll as u16;
                    let offset = (app.confirm_delete_scroll_offset as usize).min(max_scroll);
                    app.confirm_delete_scroll_offset = offset as u16;

                    let mut list_lines: Vec<Line> = Vec::new();
                    if to_delete.is_empty() {
                        list_lines.push(Line::from(Span::styled(
                            "No selected item",
                            Style::default().fg(Color::Rgb(210, 170, 170)),
                        )));
                    } else {
                        let row_name_max = list_area.width.saturating_sub(4) as usize;
                        let truncate = |s: &str, max: usize| -> String {
                            if max <= 1 {
                                return "…".to_string();
                            }
                            let len = s.chars().count();
                            if len <= max {
                                return s.to_string();
                            }
                            s.chars().take(max - 1).collect::<String>() + "…"
                        };

                        for path in to_delete.iter().skip(offset).take(visible_rows) {
                            let name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.to_string_lossy().into_owned());
                            let path_is_symlink = path.symlink_metadata()
                                .map(|m| m.file_type().is_symlink())
                                .unwrap_or(false);
                            let (icon_glyph, icon_style) = App::icon_for_path(path, app.show_icons, app.nerd_font_active, path_is_symlink);
                            let mut spans: Vec<Span> = Vec::new();
                            if app.show_icons && !icon_glyph.is_empty() {
                                spans.push(Span::styled(format!("{} ", icon_glyph), icon_style));
                            }
                            spans.push(Span::styled(
                                truncate(&name, row_name_max.max(1)),
                                Style::default().fg(Color::Rgb(240, 240, 240)),
                            ));
                            list_lines.push(Line::from(spans));
                        }
                    }
                    f.render_widget(Paragraph::new(list_lines), list_area);

                    if can_draw_scrollbar {
                        let sb_area = Rect::new(
                            list_inner.x + list_inner.width.saturating_sub(1),
                            list_inner.y,
                            1,
                            list_inner.height,
                        );
                        let track_h = sb_area.height as usize;
                        if track_h > 0 {
                            let mut sb_lines: Vec<Line> = Vec::with_capacity(track_h);
                            let thumb_h = if to_delete.is_empty() {
                                track_h
                            } else {
                                ((visible_rows * track_h + to_delete.len() - 1) / to_delete.len())
                                    .max(1)
                                    .min(track_h)
                            };
                            let scroll_space = track_h.saturating_sub(thumb_h);
                            let thumb_y = if max_scroll == 0 {
                                0
                            } else {
                                (offset * scroll_space + (max_scroll / 2)) / max_scroll
                            };

                            for row in 0..track_h {
                                let in_thumb = row >= thumb_y && row < thumb_y + thumb_h;
                                let (ch, color) = if in_thumb {
                                    ("█", Color::Rgb(255, 120, 120))
                                } else {
                                    ("│", Color::Rgb(90, 70, 70))
                                };
                                sb_lines.push(Line::from(Span::styled(ch, Style::default().fg(color))));
                            }
                            f.render_widget(Paragraph::new(sb_lines), sb_area);
                        }
                    }

                    let confirm_focused = app.confirm_delete_button_focus == 0;
                    let cancel_focused = !confirm_focused;
                    let confirm_style = if confirm_focused {
                        Style::default()
                            .fg(Color::Rgb(20, 20, 30))
                            .bg(Color::Rgb(255, 130, 130))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(220, 200, 200))
                    };
                    let cancel_style = if cancel_focused {
                        Style::default()
                            .fg(Color::Rgb(20, 20, 30))
                            .bg(Color::Rgb(200, 200, 220))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(220, 200, 200))
                    };

                    let button_line = Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled("  Confirm  ", confirm_style),
                        Span::styled("    ", Style::default()),
                        Span::styled("  Cancel  ", cancel_style),
                    ]);
                    f.render_widget(Paragraph::new(button_line).alignment(Alignment::Center), sections[1]);
                }
            }

            // --- Footer ---
            let total_entries = app.entries.len();
            let selected_ordinal = if total_entries == 0 {
                0
            } else {
                app.selected_index.min(total_entries - 1) + 1
            };
            let mut left_status_parts = vec![format!("{}/{}", selected_ordinal, total_entries)];
            if !app.clipboard.is_empty() {
                left_status_parts.push(format!("Clipboard:{}", app.clipboard.len()));
            }
            let left_status = left_status_parts.join(" │ ");
            let right_status = "c:Copy v:paste m:Move r:Rename d:Del e:Edit s:Size o:Open-GUI f:Find g:Grep h:Help q:Quit";
            let width = chunks[1].width as usize;
            let left_len = left_status.chars().count();
            let right_len = right_status.chars().count();

            let (gap, right_display) = if left_len + right_len <= width {
                (
                    " ".repeat(width.saturating_sub(left_len + right_len)),
                    right_status.to_string(),
                )
            } else {
                let available = width.saturating_sub(left_len + 1);
                let right_trimmed = if available == 0 {
                    String::new()
                } else {
                    let tail: String = right_status
                        .chars()
                        .rev()
                        .take(available)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    format!("{:>width$}", tail, width = available)
                };
                (" ".to_string(), right_trimmed)
            };

            let mut left_spans: Vec<Span> = Vec::new();
            let mut left_segment = String::new();
            let mut left_in_ws = true;
            for ch in left_status.chars() {
                let is_ws = ch.is_whitespace();
                if left_segment.is_empty() {
                    left_in_ws = is_ws;
                }
                if is_ws == left_in_ws {
                    left_segment.push(ch);
                } else {
                    if left_in_ws {
                        left_spans.push(Span::styled(left_segment.clone(), Style::default().fg(Color::DarkGray)));
                    } else if let Some(colon_idx) = left_segment.find(':') {
                        let (key, rest) = left_segment.split_at(colon_idx);
                        if !key.is_empty() {
                            left_spans.push(Span::styled(key.to_string(), Style::default().fg(Color::DarkGray)));
                        }
                        if let Some(stripped) = rest.strip_prefix(':') {
                            left_spans.push(Span::styled(":", Style::default().fg(Color::DarkGray)));
                            left_spans.push(Span::styled(stripped.to_string(), Style::default().fg(Color::White)));
                        } else {
                            left_spans.push(Span::styled(rest.to_string(), Style::default().fg(Color::DarkGray)));
                        }
                    } else {
                        left_spans.push(Span::styled(left_segment.clone(), Style::default().fg(Color::DarkGray)));
                    }
                    left_segment.clear();
                    left_segment.push(ch);
                    left_in_ws = is_ws;
                }
            }
            if !left_segment.is_empty() {
                if left_in_ws {
                    left_spans.push(Span::styled(left_segment, Style::default().fg(Color::DarkGray)));
                } else if let Some(colon_idx) = left_segment.find(':') {
                    let (key, rest) = left_segment.split_at(colon_idx);
                    if !key.is_empty() {
                        left_spans.push(Span::styled(key.to_string(), Style::default().fg(Color::DarkGray)));
                    }
                    if let Some(stripped) = rest.strip_prefix(':') {
                        left_spans.push(Span::styled(":", Style::default().fg(Color::DarkGray)));
                        left_spans.push(Span::styled(stripped.to_string(), Style::default().fg(Color::White)));
                    } else {
                        left_spans.push(Span::styled(rest.to_string(), Style::default().fg(Color::DarkGray)));
                    }
                } else {
                    left_spans.push(Span::styled(left_segment, Style::default().fg(Color::DarkGray)));
                }
            }

            let mut right_spans: Vec<Span> = Vec::new();
            let mut segment = String::new();
            let mut in_ws = true;
            for ch in right_display.chars() {
                let is_ws = ch.is_whitespace();
                if segment.is_empty() {
                    in_ws = is_ws;
                }
                if is_ws == in_ws {
                    segment.push(ch);
                } else {
                    if in_ws {
                        right_spans.push(Span::styled(segment.clone(), Style::default().fg(Color::DarkGray)));
                    } else if let Some(colon_idx) = segment.find(':') {
                        let (key, rest) = segment.split_at(colon_idx);
                        if !key.is_empty() {
                            right_spans.push(Span::styled(key.to_string(), Style::default().fg(Color::White)));
                        }
                        right_spans.push(Span::styled(rest.to_string(), Style::default().fg(Color::DarkGray)));
                    } else {
                        right_spans.push(Span::styled(segment.clone(), Style::default().fg(Color::DarkGray)));
                    }
                    segment.clear();
                    segment.push(ch);
                    in_ws = is_ws;
                }
            }
            if !segment.is_empty() {
                if in_ws {
                    right_spans.push(Span::styled(segment, Style::default().fg(Color::DarkGray)));
                } else if let Some(colon_idx) = segment.find(':') {
                    let (key, rest) = segment.split_at(colon_idx);
                    if !key.is_empty() {
                        right_spans.push(Span::styled(key.to_string(), Style::default().fg(Color::White)));
                    }
                    right_spans.push(Span::styled(rest.to_string(), Style::default().fg(Color::DarkGray)));
                } else {
                    right_spans.push(Span::styled(segment, Style::default().fg(Color::DarkGray)));
                }
            }

            let mut status_spans: Vec<Span> = left_spans;
            status_spans.push(Span::raw(gap));
            status_spans.extend(right_spans);
            let status = Line::from(status_spans);
            f.render_widget(Paragraph::new(status).block(Block::default().borders(Borders::TOP).border_type(BorderType::Rounded).border_style(Style::default().fg(Color::DarkGray))), chunks[1]);
            let selected_total_status = if app.copy_rx.is_none() && app.archive_rx.is_none() {
                app.selected_total_size_status()
            } else {
                None
            };

            let selected_total_is_shown = selected_total_status.is_some();
            let status_line_message = selected_total_status.or_else(|| {
                if app.status_message.is_empty() {
                    None
                } else {
                    Some(app.status_message.clone())
                }
            });

            if let Some(status_text) = status_line_message {
                let msg_area = Rect::new(chunks[1].x, chunks[1].y, chunks[1].width, 1);
                let lower_msg = status_text.to_ascii_lowercase();
                let is_error = lower_msg.contains("error")
                    || lower_msg.contains("failed")
                    || lower_msg.contains("not found")
                    || lower_msg.contains("refresh failed");
                let msg_style = if selected_total_is_shown {
                    Style::default().fg(Color::Rgb(150, 220, 150))
                } else if app.copy_rx.is_some() || app.archive_rx.is_some() {
                    Style::default().fg(Color::Rgb(120, 200, 255))
                } else if is_error {
                    Style::default().fg(Color::Rgb(255, 120, 120))
                } else {
                    Style::default().fg(Color::White)
                };
                let decorated = app.decorate_footer_message(&status_text);
                let message = decorated.as_str();
                let core = format!("─── {} ", message);
                let core_len = core.chars().count();
                let width = msg_area.width as usize;
                let line_msg = if core_len >= width {
                    core.chars().take(width).collect::<String>()
                } else {
                    let remaining = width - core_len;
                    format!("{}{}", core, "─".repeat(remaining))
                };
                f.render_widget(
                    Paragraph::new(line_msg).style(msg_style),
                    msg_area,
                );
            }
        })?;

        let mut next_key: Option<KeyEvent> = deferred_key.take();
        if next_key.is_none() && event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) => {
                    next_key = Some(key);
                }
                Event::Mouse(mouse) => {
                    let area = terminal.size()?;
                    if let Some(simulated_key) = app.handle_mouse_event(mouse, area) {
                        deferred_key = Some(simulated_key);
                    }
                    continue;
                }
                _ => {}
            }
        }

        if let Some(key) = next_key {
            match app.mode {
                AppMode::Browsing => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char(';') => {
                        app.begin_input_edit(AppMode::CommandInput, String::new());
                    }
                    KeyCode::Char('h') => {
                        app.help_scroll_offset = 0;
                        app.panel_tab = 0;
                        app.mode = AppMode::Help;
                    }
                    KeyCode::Char('H') => {
                        if app.integration_active("git")
                            && App::get_git_info(&app.current_dir).is_some()
                        {
                            let fmt = "%C(bold blue)%h%C(reset) - %C(cyan)%ad%C(reset) | %C(yellow)%d%C(reset) %C(white)%s%C(reset) %C(green)[%an]%C(reset)";
                            disable_raw_mode()?;
                            execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                            execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
                            let log_child = Command::new("git")
                                .args([
                                    "log",
                                    "--graph",
                                    &format!("--pretty=format:{}", fmt),
                                    "--date=short",
                                    "--all",
                                    "--color=always",
                                ])
                                .current_dir(&app.current_dir)
                                .stdout(Stdio::piped())
                                .stderr(Stdio::null())
                                .spawn();
                            if let Ok(child) = log_child {
                                let _ = Command::new("less")
                                    .args(["-R"])
                                    .stdin(child.stdout.unwrap())
                                    .status();
                            } else {
                                let _ = Command::new("git")
                                    .args([
                                        "log",
                                        "--graph",
                                        &format!("--pretty=format:{}", fmt),
                                        "--date=short",
                                        "--all",
                                    ])
                                    .current_dir(&app.current_dir)
                                    .status();
                            }
                            execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                            execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
                            enable_raw_mode()?;
                            terminal.clear()?;
                        } else {
                            app.set_status("not a git repository");
                        }
                    }
                    KeyCode::Tab => {
                        let current = app.current_path_edit_value();
                        app.begin_input_edit(AppMode::PathEditing, current);
                    }
                    KeyCode::Char(' ') | KeyCode::Insert => {
                        if !app.entries.is_empty() {
                            if app.marked_indices.contains(&app.selected_index) {
                                app.marked_indices.remove(&app.selected_index);
                            } else {
                                app.marked_indices.insert(app.selected_index);
                            }
                            app.start_selected_total_size_scan();
                            if app.selected_index < app.entries.len() - 1 {
                                app.selected_index += 1;
                                app.table_state.select(Some(app.selected_index));
                            }
                        }
                    }
                    KeyCode::Char('*') => {
                        if !app.entries.is_empty() {
                            if app.marked_indices.len() == app.entries.len() {
                                app.marked_indices.clear();
                            } else {
                                app.marked_indices = (0..app.entries.len()).collect();
                            }
                            app.start_selected_total_size_scan();
                        }
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.copy_full_paths_to_system_clipboard();
                    }
                    KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.begin_note_edit();
                    }
                    KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let _ = app.drop_to_shell();
                        let _ = terminal.clear();
                    }
                    KeyCode::Char('c') | KeyCode::F(5) => {
                        app.clipboard.clear();
                        if !app.marked_indices.is_empty() {
                            // Copy all marked
                            for &idx in &app.marked_indices {
                                if let Some(e) = app.entries.get(idx) { app.clipboard.push(e.path()); }
                            }
                        } else if let Some(e) = app.entries.get(app.selected_index) {
                            // Copy single selected
                            app.clipboard.push(e.path());
                        }
                    }
                    KeyCode::Char('v') => {
                        app.begin_paste();
                    }
                    KeyCode::Char('m') => {
                        app.begin_move();
                    }
                    KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.edit_system_clipboard_via_temp_file()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('d') | KeyCode::Delete => {
                        if !app.entries.is_empty() {
                            app.begin_confirm_delete();
                        }
                    }
                    KeyCode::Char('x') => {
                        app.toggle_executable_permissions();
                    }
                    KeyCode::Char('p') => {
                        if let Some(selected_path) = app.entries.get(app.selected_index).map(|e| e.path()) {
                            if selected_path.is_dir() {
                                app.set_status("age protection works on files only");
                            } else if !app.integration_active("age") {
                                app.set_status("age not found in PATH");
                            } else if App::is_age_protected_file(&selected_path) {
                                app.unprotect_file_with_age(&selected_path)?;
                                terminal.clear()?;
                            } else {
                                app.protect_file_with_age(&selected_path)?;
                                terminal.clear()?;
                            }
                        }
                    }
                    KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.begin_sort_menu();
                    }
                    KeyCode::Char('s') => {
                        let enabled = !app.folder_size_enabled;
                        app.set_folder_size_enabled(enabled);
                    }
                    KeyCode::Char('+') => {
                        if app.consume_quick_tree_double_tap('+') {
                            app.expand_tree_to_max_on_selected_dirs();
                        } else {
                            app.expand_tree_on_selected_dirs(1);
                        }
                    }
                    KeyCode::Char('-') => {
                        if app.consume_quick_tree_double_tap('-') {
                            app.collapse_all_tree_expansions();
                        } else {
                            app.contract_tree_on_selected_dirs(1);
                        }
                    }
                    KeyCode::Char('C') => {
                        app.run_delta_compare()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('o') => {
                        app.open_selected_with_default_app()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('t') => {
                        app.open_todo_file_in_editor()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('i') => {
                        app.open_split_shell_with_less()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('E') => {
                        app.open_split_shell_with_editor()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('l') => {
                        if let Some(entry) = app.entries.get(app.selected_index) {
                            let selected_path = entry.path();
                            if !selected_path.is_dir() {
                                disable_raw_mode()?;
                                execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                if App::is_binary_file(&selected_path) && app.integration_active("hexyl") {
                                    use std::process::Stdio;
                                    let hexyl = Command::new("hexyl")
                                        .arg(&selected_path)
                                        .stdout(Stdio::piped())
                                        .spawn();
                                    if let Ok(child) = hexyl {
                                        let _ = Command::new("less")
                                            .args(["-R"])
                                            .stdin(child.stdout.unwrap())
                                            .status();
                                    } else {
                                        let _ = Command::new("less")
                                            .args(["-R", selected_path.to_str().unwrap_or_default()])
                                            .status();
                                    }
                                } else {
                                    let _ = Command::new("less")
                                        .args(["-R", selected_path.to_str().unwrap_or_default()])
                                        .status();
                                }
                                enable_raw_mode()?;
                                execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                terminal.clear()?;
                            }
                        }
                    }
                    KeyCode::Char('n') => {
                        app.begin_input_edit(AppMode::NewFile, String::new());
                    }
                    KeyCode::Char('Z') => {
                        app.run_zip_action();
                    }
                    KeyCode::Char('~') => {
                        if let Ok(home) = env::var("HOME") {
                            let home_path = PathBuf::from(home);
                            if home_path.is_dir() {
                                app.try_enter_dir(home_path);
                            }
                        }
                    }
                    KeyCode::Char('b') => { app.panel_tab = 2; app.mode = AppMode::Bookmarks; }
                    KeyCode::Char('I') => {
                        app.integration_selected = 0;
                        app.refresh_integration_rows_cache();
                        app.panel_tab = 5;
                        app.mode = AppMode::Integrations;
                    }
                    KeyCode::Char('S') => {
                        let has_sshfs = app.integration_active("sshfs");
                        let has_rclone = app.integration_active("rclone");
                        app.refresh_remote_entries();
                        if app.remote_entries.is_empty() {
                            if !has_sshfs && !has_rclone {
                                app.set_status("No media mounts or mounted archives found (sshfs/rclone not installed)");
                            } else {
                                app.set_status("No SSH/rclone/media mounts or mounted archives found");
                            }
                        } else {
                            app.panel_tab = 3;
                            app.mode = AppMode::SshPicker;
                        }
                    }
                    KeyCode::Char(c @ '0'..='9') => {
                        let idx = (c as u8 - b'0') as usize;
                        if let Ok(path_str) = env::var(format!("SB_BOOKMARK_{}", idx)) {
                            let path = PathBuf::from(&path_str);
                            if path.is_dir() {
                                app.try_enter_dir(path);
                            }
                        }
                    }
                    KeyCode::Char('.') => {
                        app.show_hidden = !app.show_hidden;
                        app.refresh_entries_or_status();
                        app.set_status(if app.show_hidden {
                            "hidden files: shown"
                        } else {
                            "hidden files: hidden"
                        });
                    }
                    KeyCode::F(2) | KeyCode::Char('r') => {
                        if app.marked_indices.len() > 1 {
                            if !app.integration_active("vidir") {
                                app.set_status("vidir not found in PATH");
                            } else {
                                let targets: Vec<PathBuf> = app.entries
                                    .iter()
                                    .enumerate()
                                    .filter(|(i, _)| app.marked_indices.contains(i))
                                    .map(|(_, e)| e.path())
                                    .collect();
                                if targets.is_empty() {
                                    app.set_status("no selected item to rename");
                                } else {
                                    disable_raw_mode()?;
                                    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                    let mut cmd = Command::new("vidir");
                                    for p in &targets {
                                        cmd.arg(p);
                                    }
                                    let _ = cmd.status();
                                    enable_raw_mode()?;
                                    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                    terminal.clear()?;
                                    app.refresh_entries_or_status();
                                }
                            }
                        } else {
                            let target_idx = if app.marked_indices.len() == 1 {
                                *app.marked_indices.iter().next().unwrap_or(&app.selected_index)
                            } else {
                                app.selected_index
                            };
                            if let Some(e) = app.entries.get(target_idx) {
                                app.selected_index = target_idx;
                                app.table_state.select(Some(target_idx));
                                let current_name = e.file_name().to_string_lossy().into_owned();
                                app.begin_input_edit(AppMode::Renaming, current_name);
                            }
                        }
                    }
                    KeyCode::Up | KeyCode::Down => {
                        let mut steps: usize = 1;
                        while steps < 32 && event::poll(Duration::from_millis(0))? {
                            match event::read()? {
                                Event::Key(next)
                                    if next.code == key.code
                                        && next.modifiers == key.modifiers
                                        && next.kind == key.kind =>
                                {
                                    steps += 1;
                                }
                                Event::Key(next) => {
                                    deferred_key = Some(next);
                                    break;
                                }
                                _ => {}
                            }
                        }

                        let delta = if key.code == KeyCode::Up {
                            -(steps as isize)
                        } else {
                            steps as isize
                        };
                        app.move_selection_delta(delta);
                    }
                    KeyCode::PageUp => { app.selected_index = app.selected_index.saturating_sub(app.page_size); app.table_state.select(Some(app.selected_index)); }
                    KeyCode::PageDown => { if !app.entries.is_empty() { app.selected_index = (app.selected_index + app.page_size).min(app.entries.len() - 1); app.table_state.select(Some(app.selected_index)); } }
                    KeyCode::Home => { app.selected_index = 0; app.table_state.select(Some(0)); }
                    KeyCode::End => { if !app.entries.is_empty() { app.selected_index = app.entries.len() - 1; app.table_state.select(Some(app.selected_index)); } }
                    KeyCode::Left | KeyCode::Backspace => {
                        if !app.try_leave_archive() && !app.try_leave_ssh_mount() {
                            app.try_enter_parent_dir();
                        }
                    }
                    KeyCode::Enter | KeyCode::Right => {
                        if let Some(selected_path) = app.entries.get(app.selected_index).map(|e| e.path()) {
                            if selected_path.is_dir() { app.try_enter_dir(selected_path); }
                            else if App::is_age_protected_file(&selected_path) {
                                if !app.integration_active("age") {
                                    app.set_status("age not found in PATH");
                                } else if app.preview_age_file(&selected_path)? {
                                    terminal.clear()?;
                                }
                            }
                            else if App::is_fuse_zip_archive(&selected_path) && app.integration_active("fuse-zip") {
                                let _ = app.try_mount_archive(selected_path);
                            }
                            else if App::is_archivemount_archive(&selected_path) && app.integration_active("archivemount") {
                                let _ = app.try_mount_archive_with(selected_path, "archivemount");
                            }
                            else if App::is_supported_archive(&selected_path) {
                                let _ = app.preview_archive_contents(&selected_path);
                                terminal.clear()?;
                            }
                            else if App::is_image_file(&selected_path) && app.integration_active("viu") {
                                app.preview_images_with_viu(selected_path)?;
                                terminal.clear()?;
                            }
                            else if App::is_image_file(&selected_path) && app.integration_active("chafa") {
                                app.preview_images_with_chafa(selected_path)?;
                                terminal.clear()?;
                            }
                            else if App::is_markdown_file(&selected_path) && app.integration_active("glow") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                let _ = Command::new("glow")
                                    .arg("-p")
                                    .arg(&selected_path)
                                    .status();
                                execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_mermaid_file(&selected_path) && app.integration_active("mmdflux") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                if let Ok(mut child) = Command::new("mmdflux")
                                    .arg(&selected_path)
                                    .stdout(Stdio::piped())
                                    .spawn()
                                {
                                    if let Some(mmd_out) = child.stdout.take() {
                                        let _ = Command::new("less")
                                            .args(["-R"])
                                            .stdin(mmd_out)
                                            .status();
                                    }
                                    let _ = child.wait();
                                }
                                execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_html_file(&selected_path) && app.integration_active("links") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                let _ = Command::new("links").arg(&selected_path).status();
                                execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_json_file(&selected_path) && app.integration_active("jnv") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
                                let _ = App::preview_json_with_jnv(&selected_path);
                                execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_delimited_text_file(&selected_path) && app.integration_active("csvlens") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                let _ = Command::new("csvlens").arg(&selected_path).status();
                                execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_sqlite_db_file(&selected_path) {
                                if app.integration_active("sqlite3") {
                                    app.begin_sqlite_preview(selected_path);
                                } else {
                                    app.set_status("sqlite3 not found in PATH");
                                }
                            }
                            else if App::is_audio_file(&selected_path) && app.integration_active("sox") {
                                use std::process::Stdio;
                                disable_raw_mode()?;
                                execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;

                                let mut child = if App::integration_probe("play").0 {
                                    Command::new("play")
                                        .arg(&selected_path)
                                        .stdin(Stdio::null())
                                        .stdout(Stdio::null())
                                        .stderr(Stdio::null())
                                        .spawn()
                                } else {
                                    Command::new("sox")
                                        .arg(&selected_path)
                                        .arg("-d")
                                        .stdin(Stdio::null())
                                        .stdout(Stdio::null())
                                        .stderr(Stdio::null())
                                        .spawn()
                                };

                                if let Ok(ref mut proc) = child {
                                    println!("Playing: {}", selected_path.display());
                                    println!("Press q, Esc, or Left to stop playback.");
                                    enable_raw_mode()?;
                                    loop {
                                        if proc.try_wait()?.is_some() {
                                            break;
                                        }
                                        if event::poll(Duration::from_millis(120))? {
                                            if let Event::Key(k) = event::read()? {
                                                if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc | KeyCode::Left) {
                                                    let _ = proc.kill();
                                                    let _ = proc.wait();
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    disable_raw_mode()?;
                                }

                                execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_pdf_file(&selected_path) && app.integration_active("pdftotext") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;

                                let mut shown = false;
                                if let Ok(mut child) = Command::new("pdftotext")
                                    .args(["-layout", "-nopgbrk"])
                                    .arg(&selected_path)
                                    .arg("-")
                                    .stdout(Stdio::piped())
                                    .spawn()
                                {
                                    if let Some(pdf_text) = child.stdout.take() {
                                        shown = Command::new("less")
                                            .args(["-R"])
                                            .stdin(pdf_text)
                                            .status()
                                            .map(|s| s.success())
                                            .unwrap_or(false);
                                    }
                                    let _ = child.wait();
                                }

                                if !shown {
                                    let _ = Command::new("less")
                                        .args(["-R", selected_path.to_str().unwrap_or_default()])
                                        .status();
                                }

                                execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_cast_file(&selected_path) && app.integration_active("asciinema") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;

                                let _ = App::preview_cast_with_asciinema(&selected_path)?;

                                execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else { 
                                disable_raw_mode()?; execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                if App::is_binary_file(&selected_path) && app.integration_active("hexyl") {
                                    use std::process::Stdio;
                                    let hexyl = Command::new("hexyl")
                                        .arg(&selected_path)
                                        .stdout(Stdio::piped())
                                        .spawn();
                                    if let Ok(child) = hexyl {
                                        let _ = Command::new("less")
                                            .args(["-R"])
                                            .stdin(child.stdout.unwrap())
                                            .status();
                                    }
                                } else if app.integration_active("bat") {
                                    let bat_cmd = App::bat_tool().unwrap_or_else(|| "bat".to_string());
                                    let _ = Command::new(bat_cmd)
                                        .args(["--paging=always", "--style=full", "--color=always"])
                                        .arg(&selected_path)
                                        .status();
                                } else {
                                    let _ = Command::new("less").args(["-R", selected_path.to_str().unwrap()]).status();
                                }
                                enable_raw_mode()?; execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                terminal.clear()?;
                            }
                        }
                    }
                    KeyCode::Char('g') => {
                        let has_rg  = app.integration_active("rg");
                        let has_fzf = app.integration_active("fzf");
                        if has_rg {
                            let tmp = App::create_temp_selection_path("sbrs_fzf_rg_selection");
                            let cmd = if has_fzf {
                                // rg pipes into fzf; fzf writes its selection to temp file.
                                // Using inherited stdio so fzf owns the real TTY on all platforms.
                                format!(
                                    "rg --color=always --line-number --no-heading --smart-case \
                                     --fixed-strings --colors=match:fg:214 '' 2>/dev/null \
                                     | fzf --ansi --exact --layout=reverse --delimiter=: \
                                     | awk -F: '{{print $1}}' > {}",
                                    tmp.display()
                                )
                            } else {
                                // no fzf: pick first file with a match
                                format!(
                                    "rg --files-with-matches '' 2>/dev/null | head -1 > {}",
                                    tmp.display()
                                )
                            };
                            disable_raw_mode()?; execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                            let _ = Command::new("sh")
                                .args(["-c", &cmd])
                                .current_dir(&app.current_dir)
                                .stdin(Stdio::inherit())
                                .stdout(Stdio::inherit())
                                .stderr(Stdio::inherit())
                                .status();
                            enable_raw_mode()?; execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                            terminal.clear()?;
                            let selected = fs::read_to_string(&tmp).unwrap_or_default();
                            let _ = fs::remove_file(&tmp);
                            let first_line = selected.lines().next().unwrap_or("").trim().to_string();
                            if !first_line.is_empty() {
                                let selected_path = app.current_dir.join(&first_line);
                                if let Some(parent) = selected_path.parent() {
                                    app.try_enter_dir(parent.to_path_buf());
                                    if let Some(name) = selected_path.file_name() {
                                        app.select_entry_named(&name.to_string_lossy());
                                    }
                                }
                            }
                        } else {
                            app.start_internal_search_with_scope(InternalSearchScope::Content);
                            app.set_status("rg not found; opened Search in content mode");
                        }
                    }
                    KeyCode::Char('G') => {
                        if !app.integration_active("git") {
                            app.set_status("git not found in PATH");
                        } else {
                            match App::get_git_info(&app.current_dir) {
                                Some((_, true, _)) => {
                                    let confirmed = app.preview_git_diff_and_confirm_commit()?;
                                    terminal.clear()?;
                                    if confirmed {
                                        app.begin_input_edit(AppMode::GitCommitMessage, String::new());
                                        app.set_status("enter commit message (include --amend to amend+force-push)");
                                    } else {
                                        app.set_status("git commit cancelled");
                                    }
                                }
                                Some((_, false, _)) => {
                                    app.set_status("repository is clean");
                                }
                                None => {
                                    app.set_status("not a git repository");
                                }
                            }
                        }
                    }
                    KeyCode::Char('f') => {
                        if app.integration_active("fzf") {
                            let tmp = App::create_temp_selection_path("sbrs_fzf_selection");
                            let cmd = format!(
                                "find . -path '*/.*' -prune -o -print 2>/dev/null | fzf --layout=reverse > {}",
                                tmp.display()
                            );
                            disable_raw_mode()?; execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                            let _ = Command::new("sh")
                                .args(["-c", &cmd])
                                .current_dir(&app.current_dir)
                                .stdin(Stdio::inherit())
                                .stdout(Stdio::inherit())
                                .stderr(Stdio::inherit())
                                .status();
                            enable_raw_mode()?; execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                            terminal.clear()?;
                            let selected = fs::read_to_string(&tmp).unwrap_or_default();
                            let _ = fs::remove_file(&tmp);
                            let selected = selected.trim().to_string();
                            if !selected.is_empty() {
                                let selected_path = app.current_dir.join(&selected);
                                if let Some(parent) = selected_path.parent() {
                                    app.try_enter_dir(parent.to_path_buf());
                                    if let Some(name) = selected_path.file_name() {
                                        app.select_entry_named(&name.to_string_lossy());
                                    }
                                }
                            }
                        } else {
                            app.start_internal_search();
                        }
                    }
                    KeyCode::Char('e') | KeyCode::F(4) => {
                        if let Some(e) = app.entries.get(app.selected_index) {
                            let path = e.path();
                            if path.is_dir() {
                                let current_name = e.file_name().to_string_lossy().into_owned();
                                app.begin_input_edit(AppMode::Renaming, current_name);
                            } else if App::is_age_protected_file(&path) {
                                if !app.integration_active("age") {
                                    app.set_status("age not found in PATH");
                                } else if app.edit_age_file(&path)? {
                                    terminal.clear()?;
                                }
                            } else {
                                disable_raw_mode()?; execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                execute!(io::stdout(), Show)?;
                                if !path.is_dir() && App::is_binary_file(&path) && app.integration_active("hexedit") {
                                    let _ = Command::new("hexedit").arg(&path).status();
                                } else {
                                    let _ = Command::new(env::var("EDITOR").unwrap_or_else(|_| "nano".to_string())).arg(&path).status();
                                }
                                enable_raw_mode()?; execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                execute!(io::stdout(), Hide)?;
                                terminal.clear()?;
                                app.refresh_entries_or_status();
                            }
                        }
                    }
                    _ => {}
                },
                AppMode::PathEditing => match key.code {
                    KeyCode::Enter | KeyCode::Tab => {
                        app.apply_path_input();
                    }
                    KeyCode::Esc => {
                        let had_filter = app.path_input_filter.take().is_some();
                        if had_filter && app.refresh_entries_or_status() {
                            app.set_status("path filter cleared");
                        }
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                    }
                    KeyCode::Backspace => app.input_backspace(),
                    KeyCode::Delete => app.input_delete(),
                    KeyCode::Left => app.input_move_left(),
                    KeyCode::Right => app.input_move_right(),
                    KeyCode::Home => app.input_move_home(),
                    KeyCode::End => app.input_move_end(),
                    KeyCode::Char(c) => app.input_insert_char(c),
                    _ => {}
                },
                AppMode::DbPreview => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        app.mode = AppMode::Browsing;
                    }
                    KeyCode::Left => {
                        app.switch_sqlite_preview_table(-1);
                    }
                    KeyCode::Right => {
                        app.switch_sqlite_preview_table(1);
                    }
                    KeyCode::Home => {
                        if !app.db_preview_tables.is_empty() {
                            app.db_preview_selected = 0;
                            app.refresh_sqlite_preview_rows();
                        }
                    }
                    KeyCode::End => {
                        if !app.db_preview_tables.is_empty() {
                            app.db_preview_selected = app.db_preview_tables.len() - 1;
                            app.refresh_sqlite_preview_rows();
                        }
                    }
                    _ => {}
                },
                AppMode::CommandInput => match key.code {
                    KeyCode::Enter => {
                        let command = app.input_buffer.clone();
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                        app.run_shell_command_and_wait_key(&command)?;
                        terminal.clear()?;
                    }
                    KeyCode::Esc => {
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                        app.set_status("command cancelled");
                    }
                    KeyCode::Backspace => app.input_backspace(),
                    KeyCode::Delete => app.input_delete(),
                    KeyCode::Left => app.input_move_left(),
                    KeyCode::Right => app.input_move_right(),
                    KeyCode::Home => app.input_move_home(),
                    KeyCode::End => app.input_move_end(),
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        app.input_insert_char(c)
                    }
                    _ => {}
                },
                AppMode::GitCommitMessage => match key.code {
                    KeyCode::Enter => {
                        let raw = app.input_buffer.clone();
                        let (commit_message, amend) = App::parse_git_commit_message(&raw);
                        if commit_message.is_empty() {
                            app.set_status("commit message cannot be empty");
                        } else {
                            app.clear_input_edit();
                            app.mode = AppMode::Browsing;
                            app.run_git_commit_and_push(&commit_message, amend)?;
                            terminal.clear()?;
                        }
                    }
                    KeyCode::Esc => {
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                        app.set_status("git commit cancelled");
                        terminal.clear()?;
                    }
                    KeyCode::Backspace => app.input_backspace(),
                    KeyCode::Delete => app.input_delete(),
                    KeyCode::Left => app.input_move_left(),
                    KeyCode::Right => app.input_move_right(),
                    KeyCode::Home => app.input_move_home(),
                    KeyCode::End => app.input_move_end(),
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        app.input_insert_char(c)
                    }
                    _ => {}
                },
                AppMode::GitTagInput => match key.code {
                    KeyCode::Enter => {
                        let tag = app.input_buffer.trim().to_string();
                        if tag.is_empty() {
                            app.set_status("tag cannot be empty");
                        } else {
                            app.clear_input_edit();
                            app.mode = AppMode::Browsing;
                            app.run_git_tag_and_push(&tag)?;
                            terminal.clear()?;
                        }
                    }
                    KeyCode::Esc => {
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                        app.set_status("tag creation cancelled");
                        terminal.clear()?;
                    }
                    KeyCode::Backspace => app.input_backspace(),
                    KeyCode::Delete => app.input_delete(),
                    KeyCode::Left => app.input_move_left(),
                    KeyCode::Right => app.input_move_right(),
                    KeyCode::Home => app.input_move_home(),
                    KeyCode::End => app.input_move_end(),
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        app.input_insert_char(c)
                    }
                    _ => {}
                },
                AppMode::NoteEditing => match key.code {
                    KeyCode::Enter => {
                        app.commit_note_edit();
                    }
                    KeyCode::Esc => {
                        app.note_edit_targets.clear();
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                    }
                    KeyCode::Backspace => app.input_backspace(),
                    KeyCode::Delete => app.input_delete(),
                    KeyCode::Left => app.input_move_left(),
                    KeyCode::Right => app.input_move_right(),
                    KeyCode::Home => app.input_move_home(),
                    KeyCode::End => app.input_move_end(),
                    KeyCode::Char(c) => app.input_insert_char(c),
                    _ => {}
                },
                AppMode::InternalSearch => match key.code {
                    KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if app.internal_search_scope == InternalSearchScope::Content {
                            app.internal_search_limits_menu_open = !app.internal_search_limits_menu_open;
                        }
                    }
                    KeyCode::Esc if app.internal_search_limits_menu_open => {
                        app.internal_search_limits_menu_open = false;
                    }
                    KeyCode::Enter if app.internal_search_limits_menu_open => {
                        app.internal_search_limits_menu_open = false;
                    }
                    KeyCode::Up if app.internal_search_limits_menu_open => {
                        app.internal_search_limits_selected = app.internal_search_limits_selected.saturating_sub(1);
                    }
                    KeyCode::Down if app.internal_search_limits_menu_open => {
                        app.internal_search_limits_selected = (app.internal_search_limits_selected + 1).min(2);
                    }
                    KeyCode::Left if app.internal_search_limits_menu_open => {
                        app.adjust_internal_search_content_limit(false, key.modifiers.contains(KeyModifiers::SHIFT));
                    }
                    KeyCode::Right if app.internal_search_limits_menu_open => {
                        app.adjust_internal_search_content_limit(true, key.modifiers.contains(KeyModifiers::SHIFT));
                    }
                    KeyCode::Char('-') if app.internal_search_limits_menu_open => {
                        app.adjust_internal_search_content_limit(false, key.modifiers.contains(KeyModifiers::SHIFT));
                    }
                    KeyCode::Char('+') if app.internal_search_limits_menu_open => {
                        app.adjust_internal_search_content_limit(true, key.modifiers.contains(KeyModifiers::SHIFT));
                    }
                    KeyCode::Char('=') if app.internal_search_limits_menu_open => {
                        app.adjust_internal_search_content_limit(true, key.modifiers.contains(KeyModifiers::SHIFT));
                    }
                    KeyCode::Char('r') if app.internal_search_limits_menu_open => {
                        app.reset_internal_search_content_limits_to_defaults();
                    }
                    KeyCode::Backspace | KeyCode::Delete | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
                        if app.internal_search_limits_menu_open =>
                    {
                    }
                    KeyCode::Char(_)
                        if app.internal_search_limits_menu_open
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                    }
                    KeyCode::Esc => {
                        app.cancel_internal_search_candidate_scan();
                        app.cancel_internal_search_content_request();
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                    }
                    KeyCode::BackTab => {
                        app.cancel_internal_search_candidate_scan();
                        app.cancel_internal_search_content_request();
                        app.panel_tab = 0;
                        app.help_scroll_offset = 0;
                        app.mode = AppMode::Help;
                    }
                    KeyCode::Tab => {
                        app.cancel_internal_search_candidate_scan();
                        app.cancel_internal_search_content_request();
                        app.panel_tab = 2;
                        app.mode = AppMode::Bookmarks;
                    }
                    KeyCode::Enter => {
                        let selected_path = app.selected_internal_search_path();
                        app.cancel_internal_search_candidate_scan();
                        app.cancel_internal_search_content_request();
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                        if let Some(path) = selected_path {
                            if let Some(parent) = path.parent() {
                                app.try_enter_dir(parent.to_path_buf());
                                if let Some(name) = path.file_name() {
                                    app.select_entry_named(&name.to_string_lossy());
                                }
                            }
                        }
                    }
                    KeyCode::Up => {
                        app.internal_search_selected = app.internal_search_selected.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        let max_idx = app.internal_search_results.len().saturating_sub(1);
                        app.internal_search_selected = (app.internal_search_selected + 1).min(max_idx);
                    }
                    KeyCode::PageUp => {
                        app.internal_search_selected = app.internal_search_selected.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        let max_idx = app.internal_search_results.len().saturating_sub(1);
                        app.internal_search_selected = (app.internal_search_selected + 10).min(max_idx);
                    }
                    KeyCode::Backspace => {
                        app.input_backspace();
                        app.refresh_internal_search_results();
                    }
                    KeyCode::Delete => {
                        app.input_delete();
                        app.refresh_internal_search_results();
                    }
                    KeyCode::Left => app.input_move_left(),
                    KeyCode::Right => app.input_move_right(),
                    KeyCode::Home => {
                        app.input_move_home();
                    }
                    KeyCode::End => {
                        app.input_move_end();
                    }
                    KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_internal_search_scope();
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        app.input_insert_char(c);
                        app.refresh_internal_search_results();
                    }
                    _ => {}
                },
                AppMode::Renaming => match key.code {
                    KeyCode::Enter => {
                        if let Some(e) = app.entries.get(app.selected_index) {
                            let _ = fs::rename(e.path(), app.current_dir.join(&app.input_buffer));
                        }
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                        app.refresh_entries_or_status();
                    }
                    KeyCode::Esc => { app.clear_input_edit(); app.mode = AppMode::Browsing; }
                    KeyCode::Backspace => app.input_backspace(),
                    KeyCode::Delete => app.input_delete(),
                    KeyCode::Left => app.input_move_left(),
                    KeyCode::Right => app.input_move_right(),
                    KeyCode::Home => app.input_move_home(),
                    KeyCode::End => app.input_move_end(),
                    KeyCode::Char(c) => app.input_insert_char(c),
                    _ => {}
                },
                AppMode::PasteRenaming => match key.code {
                    KeyCode::Enter => {
                        let new_name = app.input_buffer.trim().to_string();
                        if new_name.is_empty() {
                            app.set_status("name cannot be empty");
                        } else if let Some(src) = app.paste_current_src.clone() {
                            let target_dir = app
                                .paste_target_dir
                                .as_ref()
                                .cloned()
                                .unwrap_or_else(|| app.current_dir.clone());
                            let dest = target_dir.join(&new_name);
                            if dest.exists() {
                                app.set_status("target still exists: choose another name");
                            } else {
                                app.paste_current_src = None;
                                app.clear_input_edit();
                                app.mode = AppMode::Browsing;
                                if app.paste_move_mode && fs::rename(&src, &dest).is_ok() {
                                    app.paste_ok_items += 1;
                                    let _ = app.refresh_entries();
                                    app.advance_paste_queue();
                                    continue;
                                }
                                app.start_copy_job(src, dest, new_name);
                            }
                        } else {
                            app.mode = AppMode::Browsing;
                        }
                    }
                    KeyCode::Esc => {
                        app.paste_queue.clear();
                        app.paste_current_src = None;
                        app.paste_move_mode = false;
                        app.paste_target_dir = None;
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                        app.set_status("paste cancelled");
                        app.refresh_entries_or_status();
                    }
                    KeyCode::Backspace => app.input_backspace(),
                    KeyCode::Delete => app.input_delete(),
                    KeyCode::Left => app.input_move_left(),
                    KeyCode::Right => app.input_move_right(),
                    KeyCode::Home => app.input_move_home(),
                    KeyCode::End => app.input_move_end(),
                    KeyCode::Char(c) => app.input_insert_char(c),
                    _ => {}
                },
                AppMode::NewFile | AppMode::NewFolder => match key.code {
                    KeyCode::Enter => {
                        if key.modifiers.contains(KeyModifiers::SHIFT)
                            || key.modifiers.contains(KeyModifiers::ALT)
                        {
                            app.input_insert_char('\n');
                        } else {
                            let default_is_dir = app.mode == AppMode::NewFolder;
                            app.create_entries_from_input(default_is_dir);
                        }
                    }

                    KeyCode::Esc => {
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                    }
                    KeyCode::Backspace => app.input_backspace(),
                    KeyCode::Delete => app.input_delete(),
                    KeyCode::Left => app.input_move_left(),
                    KeyCode::Right => app.input_move_right(),
                    KeyCode::Home => app.input_move_home(),
                    KeyCode::End => app.input_move_end(),
                    KeyCode::Char(c) => app.input_insert_char(c),
                    _ => {}
                },
                AppMode::ArchiveCreate => match key.code {
                    KeyCode::Enter => {
                        app.create_archive_from_input();
                    }
                    KeyCode::Esc => {
                        app.archive_create_targets.clear();
                        app.clear_input_edit();
                        app.mode = AppMode::Browsing;
                        app.set_status("archive creation cancelled");
                    }
                    KeyCode::Backspace => app.input_backspace(),
                    KeyCode::Delete => app.input_delete(),
                    KeyCode::Left => app.input_move_left(),
                    KeyCode::Right => app.input_move_right(),
                    KeyCode::Home => app.input_move_home(),
                    KeyCode::End => app.input_move_end(),
                    KeyCode::Char(c) => app.input_insert_char(c),
                    _ => {}
                },
                AppMode::Help => match key.code {
                    KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('q') => {
                        app.mode = AppMode::Browsing;
                    }
                    KeyCode::BackTab => {
                        app.panel_tab = 5;
                        app.integration_selected = 0;
                        app.refresh_integration_rows_cache();
                        app.mode = AppMode::Integrations;
                    }
                    KeyCode::Up => {
                        app.help_scroll_offset = app.help_scroll_offset.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        app.help_scroll_offset = (app.help_scroll_offset + 1).min(app.help_max_offset);
                    }
                    KeyCode::PageUp => {
                        app.help_scroll_offset = app.help_scroll_offset.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        app.help_scroll_offset = (app.help_scroll_offset + 10).min(app.help_max_offset);
                    }
                    KeyCode::Home => {
                        app.help_scroll_offset = 0;
                    }
                    KeyCode::End => {
                        app.help_scroll_offset = app.help_max_offset;
                    }
                    KeyCode::Tab => {
                        app.panel_tab = 1;
                        app.start_internal_search();
                    }
                    _ => {}
                }
                AppMode::Integrations => {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('I') | KeyCode::Char('q') => {
                            app.mode = AppMode::Browsing;
                        }
                        KeyCode::BackTab => {
                            app.begin_sort_menu();
                        }
                        KeyCode::Up => {
                            app.integration_selected = app.integration_selected.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            let max_idx = app.integration_count().saturating_sub(1);
                            app.integration_selected = (app.integration_selected + 1).min(max_idx);
                        }
                        KeyCode::Char(' ') => {
                            if app.integration_selected == 0 {
                                let all_on = app.all_optional_integrations_enabled();
                                app.set_all_optional_integrations(!all_on);
                            } else {
                                let catalog = App::integration_catalog();
                                if let Some(spec) = catalog.get(app.integration_selected - 1) {
                                    let current = app.integration_enabled(spec.key);
                                    app.set_integration_enabled(spec.key, !current);
                                }
                            }
                            app.refresh_integration_rows_cache();
                        }
                        KeyCode::Enter => {
                            app.begin_integration_install_prompt_for_selected();
                        }
                        KeyCode::Tab => {
                            app.panel_tab = 0;
                            app.help_scroll_offset = 0;
                            app.mode = AppMode::Help;
                        }
                        _ => {}
                    }
                }
                AppMode::SortMenu => {
                    match key.code {
                        KeyCode::BackTab => {
                            app.panel_tab = 3;
                            app.refresh_remote_entries();
                            app.mode = AppMode::SshPicker;
                        }
                        KeyCode::Tab => {
                            app.panel_tab = 5;
                            app.integration_selected = 0;
                            app.refresh_integration_rows_cache();
                            app.mode = AppMode::Integrations;
                        }
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left => {
                            app.mode = AppMode::Browsing;
                        }
                        KeyCode::Up => {
                            app.sort_menu_selected = app.sort_menu_selected.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            let max_idx = App::sort_mode_options().len().saturating_sub(1);
                            app.sort_menu_selected = (app.sort_menu_selected + 1).min(max_idx);
                        }
                        KeyCode::Enter | KeyCode::Right => {
                            app.commit_sort_menu_choice();
                        }
                        _ => {}
                    }
                }
                AppMode::SshPicker => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => { app.mode = AppMode::Browsing; }
                    KeyCode::BackTab => {
                        app.panel_tab = 2;
                        app.mode = AppMode::Bookmarks;
                    }
                    KeyCode::Tab => {
                        app.begin_sort_menu();
                    }
                    KeyCode::Up => {
                        if app.ssh_picker_selection > 0 {
                            app.ssh_picker_selection -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if !app.remote_entries.is_empty() && app.ssh_picker_selection < app.remote_entries.len() - 1 {
                            app.ssh_picker_selection += 1;
                        }
                    }
                    KeyCode::Enter | KeyCode::Right => {
                        if let Some(entry) = app.remote_entries.get(app.ssh_picker_selection).cloned() {
                            let alias = entry.alias().to_string();
                            match entry {
                                RemoteEntry::Ssh(host) => {
                                    let already_mounted = app.ssh_mounts.iter().any(|m| m._host_alias == alias);
                                    if already_mounted {
                                        app.mount_ssh_host(&host)?;
                                    } else {
                                        disable_raw_mode()?;
                                        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                        let result = app.mount_ssh_host(&host);
                                        enable_raw_mode()?;
                                        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                        terminal.clear()?;
                                        if result.is_err() {
                                            app.set_status(format!("Failed to mount {}", alias));
                                            app.mode = AppMode::Browsing;
                                        }
                                    }
                                }
                                RemoteEntry::Rclone { name, rtype } => {
                                    let already_mounted = app.ssh_mounts.iter().any(|m| m._host_alias == alias);
                                    if already_mounted {
                                        app.mount_rclone_remote(&name, &rtype)?;
                                    } else {
                                        disable_raw_mode()?;
                                        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
                                        println!("Connecting to rclone remote: {}…", name);
                                        let result = app.mount_rclone_remote(&name, &rtype);
                                        enable_raw_mode()?;
                                        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
                                        terminal.clear()?;
                                        if result.is_err() {
                                            app.set_status(format!("Failed to mount rclone remote {}", name));
                                            app.mode = AppMode::Browsing;
                                        }
                                    }
                                }
                                RemoteEntry::ArchiveMount { mount_path, archive_name } => {
                                    if mount_path.is_dir() {
                                        app.mode = AppMode::Browsing;
                                        app.try_enter_dir(mount_path);
                                    } else {
                                        app.set_status(format!("mount not available: {}", archive_name));
                                        app.mode = AppMode::Browsing;
                                    }
                                }
                                RemoteEntry::LocalMount { mount_path, name, .. } => {
                                    if mount_path.is_dir() {
                                        app.mode = AppMode::Browsing;
                                        app.try_enter_dir(mount_path);
                                    } else {
                                        app.set_status(format!("mount not available: {}", name));
                                        app.mode = AppMode::Browsing;
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char('u') | KeyCode::Delete => {
                        if let Some(entry) = app.remote_entries.get(app.ssh_picker_selection).cloned() {
                            match entry {
                                RemoteEntry::Ssh(host) => {
                                    if app.unmount_ssh_mount_by_alias(&host.alias) {
                                        app.set_status(format!("unmounted {}", host.alias));
                                    } else {
                                        app.set_status(format!("not mounted: {}", host.alias));
                                    }
                                }
                                RemoteEntry::Rclone { name, .. } => {
                                    if app.unmount_ssh_mount_by_alias(&name) {
                                        app.set_status(format!("unmounted {}", name));
                                    } else {
                                        app.set_status(format!("not mounted: {}", name));
                                    }
                                }
                                RemoteEntry::ArchiveMount { mount_path, archive_name } => {
                                    if app.unmount_archive_mount_by_path(&mount_path) {
                                        app.set_status(format!("unmounted {}", archive_name));
                                    } else {
                                        app.set_status(format!("not mounted: {}", archive_name));
                                    }
                                }
                                RemoteEntry::LocalMount { name, .. } => {
                                    app.set_status(format!("external mount: {} (unmount outside sb)", name));
                                }
                            }

                            app.refresh_remote_entries();
                        }
                    }
                    _ => {}
                },
                AppMode::Bookmarks => match key.code {
                    KeyCode::Esc | KeyCode::Char('b') | KeyCode::Char('q') => { app.mode = AppMode::Browsing; }
                    KeyCode::BackTab => {
                        app.panel_tab = 1;
                        app.start_internal_search();
                    }
                    KeyCode::Tab => {
                        app.panel_tab = 3;
                        app.refresh_remote_entries();
                        app.mode = AppMode::SshPicker;
                    }
                    KeyCode::Up => {
                        app.bookmark_selected = app.bookmark_selected.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        let max_idx = App::load_bookmarks().len().saturating_sub(1);
                        app.bookmark_selected = (app.bookmark_selected + 1).min(max_idx);
                    }
                    KeyCode::Enter | KeyCode::Right => {
                        let idx = app.bookmark_selected;
                        if let Ok(path_str) = env::var(format!("SB_BOOKMARK_{}", idx)) {
                            let path = PathBuf::from(&path_str);
                            if path.is_dir() {
                                app.try_enter_dir(path);
                            }
                        }
                        app.mode = AppMode::Browsing;
                    }
                    KeyCode::Char(c @ '0'..='9') => {
                        let idx = (c as u8 - b'0') as usize;
                        if let Ok(path_str) = env::var(format!("SB_BOOKMARK_{}", idx)) {
                            let path = PathBuf::from(&path_str);
                            if path.is_dir() {
                                app.try_enter_dir(path);
                            }
                        }
                        app.mode = AppMode::Browsing;
                    }
                    _ => {}
                },
                AppMode::ConfirmDelete => match key.code {
                    KeyCode::Left => {
                        app.confirm_delete_button_focus = 0;
                    }
                    KeyCode::Right => {
                        app.confirm_delete_button_focus = 1;
                    }
                    KeyCode::Up => {
                        app.confirm_delete_scroll_offset = app.confirm_delete_scroll_offset.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        app.confirm_delete_scroll_offset =
                            (app.confirm_delete_scroll_offset + 1).min(app.confirm_delete_max_offset);
                    }
                    KeyCode::PageUp => {
                        app.confirm_delete_scroll_offset = app.confirm_delete_scroll_offset.saturating_sub(8);
                    }
                    KeyCode::PageDown => {
                        app.confirm_delete_scroll_offset =
                            (app.confirm_delete_scroll_offset + 8).min(app.confirm_delete_max_offset);
                    }
                    KeyCode::Enter | KeyCode::Char('y') => {
                        if key.code == KeyCode::Enter && app.confirm_delete_button_focus == 1 {
                            app.mode = AppMode::Browsing;
                        } else {
                            app.confirm_delete_selected_targets();
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Esc => { app.mode = AppMode::Browsing; }
                    _ => {}
                },
                AppMode::ConfirmExtract => match key.code {
                    KeyCode::Char('y') => {
                        app.mode = AppMode::Browsing;
                        app.extract_archives_confirmed();
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        app.archive_extract_targets.clear();
                        app.mode = AppMode::Browsing;
                        app.set_status("extract cancelled");
                    }
                    _ => {}
                },
                AppMode::ConfirmIntegrationInstall => match key.code {
                    KeyCode::Char('y') => {
                        app.confirm_integration_install()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        app.mode = AppMode::Integrations;
                        app.clear_integration_install_prompt();
                        app.set_status("integration install cancelled");
                    }
                    _ => {}
                },
            }
        }
    }
    app.cleanup_archive_mounts();
    app.cleanup_ssh_mounts();
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        TermClear(ClearType::All),
        MoveTo(0, 0)
    )?;
    let _ = std::fs::write("/tmp/sb_path", app.current_dir.to_string_lossy().as_bytes());
    Ok(())
}