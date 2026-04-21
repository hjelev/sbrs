use chrono::{DateTime, Local};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{style, Attribute, Color as CtColor, Stylize},
    terminal::{disable_raw_mode, enable_raw_mode, Clear as TermClear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use devicons::{icon_for_file, File as DevFile, Theme};
use regex::{Regex, RegexBuilder};
use ratatui::{prelude::*, widgets::*};
use rayon::prelude::*;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    fs,
    io::{self, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    str::FromStr,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

#[derive(Clone)]
struct EntryRenderCache {
    raw_name: String,
    icon_glyph: String,
    icon_style: Style,
    name_style: Style,
    perms_col: String,
    group_name: String,
    owner_name: String,
    size_col: String,
    size_bytes: Option<u64>,
    date_col: String,
}

#[derive(Clone, Copy)]
struct EntryRenderConfig {
    nerd_font_active: bool,
    show_icons: bool,
}

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
enum SortMode {
    NameAsc,
    NameDesc,
    ExtensionAsc,
    SizeAsc,
    SizeDesc,
    ModifiedNewest,
    ModifiedOldest,
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
    CommandInput,
    GitCommitMessage,
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
    integration_overrides: HashMap<String, bool>,
    integration_rows_cache: Vec<IntegrationRow>,
    integration_install_key: Option<String>,
    integration_install_package: Option<String>,
    integration_install_brew_path: Option<String>,
    help_scroll_offset: u16,
    help_max_offset: u16,
    git_info_cache: Option<GitInfoCache>,
    git_info_rx: Option<Receiver<(PathBuf, Option<(String, bool, Option<(String, u64)>)>)>>,
    folder_size_enabled: bool,
    folder_size_rx: Option<Receiver<FolderSizeMsg>>,
    folder_size_scan_id: u64,
    current_dir_total_size_rx: Option<Receiver<CurrentDirTotalSizeMsg>>,
    current_dir_total_size_scan_id: u64,
    current_dir_total_size_pending: bool,
    current_dir_total_size_bytes: Option<u64>,
    current_dir_free_bytes: Option<u64>,
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
}

#[derive(Clone)]
struct IntegrationSpec {
    key: &'static str,
    description: &'static str,
    category: &'static str,
    required: bool,
}

#[derive(Clone)]
struct IntegrationRow {
    key: String,
    label: String,
    state: String,
    category: String,
    description: String,
    available: bool,
    required: bool,
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
            integration_overrides: HashMap::new(),
            integration_rows_cache: Vec::new(),
            integration_install_key: None,
            integration_install_package: None,
            integration_install_brew_path: None,
            help_scroll_offset: 0,
            help_max_offset: 0,
            git_info_cache: None,
            git_info_rx: None,
            folder_size_enabled: false,
            folder_size_rx: None,
            folder_size_scan_id: 0,
            current_dir_total_size_rx: None,
            current_dir_total_size_scan_id: 0,
            current_dir_total_size_pending: false,
            current_dir_total_size_bytes: None,
            current_dir_free_bytes: None,
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
        };
        app.refresh_entries()?;
        app.request_notes_for_current_dir_once();
        app.request_git_info_for_current_dir_once();
        Ok(app)
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
        execute!(io::stdout(), LeaveAlternateScreen)?;
        let result = Self::age_encrypt_file_interactive(input, &protected_path);
        execute!(io::stdout(), EnterAlternateScreen)?;
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
        execute!(io::stdout(), LeaveAlternateScreen)?;
        let result = Self::age_decrypt_file_interactive(input, &plain_path);
        execute!(io::stdout(), EnterAlternateScreen)?;
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
        execute!(io::stdout(), LeaveAlternateScreen)?;
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

        execute!(io::stdout(), EnterAlternateScreen)?;
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
        execute!(io::stdout(), LeaveAlternateScreen)?;
        execute!(io::stdout(), Show)?;
        let decrypted = Self::age_decrypt_file_interactive(input, &tmp_path);
        if decrypted.is_err() {
            execute!(io::stdout(), EnterAlternateScreen)?;
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
        execute!(io::stdout(), EnterAlternateScreen)?;
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

    fn sort_entries_by_mode(entries: &mut Vec<fs::DirEntry>, mode: SortMode) {
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
        let sizes: Vec<u64>    = metas.iter()
            .map(|m| m.as_ref().map(|m| m.len()).unwrap_or(0))
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
        let selected_path = self.entries.get(self.selected_index).map(|e| e.path());
        let marked_paths: HashSet<PathBuf> = self
            .marked_indices
            .iter()
            .filter_map(|idx| self.entries.get(*idx).map(|e| e.path()))
            .collect();

        Self::sort_entries_by_mode(&mut self.entries, self.sort_mode);

        let config = EntryRenderConfig { nerd_font_active: self.nerd_font_active, show_icons: self.show_icons };
        let uid_cache = App::build_uid_cache(&self.entries);
        let gid_cache = App::build_gid_cache(&self.entries);
            self.entry_render_cache = self.entries.iter()
            .map(|entry| App::build_entry_render_cache(entry, config, &uid_cache, &gid_cache))
            .collect();
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

    fn clear_selected_total_size_state(&mut self) {
        self.selected_total_size_scan_id = self.selected_total_size_scan_id.wrapping_add(1);
        self.selected_total_size_rx = None;
        self.selected_total_size_pending = false;
        self.selected_total_size_bytes = None;
        self.selected_total_size_items = 0;
    }

    fn start_selected_total_size_scan(&mut self) {
        if !self.folder_size_enabled || self.marked_indices.len() < 2 {
            self.clear_selected_total_size_state();
            return;
        }

        let targets: Vec<PathBuf> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(i, _)| self.marked_indices.contains(i))
            .map(|(_, e)| e.path())
            .collect();

        if targets.len() < 2 {
            self.clear_selected_total_size_state();
            return;
        }

        self.selected_total_size_scan_id = self.selected_total_size_scan_id.wrapping_add(1);
        let scan_id = self.selected_total_size_scan_id;
        self.selected_total_size_items = targets.len();
        self.selected_total_size_pending = true;
        self.selected_total_size_bytes = None;

        let (tx, rx) = mpsc::channel();
        self.selected_total_size_rx = Some(rx);
        thread::spawn(move || {
            let total = targets
                .par_iter()
                .map(|p| App::compute_total_display_bytes(p).unwrap_or(0))
                .reduce(|| 0u64, |acc, v| acc.saturating_add(v));
            let _ = tx.send(SelectedTotalSizeMsg::Finished(scan_id, total));
        });
    }

    fn pump_selected_total_size_progress(&mut self) {
        let Some(rx) = self.selected_total_size_rx.take() else {
            return;
        };

        let mut keep_rx = true;
        loop {
            match rx.try_recv() {
                Ok(SelectedTotalSizeMsg::Finished(scan_id, bytes)) => {
                    if scan_id == self.selected_total_size_scan_id {
                        self.selected_total_size_bytes = Some(bytes);
                        self.selected_total_size_pending = false;
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

        if keep_rx && self.folder_size_enabled {
            self.selected_total_size_rx = Some(rx);
        }
    }

    fn selected_total_size_status(&self) -> Option<String> {
        let selected_count = self.marked_indices.len();
        if selected_count == 0 {
            return None;
        }

        let noun = if selected_count == 1 { "item" } else { "items" };
        if !self.folder_size_enabled || selected_count < 2 {
            return Some(format!("selected: {} {}", selected_count, noun));
        }

        if self.selected_total_size_pending {
            return Some(format!(
                "selected: {} {} | total size: scanning...",
                self.selected_total_size_items.max(selected_count),
                noun
            ));
        }

        Some(match self.selected_total_size_bytes {
            Some(bytes) => format!(
                "selected: {} {} | total size: {}",
                self.selected_total_size_items.max(selected_count),
                noun,
                Self::format_size(bytes)
            ),
            None => format!("selected: {} {}", selected_count, noun),
        })
    }

    fn start_folder_size_scan(&mut self) {
        if !self.folder_size_enabled {
            return;
        }

        self.folder_size_scan_id = self.folder_size_scan_id.wrapping_add(1);
        let scan_id = self.folder_size_scan_id;

        let dir_paths: Vec<PathBuf> = self
            .entries
            .iter()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();

        if dir_paths.is_empty() {
            self.folder_size_rx = None;
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.folder_size_rx = Some(rx);
        thread::spawn(move || {
            let sized: Vec<(PathBuf, u64)> = dir_paths
                .par_iter()
                .map(|dir| (dir.clone(), App::compute_total_display_bytes(dir).unwrap_or(0)))
                .collect();
            for (dir, size) in sized {
                let _ = tx.send(FolderSizeMsg::EntrySize(scan_id, dir, size));
            }
            let _ = tx.send(FolderSizeMsg::Finished(scan_id));
        });
    }

    fn clear_current_dir_total_size_state(&mut self) {
        self.current_dir_total_size_scan_id = self.current_dir_total_size_scan_id.wrapping_add(1);
        self.current_dir_total_size_rx = None;
        self.current_dir_total_size_pending = false;
        self.current_dir_total_size_bytes = None;
    }

    fn filesystem_free_space_bytes(path: &PathBuf) -> Option<u64> {
        let output = Command::new("df")
            .args(["-kP"])
            .arg(path)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())?;
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            return None;
        }

        let available_kb = u64::from_str(cols[3]).ok()?;
        Some(available_kb.saturating_mul(1024))
    }

    fn refresh_current_dir_free_space(&mut self) {
        self.current_dir_free_bytes = Self::filesystem_free_space_bytes(&self.current_dir);
    }

    fn start_current_dir_total_size_scan(&mut self) {
        if !self.folder_size_enabled {
            return;
        }

        self.current_dir_total_size_scan_id = self.current_dir_total_size_scan_id.wrapping_add(1);
        let scan_id = self.current_dir_total_size_scan_id;
        let current_dir = self.current_dir.clone();
        self.current_dir_total_size_pending = true;
        self.current_dir_total_size_bytes = None;

        let (tx, rx) = mpsc::channel();
        self.current_dir_total_size_rx = Some(rx);
        thread::spawn(move || {
            let total = App::compute_total_display_bytes(&current_dir).unwrap_or(0);
            let _ = tx.send(CurrentDirTotalSizeMsg::Finished(scan_id, total));
        });
    }

    fn pump_current_dir_total_size_progress(&mut self) {
        let Some(rx) = self.current_dir_total_size_rx.take() else {
            return;
        };

        let mut keep_rx = true;
        loop {
            match rx.try_recv() {
                Ok(CurrentDirTotalSizeMsg::Finished(scan_id, bytes)) => {
                    if scan_id == self.current_dir_total_size_scan_id {
                        self.current_dir_total_size_bytes = Some(bytes);
                        self.current_dir_total_size_pending = false;
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

        if keep_rx && self.folder_size_enabled {
            self.current_dir_total_size_rx = Some(rx);
        }
    }

    fn current_dir_total_size_header_suffix(&self) -> Option<String> {
        if !self.folder_size_enabled {
            return None;
        }

        let free_part = self
            .current_dir_free_bytes
            .map(|bytes| format!("free: {}", Self::format_size(bytes)))
            .unwrap_or_else(|| "free: ?".to_string());

        if self.current_dir_total_size_pending {
            return Some(format!("dir size: scanning... | {}", free_part));
        }

        Some(match self.current_dir_total_size_bytes {
            Some(bytes) => format!("dir size: {} | {}", Self::format_size(bytes), free_part),
            None => format!("dir size: ? | {}", free_part),
        })
    }

    fn reset_folder_size_columns(&mut self) {
        let size_width = 6usize;
        for (idx, entry) in self.entries.iter().enumerate() {
            if entry.path().is_dir() {
                self.entry_render_cache[idx].size_col = format!("{:>width$}", "-", width = size_width);
                self.entry_render_cache[idx].size_bytes = None;
            }
        }
    }

    fn set_folder_size_enabled(&mut self, enabled: bool) {
        if enabled == self.folder_size_enabled {
            return;
        }

        self.folder_size_enabled = enabled;
        self.folder_size_scan_id = self.folder_size_scan_id.wrapping_add(1);
        self.folder_size_rx = None;
        self.reset_folder_size_columns();

        if enabled {
            self.set_status("folder size calc: on");
            self.start_folder_size_scan();
            self.start_current_dir_total_size_scan();
            self.start_selected_total_size_scan();
        } else {
            self.set_status("folder size calc: off");
            self.clear_current_dir_total_size_state();
            self.clear_selected_total_size_state();
        }
    }

    fn pump_folder_size_progress(&mut self) {
        let Some(rx) = self.folder_size_rx.take() else {
            return;
        };

        let mut keep_rx = true;
        loop {
            match rx.try_recv() {
                Ok(FolderSizeMsg::EntrySize(scan_id, dir_path, size)) => {
                    if !self.folder_size_enabled || scan_id != self.folder_size_scan_id {
                        continue;
                    }
                    if let Some(idx) = self.entries.iter().position(|e| e.path() == dir_path) {
                        self.entry_render_cache[idx].size_col = format!("{:>width$}", Self::format_size(size), width = 6);
                        self.entry_render_cache[idx].size_bytes = Some(size);
                    }
                }
                Ok(FolderSizeMsg::Finished(scan_id)) => {
                    if scan_id == self.folder_size_scan_id {
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

        if keep_rx && self.folder_size_enabled {
            self.folder_size_rx = Some(rx);
        }
    }

    fn pump_git_info(&mut self) {
        let Some(rx) = self.git_info_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok((path, info)) => {
                self.git_info_cache = Some(GitInfoCache {
                    path,
                    info,
                });
                self.git_info_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.git_info_rx = None;
            }
        }
    }

    fn request_git_info_for_current_dir_once(&mut self) {
        if !self.integration_enabled("git") {
            self.git_info_rx = None;
            self.git_info_cache = None;
            return;
        }
        if self.git_info_rx.is_some() {
            return;
        }
        if self
            .git_info_cache
            .as_ref()
            .map(|cache| cache.path == self.current_dir)
            .unwrap_or(false)
        {
            return;
        }

        // Clear stale data from a previously visited path until the new result arrives.
        self.git_info_cache = None;
        let path = self.current_dir.clone();
        let (tx, rx) = mpsc::channel();
        self.git_info_rx = Some(rx);
        thread::spawn(move || {
            let info = App::get_git_info(&path);
            let _ = tx.send((path, info));
        });
    }

    fn cached_git_info_for_current_dir(&self) -> Option<(&str, bool, Option<(&str, u64)>)> {
        let cache = self.git_info_cache.as_ref()?;
        if cache.path != self.current_dir {
            return None;
        }
        cache
            .info
            .as_ref()
            .map(|(branch, dirty, tag)| {
                let tag_info = tag
                    .as_ref()
                    .map(|(name, ahead)| (name.as_str(), *ahead));
                (branch.as_str(), *dirty, tag_info)
            })
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = msg.into();
    }

    fn status_icon_for_message(&self, msg: &str) -> &'static str {
        let lower = msg.to_ascii_lowercase();
        if lower.contains("error")
            || lower.contains("failed")
            || lower.contains("not found")
            || lower.contains("invalid")
        {
            ""
        } else if lower.contains("cancel") {
            "󰜺"
        } else if lower.starts_with("selected:") {
            "󰱒"
        } else if lower.contains("git")
            || lower.contains("commit")
            || lower.contains("branch")
            || lower.contains("tag")
        {
            ""
        } else if lower.contains("archive")
            || lower.contains("extract")
            || lower.contains("zip")
            || lower.contains("tar")
            || lower.contains("7z")
            || lower.contains("rar")
        {
            ""
        } else if lower.contains("copy")
            || lower.contains("paste")
            || lower.contains("clipboard")
            || lower.contains("transfer")
            || lower.contains("move")
        {
            ""
        } else if lower.contains("search") || lower.contains("find") || lower.contains("index") {
            "󰍉"
        } else if lower.contains("mount") || lower.contains("ssh") || lower.contains("rclone") {
            "󰖟"
        } else if lower.contains("created")
            || lower.contains("saved")
            || lower.contains("installed")
            || lower.contains("opened")
            || lower.contains("updated")
            || lower.contains("toggled")
            || lower.contains("complete")
        {
            ""
        } else {
            ""
        }
    }

    fn decorate_footer_message(&self, msg: &str) -> String {
        if !self.nerd_font_active {
            return msg.to_string();
        }

        let trimmed = msg.trim_start();
        if trimmed.starts_with("")
            || trimmed.starts_with("󰜺")
            || trimmed.starts_with("󰱒")
            || trimmed.starts_with("")
            || trimmed.starts_with("")
            || trimmed.starts_with("")
            || trimmed.starts_with("󰍉")
            || trimmed.starts_with("󰖟")
            || trimmed.starts_with("")
            || trimmed.starts_with("")
        {
            return msg.to_string();
        }

        format!("{} {}", self.status_icon_for_message(msg), msg)
    }

    fn begin_input_edit(&mut self, mode: AppMode, initial: String) {
        self.mode = mode;
        self.input_buffer = initial;
        self.input_cursor = self.input_buffer.chars().count();
    }

    fn clear_input_edit(&mut self) {
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

    fn clamp_input_cursor(&mut self) {
        let len = self.input_buffer.chars().count();
        self.input_cursor = self.input_cursor.min(len);
    }

    fn move_selection_delta(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let max_idx = (self.entries.len() - 1) as isize;
        let next = ((self.selected_index as isize) + delta).clamp(0, max_idx) as usize;
        self.selected_index = next;
        self.table_state.select(Some(next));
    }

    fn build_entry_render_cache(
        entry: &fs::DirEntry,
        config: EntryRenderConfig,
        uid_cache: &HashMap<u32, String>,
        gid_cache: &HashMap<u32, String>,
    ) -> EntryRenderCache {
        let path = entry.path();
        let meta = entry.metadata().ok();
        let is_hidden = entry.file_name().to_string_lossy().starts_with('.');
        let is_symlink = entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false);
        // Reuse already-fetched metadata to determine is_dir — avoids a redundant stat().
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let icon_data = if config.nerd_font_active {
            Some(icon_for_file(&DevFile::new(&path), Some(Theme::Dark)))
        } else {
            None
        };

        let (icon_glyph, icon_style) = if !config.show_icons {
            (String::new(), Style::default())
        } else if config.nerd_font_active {
            if is_symlink {
                ("".to_string(), Style::default().fg(Color::Rgb(100, 220, 220)))
            } else if is_dir {
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let dir_style = Style::default().fg(Color::Rgb(100, 160, 240)).add_modifier(Modifier::BOLD);
                if let Some((glyph, _)) = named_dir_icon(dir_name) {
                    (glyph.to_string(), dir_style)
                } else {
                    ("\u{F07B}".to_string(), dir_style)
                }
            } else if Self::is_age_protected_file(&path) {
                ("".to_string(), Style::default().fg(Color::Rgb(230, 190, 90)))
            } else {
                let icon = icon_data.as_ref().map(|i| i.icon.to_string()).unwrap_or_else(|| "?".to_string());
                let color = icon_data
                    .as_ref()
                    .and_then(|i| Color::from_str(i.color).ok())
                    .unwrap_or(Color::White);
                (icon, Style::default().fg(color))
            }
        } else if is_dir {
            ("📁".to_string(), Style::default().fg(Color::Rgb(100, 160, 240)).add_modifier(Modifier::BOLD))
        } else {
            ("📄".to_string(), Style::default().fg(Color::White))
        };

        let mut name_style = if is_dir {
            Style::default().fg(Color::Rgb(100, 160, 240)).add_modifier(Modifier::BOLD)
        } else if Self::is_age_protected_file(&path) {
            Style::default().fg(Color::Rgb(230, 190, 90))
        } else {
            let file_color = icon_data
                .as_ref()
                .and_then(|i| Color::from_str(i.color).ok())
                .unwrap_or(Color::White);
            Style::default().fg(file_color)
        };

        if is_symlink {
            name_style = Style::default().fg(Color::Rgb(100, 220, 220));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !is_dir && meta.as_ref().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false) {
                name_style = Style::default().fg(Color::Rgb(120, 220, 120));
            }
        }

        if is_hidden {
            name_style = name_style.add_modifier(Modifier::DIM);
        }

        let perms_width = 11usize;
        let size_width = 6usize;
        let date_width = 16usize;
        let perms = meta.as_ref().map(App::parse_permissions).unwrap_or_else(|| "----------".to_string());
        // Look up owner/group names from per-refresh caches.
        let owner = meta.as_ref().map(|m| {
            #[cfg(unix)] {
                use std::os::unix::fs::MetadataExt;
                let uid = m.uid();
                uid_cache.get(&uid).cloned().unwrap_or_else(|| uid.to_string())
            }
            #[cfg(not(unix))] { "-".to_string() }
        }).unwrap_or_else(|| "-".to_string());
        let group = meta.as_ref().map(|m| {
            #[cfg(unix)] {
                use std::os::unix::fs::MetadataExt;
                let gid = m.gid();
                gid_cache.get(&gid).cloned().unwrap_or_else(|| gid.to_string())
            }
            #[cfg(not(unix))] { "-".to_string() }
        }).unwrap_or_else(|| "-".to_string());
        let perms_col = format!("{:<width$}", perms, width = perms_width);
        let size_bytes = meta.as_ref().and_then(|m| if m.is_dir() { None } else { Some(Self::display_leaf_size(m)) });
        let size = size_bytes
            .map(App::format_size)
            .unwrap_or_else(|| "-".to_string());
        let size_col = format!("{:>width$}", size, width = size_width);
        let date = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(|t| DateTime::<Local>::from(t).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let date_col = format!("{:>width$}", date, width = date_width);

        EntryRenderCache {
            raw_name: entry.file_name().to_string_lossy().into_owned(),
            icon_glyph,
            icon_style,
            name_style,
            perms_col,
            group_name: group,
            owner_name: owner,
            size_col,
            size_bytes,
            date_col,
        }
    }

    fn refresh_meta_identity_widths(&mut self) {
        let mut group_w = 1usize;
        let mut owner_w = 1usize;
        for entry in &self.entry_render_cache {
            group_w = group_w.max(entry.group_name.chars().count());
            owner_w = owner_w.max(entry.owner_name.chars().count());
        }
        self.meta_group_width = group_w.min(16);
        self.meta_owner_width = owner_w.min(20);
    }

    fn byte_index_for_char(s: &str, char_index: usize) -> usize {
        if char_index == 0 {
            return 0;
        }
        s.char_indices()
            .nth(char_index)
            .map(|(idx, _)| idx)
            .unwrap_or_else(|| s.len())
    }

    fn input_insert_char(&mut self, c: char) {
        self.clamp_input_cursor();
        let insert_at = Self::byte_index_for_char(&self.input_buffer, self.input_cursor);
        self.input_buffer.insert(insert_at, c);
        self.input_cursor = self.input_cursor.saturating_add(1);
    }

    fn input_backspace(&mut self) {
        self.clamp_input_cursor();
        if self.input_cursor == 0 {
            return;
        }
        let start = Self::byte_index_for_char(&self.input_buffer, self.input_cursor - 1);
        let end = Self::byte_index_for_char(&self.input_buffer, self.input_cursor);
        self.input_buffer.drain(start..end);
        self.input_cursor -= 1;
    }

    fn input_delete(&mut self) {
        self.clamp_input_cursor();
        let len = self.input_buffer.chars().count();
        if self.input_cursor >= len {
            return;
        }
        let start = Self::byte_index_for_char(&self.input_buffer, self.input_cursor);
        let end = Self::byte_index_for_char(&self.input_buffer, self.input_cursor + 1);
        self.input_buffer.drain(start..end);
    }

    fn input_move_left(&mut self) {
        self.input_cursor = self.input_cursor.saturating_sub(1);
    }

    fn input_move_right(&mut self) {
        let len = self.input_buffer.chars().count();
        self.input_cursor = (self.input_cursor + 1).min(len);
    }

    fn input_move_home(&mut self) {
        self.input_cursor = 0;
    }

    fn input_move_end(&mut self) {
        self.input_cursor = self.input_buffer.chars().count();
    }

    fn collect_internal_search_candidates(root: &PathBuf, max_items: usize) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let mut stack: Vec<PathBuf> = vec![root.clone()];

        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }

                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    if let Ok(rel) = path.strip_prefix(root) {
                        out.push(rel.to_path_buf());
                    }
                }

                if out.len() >= max_items {
                    break;
                }
            }

            if out.len() >= max_items {
                break;
            }
        }

        out.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
        out
    }

    fn fuzzy_score_and_ranges(candidate: &str, query: &str) -> Option<(i64, Vec<(usize, usize)>)> {
        if query.is_empty() {
            return Some((0, Vec::new()));
        }

        let cand_chars: Vec<char> = candidate.chars().collect();
        let query_chars: Vec<char> = query.chars().collect();
        let mut q_idx = 0usize;
        let mut last_match: Option<usize> = None;
        let mut score = 0i64;
        let mut matched_char_indices: Vec<usize> = Vec::new();

        for (c_idx, ch) in cand_chars.iter().enumerate() {
            if q_idx >= query_chars.len() {
                break;
            }

            if ch.eq_ignore_ascii_case(&query_chars[q_idx]) {
                score += 5;
                if c_idx == 0 || matches!(cand_chars[c_idx - 1], '/' | '_' | '-' | ' ') {
                    score += 8;
                }
                if let Some(prev) = last_match {
                    if c_idx == prev + 1 {
                        score += 12;
                    }
                }
                last_match = Some(c_idx);
                matched_char_indices.push(c_idx);
                q_idx += 1;
            }
        }

        if q_idx != query_chars.len() {
            return None;
        }

        let mut byte_offsets: Vec<usize> = candidate.char_indices().map(|(idx, _)| idx).collect();
        byte_offsets.push(candidate.len());

        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for idx in matched_char_indices {
            if idx + 1 < byte_offsets.len() {
                ranges.push((byte_offsets[idx], byte_offsets[idx + 1]));
            }
        }
        let merged = Self::merge_byte_ranges(ranges);

        Some((score - candidate.chars().count() as i64, merged))
    }

    fn merge_byte_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
        if ranges.is_empty() {
            return ranges;
        }
        ranges.sort_by_key(|(start, _)| *start);
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            if let Some((_, last_end)) = merged.last_mut() {
                if start <= *last_end {
                    *last_end = (*last_end).max(end);
                    continue;
                }
            }
            merged.push((start, end));
        }
        merged
    }

    fn search_spans_with_ranges(
        text: &str,
        ranges: &[(usize, usize)],
        base_style: Style,
        match_style: Style,
    ) -> Vec<Span<'static>> {
        if text.is_empty() {
            return vec![Span::styled(String::new(), base_style)];
        }

        if ranges.is_empty() {
            return vec![Span::styled(text.to_string(), base_style)];
        }

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut cursor = 0usize;
        for &(start, end) in ranges {
            if start > cursor {
                spans.push(Span::styled(text[cursor..start].to_string(), base_style));
            }
            if end > start {
                spans.push(Span::styled(text[start..end].to_string(), match_style));
            }
            cursor = end;
        }
        if cursor < text.len() {
            spans.push(Span::styled(text[cursor..].to_string(), base_style));
        }

        spans
    }

    fn parse_regex_query(query: &str) -> Option<(String, bool)> {
        let trimmed = query.trim();
        if let Some(rest) = trimmed.strip_prefix("re:") {
            return Some((rest.trim().to_string(), true));
        }

        if trimmed.starts_with('/') {
            let closing = trimmed.rfind('/').unwrap_or(0);
            if closing > 0 {
                let pattern = &trimmed[1..closing];
                if pattern.is_empty() {
                    return Some((String::new(), true));
                }
                let flags = &trimmed[closing + 1..];
                let case_insensitive = flags.contains('i');
                if flags.chars().all(|c| c == 'i') {
                    return Some((pattern.to_string(), case_insensitive));
                }
            }
        }

        None
    }

    fn literal_match_ranges_ascii(text: &str, needle: &str) -> Vec<(usize, usize)> {
        let query_chars: Vec<char> = needle.chars().collect();
        if query_chars.is_empty() {
            return Vec::new();
        }

        let text_chars: Vec<(usize, char)> = text.char_indices().collect();
        if query_chars.len() > text_chars.len() {
            return Vec::new();
        }

        let mut out: Vec<(usize, usize)> = Vec::new();
        let mut i = 0usize;
        while i + query_chars.len() <= text_chars.len() {
            let mut matched = true;
            for (j, qch) in query_chars.iter().enumerate() {
                let tch = text_chars[i + j].1;
                if !tch.eq_ignore_ascii_case(qch) {
                    matched = false;
                    break;
                }
            }

            if matched {
                let start = text_chars[i].0;
                let end_idx = i + query_chars.len();
                let end = if end_idx < text_chars.len() {
                    text_chars[end_idx].0
                } else {
                    text.len()
                };
                out.push((start, end));
                i += query_chars.len();
            } else {
                i += 1;
            }
        }

        out
    }

    fn refresh_internal_search_filename_results(&mut self, query: &str) {
        if let Some(regex) = self.internal_search_regex.as_ref() {
            let mut matched: Vec<(usize, usize, usize, String, InternalSearchResult)> = Vec::new();
            for rel in &self.internal_search_candidates {
                let rel_str = rel.to_string_lossy().into_owned();
                let ranges = Self::merge_byte_ranges(
                    regex
                        .find_iter(&rel_str)
                        .map(|m| (m.start(), m.end()))
                        .collect(),
                );
                if let Some((first_start, _)) = ranges.first() {
                    matched.push((
                        *first_start,
                        rel_str.chars().count(),
                        rel_str.len(),
                        rel_str,
                        InternalSearchResult::Filename {
                            rel_path: rel.clone(),
                            match_ranges: ranges,
                        },
                    ));
                }
            }

            matched.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.1.cmp(&b.1))
                    .then_with(|| a.2.cmp(&b.2))
                    .then_with(|| a.3.cmp(&b.3))
            });

            self.internal_search_results = matched.into_iter().map(|(_, _, _, _, item)| item).collect();
            return;
        }

        let mut scored: Vec<(i64, usize, String, InternalSearchResult)> = Vec::new();
        for rel in &self.internal_search_candidates {
            let rel_str = rel.to_string_lossy().into_owned();
            if let Some((score, ranges)) = Self::fuzzy_score_and_ranges(&rel_str, query) {
                scored.push((
                    score,
                    rel_str.chars().count(),
                    rel_str,
                    InternalSearchResult::Filename {
                        rel_path: rel.clone(),
                        match_ranges: ranges,
                    },
                ));
            }
        }

        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });

        self.internal_search_results = scored.into_iter().map(|(_, _, _, item)| item).collect();
    }

    fn internal_search_content_limits() -> InternalSearchContentLimits {
        let parse_env_usize = |key: &str, default: usize| {
            env::var(key)
                .ok()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(default)
        };

        InternalSearchContentLimits {
            max_files: parse_env_usize("SB_SEARCH_CONTENT_MAX_FILES", 20_000),
            max_hits: parse_env_usize("SB_SEARCH_CONTENT_MAX_HITS", 2_000),
            max_file_bytes: parse_env_usize("SB_SEARCH_CONTENT_MAX_FILE_BYTES", 2 * 1024 * 1024),
        }
    }

    fn adjust_internal_search_content_limit(&mut self, increase: bool, fast: bool) {
        let factor = if fast { 10usize } else { 1usize };

        let (current, step, min_value) = match self.internal_search_limits_selected {
            0 => (
                self.internal_search_content_limits.max_files,
                500usize.saturating_mul(factor),
                100usize,
            ),
            1 => (
                self.internal_search_content_limits.max_hits,
                100usize.saturating_mul(factor),
                50usize,
            ),
            _ => (
                self.internal_search_content_limits.max_file_bytes,
                (256usize * 1024).saturating_mul(factor),
                64usize * 1024,
            ),
        };

        let new_value = if increase {
            current.saturating_add(step)
        } else {
            current.saturating_sub(step).max(min_value)
        };

        match self.internal_search_limits_selected {
            0 => self.internal_search_content_limits.max_files = new_value,
            1 => self.internal_search_content_limits.max_hits = new_value,
            _ => self.internal_search_content_limits.max_file_bytes = new_value,
        }

        if self.internal_search_scope == InternalSearchScope::Content {
            self.refresh_internal_search_results();
        }
    }

    fn reset_internal_search_content_limits_to_defaults(&mut self) {
        self.internal_search_content_limits = Self::internal_search_content_limits();
        if self.internal_search_scope == InternalSearchScope::Content {
            self.refresh_internal_search_results();
        }
    }

    fn build_internal_search_limit_note(
        limits: InternalSearchContentLimits,
        scanned_candidates: usize,
        files_limit_hit: bool,
        large_files_skipped: usize,
        hits_limit_hit: bool,
    ) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if files_limit_hit {
            parts.push(format!("scanned first {} files", limits.max_files));
        }
        if hits_limit_hit {
            parts.push(format!("capped at {} matches", limits.max_hits));
        }
        if large_files_skipped > 0 {
            parts.push(format!(
                "skipped {} file(s) > {}",
                large_files_skipped,
                Self::format_size(limits.max_file_bytes as u64)
            ));
        }
        if parts.is_empty() {
            return None;
        }

        Some(format!(
            "Limits: {} (candidates: {})",
            parts.join("; "),
            scanned_candidates
        ))
    }

    fn run_internal_search_content_query(
        current_dir: PathBuf,
        candidates: Vec<PathBuf>,
        pattern: InternalSearchPattern,
        limits: InternalSearchContentLimits,
    ) -> (Vec<InternalSearchResult>, Option<String>) {
        let regex = match &pattern {
            InternalSearchPattern::Regex {
                pattern,
                case_insensitive,
            } => RegexBuilder::new(pattern)
                .case_insensitive(*case_insensitive)
                .build()
                .ok(),
            InternalSearchPattern::Literal(_) => None,
        };

        let mut out: Vec<InternalSearchResult> = Vec::new();
        let mut large_files_skipped = 0usize;
        let mut files_limit_hit = false;
        let mut hits_limit_hit = false;

        for (idx, rel) in candidates.iter().enumerate() {
            if idx >= limits.max_files {
                files_limit_hit = true;
                break;
            }

            let abs = current_dir.join(rel);
            if !abs.is_file() || Self::is_binary_file(&abs) {
                continue;
            }

            let Ok(meta) = fs::metadata(&abs) else {
                continue;
            };
            if meta.len() as usize > limits.max_file_bytes {
                large_files_skipped += 1;
                continue;
            }

            let Ok(bytes) = fs::read(&abs) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);

            for (line_idx, line) in text.lines().enumerate() {
                let ranges = match (&pattern, regex.as_ref()) {
                    (InternalSearchPattern::Regex { .. }, Some(re)) => Self::merge_byte_ranges(
                        re.find_iter(line).map(|m| (m.start(), m.end())).collect(),
                    ),
                    (InternalSearchPattern::Literal(query), _) => {
                        Self::literal_match_ranges_ascii(line, query)
                    }
                    _ => Vec::new(),
                };

                if ranges.is_empty() {
                    continue;
                }

                out.push(InternalSearchResult::Content {
                    rel_path: rel.clone(),
                    line_number: line_idx + 1,
                    line_text: line.to_string(),
                    match_ranges: ranges,
                });

                if out.len() >= limits.max_hits {
                    hits_limit_hit = true;
                    break;
                }
            }

            if hits_limit_hit {
                break;
            }
        }

        let note = Self::build_internal_search_limit_note(
            limits,
            candidates.len(),
            files_limit_hit,
            large_files_skipped,
            hits_limit_hit,
        );

        (out, note)
    }

    fn cancel_internal_search_content_request(&mut self) {
        self.internal_search_content_request_id = self.internal_search_content_request_id.wrapping_add(1);
        self.internal_search_content_rx = None;
        self.internal_search_content_pending = false;
    }

    fn refresh_internal_search_content_results_async(
        &mut self,
        query: &str,
        regex_pattern: Option<(String, bool)>,
    ) {
        if query.is_empty() {
            self.cancel_internal_search_content_request();
            self.internal_search_results.clear();
            self.internal_search_content_limit_note = None;
            return;
        }

        let limits = self.internal_search_content_limits;
        let request_id = self.internal_search_content_request_id.wrapping_add(1);
        self.internal_search_content_request_id = request_id;
        self.internal_search_content_pending = true;
        self.internal_search_content_limit_note = Some(format!(
            "Limits: files <= {}, hits <= {}, file <= {}",
            limits.max_files,
            limits.max_hits,
            Self::format_size(limits.max_file_bytes as u64),
        ));

        let current_dir = self.current_dir.clone();
        let candidates = self.internal_search_candidates.clone();
        let pattern = if let Some((pattern, case_insensitive)) = regex_pattern {
            InternalSearchPattern::Regex {
                pattern,
                case_insensitive,
            }
        } else {
            InternalSearchPattern::Literal(query.to_string())
        };

        let (tx, rx) = mpsc::channel();
        self.internal_search_content_rx = Some(rx);
        thread::spawn(move || {
            let (results, limit_note) =
                App::run_internal_search_content_query(current_dir, candidates, pattern, limits);
            let _ = tx.send(InternalSearchContentMsg::Finished {
                request_id,
                results,
                limit_note,
            });
        });
    }

    fn pump_internal_search_content_progress(&mut self) {
        let Some(rx) = self.internal_search_content_rx.take() else {
            return;
        };

        let mut keep_rx = true;
        loop {
            match rx.try_recv() {
                Ok(InternalSearchContentMsg::Finished {
                    request_id,
                    results,
                    limit_note,
                }) => {
                    if request_id == self.internal_search_content_request_id {
                        self.internal_search_results = results;
                        self.internal_search_content_limit_note = limit_note;
                        self.internal_search_content_pending = false;
                        if self.internal_search_results.is_empty() {
                            self.internal_search_selected = 0;
                        } else {
                            self.internal_search_selected = self
                                .internal_search_selected
                                .min(self.internal_search_results.len() - 1);
                        }
                        keep_rx = false;
                    } else {
                        keep_rx = false;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.internal_search_content_pending = false;
                    keep_rx = false;
                    break;
                }
            }
        }

        if keep_rx {
            self.internal_search_content_rx = Some(rx);
        }
    }

    fn refresh_internal_search_results(&mut self) {
        let query = self.input_buffer.trim().to_string();
        self.internal_search_regex_mode = false;
        self.internal_search_regex = None;
        self.internal_search_regex_error = None;

        let mut compiled_regex: Option<Regex> = None;

        let parsed_regex = Self::parse_regex_query(&query);

        if let Some((pattern, case_insensitive)) = parsed_regex.as_ref() {
            self.internal_search_regex_mode = true;

            let regex = RegexBuilder::new(&pattern)
            .case_insensitive(*case_insensitive)
                .build();

            let Ok(regex) = regex else {
                self.cancel_internal_search_content_request();
                self.internal_search_results.clear();
                self.internal_search_selected = 0;
                self.internal_search_content_limit_note = None;
                self.internal_search_regex_error = Some("invalid regex".to_string());
                return;
            };
            compiled_regex = Some(regex);
        }

        self.internal_search_regex = compiled_regex;

        match self.internal_search_scope {
            InternalSearchScope::Filename => {
                self.cancel_internal_search_content_request();
                self.internal_search_content_limit_note = None;
                self.refresh_internal_search_filename_results(&query);
            }
            InternalSearchScope::Content => {
                self.refresh_internal_search_content_results_async(&query, parsed_regex);
            }
        }

        if self.internal_search_results.is_empty() {
            self.internal_search_selected = 0;
        } else {
            self.internal_search_selected = self
                .internal_search_selected
                .min(self.internal_search_results.len() - 1);
        }
    }

    fn cancel_internal_search_candidate_scan(&mut self) {
        self.internal_search_candidates_scan_id = self.internal_search_candidates_scan_id.wrapping_add(1);
        self.internal_search_candidates_rx = None;
        self.internal_search_candidates_pending = false;
    }

    fn start_internal_search_candidate_scan(&mut self) {
        const INTERNAL_SEARCH_MAX_ITEMS: usize = 20_000;

        self.cancel_internal_search_candidate_scan();
        self.internal_search_candidates_truncated = false;
        self.internal_search_candidates.clear();
        self.internal_search_results.clear();
        self.internal_search_selected = 0;

        self.internal_search_candidates_scan_id = self.internal_search_candidates_scan_id.wrapping_add(1);
        let scan_id = self.internal_search_candidates_scan_id;
        self.internal_search_candidates_pending = true;

        let root = self.current_dir.clone();
        let (tx, rx) = mpsc::channel();
        self.internal_search_candidates_rx = Some(rx);
        thread::spawn(move || {
            let candidates = App::collect_internal_search_candidates(&root, INTERNAL_SEARCH_MAX_ITEMS);
            let truncated = candidates.len() >= INTERNAL_SEARCH_MAX_ITEMS;
            let _ = tx.send(InternalSearchCandidatesMsg::Finished {
                scan_id,
                candidates,
                truncated,
            });
        });
    }

    fn pump_internal_search_candidates_progress(&mut self) {
        let Some(rx) = self.internal_search_candidates_rx.take() else {
            return;
        };

        let mut keep_rx = true;
        loop {
            match rx.try_recv() {
                Ok(InternalSearchCandidatesMsg::Finished {
                    scan_id,
                    candidates,
                    truncated,
                }) => {
                    if scan_id == self.internal_search_candidates_scan_id {
                        self.internal_search_candidates = candidates;
                        self.internal_search_candidates_truncated = truncated;
                        self.internal_search_candidates_pending = false;
                        self.refresh_internal_search_results();

                        if self.internal_search_candidates.is_empty() {
                            self.set_status("search: no files found");
                        } else if self.internal_search_candidates_truncated {
                            self.set_status("search indexed first 20000 files");
                        }
                        keep_rx = false;
                    } else {
                        keep_rx = false;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.internal_search_candidates_pending = false;
                    keep_rx = false;
                    break;
                }
            }
        }

        if keep_rx {
            self.internal_search_candidates_rx = Some(rx);
        }
    }

    fn start_internal_search(&mut self) {
        self.start_internal_search_with_scope(InternalSearchScope::Filename);
    }

    fn start_internal_search_with_scope(&mut self, scope: InternalSearchScope) {
        self.internal_search_selected = 0;
        self.internal_search_scope = scope;
        self.internal_search_content_limit_note = None;
        self.internal_search_limits_menu_open = false;
        self.internal_search_limits_selected = 0;
        self.panel_tab = 1;
        self.begin_input_edit(AppMode::InternalSearch, String::new());
        self.start_internal_search_candidate_scan();
        self.refresh_internal_search_results();
        self.set_status("search: indexing files asynchronously...");
    }

    fn toggle_internal_search_scope(&mut self) {
        self.internal_search_scope = match self.internal_search_scope {
            InternalSearchScope::Filename => InternalSearchScope::Content,
            InternalSearchScope::Content => InternalSearchScope::Filename,
        };
        if self.internal_search_scope == InternalSearchScope::Filename {
            self.internal_search_limits_menu_open = false;
        }
        self.internal_search_selected = 0;
        self.refresh_internal_search_results();
    }

    fn selected_internal_search_path(&self) -> Option<PathBuf> {
        let result = self.internal_search_results.get(self.internal_search_selected)?;
        let rel = match result {
            InternalSearchResult::Filename { rel_path, .. } => rel_path,
            InternalSearchResult::Content { rel_path, .. } => rel_path,
        };
        Some(self.current_dir.join(rel))
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
        self.remember_current_selection();
        self.current_dir = target;
        if !self.refresh_entries_or_status() {
            self.current_dir = previous_dir;
        } else {
            self.restore_selection_for_current_dir();
            self.request_git_info_for_current_dir_once();
        }
    }

    fn is_supported_archive(path: &PathBuf) -> bool {
        let lower_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();

        let tar_like = lower_name.ends_with(".tar")
            || lower_name.ends_with(".tar.gz")
            || lower_name.ends_with(".tgz")
            || lower_name.ends_with(".tar.bz2")
            || lower_name.ends_with(".tbz")
            || lower_name.ends_with(".tbz2")
            || lower_name.ends_with(".tar.xz")
            || lower_name.ends_with(".txz")
            || lower_name.ends_with(".tar.zst")
            || lower_name.ends_with(".tzst");

        let seven_zip = lower_name.ends_with(".7z");
        let rar = lower_name.ends_with(".rar");

        let ext_supported = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ZIP_BASED_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false);

        ext_supported || tar_like || seven_zip || rar || Self::has_zip_signature(path)
    }

    fn is_fuse_zip_archive(path: &PathBuf) -> bool {
        matches!(Self::archive_kind(path), Some(ArchiveKind::Zip))
    }

    fn is_archivemount_archive(path: &PathBuf) -> bool {
        matches!(Self::archive_kind(path), Some(ArchiveKind::Tar) | Some(ArchiveKind::Zip))
    }

    fn archive_kind(path: &PathBuf) -> Option<ArchiveKind> {
        let lower_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();

        let is_zip = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ZIP_BASED_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
            || Self::has_zip_signature(path);
        if is_zip {
            return Some(ArchiveKind::Zip);
        }

        if lower_name.ends_with(".tar")
            || lower_name.ends_with(".tar.gz")
            || lower_name.ends_with(".tgz")
            || lower_name.ends_with(".tar.bz2")
            || lower_name.ends_with(".tbz")
            || lower_name.ends_with(".tbz2")
            || lower_name.ends_with(".tar.xz")
            || lower_name.ends_with(".txz")
            || lower_name.ends_with(".tar.zst")
            || lower_name.ends_with(".tzst")
        {
            return Some(ArchiveKind::Tar);
        }
        if lower_name.ends_with(".7z") {
            return Some(ArchiveKind::SevenZip);
        }
        if lower_name.ends_with(".rar") {
            return Some(ArchiveKind::Rar);
        }
        None
    }

    fn seven_zip_tool() -> Option<String> {
        for cmd in ["7z", "7zz", "7zr"] {
            if let (true, path) = Self::integration_probe(cmd) {
                return Some(path);
            }
        }
        None
    }

    fn rar_tool() -> Option<String> {
        if let (true, path) = Self::integration_probe("unrar") {
            return Some(path);
        }
        if let (true, path) = Self::integration_probe("rar") {
            return Some(path);
        }
        None
    }

    fn bat_tool() -> Option<String> {
        if let (true, path) = Self::integration_probe("bat") {
            return Some(path);
        }
        if let (true, path) = Self::integration_probe("batcat") {
            return Some(path);
        }
        None
    }

    fn can_extract_archive(&self, path: &PathBuf) -> bool {
        match Self::archive_kind(path) {
            Some(ArchiveKind::Zip) => self.integration_enabled("zip") && Self::integration_probe("unzip").0,
            Some(ArchiveKind::Tar) => self.integration_active("tar"),
            Some(ArchiveKind::SevenZip) => self.integration_enabled("7z") && Self::seven_zip_tool().is_some(),
            Some(ArchiveKind::Rar) => self.integration_enabled("rar") && Self::rar_tool().is_some(),
            None => false,
        }
    }

    fn is_image_file(path: &PathBuf) -> bool {
        const IMAGE_EXTENSIONS: &[&str] = &[
            "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "avif", "heic", "ico",
        ];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| IMAGE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    fn is_audio_file(path: &PathBuf) -> bool {
        const AUDIO_EXTENSIONS: &[&str] = &[
            "mp3", "flac", "wav", "ogg", "opus", "m4a", "aac", "wma", "aiff", "aif", "alac", "mid", "midi",
        ];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| AUDIO_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    fn is_json_file(path: &PathBuf) -> bool {
        const JSON_EXTENSIONS: &[&str] = &[
            "json", "jsonc", "jsonl", "ndjson", "geojson",
        ];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| JSON_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    fn is_markdown_file(path: &PathBuf) -> bool {
        const MARKDOWN_EXTENSIONS: &[&str] = &[
            "md", "markdown", "mdown", "mkd", "mkdn",
        ];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| MARKDOWN_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    fn is_html_file(path: &PathBuf) -> bool {
        const HTML_EXTENSIONS: &[&str] = &["html", "htm", "xhtml"];
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| HTML_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    fn is_pdf_file(path: &PathBuf) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
    }

    fn is_cast_file(path: &PathBuf) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("cast"))
            .unwrap_or(false)
    }

    fn is_age_protected_file(path: &PathBuf) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("age"))
            .unwrap_or(false)
    }

    fn age_protected_output_path(path: &PathBuf) -> PathBuf {
        PathBuf::from(format!("{}.age", path.to_string_lossy()))
    }

    fn age_plain_output_path(path: &PathBuf) -> PathBuf {
        let mut out = path.clone();
        out.set_extension("");
        if out == *path {
            path.with_extension("decrypted")
        } else {
            out
        }
    }

    fn age_temp_decrypt_paths(path: &PathBuf, purpose: &str) -> io::Result<(PathBuf, PathBuf)> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp_dir = env::temp_dir().join(format!(
            "sbrs_age_{}_{}_{}",
            purpose,
            std::process::id(),
            stamp
        ));
        fs::create_dir_all(&tmp_dir)?;

        let plain_name = Self::age_plain_output_path(path)
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_else(|| "decrypted.bin".into());
        let tmp_path = tmp_dir.join(plain_name);
        Ok((tmp_dir, tmp_path))
    }

    fn is_delimited_text_file(path: &PathBuf) -> bool {
        const DELIMITED_EXTENSIONS: &[&str] = &[
            "csv", "tsv", "tab", "psv", "dsv", "ssv",
        ];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| DELIMITED_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    fn is_binary_file(path: &PathBuf) -> bool {
        use std::io::Read;
        let Ok(mut file) = std::fs::File::open(path) else { return false; };
        let mut buf = [0u8; 8192];
        let Ok(n) = file.read(&mut buf) else { return false; };
        buf[..n].contains(&0u8)
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
        execute!(io::stdout(), LeaveAlternateScreen)?;

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

        execute!(io::stdout(), EnterAlternateScreen)?;
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
        execute!(io::stdout(), LeaveAlternateScreen)?;

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

        execute!(io::stdout(), EnterAlternateScreen)?;
        enable_raw_mode()?;

        if let Some(name) = images[idx].file_name() {
            self.select_entry_named(&name.to_string_lossy());
        }

        Ok(())
    }

    fn has_zip_signature(path: &PathBuf) -> bool {
        use std::io::Read;

        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(_) => return false,
        };

        let mut magic = [0u8; 4];
        match file.read(&mut magic) {
            Ok(read) if read >= 4 => {
                magic == [0x50, 0x4B, 0x03, 0x04]
                    || magic == [0x50, 0x4B, 0x05, 0x06]
                    || magic == [0x50, 0x4B, 0x07, 0x08]
            }
            _ => false,
        }
    }

    fn create_archive_mount_path(&self) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        env::temp_dir().join(format!("sbrs_zip_{}_{}", std::process::id(), stamp))
    }

    fn create_temp_selection_path(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        env::temp_dir().join(format!("{}_{}_{}.txt", prefix, std::process::id(), stamp))
    }

    fn try_mount_archive(&mut self, archive_path: PathBuf) -> bool {
        self.try_mount_archive_with(archive_path, "fuse-zip")
    }

    fn try_mount_archive_with(&mut self, archive_path: PathBuf, tool: &str) -> bool {
        if !self.integration_active(tool) {
            self.set_status(&format!("{} not installed", tool));
            return false;
        }

        if let Some(existing_idx) = self
            .archive_mounts
            .iter()
            .position(|m| m.archive_path == archive_path && m.mount_path.is_dir())
        {
            let archive_name = archive_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| archive_path.to_string_lossy().into_owned());
            let mount_path = self.archive_mounts[existing_idx].mount_path.clone();
            self.archive_mounts[existing_idx].return_dir = self.current_dir.clone();
            self.archive_mounts[existing_idx].archive_name = archive_name;
            self.try_enter_dir(mount_path);
            return true;
        }

        let mount_path = self.create_archive_mount_path();
        if fs::create_dir_all(&mount_path).is_err() {
            self.set_status("failed to create archive mount directory");
            return false;
        }

        match Command::new(tool).arg(&archive_path).arg(&mount_path).status() {
            Ok(status) if status.success() => {
                let archive_name = archive_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| archive_path.to_string_lossy().into_owned());
                let return_dir = self.current_dir.clone();
                self.archive_mounts.push(ArchiveMount {
                    archive_path,
                    mount_path: mount_path.clone(),
                    return_dir,
                    archive_name,
                });
                self.try_enter_dir(mount_path);
                true
            }
            _ => {
                let _ = fs::remove_dir(&mount_path);
                self.set_status(&format!("failed to mount archive with {}", tool));
                false
            }
        }
    }

    fn preview_archive_contents(&mut self, archive_path: &PathBuf) -> bool {
        let archive_name = archive_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| archive_path.to_string_lossy().into_owned());

        let mut cmd = match Self::archive_kind(archive_path) {
            Some(ArchiveKind::Zip)
                if self.integration_enabled("zip") && Self::integration_probe("unzip").0 =>
            {
                let mut c = Command::new("unzip");
                c.arg("-l").arg(archive_path);
                c
            }
            Some(ArchiveKind::Tar) if self.integration_active("tar") => {
                let mut c = Command::new("tar");
                c.arg("-tvf").arg(archive_path);
                c
            }
            Some(ArchiveKind::SevenZip)
                if self.integration_enabled("7z") && Self::seven_zip_tool().is_some() =>
            {
                let tool = Self::seven_zip_tool().unwrap_or_else(|| "7z".to_string());
                let mut c = Command::new(tool);
                c.arg("l").arg(archive_path);
                c
            }
            Some(ArchiveKind::Rar)
                if self.integration_enabled("rar") && Self::rar_tool().is_some() =>
            {
                let tool = Self::rar_tool().unwrap_or_else(|| "unrar".to_string());
                let mut c = Command::new(tool);
                c.arg("l").arg(archive_path);
                c
            }
            _ => {
                self.set_status(format!("no archive preview tool available for {}", archive_name));
                return false;
            }
        };

        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);

        let mut shown = false;
        if let Ok(mut child) = cmd.stdout(Stdio::piped()).spawn() {
            if let Some(stdout) = child.stdout.take() {
                shown = Command::new("less")
                    .arg("-R")
                    .stdin(stdout)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            }
            let _ = child.wait();
        }

        let _ = execute!(io::stdout(), EnterAlternateScreen);
        let _ = enable_raw_mode();

        if shown {
            self.set_status(format!("previewed archive listing: {}", archive_name));
        } else {
            self.set_status(format!("failed to preview archive: {}", archive_name));
        }

        shown
    }

    fn unmount_archive_path(path: &PathBuf) {
        let _ = Command::new("fusermount")
            .args(["-u", path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("fusermount3")
            .args(["-u", path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("fusermount")
            .args(["-uz", path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("fusermount3")
            .args(["-uz", path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("umount")
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("umount")
            .args(["-l", path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    fn try_leave_archive(&mut self) -> bool {
        let Some(mount_idx) = self
            .archive_mounts
            .iter()
            .rposition(|mount| mount.mount_path == self.current_dir)
        else {
            return false;
        };

        self.remember_current_selection();
        let return_dir = self.archive_mounts[mount_idx].return_dir.clone();
        let archive_name = self.archive_mounts[mount_idx].archive_name.clone();
        self.current_dir = return_dir;
        if self.refresh_entries_or_status() {
            self.select_entry_named(&archive_name);
        }
        true
    }

    fn cleanup_archive_mounts(&mut self) {
        // If current_dir is inside an archive mount, switch back to that mount's
        // return directory before unmounting so shell integration doesn't keep
        // a now-removed temp path.
        if let Some(mount) = self
            .archive_mounts
            .iter()
            .rev()
            .find(|m| self.current_dir == m.mount_path || self.current_dir.starts_with(&m.mount_path))
        {
            self.current_dir = mount.return_dir.clone();
        }

        while let Some(mount) = self.archive_mounts.pop() {
            let _ = mount.archive_path;
            Self::unmount_archive_path(&mount.mount_path);
            let _ = fs::remove_dir(&mount.mount_path);
        }
    }

    fn unmount_archive_mount_by_path(&mut self, mount_path: &PathBuf) -> bool {
        let Some(idx) = self
            .archive_mounts
            .iter()
            .rposition(|m| &m.mount_path == mount_path)
        else {
            return false;
        };

        let mount = self.archive_mounts.remove(idx);
        let was_inside = self.current_dir == mount.mount_path || self.current_dir.starts_with(&mount.mount_path);
        if was_inside {
            self.current_dir = mount.return_dir.clone();
            if self.refresh_entries_or_status() {
                self.select_entry_named(&mount.archive_name);
            }
        }
        Self::unmount_archive_path(&mount.mount_path);
        let _ = fs::remove_dir(&mount.mount_path);
        true
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
        let target = self.resolve_input_path(&self.input_buffer);
        if target.is_dir() {
            self.try_enter_dir(target);
            self.mode = AppMode::Browsing;
            self.clear_input_edit();
        } else {
            self.set_status("path is not a directory");
        }
    }

    fn create_entry_from_input(&mut self, is_dir: bool) {
        let name = self.input_buffer.trim().to_string();
        if name.is_empty() {
            self.set_status("name cannot be empty");
            return;
        }

        let target = self.current_dir.join(&name);
        if target.exists() {
            self.set_status("target already exists");
            return;
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
            Ok(()) => {
                self.mode = AppMode::Browsing;
                self.clear_input_edit();
                self.refresh_entries_or_status();
                self.select_entry_named(&name);
                self.set_status(if is_dir { "folder created" } else { "file created" });
            }
            Err(e) => {
                self.set_status(format!("create failed: {}", e));
            }
        }
    }

    fn refresh_entries(&mut self) -> io::Result<()> {
        let mut entries: Vec<_> = fs::read_dir(&self.current_dir)?
            .filter_map(|res| res.ok())
            .filter(|e| self.show_hidden || !e.file_name().to_string_lossy().starts_with('.'))
            .collect();

        Self::sort_entries_by_mode(&mut entries, self.sort_mode);
        self.entries = entries;
        let config = EntryRenderConfig { nerd_font_active: self.nerd_font_active, show_icons: self.show_icons };
        let uid_cache = App::build_uid_cache(&self.entries);
        let gid_cache = App::build_gid_cache(&self.entries);
            self.entry_render_cache = self.entries.iter()
            .map(|entry| App::build_entry_render_cache(entry, config, &uid_cache, &gid_cache))
            .collect();
        self.refresh_meta_identity_widths();
        self.refresh_current_dir_free_space();
        self.folder_size_scan_id = self.folder_size_scan_id.wrapping_add(1);
        self.folder_size_rx = None;
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
        execute!(io::stdout(), LeaveAlternateScreen)?;
        let _ = Command::new(&shell)
            .current_dir(&self.current_dir)
            .status();
        execute!(io::stdout(), EnterAlternateScreen)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;
        self.set_status("returned from shell");
        self.refresh_entries_or_status();
        Ok(())
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
        execute!(io::stdout(), LeaveAlternateScreen)?;
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

        execute!(io::stdout(), EnterAlternateScreen)?;
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
        execute!(io::stdout(), LeaveAlternateScreen)?;

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

        execute!(io::stdout(), EnterAlternateScreen)?;
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

    fn preview_git_diff_and_confirm_commit(&mut self) -> io::Result<bool> {
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;
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

        print!("\nDo you really want to commit these changes? [y/N]: ");
        let _ = io::stdout().flush();
        let mut answer = String::new();
        let _ = io::stdin().read_line(&mut answer);
        let confirmed = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");

        execute!(io::stdout(), EnterAlternateScreen)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;

        Ok(confirmed)
    }

    fn run_git_commit_and_push(&mut self, commit_message: &str, amend: bool) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;

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

        println!("\nPress Enter to return to sbrs...");
        let _ = io::stdout().flush();
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);

        execute!(io::stdout(), EnterAlternateScreen)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;

        if let Some(step) = failed_step {
            self.set_status(step);
        } else if push_forced {
            self.set_status("amend commit pushed with -f");
        } else {
            self.set_status("commit pushed");
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
        execute!(io::stdout(), LeaveAlternateScreen)?;
        let _ = Command::new("delta")
            .arg("--side-by-side")
            .arg("--paging=always")
            .arg(&marked_path)
            .arg(&cursor_path)
            .status();
        execute!(io::stdout(), EnterAlternateScreen)?;
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
        execute!(io::stdout(), LeaveAlternateScreen)?;
        execute!(io::stdout(), Show)?;

        let edit_result = (|| -> io::Result<String> {
            let _ = Command::new(env::var("EDITOR").unwrap_or_else(|_| "nano".to_string()))
                .arg(&tmp)
                .status();
            fs::read_to_string(&tmp)
        })();

        execute!(io::stdout(), EnterAlternateScreen)?;
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

    fn compute_total_bytes(src: &PathBuf) -> io::Result<u64> {
        Self::compute_total_bytes_inner(src, true)
    }

    fn compute_total_display_bytes(src: &PathBuf) -> io::Result<u64> {
        Self::compute_total_display_bytes_inner(src, false)
    }

    fn compute_total_bytes_inner(src: &PathBuf, follow_symlink_dir: bool) -> io::Result<u64> {
        // Best-effort size walk: skip unreadable nodes instead of failing the whole tree.
        let metadata = match fs::symlink_metadata(src) {
            Ok(m) => m,
            Err(_) => return Ok(0),
        };

        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            if follow_symlink_dir {
                if let Ok(target_meta) = fs::metadata(src) {
                    if target_meta.is_dir() {
                        return Self::compute_dir_total_bytes(src);
                    }
                }
            }
            return Ok(metadata.len());
        }

        if file_type.is_dir() {
            return Self::compute_dir_total_bytes(src);
        }

        Ok(metadata.len())
    }

    fn compute_total_display_bytes_inner(src: &PathBuf, follow_symlink_dir: bool) -> io::Result<u64> {
        // Best-effort size walk for display: uses disk-usage bytes on Unix to avoid
        // huge apparent sizes from virtual files (for example /proc/kcore).
        let metadata = match fs::symlink_metadata(src) {
            Ok(m) => m,
            Err(_) => return Ok(0),
        };

        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            if follow_symlink_dir {
                if let Ok(target_meta) = fs::metadata(src) {
                    if target_meta.is_dir() {
                        return Self::compute_dir_total_display_bytes(src);
                    }
                }
            }
            return Ok(Self::display_leaf_size(&metadata));
        }

        if file_type.is_dir() {
            return Self::compute_dir_total_display_bytes(src);
        }

        Ok(Self::display_leaf_size(&metadata))
    }

    fn compute_dir_total_bytes(dir: &PathBuf) -> io::Result<u64> {
        const SIZE_WALK_PAR_THRESHOLD: usize = 32;
        let children = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return Ok(0),
        };

        let child_paths: Vec<PathBuf> = children
            .filter_map(|child| child.ok().map(|entry| entry.path()))
            .collect();

        let total = if child_paths.len() >= SIZE_WALK_PAR_THRESHOLD {
            child_paths
                .par_iter()
                .map(|child_path| Self::compute_total_bytes_inner(child_path, false).unwrap_or(0))
                .reduce(|| 0u64, |acc, v| acc.saturating_add(v))
        } else {
            child_paths
                .iter()
                .map(|child_path| Self::compute_total_bytes_inner(child_path, false).unwrap_or(0))
                .fold(0u64, |acc, v| acc.saturating_add(v))
        };

        Ok(total)
    }

    fn compute_dir_total_display_bytes(dir: &PathBuf) -> io::Result<u64> {
        const SIZE_WALK_PAR_THRESHOLD: usize = 32;
        let children = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return Ok(0),
        };

        let child_paths: Vec<PathBuf> = children
            .filter_map(|child| child.ok().map(|entry| entry.path()))
            .collect();

        let total = if child_paths.len() >= SIZE_WALK_PAR_THRESHOLD {
            child_paths
                .par_iter()
                .map(|child_path| Self::compute_total_display_bytes_inner(child_path, false).unwrap_or(0))
                .reduce(|| 0u64, |acc, v| acc.saturating_add(v))
        } else {
            child_paths
                .iter()
                .map(|child_path| Self::compute_total_display_bytes_inner(child_path, false).unwrap_or(0))
                .fold(0u64, |acc, v| acc.saturating_add(v))
        };

        Ok(total)
    }

    fn display_leaf_size(metadata: &fs::Metadata) -> u64 {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.blocks().saturating_mul(512)
        }
        #[cfg(not(unix))]
        {
            metadata.len()
        }
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
        let mins = total_seconds / 60;
        let secs = total_seconds % 60;
        if mins > 0 {
            format!("{}m{:02}s", mins, secs)
        } else {
            format!("{}s", secs)
        }
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

    fn get_git_info(path: &PathBuf) -> Option<(String, bool, Option<(String, u64)>)> {
        let path_str = path.to_str()?;

        let branch = Command::new("git")
            .args(["-C", path_str, "symbolic-ref", "--short", "-q", "HEAD"])
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if value.is_empty() { None } else { Some(value) }
                } else {
                    None
                }
            })
            .or_else(|| {
                Command::new("git")
                    .args(["-C", path_str, "rev-parse", "--short", "HEAD"])
                    .output()
                    .ok()
                    .and_then(|out| {
                        if out.status.success() {
                            let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
                            if value.is_empty() { None } else { Some(value) }
                        } else {
                            None
                        }
                    })
            })?;

        // Fast tracked-change dirty check: exit code 1 means dirty, 0 means clean.
        let dirty_status = Command::new("git")
            .args(["-C", path_str, "diff-index", "--quiet", "HEAD", "--"])
            .status()
            .ok()?;

        let is_dirty = match dirty_status.code() {
            Some(0) => false,
            Some(1) => true,
            _ => return None,
        };

        let latest_tag = Command::new("git")
            .args([
                "-C",
                path_str,
                "for-each-ref",
                "refs/tags",
                "--sort=-v:refname",
                "--count=1",
                "--format=%(refname:short)",
            ])
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if value.is_empty() { None } else { Some(value) }
                } else {
                    None
                }
            });

        let tag_info = latest_tag.and_then(|tag| {
            let ahead = Command::new("git")
                .args(["-C", path_str, "rev-list", "--count", &format!("{}..HEAD", tag)])
                .output()
                .ok()
                .and_then(|out| {
                    if out.status.success() {
                        String::from_utf8_lossy(&out.stdout)
                            .trim()
                            .parse::<u64>()
                            .ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            Some((tag, ahead))
        });

        Some((branch, is_dirty, tag_info))
    }

    fn integration_catalog() -> Vec<IntegrationSpec> {
        vec![
            IntegrationSpec { key: "git", description: "branch & dirty status in header", category: "vcs", required: false },
            IntegrationSpec { key: "less", description: "view files (Enter fallback)", category: "viewer", required: true },
            IntegrationSpec { key: "$EDITOR", description: "edit files (e / F4)", category: "editor", required: true },
            IntegrationSpec { key: "bat", description: "syntax-highlighted view on Enter", category: "viewer", required: false },
            IntegrationSpec { key: "glow", description: "Markdown preview on Enter", category: "viewer", required: false },
            IntegrationSpec { key: "links", description: "HTML preview on Enter", category: "viewer", required: false },
            IntegrationSpec { key: "pdftotext", description: "PDF text preview on Enter", category: "preview", required: false },
            IntegrationSpec { key: "asciinema", description: "terminal recording playback (.cast) on Enter (q/Esc to stop)", category: "preview", required: false },
            IntegrationSpec { key: "age", description: "password-protect/decrypt files (.age) with p/Enter/e", category: "security", required: false },
            IntegrationSpec { key: "jnv", description: "interactive JSON preview on Enter", category: "preview", required: false },
            IntegrationSpec { key: "csvlens", description: "interactive delimited preview (.csv/.tsv/.tab/.psv/.dsv/.ssv)", category: "preview", required: false },
            IntegrationSpec { key: "delta", description: "side-by-side colored compare (C: marked file vs cursor)", category: "diff", required: false },
            IntegrationSpec { key: "hexyl", description: "hex view for binary files on Enter", category: "preview", required: false },
            IntegrationSpec { key: "hexedit", description: "hex edit for binary files (e / F4)", category: "editor", required: false },
            IntegrationSpec { key: "vidir", description: "bulk rename when >1 marked (F2/r)", category: "rename", required: false },
            IntegrationSpec { key: "zip", description: "create/extract archives (Z)", category: "archive", required: false },
            IntegrationSpec { key: "tar", description: "extract tar/tar.gz/tar.xz/... archives", category: "archive", required: false },
            IntegrationSpec { key: "7z", description: "extract .7z archives", category: "archive", required: false },
            IntegrationSpec { key: "rar", description: "extract .rar archives", category: "archive", required: false },
            IntegrationSpec { key: "fuse-zip", description: "browse zip-based archives as folders", category: "archive", required: false },
            IntegrationSpec { key: "archivemount", description: "browse tar/zip archives as folders (Enter)", category: "archive", required: false },
            IntegrationSpec { key: "sox", description: "play audio files on Enter", category: "preview", required: false },
            IntegrationSpec { key: "viu", description: "image preview on Enter (preferred)", category: "preview", required: false },
            IntegrationSpec { key: "chafa", description: "image preview on Enter", category: "preview", required: false },
            IntegrationSpec { key: "sshfs", description: "mount SSH hosts via S picker", category: "network", required: false },
            IntegrationSpec { key: "rclone", description: "mount rclone remotes via S picker", category: "network", required: false },
            IntegrationSpec { key: "tmux", description: "split shell + less preview (i)", category: "terminal", required: false },
            IntegrationSpec { key: "rg", description: "content search, fzf preview if avail (g)", category: "search", required: false },
            IntegrationSpec { key: "fzf", description: "fuzzy file search (f)", category: "search", required: false },
            IntegrationSpec { key: "wl-copy", description: "Wayland clipboard backend used by Ctrl+c full-path copy", category: "clipboard", required: false },
            IntegrationSpec { key: "xclip", description: "X11 clipboard backend used by Ctrl+c full-path copy", category: "clipboard", required: false },
            IntegrationSpec { key: "xsel", description: "X11 clipboard backend used by Ctrl+c full-path copy", category: "clipboard", required: false },
            IntegrationSpec { key: "pbcopy", description: "macOS clipboard backend used by Ctrl+c full-path copy", category: "clipboard", required: false },
        ]
    }

    fn integration_count(&self) -> usize {
        1 + Self::integration_catalog().len()
    }

    fn integration_brew_package(key: &str) -> Option<&'static str> {
        match key {
            "__all_optional__" | "$EDITOR" | "less" | "pbcopy" => None,
            "git" => Some("git"),
            "bat" => Some("bat"),
            "glow" => Some("glow"),
            "links" => Some("links"),
            "7z" => Some("p7zip"),
            "zip" => Some("zip"),
            "tar" => Some("gnu-tar"),
            "rar" => Some("rar"),
            "asciinema" => Some("asciinema"),
            "age" => Some("age"),
            "jnv" => Some("jnv"),
            "csvlens" => Some("csvlens"),
            "delta" => Some("git-delta"),
            "hexyl" => Some("hexyl"),
            "hexedit" => Some("hexedit"),
            "vidir" => Some("moreutils"),
            "fuse-zip" => {
                #[cfg(target_os = "macos")]
                {
                    Some("fuse-zip-mac")
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Some("fuse-zip")
                }
            }
            "archivemount" => {
                #[cfg(target_os = "macos")]
                {
                    Some("gromgit/fuse/archivemount")
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Some("archivemount")
                }
            }
            "sox" => Some("sox"),
            "viu" => Some("viu"),
            "chafa" => Some("chafa"),
            "sshfs" => Some("sshfs"),
            "rclone" => Some("rclone"),
            "tmux" => Some("tmux"),
            "rg" => Some("ripgrep"),
            "fzf" => Some("fzf"),
            "wl-copy" => Some("wl-clipboard"),
            "xclip" => Some("xclip"),
            "xsel" => Some("xsel"),
            "pdftotext" => Some("poppler"),
            _ => None,
        }
    }

    fn brew_command_path() -> Option<String> {
        let (found, path) = Self::integration_probe("brew");
        if found {
            return Some(path);
        }

        let mut candidates: Vec<PathBuf> = Vec::new();
        #[cfg(target_os = "macos")]
        {
            candidates.push(PathBuf::from("/opt/homebrew/bin/brew"));
            candidates.push(PathBuf::from("/usr/local/bin/brew"));
        }
        #[cfg(target_os = "linux")]
        {
            candidates.push(PathBuf::from("/home/linuxbrew/.linuxbrew/bin/brew"));
            if let Ok(home) = env::var("HOME") {
                candidates.push(PathBuf::from(home).join(".linuxbrew/bin/brew"));
            }
        }

        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }

        None
    }

    fn clear_integration_install_prompt(&mut self) {
        self.integration_install_key = None;
        self.integration_install_package = None;
        self.integration_install_brew_path = None;
    }

    fn begin_integration_install_prompt_for_selected(&mut self) {
        if self.integration_rows_cache.is_empty() {
            self.set_status("no integration selected");
            return;
        }

        let Some(row) = self.integration_rows_cache.get(self.integration_selected).cloned() else {
            self.set_status("invalid integration selection");
            return;
        };

        if row.key == "__all_optional__" {
            self.set_status("select a specific integration to install");
            return;
        }

        if row.required {
            self.set_status("required integration cannot be installed from here");
            return;
        }

        if row.available {
            self.set_status(format!("{} is already available", row.label));
            return;
        }

        let Some(package) = Self::integration_brew_package(&row.key) else {
            self.set_status(format!("no brew package mapping for {}", row.label));
            return;
        };

        self.integration_install_key = Some(row.key);
        self.integration_install_package = Some(package.to_string());
        self.integration_install_brew_path = Self::brew_command_path();
        self.mode = AppMode::ConfirmIntegrationInstall;
        self.set_status("confirm integration install: y to continue");
    }

    fn show_brew_setup_guidance(&mut self) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;

        println!("Homebrew was not found on this system.");
        println!();
        println!("Install Homebrew first, then retry from Integrations:");
        println!("  /bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"");
        println!();
        println!("After install, verify with: brew --version");
        println!();
        println!("Press Enter to return to sbrs...");
        let _ = io::stdout().flush();
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);

        execute!(io::stdout(), EnterAlternateScreen)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;
        Ok(())
    }

    fn confirm_integration_install(&mut self) -> io::Result<()> {
        let Some(key) = self.integration_install_key.clone() else {
            self.mode = AppMode::Integrations;
            self.set_status("no pending integration install");
            return Ok(());
        };
        let Some(package) = self.integration_install_package.clone() else {
            self.mode = AppMode::Integrations;
            self.set_status("no pending integration package");
            return Ok(());
        };

        let brew_path = self
            .integration_install_brew_path
            .clone()
            .or_else(Self::brew_command_path);

        if brew_path.is_none() {
            self.show_brew_setup_guidance()?;
            self.mode = AppMode::Integrations;
            self.clear_integration_install_prompt();
            self.refresh_integration_rows_cache();
            self.set_status("brew not found; setup instructions shown");
            return Ok(());
        }

        let brew = brew_path.unwrap_or_default();

        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;

        println!("Installing integration '{}' with Homebrew", key);

        let mut install_steps: Vec<Vec<String>> = Vec::new();
        #[cfg(target_os = "macos")]
        {
            if key == "archivemount" || key == "fuse-zip" {
                install_steps.push(vec!["install".to_string(), "--cask".to_string(), "macfuse".to_string()]);
            }
        }
        install_steps.push(vec!["install".to_string(), package.clone()]);

        let mut failed_step: Option<String> = None;
        for step in install_steps {
            let pretty = step.join(" ");
            println!("$ {} {}", brew, pretty);
            let status = Command::new(&brew)
                .args(step.iter().map(|s| s.as_str()))
                .status();

            match &status {
                Ok(s) => {
                    if let Some(code) = s.code() {
                        println!("\n[exit code: {}]", code);
                    } else {
                        println!("\n[process terminated by signal]");
                    }
                    if !s.success() {
                        failed_step = Some(pretty);
                        break;
                    }
                }
                Err(e) => {
                    println!("\n[failed to execute brew: {}]", e);
                    failed_step = Some(pretty);
                    break;
                }
            }
        }

        println!("\nPress Enter to return to sbrs...");
        let _ = io::stdout().flush();
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);

        execute!(io::stdout(), EnterAlternateScreen)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;

        match failed_step {
            None => {
                self.set_integration_enabled(&key, true);
                self.set_status(format!("installed {} with brew", package));
            }
            Some(step) => {
                self.set_status(format!("brew install failed: {}", step));
            }
        }

        self.refresh_integration_rows_cache();
        self.mode = AppMode::Integrations;
        self.clear_integration_install_prompt();
        Ok(())
    }

    fn integration_probe(cmd: &str) -> (bool, String) {
        if let Ok(out) = Command::new("which").arg(cmd).output() {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() {
                    return (true, path);
                }
            }
        }

        // Fallback for environments where `which` is unavailable or shell setup differs.
        match Command::new(cmd)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(_) => (true, cmd.to_string()),
            Err(_) => (false, String::new()),
        }
    }

    fn integration_availability_and_detail(key: &str) -> (bool, String) {
        match key {
            "$EDITOR" => {
                let editor_var = env::var("EDITOR").unwrap_or_else(|_| "(not set)".to_string());
                let editor_cmd = env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
                let (ok, path) = Self::integration_probe(&editor_cmd);
                if ok { (true, path) } else { (false, format!("$EDITOR={}", editor_var)) }
            }
            "age" => {
                let (age_ok, age_path) = Self::integration_probe("age");
                (age_ok, age_path)
            }
            "zip" => {
                let (zip_ok, zip_path) = Self::integration_probe("zip");
                let (unzip_ok, unzip_path) = Self::integration_probe("unzip");
                let detail = if zip_ok && unzip_ok {
                    format!("{} | {}", zip_path, unzip_path)
                } else if zip_ok {
                    zip_path
                } else if unzip_ok {
                    unzip_path
                } else {
                    String::new()
                };
                (zip_ok || unzip_ok, detail)
            }
            "sox" => {
                let (play_ok, play_path) = Self::integration_probe("play");
                let (sox_ok, sox_path) = Self::integration_probe("sox");
                let detail = if play_ok {
                    play_path
                } else if sox_ok {
                    sox_path
                } else {
                    String::new()
                };
                (play_ok || sox_ok, detail)
            }
            "bat" => {
                if let Some(path) = Self::bat_tool() {
                    (true, path)
                } else {
                    (false, String::new())
                }
            }
            "tar" => Self::integration_probe("tar"),
            "7z" => {
                if let Some(path) = Self::seven_zip_tool() {
                    (true, path)
                } else {
                    (false, String::new())
                }
            }
            "rar" => {
                if let Some(path) = Self::rar_tool() {
                    (true, path)
                } else {
                    (false, String::new())
                }
            }
            other => Self::integration_probe(other),
        }
    }

    fn integration_enabled(&self, key: &str) -> bool {
        if Self::integration_catalog().iter().any(|s| s.key == key && s.required) {
            true
        } else {
            self.integration_overrides.get(key).copied().unwrap_or(true)
        }
    }

    fn integration_active(&self, key: &str) -> bool {
        let (available, _) = Self::integration_availability_and_detail(key);
        self.integration_enabled(key) && available
    }

    fn set_integration_enabled(&mut self, key: &str, enabled: bool) {
        if Self::integration_catalog().iter().any(|s| s.key == key && s.required) {
            return;
        }
        self.integration_overrides.insert(key.to_string(), enabled);
    }

    fn set_all_optional_integrations(&mut self, enabled: bool) {
        for spec in Self::integration_catalog().iter().filter(|s| !s.required) {
            self.integration_overrides.insert(spec.key.to_string(), enabled);
        }
    }

    fn all_optional_integrations_enabled(&self) -> bool {
        Self::integration_catalog()
            .iter()
            .filter(|s| !s.required)
            .all(|s| self.integration_enabled(s.key))
    }

    fn panel_tab_bar_line(active: u8) -> Line<'static> {
        let tabs: &[(&str, u8)] = &[
            (" Help ", 0),
            (" Search ", 1),
            (" Bookmarks ", 2),
            (" Remote Mounts ", 3),
            (" Sorting ", 4),
            (" Integrations ", 5),
        ];
        let active_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
        let inactive_style = Style::default().fg(Color::Rgb(100, 100, 100));
        let sep_style = Style::default().fg(Color::Rgb(80, 80, 80));
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (i, (label, idx)) in tabs.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("│", sep_style));
            }
            let style = if *idx == active { active_style } else { inactive_style };
            spans.push(Span::styled(*label, style));
        }
        Line::from(spans)
    }

    fn integration_rows(&self) -> Vec<IntegrationRow> {
        let mut rows = Vec::new();
        let all_on = self.all_optional_integrations_enabled();
        rows.push(IntegrationRow {
            key: "__all_optional__".to_string(),
            label: "__all_optional__".to_string(),
            state: if all_on { "[on]".to_string() } else { "[off]".to_string() },
            category: "global".to_string(),
            description: "Toggle all optional integrations on/off".to_string(),
            available: true,
            required: false,
        });

        for spec in Self::integration_catalog() {
            let (available, _) = Self::integration_availability_and_detail(spec.key);
            let enabled = self.integration_enabled(spec.key);
            let state = if spec.required {
                "[required]".to_string()
            } else if enabled && available {
                "[active]".to_string()
            } else if enabled {
                "[on]".to_string()
            } else {
                "[off]".to_string()
            };

            rows.push(IntegrationRow {
                key: spec.key.to_string(),
                label: spec.key.to_string(),
                state,
                category: spec.category.to_string(),
                description: spec.description.to_string(),
                available,
                required: spec.required,
            });
        }
        rows
    }

    fn refresh_integration_rows_cache(&mut self) {
        self.integration_rows_cache = self.integration_rows();
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

    fn parse_permissions(meta: &fs::Metadata) -> String {
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            let mode = meta.permissions().mode();
            let mut p = String::with_capacity(10);
            p.push(if meta.is_dir() { 'd' } else { '-' });
            let chars = ['r', 'w', 'x'];
            for i in (0..9).rev() {
                if mode & (1 << i) != 0 { p.push(chars[2 - (i % 3)]); } else { p.push('-'); }
            }
            p
        }
        #[cfg(not(unix))] { "----------".to_string() }
    }

    fn parse_owner(meta: &fs::Metadata) -> String {
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let uid = meta.uid();
            users::get_user_by_uid(uid)
                .map(|user| user.name().to_string_lossy().into_owned())
                .unwrap_or_else(|| uid.to_string())
        }
        #[cfg(not(unix))] {
            "-".to_string()
        }
    }

    fn parse_group(meta: &fs::Metadata) -> String {
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let gid = meta.gid();
            users::get_group_by_gid(gid)
                .map(|group| group.name().to_string_lossy().into_owned())
                .unwrap_or_else(|| gid.to_string())
        }
        #[cfg(not(unix))] {
            "-".to_string()
        }
    }

    /// Build a UID → username map for all entries in the current directory.
    /// Performs at most one NSS lookup per unique UID, so large directories owned
    /// by the same user pay exactly one lookup regardless of file count.
    fn build_uid_cache(entries: &[fs::DirEntry]) -> HashMap<u32, String> {
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let mut map: HashMap<u32, String> = HashMap::new();
            for entry in entries {
                if let Ok(meta) = entry.metadata() {
                    let uid = meta.uid();
                    map.entry(uid).or_insert_with(|| {
                        users::get_user_by_uid(uid)
                            .map(|u| u.name().to_string_lossy().into_owned())
                            .unwrap_or_else(|| uid.to_string())
                    });
                }
            }
            map
        }
        #[cfg(not(unix))] { HashMap::new() }
    }

    /// Build a GID -> group-name map for all entries in the current directory.
    fn build_gid_cache(entries: &[fs::DirEntry]) -> HashMap<u32, String> {
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let mut map: HashMap<u32, String> = HashMap::new();
            for entry in entries {
                if let Ok(meta) = entry.metadata() {
                    let gid = meta.gid();
                    map.entry(gid).or_insert_with(|| {
                        users::get_group_by_gid(gid)
                            .map(|g| g.name().to_string_lossy().into_owned())
                            .unwrap_or_else(|| gid.to_string())
                    });
                }
            }
            map
        }
        #[cfg(not(unix))] { HashMap::new() }
    }

    fn format_size(bytes: u64) -> String {
        let units = ["B", "K", "M", "G", "T"];
        let mut size = bytes as f64;
        let mut unit_idx = 0usize;
        while size >= 1024.0 && unit_idx < units.len() - 1 {
            size /= 1024.0;
            unit_idx += 1;
        }
        if unit_idx == 0 {
            format!("{}{}", bytes, units[unit_idx])
        } else if size >= 10.0 {
            format!("{:.0}{}", size, units[unit_idx])
        } else {
            format!("{:.1}{}", size, units[unit_idx])
        }
    }
}

/// Returns (glyph, (r, g, b)) for well-known directory names, or None for generic folders.
fn named_dir_icon(name: &str) -> Option<(&'static str, (u8, u8, u8))> {

    match name.to_lowercase().as_str() {
        // --- XDG User Dirs ---
        "desktop"                          => Some(("\u{F108}", (100, 160, 240))),
        "documents" | "docs"               => Some(("\u{F02D}", (100, 160, 240))),
        "downloads"                        => Some(("\u{F019}", (100, 200, 120))),
        "music"                            => Some(("\u{F001}", (180, 100, 220))),
        "pictures" | "photos" | "images"   => Some(("\u{F03E}", (255, 200,  60))),
        "videos" | "movies"                => Some(("\u{F03D}", (220,  80,  80))),
        "public"                           => Some(("\u{F0C0}", ( 80, 180, 220))),
        "templates"                        => Some(("\u{F0C5}", (180, 180, 180))),
        "trash" | ".trash"                 => Some(("\u{F014}", (140, 140, 140))),

        // --- Legal & Licensing ---
        "license" | "licenses" | "legal" | 
        "copying" | "copyright"            => Some(("\u{F423}", (240, 190,  40))),

        // --- Version Control ---
        ".git" | "git"                     => Some(("\u{E702}", (240,  93,  37))),
        ".github" | "github"               => Some(("\u{F09B}", (220, 220, 220))),
        ".gitlab" | "gitlab"               => Some(("\u{F296}", (252, 109,  38))),

        // --- Development & Runtimes ---
        "go"                               => Some(("\u{E724}", (  0, 173, 216))),
        "node_modules"                     => Some(("\u{E718}", ( 76, 175,  80))),
        "venv" | ".venv" | "env"           => Some(("\u{E235}", ( 59, 153,  11))),
        "python" | "py" | "__pycache__"    => Some(("\u{E235}", (255, 212,  59))),
        ".cargo" | "cargo" | "rust"        => Some(("\u{E7A8}", (222, 165, 132))),
        "java" | "maven" | "gradle"        => Some(("\u{E738}", (231,  10,  26))),
        "ruby" | "gems"                    => Some(("\u{E739}", (170,  20,   1))),
        "php"                              => Some(("\u{E73D}", (119, 123, 179))),

        // --- Project Structure ---
        "src" | "source" | "sources"       => Some(("\u{F121}", (100, 181, 246))),
        "lib" | "libs" | "library"         => Some(("\u{F1B2}", (100, 181, 246))),
        "bin" | "sbin"                     => Some(("\u{F489}", (255, 183,  77))),
        "scripts" | "script"               => Some(("\u{F085}", (255, 183,  77))),
        "include" | "includes" | "headers" => Some(("\u{F121}", ( 77, 208, 225))),
        "test" | "tests" | "spec" | "specs"=> Some(("\u{F0C3}", (244,  67,  54))),
        "target" | "build" | "dist" | "out"| 
        "release" | "debug"                => Some(("\u{F0AD}", (200, 140, 110))),
        "assets" | "resources" | "res"     => Some(("\u{F044}", (255, 235,  59))),
        "vendor" | "third_party"           => Some(("\u{F1B3}", (144, 164, 174))),

        // --- Config & System ---
        ".config" | "config" | "conf"      => Some(("\u{F013}", (200, 200, 200))),
        ".local"                           => Some(("\u{F07B}", (160, 160, 160))),
        ".ssh" | "ssh" | "keys" | "certs"  => Some(("\u{F023}", (255, 183,  77))),
        ".cache" | "cache"                 => Some(("\u{F4A1}", (158, 158, 158))),
        "var" | "tmp" | "temp"             => Some(("\u{F017}", (210, 105,  30))),
        "logs" | "log"                     => Some(("\u{F18D}", (160, 160, 160))),
        "snap"                             => Some(("\u{F17C}", (230,  70,  70))),
        "applications"                     => Some(("\u{F009}", ( 66, 133, 244))),
        "android"                          => Some(("\u{F17B}", ( 61, 220, 132))),

        // --- Containers & Cloud ---
        ".docker" | "docker"               => Some(("\u{F308}", ( 13, 110, 253))),
        ".kube" | "kubernetes" | "k8s"     => Some(("\u{F30F}", ( 50, 108, 230))),
        ".aws" | "aws"                     => Some(("\u{F270}", (255, 153,   0))),
        ".terraform" | "terraform"         => Some(("\u{F110}", ( 92,  78, 229))),

        // --- Tools & Editors ---
        ".vscode" | "vscode"               => Some(("\u{E70C}", (  0, 120, 212))),
        ".idea" | "intellij"               => Some(("\u{E7B5}", (254,  40,  85))),
        ".vim" | "nvim" | "lua"            => Some(("\u{E62B}", ( 87, 158,  58))),

        // --- Storage & Web ---
        "dropbox"                          => Some(("\u{F16B}", (  0,  97, 255))),
        "onedrive"                         => Some(("\u{F48F}", (  0, 120, 212))),
        "google_drive" | "gdrive"          => Some(("\u{F4D8}", ( 52, 168,  83))),
        "www" | "public_html" | "site"     => Some(("\u{F0AC}", ( 76, 175,  80))),
        "fonts"                            => Some(("\u{F031}", (255, 200, 100))),

        _ => None,
    }
}

fn list_current_directory(include_hidden: bool, include_total_size: bool, path: Option<&str>) -> io::Result<()> {
    let current_dir = if let Some(p) = path {
        std::path::PathBuf::from(p)
    } else {
        env::current_dir()?
    };
    let nerd_font_active = env::var("NERD_FONT_ACTIVE").map(|v| v == "1").unwrap_or(false);
    let no_color = env_flag_true(&["NO_COLOR"]);
    let show_icons = env::var("TERMINAL_ICONS").map(|v| v != "0").unwrap_or(true);
    let term_w = crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(120);
    let show_date = term_w >= 90;
    let show_size = term_w >= 70 || include_total_size;
    let show_pct = include_total_size && show_size;
    let show_meta = term_w >= 50;

    let date_width = 16usize;
    let size_width = 6usize;
    let pct_width = 6usize;
    let perms_width = 11usize;

    struct ListRowData {
        entry: fs::DirEntry,
        path: PathBuf,
        meta: Option<fs::Metadata>,
        owner: String,
        group: String,
        total_display_bytes: Option<u64>,
    }

    fn rt_to_ct_color(color: ratatui::style::Color) -> CtColor {
        match color {
            ratatui::style::Color::Black => CtColor::Black,
            ratatui::style::Color::Red => CtColor::Red,
            ratatui::style::Color::Green => CtColor::Green,
            ratatui::style::Color::Yellow => CtColor::Yellow,
            ratatui::style::Color::Blue => CtColor::Blue,
            ratatui::style::Color::Magenta => CtColor::Magenta,
            ratatui::style::Color::Cyan => CtColor::Cyan,
            ratatui::style::Color::Gray => CtColor::Grey,
            ratatui::style::Color::DarkGray => CtColor::DarkGrey,
            ratatui::style::Color::LightRed => CtColor::DarkRed,
            ratatui::style::Color::LightGreen => CtColor::DarkGreen,
            ratatui::style::Color::LightYellow => CtColor::DarkYellow,
            ratatui::style::Color::LightBlue => CtColor::DarkBlue,
            ratatui::style::Color::LightMagenta => CtColor::DarkMagenta,
            ratatui::style::Color::LightCyan => CtColor::DarkCyan,
            ratatui::style::Color::White => CtColor::White,
            ratatui::style::Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
            ratatui::style::Color::Indexed(i) => CtColor::AnsiValue(i),
            ratatui::style::Color::Reset => CtColor::Reset,
        }
    }

    fn truncate_to(s: &str, max: usize) -> String {
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
    }

    fn truncate_to_display_width(s: &str, max: usize) -> String {
        if UnicodeWidthStr::width(s) <= max {
            return s.to_string();
        }
        if max <= 1 {
            return "…".to_string();
        }

        let mut out = String::new();
        let mut used = 0usize;
        let target = max - 1;
        for ch in s.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + ch_width > target {
                break;
            }
            out.push(ch);
            used += ch_width;
        }
        out.push('…');
        out
    }

    fn pad_to_display_width(s: &str, width: usize) -> String {
        let used = UnicodeWidthStr::width(s);
        if used >= width {
            return s.to_string();
        }
        format!("{}{}", s, " ".repeat(width - used))
    }

    let mut entries: Vec<_> = fs::read_dir(&current_dir)?
        .filter_map(|res| res.ok())
        .filter(|e| include_hidden || !e.file_name().to_string_lossy().starts_with('.'))
        .collect();

    entries.sort_by_key(|e| (e.path().is_file(), e.file_name()));

    let mut rows: Vec<ListRowData> = Vec::with_capacity(entries.len());
    let mut group_width = 1usize;
    let mut owner_width = 1usize;
    for entry in entries {
        let path = entry.path();
        let meta = entry.metadata().ok();
        let owner = meta
            .as_ref()
            .map(App::parse_owner)
            .unwrap_or_else(|| "-".to_string());
        let group = meta
            .as_ref()
            .map(App::parse_group)
            .unwrap_or_else(|| "-".to_string());

        group_width = group_width.max(group.chars().count());
        owner_width = owner_width.max(owner.chars().count());

        let total_display_bytes = if include_total_size {
            Some(App::compute_total_display_bytes(&path).unwrap_or(0))
        } else {
            None
        };

        rows.push(ListRowData {
            entry,
            path,
            meta,
            owner,
            group,
            total_display_bytes,
        });
    }

    group_width = group_width.min(16).max(1);
    owner_width = owner_width.min(20).max(1);

    let mut reserved = 0usize;
    if show_meta {
        reserved += perms_width + group_width + owner_width + 3;
    }
    if show_size {
        reserved += size_width + 1;
    }
    if show_pct {
        reserved += pct_width + 1;
    }
    if show_date {
        reserved += date_width + 1;
    }
    let name_width = term_w.saturating_sub(reserved).max(20);

    let total_listing_display_bytes = if include_total_size {
        Some(
            rows.iter()
                .map(|row| row.total_display_bytes.unwrap_or(0))
                .fold(0u64, |acc, v| acc.saturating_add(v)),
        )
    } else {
        None
    };

    for row in rows {
        let path = row.path;
        let meta = row.meta;
        let owner = row.owner;
        let group = row.group;
        let entry_total_bytes = row.total_display_bytes;
        let entry = row.entry;
        let is_hidden = entry.file_name().to_string_lossy().starts_with('.');
        let is_symlink = entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false);
        let is_dir = path.is_dir();

        let icon_data = if nerd_font_active {
            Some(icon_for_file(&DevFile::new(&path), Some(Theme::Dark)))
        } else {
            None
        };

        let (icon_glyph, mut icon_color) = if !show_icons {
            (String::new(), CtColor::Reset)
        } else if nerd_font_active {
            if is_symlink {
                ("".to_string(), CtColor::Rgb { r: 100, g: 220, b: 220 })
            } else if is_dir {
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if let Some((glyph, (r, g, b))) = named_dir_icon(dir_name) {
                    (glyph.to_string(), CtColor::Rgb { r, g, b })
                } else {
                    ("\u{F07B}".to_string(), CtColor::Rgb { r: 100, g: 160, b: 240 })
                }
            } else if App::is_age_protected_file(&path) {
                ("".to_string(), CtColor::Rgb { r: 230, g: 190, b: 90 })
            } else {
                let icon = icon_data
                    .as_ref()
                    .map(|i| i.icon.to_string())
                    .unwrap_or_else(|| "?".to_string());
                let color = icon_data
                    .as_ref()
                    .and_then(|i| ratatui::style::Color::from_str(i.color).ok())
                    .map(rt_to_ct_color)
                    .unwrap_or(CtColor::White);
                (icon, color)
            }
        } else if is_dir {
            ("📁".to_string(), CtColor::Rgb { r: 100, g: 160, b: 240 })
        } else {
            ("📄".to_string(), CtColor::White)
        };

        let name = entry.file_name().to_string_lossy().into_owned();
        let mut name_color = if is_dir {
            CtColor::Rgb { r: 100, g: 160, b: 240 }
        } else if App::is_age_protected_file(&path) {
            CtColor::Rgb { r: 230, g: 190, b: 90 }
        } else {
            icon_data
                .as_ref()
                .and_then(|i| ratatui::style::Color::from_str(i.color).ok())
                .map(rt_to_ct_color)
                .unwrap_or(CtColor::White)
        };
        if is_symlink {
            name_color = CtColor::Rgb { r: 100, g: 220, b: 220 };
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !is_dir && meta.as_ref().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false) {
                name_color = CtColor::Rgb { r: 120, g: 220, b: 120 };
            }
        }
        if no_color {
            name_color = CtColor::Reset;
            icon_color = CtColor::Reset;
        }

        let icon_prefix = if show_icons && !icon_glyph.is_empty() {
            format!("{} ", icon_glyph)
        } else {
            String::new()
        };
        let rendered_name = truncate_to_display_width(&format!("{}{}", icon_prefix, name), name_width);
        let rendered_name = pad_to_display_width(&rendered_name, name_width);

        let mut styled_name = style(rendered_name).with(name_color);
        if is_dir {
            styled_name = styled_name.attribute(Attribute::Bold);
        }
        if is_hidden {
            styled_name = styled_name.attribute(Attribute::Dim);
        }

        let styled_icon = style(format!("{}", icon_glyph)).with(icon_color);

        if show_meta || show_size || show_date {
            let perms = meta
                .as_ref()
                .map(App::parse_permissions)
                .unwrap_or_else(|| "----------".to_string());
            let owner = meta
                .as_ref()
                .map(|_| owner.clone())
                .unwrap_or_else(|| "-".to_string());
            let group = meta
                .as_ref()
                .map(|_| group.clone())
                .unwrap_or_else(|| "-".to_string());
            let perms_col = format!("{:<width$}", truncate_to(&perms, perms_width), width = perms_width);
            let group_col = format!("{:>width$}", truncate_to(&group, group_width), width = group_width);
            let owner_col = format!("{:<width$}", truncate_to(&owner, owner_width), width = owner_width);

            let size = if include_total_size {
                App::format_size(entry_total_bytes.unwrap_or(0))
            } else {
                meta
                    .as_ref()
                    .map(|m| if m.is_dir() { "-".to_string() } else { App::format_size(m.len()) })
                    .unwrap_or_else(|| "-".to_string())
            };
            let size_col = format!("{:>width$}", size, width = size_width);

            let pct_col = if show_pct {
                match (total_listing_display_bytes, entry_total_bytes) {
                    (Some(total), Some(entry_bytes)) if total > 0 => {
                        let pct = (entry_bytes as f64 * 100.0) / (total as f64);
                        format!("{:>5.0}%", pct)
                    }
                    _ => format!("{:>width$}", "-", width = pct_width),
                }
            } else {
                String::new()
            };

            let date = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|t| DateTime::<Local>::from(t).format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "-".to_string());
            let date_col = format!("{:<width$}", truncate_to(&date, date_width), width = date_width);

            print!("{}", styled_name);
            if show_meta {
                print!(
                    " {} {} {}",
                    style(perms_col).with(CtColor::Rgb { r: 180, g: 150, b: 100 }),
                    style(group_col).with(CtColor::Rgb { r: 180, g: 150, b: 100 }),
                    style(owner_col).with(CtColor::Rgb { r: 180, g: 150, b: 100 })
                );
            }
            if show_size {
                print!(" {}", style(size_col).with(CtColor::Green));
            }
            if show_pct {
                print!(" {}", style(pct_col).with(CtColor::Rgb { r: 220, g: 200, b: 120 }));
            }
            if show_date {
                print!(" {}", style(date_col).with(CtColor::Rgb { r: 120, g: 190, b: 210 }));
            }
            println!();
        } else {
            println!("{} {}", styled_icon, styled_name);
        }
    }

    Ok(())
}

fn parse_list_mode_args<'a>(args: &'a [String]) -> Option<(bool, bool, Option<&'a str>)> {
    let mut list_mode_seen = false;
    let mut include_hidden = false;
    let mut include_total_size = false;
    let mut list_path: Option<&str> = None;

    for arg in args {
        match arg.as_str() {
            "-l" => {
                list_mode_seen = true;
            }
            "-la" => {
                list_mode_seen = true;
                include_hidden = true;
            }
            "--total-size" => {
                include_total_size = true;
            }
            other if !other.starts_with('-') && list_path.is_none() => {
                list_path = Some(other);
            }
            _ => {}
        }
    }

    if list_mode_seen {
        Some((include_hidden, include_total_size, list_path))
    } else {
        None
    }
}

fn print_version() {
    let name = "Shell Buddy (sb)";
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "{} {}",
        style(name)
            .attribute(Attribute::Bold),
        style(format!("v{}", version))
    );
}

fn print_help() {
    let logo = [
        " ┌─┐┬ ┬┌─┐┬  ┬    ┌┐ ┬ ┬┌┬┐┌┬┐┬ ┬",
        " └─┐├─┤├┤ │  │    ├┴┐│ │ ││ ││└┬┘",
        " └─┘┴ ┴└─┘┴─┘┴─┘  └─┘└─┘─┴┘─┴┘ ┴",
    ];

    for (i, line) in logo.iter().enumerate() {
        let color = match i {
            0 => CtColor::Rgb { r: 125, g: 205, b: 255 },
            1 => CtColor::Rgb { r: 110, g: 190, b: 245 },
            _ => CtColor::Rgb { r: 95, g: 175, b: 235 },
        };
        println!("{}", style(*line).with(color).attribute(Attribute::Bold));
    }

    println!(
        "{}",
        style("Bringing your tools together")
            .with(CtColor::Rgb { r: 185, g: 185, b: 185 })
            .attribute(Attribute::Italic)
    );
    println!();

    println!(
        "{}",
        style("Usage:").with(CtColor::Rgb { r: 125, g: 205, b: 255 }).attribute(Attribute::Bold)
    );
    println!("  sb [OPTIONS]");
    println!();
    println!(
        "{}",
        style("Options:").with(CtColor::Rgb { r: 125, g: 205, b: 255 }).attribute(Attribute::Bold)
    );
    println!("  -l [PATH]      List folder and exit");
    println!("  -la [PATH]     List folder including hidden files and exit");
    println!("  --total-size   With -l/-la: recursive size + percent columns");
    println!("  -h, --help     Show this help message");
    println!("  -V, --version  Show app name and current version");
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        print_version();
        return Ok(());
    }
    if let Some((include_hidden, include_total_size, path)) = parse_list_mode_args(&args) {
        return list_current_directory(include_hidden, include_total_size, path);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let mut app = App::new()?;
    let mut deferred_key: Option<KeyEvent> = None;
    let hostname = hostname::get().map(|h| h.to_string_lossy().into_owned()).unwrap_or_else(|_| "host".to_string());
    let user = env::var("USER").unwrap_or_else(|_| "user".to_string());

    loop {
        app.pump_archive_progress();
        app.pump_copy_total_prescan();
        app.pump_copy_progress();
        app.pump_folder_size_progress();
        app.pump_current_dir_total_size_progress();
        app.pump_selected_total_size_progress();
        app.pump_git_info();
        app.pump_notes_progress();
        app.pump_internal_search_candidates_progress();
        app.pump_internal_search_content_progress();
        terminal.draw(|f| {
            let chunks = Layout::default()
                .constraints([Constraint::Min(3), Constraint::Length(2)])
                .split(f.size());

            // --- Header ---
            let header_identity = app.current_header_identity(&user, &hostname);
            let current_display_path = if app.mode == AppMode::PathEditing {
                app.input_buffer.clone()
            } else {
                app.current_dir_display_path()
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
            if let Some(total_suffix) = app.current_dir_total_size_header_suffix() {
                let suffix_width = UnicodeWidthStr::width(total_suffix.as_str()) as u16;
                let right_width = suffix_width.saturating_add(1).min(chunks[0].width.saturating_sub(1));
                if right_width > 0 {
                    let header_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Min(1), Constraint::Length(right_width)])
                        .split(chunks[0]);
                    f.render_widget(Paragraph::new(Line::from(path_spans)), header_chunks[0]);
                    f.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(total_suffix, Style::default().fg(Color::Rgb(150, 220, 150))),
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
            f.render_widget(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(Color::DarkGray)), 
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
            let size_width = 6usize;
            let pct_width = 6usize;
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

            let rows: Vec<Row> = app.entry_render_cache.iter().enumerate().map(|(idx, entry_cache)| {
                let is_marked = app.marked_indices.contains(&idx);
                let is_selected = idx == app.selected_index;
                let (icon_style, name_style) = entry_styles(entry_cache.icon_style, entry_cache.name_style, is_selected);

                let meta_style = Style::default().fg(Color::Rgb(180, 150, 100));
                let size_style = Style::default().fg(Color::Green);
                let pct_style = Style::default().fg(Color::Rgb(220, 200, 120));
                let date_style = Style::default().fg(Color::Rgb(120, 190, 210));
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
                let rendered_name = truncate_with_ellipsis(&entry_cache.raw_name, name_text_width);
                let mut rendered_note = String::new();
                if !note_text.is_empty() {
                    let used = rendered_name.chars().count();
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
                    cells.push(Cell::from(Span::styled(entry_cache.perms_col.as_str(), meta_style)));
                    cells.push(Cell::from(Span::styled(
                        format!("{:>width$}", entry_cache.group_name, width = group_width),
                        meta_style,
                    )));
                    cells.push(Cell::from(Span::styled(
                        format!("{:<width$}", entry_cache.owner_name, width = owner_width),
                        meta_style,
                    )));
                }
                if show_size { cells.push(Cell::from(Span::styled(entry_cache.size_col.as_str(), size_style))); }
                if show_pct {
                    let pct_col = match (app.current_dir_total_size_bytes, entry_cache.size_bytes) {
                        (Some(total), Some(entry_bytes)) if total > 0 => {
                            let pct = (entry_bytes as f64 * 100.0) / (total as f64);
                            format!("{:>5.0}%", pct)
                        }
                        _ => format!("{:>width$}", "-", width = pct_width),
                    };
                    cells.push(Cell::from(Span::styled(pct_col, pct_style)));
                }
                if show_date { cells.push(Cell::from(Span::styled(entry_cache.date_col.as_str(), date_style))); }
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

            // If the selected item is truncated, temporarily hide its metadata and
            // render its full name across the whole row width.
            if let Some(selected_idx) = app.table_state.selected() {
                if let Some(entry_cache) = app.entry_render_cache.get(selected_idx) {
                    let full_name = entry_cache.raw_name.as_str();
                    if full_name.chars().count() > file_name_width {
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
                                    .get(full_name)
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
                                        if app.show_icons {
                                            spans.push(Span::styled(format!("{} ", entry_cache.icon_glyph), icon_style));
                                        }
                                        spans.push(Span::styled(full_name, name_style));
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

                let scope_label = match app.internal_search_scope {
                    InternalSearchScope::Filename => "Filename",
                    InternalSearchScope::Content => "Content",
                };
                let mut lines: Vec<Line> = vec![
                    Line::from(vec![
                        Span::styled("Scope: ", Style::default().fg(Color::Rgb(170, 170, 170))),
                        Span::styled(scope_label, Style::default().fg(Color::Rgb(120, 220, 180)).add_modifier(Modifier::BOLD)),
                        Span::styled("  (Ctrl+T to toggle)", Style::default().fg(Color::DarkGray)),
                    ]),
                    Line::from(vec![
                        Span::styled("Query: ", Style::default().fg(Color::Rgb(170, 170, 170))),
                        Span::styled(app.input_buffer.as_str(), Style::default().fg(Color::Rgb(255, 220, 120))),
                    ]),
                    Line::from(Span::styled(
                        "Up/Down navigate  Enter open  Ctrl+T toggle scope  Tab/Shift+Tab switch tabs  Esc cancel  Regex: re:pattern or /pattern/i",
                        Style::default().fg(Color::DarkGray),
                    )),
                ];

                let mode_line = if app.internal_search_regex_mode {
                    Span::styled("Mode: regex", Style::default().fg(Color::Rgb(120, 220, 180)))
                } else if app.internal_search_scope == InternalSearchScope::Content {
                    Span::styled("Mode: literal", Style::default().fg(Color::Rgb(120, 170, 255)))
                } else {
                    Span::styled("Mode: fuzzy", Style::default().fg(Color::Rgb(120, 170, 255)))
                };
                lines.push(Line::from(mode_line));

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
                let visible_rows = popup_area.height.saturating_sub(2) as usize;
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
                        let marker = if is_selected { "> " } else { "  " };
                        let mut spans: Vec<Span> = vec![Span::styled(marker, base_style)];

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

                        lines.push(Line::from(spans));
                    }
                }

                f.render_widget(Clear, popup_area);
                f.render_widget(
                    Paragraph::new(lines).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(App::panel_tab_bar_line(app.panel_tab))
                            .border_style(Style::default().fg(Color::Rgb(120, 190, 255))),
                    ),
                    popup_area,
                );

                app.clamp_input_cursor();
                let query_prefix = "Query: ".chars().count() as u16;
                let cursor_x = popup_area.x + 1 + query_prefix + app.input_cursor as u16;
                let cursor_y = popup_area.y + 2;
                f.set_cursor(
                    cursor_x.min(popup_area.x + popup_area.width.saturating_sub(1)),
                    cursor_y,
                );
            } else if app.mode == AppMode::Help {
                let help_w = tab_overlay_anchor.width;
                let inner_w = help_w.saturating_sub(4) as usize;
                let shortcut_w = inner_w.clamp(10, 18);
                let section_style = Style::default().fg(Color::Rgb(120, 200, 255)).add_modifier(Modifier::BOLD);
                let shortcut_style = Style::default().fg(Color::Rgb(255, 220, 140)).add_modifier(Modifier::BOLD);
                let desc_style = Style::default().fg(Color::Rgb(200, 200, 200));

                let mut lines: Vec<Line> = vec![
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
                            ("E", "Edit system clipboard content via temporary file"),
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
                            ("n", "Create new file"),
                            ("N", "Create new folder"),
                            ("Ctrl+n", "Add/edit note for selected item(s)"),
                            ("Ctrl+z", "Drop to shell in current directory"),
                            ("F2 / r", "Rename or bulk rename"),
                            ("e / F4", "Edit file, or rename if selection is a folder"),
                            ("d / Del", "Delete selected/marked item(s)"),
                            ("x / p", "Toggle executable bit / protect/unprotect file"),
                            ("Z", "Create or extract archive"),
                            ("o", "Open with default GUI app"),
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
                            ("i", "Split shell (left) + less preview (right 30%)"),
                            ("I", "Open integrations panel"),
                            ("b / 0-9", "Open bookmarks / jump to bookmark"),
                        ],
                    ),
                    (
                        "General",
                        [
                            ("h", "Open help"),
                            ("q / Esc", "Quit Shell Buddy"),
                            ("", ""),
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

                let desired_h = (lines.len() as u16 + 2).max(18);
                let help_h = desired_h.min(tab_overlay_anchor.height);
                let help_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    help_w,
                    help_h,
                );
                f.render_widget(Clear, help_area);

                let visible_lines = (help_area.height as usize).saturating_sub(2);
                let total_lines = lines.len();
                let max_scroll = total_lines.saturating_sub(visible_lines);
                app.help_max_offset = max_scroll as u16;
                let clamped_offset = (app.help_scroll_offset as usize).min(max_scroll) as u16;
                
                f.render_widget(
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: true })
                        .scroll((clamped_offset, 0))
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(App::panel_tab_bar_line(app.panel_tab))
                                .border_style(Style::default().fg(Color::Rgb(110, 170, 240))),
                        ),
                    help_area,
                );
            } else if matches!(app.mode, AppMode::Renaming | AppMode::PasteRenaming | AppMode::NewFile | AppMode::NewFolder | AppMode::ArchiveCreate | AppMode::NoteEditing | AppMode::CommandInput | AppMode::GitCommitMessage) {
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
                    _ => " New Name ",
                };
                let prompt_value = app.input_buffer.clone();
                f.render_widget(Paragraph::new(prompt_value).block(Block::default().borders(Borders::ALL).title(title)), rename_area);
                app.clamp_input_cursor();
                let cursor_x = rename_area.x + 1 + app.input_cursor as u16;
                let cursor_y = rename_area.y + 1;
                f.set_cursor(cursor_x.min(rename_area.x + rename_area.width.saturating_sub(1)), cursor_y);
            } else if app.mode == AppMode::Bookmarks {
                let area = f.size();
                let bookmarks = App::load_bookmarks();
                let mut lines: Vec<Line> = vec![
                    Line::from(Span::styled("Press 0-9 to jump  ·  Tab/Shift+Tab switch tabs  ·  Esc/b/q close", Style::default().fg(Color::DarkGray))),
                    Line::from(""),
                ];
                for (i, path) in &bookmarks {
                    let (label, style) = match path {
                        Some(p) => (
                            format!("[{}]  {}", i, p.display()),
                            Style::default().fg(Color::Rgb(100, 220, 120)),
                        ),
                        None => (
                            format!("[{}]  (not set)", i),
                            Style::default().fg(Color::Rgb(80, 80, 80)),
                        ),
                    };
                    lines.push(Line::from(Span::styled(label, style)));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("Add to your shell config to set bookmarks:", Style::default().fg(Color::Rgb(200, 180, 80)))));
                lines.push(Line::from(Span::styled("  export SB_BOOKMARK_1=\"$HOME/.config\"", Style::default().fg(Color::DarkGray))));
                lines.push(Line::from(Span::styled("  export SB_BOOKMARK_2=\"/var/log\"", Style::default().fg(Color::DarkGray))));
                let bm_h = (lines.len() as u16 + 2).max(17).min(tab_overlay_anchor.height);
                let bm_w = (area.width * 2 / 3).max(50).min(tab_overlay_anchor.width);
                let bm_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    bm_w,
                    bm_h,
                );
                f.render_widget(Clear, bm_area);
                f.render_widget(
                    Paragraph::new(lines)
                        .block(Block::default().borders(Borders::ALL).title(App::panel_tab_bar_line(app.panel_tab))
                            .border_style(Style::default().fg(Color::Rgb(100, 150, 255)))),
                    bm_area,
                );
            } else if app.mode == AppMode::Integrations {
                let area = f.size();
                let integrations = app.integration_rows_cache.clone();
                if !integrations.is_empty() && app.integration_selected >= integrations.len() {
                    app.integration_selected = integrations.len() - 1;
                }
                let mut lines: Vec<Line> = vec![
                    Line::from(Span::styled("↑↓ navigate  Space toggle  Enter install missing  Tab/Shift+Tab switch tabs  Esc/I/q close", Style::default().fg(Color::DarkGray))),
                    Line::from(""),
                ];
                for (i, row) in integrations.iter().enumerate() {
                    let is_selected = i == app.integration_selected;
                    let marker = if is_selected { ">" } else { " " };
                    let status_span = if row.required || (app.integration_enabled(&row.key) && row.available) {
                        Span::styled(" ✓ ", Style::default().fg(Color::Rgb(100, 220, 120)))
                    } else {
                        Span::styled(" ✕ ", Style::default().fg(Color::Rgb(220, 80, 80)))
                    };
                    let base_style = if is_selected {
                        Style::default().bg(Color::Rgb(60, 60, 60)).fg(Color::White)
                    } else {
                        Style::default().fg(Color::Rgb(190, 190, 190))
                    };
                    let name_span = Span::styled(
                        format!("{} {:<12}", marker, row.label),
                        base_style,
                    );
                    let state_span = Span::styled(
                        format!(" {:<10}", row.state),
                        if row.required {
                            base_style.fg(Color::Rgb(200, 200, 200))
                        } else if app.integration_enabled(&row.key) {
                            base_style.fg(Color::Rgb(255, 220, 140))
                        } else {
                            base_style.fg(Color::Rgb(150, 150, 150))
                        },
                    );
                    let category_span = Span::styled(
                        format!(" {:<9}", row.category),
                        base_style,
                    );
                    let purpose_span = Span::styled(
                        format!(" {}", row.description),
                        base_style,
                    );
                    lines.push(Line::from(vec![status_span, name_span, state_span, category_span, purpose_span]));
                }
                let int_h = (lines.len() as u16 + 2).min(tab_overlay_anchor.height);
                let int_w = (area.width * 5 / 6).max(70).min(tab_overlay_anchor.width);
                // Auto-scroll so the selected row stays visible
                let visible_rows = (int_h as usize).saturating_sub(2); // minus top/bottom borders
                let selected_line = app.integration_selected + 2; // +2 for hint line + blank line
                let int_scroll = if selected_line + 1 <= visible_rows {
                    0u16
                } else {
                    (selected_line + 1 - visible_rows) as u16
                };
                let int_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    int_w,
                    int_h,
                );
                f.render_widget(Clear, int_area);
                f.render_widget(
                    Paragraph::new(lines)
                        .scroll((int_scroll, 0))
                        .block(Block::default().borders(Borders::ALL).title(App::panel_tab_bar_line(app.panel_tab))
                            .border_style(Style::default().fg(Color::Rgb(180, 130, 255)))),
                    int_area,
                );
            } else if app.mode == AppMode::SortMenu {
                let options = App::sort_mode_options();
                let mut lines: Vec<Line> = vec![
                    Line::from(Span::styled("↑↓ pick  Enter/→ apply  Shift+Tab/Tab switch tabs  Esc/q/← close", Style::default().fg(Color::DarkGray))),
                    Line::from(""),
                ];
                for (idx, mode) in options.iter().enumerate() {
                    let is_selected = idx == app.sort_menu_selected;
                    let is_current = *mode == app.sort_mode;
                    let row_text = format!("{}", mode.label());
                    let style = if is_selected {
                        Style::default().bg(Color::Rgb(60, 60, 60)).fg(Color::White)
                    } else if is_current {
                        Style::default().fg(Color::Rgb(255, 220, 140))
                    } else {
                        Style::default().fg(Color::Rgb(190, 190, 190))
                    };
                    lines.push(Line::from(Span::styled(row_text, style)));
                }

                let sort_h = (lines.len() as u16 + 2).max(10).min(tab_overlay_anchor.height);
                let sort_w = tab_overlay_anchor.width;
                let sort_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    sort_w,
                    sort_h,
                );
                f.render_widget(Clear, sort_area);
                f.render_widget(
                    Paragraph::new(lines)
                        .block(Block::default().borders(Borders::ALL).title(App::panel_tab_bar_line(app.panel_tab))
                            .border_style(Style::default().fg(Color::Rgb(120, 190, 255)))),
                    sort_area,
                );
            } else if app.mode == AppMode::SshPicker {
                let area = f.size();
                let ssh_w = (area.width * 2 / 3).max(60).min(area.width);
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

                let mut lines: Vec<Line> = vec![
                    Line::from(Span::styled("\u{2191}\u{2193}: navigate  Enter/→: open or mount  Shift+Tab/Tab: switch tabs  u/Delete: unmount  Esc/q: close", Style::default().fg(Color::DarkGray))),
                    Line::from(""),
                ];
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
                        let style = if is_selected {
                            Style::default().fg(Color::Rgb(20, 20, 30)).bg(Color::Rgb(80, 200, 180)).add_modifier(Modifier::BOLD)
                        } else if is_mounted {
                            Style::default().fg(Color::Rgb(80, 220, 160))
                        } else {
                            Style::default().fg(Color::Rgb(200, 200, 200))
                        };
                        lines.push(Line::from(Span::styled(label, style)));
                    }
                }
                let ssh_h = (lines.len() as u16 + 2).max(8).min(tab_overlay_anchor.height);
                let ssh_area = Rect::new(
                    tab_overlay_anchor.x,
                    tab_overlay_anchor.y,
                    ssh_w.min(tab_overlay_anchor.width),
                    ssh_h,
                );
                f.render_widget(Clear, ssh_area);
                f.render_widget(
                    Paragraph::new(lines)
                        .block(Block::default().borders(Borders::ALL).title(App::panel_tab_bar_line(app.panel_tab))
                            .border_style(Style::default().fg(Color::Rgb(80, 200, 180)))),
                    ssh_area,
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
                        .block(Block::default().borders(Borders::ALL).title(" Confirm Extract ")),
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
                        .block(Block::default().borders(Borders::ALL).title(" Install Integration ")),
                    confirm_area,
                );
            } else if app.mode == AppMode::ConfirmDelete {
                let area = f.size();
                let to_delete = app.delete_targets();
                let mut msg_lines: Vec<String> = vec!["Delete these files?".to_string(), String::new()];
                let max_list_rows = ((area.height.saturating_sub(10) as usize).min(14)).max(1);
                for (idx, path) in to_delete.iter().enumerate() {
                    if idx >= max_list_rows {
                        break;
                    }
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.to_string_lossy().into_owned());
                    msg_lines.push(format!(" - {}", name));
                }
                if to_delete.len() > max_list_rows {
                    let remaining = to_delete.len() - max_list_rows;
                    msg_lines.push(format!(" ... and {} more", remaining));
                }
                msg_lines.push(String::new());
                msg_lines.push("  y = confirm    n / Esc = cancel".to_string());
                let msg = msg_lines.join("\n");

                let content_w = msg_lines
                    .iter()
                    .map(|line| line.chars().count() as u16)
                    .max()
                    .unwrap_or(24);
                let content_h = msg_lines.len() as u16;
                let max_w = area.width.saturating_sub(4).max(1);
                let max_h = area.height.saturating_sub(4).max(1);
                let dialog_w = (content_w + 2)
                    .max(36)
                    .min(max_w);
                let dialog_h = (content_h + 2)
                    .max(6)
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
                        .style(Style::default().fg(Color::Rgb(255, 100, 100)))
                        .block(Block::default().borders(Borders::ALL).title(" Confirm Delete ")),
                    confirm_area,
                );
            }

            // --- Footer ---
            let mut left_status_parts = vec![format!("Total:{}", app.entries.len())];
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
            f.render_widget(Paragraph::new(status).block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(Color::DarkGray))), chunks[1]);
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
            if let Event::Key(key) = event::read()? {
                next_key = Some(key);
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
                            execute!(io::stdout(), LeaveAlternateScreen)?;
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
                            execute!(io::stdout(), EnterAlternateScreen)?;
                            execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
                            enable_raw_mode()?;
                            terminal.clear()?;
                        } else {
                            app.set_status("not a git repository");
                        }
                    }
                    KeyCode::Tab => {
                        let current = app.current_dir.to_string_lossy().into_owned();
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
                    KeyCode::Char('E') => {
                        app.edit_system_clipboard_via_temp_file()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('d') | KeyCode::Delete => {
                        if !app.entries.is_empty() {
                            app.mode = AppMode::ConfirmDelete;
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
                    KeyCode::Char('C') => {
                        app.run_delta_compare()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('o') => {
                        app.open_selected_with_default_app()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('i') => {
                        app.open_split_shell_with_less()?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('l') => {
                        if let Some(entry) = app.entries.get(app.selected_index) {
                            let selected_path = entry.path();
                            if !selected_path.is_dir() {
                                disable_raw_mode()?;
                                execute!(io::stdout(), LeaveAlternateScreen)?;
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
                                execute!(io::stdout(), EnterAlternateScreen)?;
                                terminal.clear()?;
                            }
                        }
                    }
                    KeyCode::Char('n') => {
                        app.begin_input_edit(AppMode::NewFile, String::new());
                    }
                    KeyCode::Char('N') => {
                        app.begin_input_edit(AppMode::NewFolder, String::new());
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
                                    execute!(io::stdout(), LeaveAlternateScreen)?;
                                    let mut cmd = Command::new("vidir");
                                    for p in &targets {
                                        cmd.arg(p);
                                    }
                                    let _ = cmd.status();
                                    enable_raw_mode()?;
                                    execute!(io::stdout(), EnterAlternateScreen)?;
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
                                execute!(io::stdout(), LeaveAlternateScreen)?;
                                let _ = Command::new("glow")
                                    .arg("-p")
                                    .arg(&selected_path)
                                    .status();
                                execute!(io::stdout(), EnterAlternateScreen)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_html_file(&selected_path) && app.integration_active("links") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), LeaveAlternateScreen)?;
                                let _ = Command::new("links").arg(&selected_path).status();
                                execute!(io::stdout(), EnterAlternateScreen)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_json_file(&selected_path) && app.integration_active("jnv") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), LeaveAlternateScreen)?;
                                execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
                                let _ = App::preview_json_with_jnv(&selected_path);
                                execute!(io::stdout(), EnterAlternateScreen)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_delimited_text_file(&selected_path) && app.integration_active("csvlens") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), LeaveAlternateScreen)?;
                                let _ = Command::new("csvlens").arg(&selected_path).status();
                                execute!(io::stdout(), EnterAlternateScreen)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_audio_file(&selected_path) && app.integration_active("sox") {
                                use std::process::Stdio;
                                disable_raw_mode()?;
                                execute!(io::stdout(), LeaveAlternateScreen)?;
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

                                execute!(io::stdout(), EnterAlternateScreen)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_pdf_file(&selected_path) && app.integration_active("pdftotext") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), LeaveAlternateScreen)?;

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

                                execute!(io::stdout(), EnterAlternateScreen)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else if App::is_cast_file(&selected_path) && app.integration_active("asciinema") {
                                disable_raw_mode()?;
                                execute!(io::stdout(), LeaveAlternateScreen)?;

                                let _ = App::preview_cast_with_asciinema(&selected_path)?;

                                execute!(io::stdout(), EnterAlternateScreen)?;
                                enable_raw_mode()?;
                                terminal.clear()?;
                            }
                            else { 
                                disable_raw_mode()?; execute!(io::stdout(), LeaveAlternateScreen)?;
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
                                enable_raw_mode()?; execute!(io::stdout(), EnterAlternateScreen)?;
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
                            disable_raw_mode()?; execute!(io::stdout(), LeaveAlternateScreen)?;
                            let _ = Command::new("sh")
                                .args(["-c", &cmd])
                                .current_dir(&app.current_dir)
                                .stdin(Stdio::inherit())
                                .stdout(Stdio::inherit())
                                .stderr(Stdio::inherit())
                                .status();
                            enable_raw_mode()?; execute!(io::stdout(), EnterAlternateScreen)?;
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
                            disable_raw_mode()?; execute!(io::stdout(), LeaveAlternateScreen)?;
                            let _ = Command::new("sh")
                                .args(["-c", &cmd])
                                .current_dir(&app.current_dir)
                                .stdin(Stdio::inherit())
                                .stdout(Stdio::inherit())
                                .stderr(Stdio::inherit())
                                .status();
                            enable_raw_mode()?; execute!(io::stdout(), EnterAlternateScreen)?;
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
                                disable_raw_mode()?; execute!(io::stdout(), LeaveAlternateScreen)?;
                                execute!(io::stdout(), Show)?;
                                if !path.is_dir() && App::is_binary_file(&path) && app.integration_active("hexedit") {
                                    let _ = Command::new("hexedit").arg(&path).status();
                                } else {
                                    let _ = Command::new(env::var("EDITOR").unwrap_or_else(|_| "nano".to_string())).arg(&path).status();
                                }
                                enable_raw_mode()?; execute!(io::stdout(), EnterAlternateScreen)?;
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
                        let is_dir = app.mode == AppMode::NewFolder;
                        app.create_entry_from_input(is_dir);
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
                                        execute!(io::stdout(), LeaveAlternateScreen)?;
                                        let result = app.mount_ssh_host(&host);
                                        enable_raw_mode()?;
                                        execute!(io::stdout(), EnterAlternateScreen)?;
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
                                        execute!(io::stdout(), LeaveAlternateScreen)?;
                                        println!("Connecting to rclone remote: {}…", name);
                                        let result = app.mount_rclone_remote(&name, &rtype);
                                        enable_raw_mode()?;
                                        execute!(io::stdout(), EnterAlternateScreen)?;
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
                    KeyCode::Char('y') => {
                        let to_delete = app.delete_targets();
                        for path in to_delete {
                            if path.is_dir() { let _ = fs::remove_dir_all(&path); }
                            else { let _ = fs::remove_file(&path); }
                        }
                        app.mode = AppMode::Browsing;
                        app.refresh_entries_or_status();
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
        LeaveAlternateScreen,
        TermClear(ClearType::All),
        MoveTo(0, 0)
    )?;
    let _ = std::fs::write("/tmp/sb_path", app.current_dir.to_string_lossy().as_bytes());
    Ok(())
}