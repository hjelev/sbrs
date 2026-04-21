use std::{
    fs, io,
    path::PathBuf,
    process::Command,
    str::FromStr,
    sync::mpsc,
    thread,
};

use rayon::prelude::*;

use crate::{App, CurrentDirTotalSizeMsg, FolderSizeMsg, SelectedTotalSizeMsg};

impl App {
    pub(crate) fn clear_selected_total_size_state(&mut self) {
        self.selected_total_size_scan_id = self.selected_total_size_scan_id.wrapping_add(1);
        self.selected_total_size_rx = None;
        self.selected_total_size_pending = false;
        self.selected_total_size_bytes = None;
        self.selected_total_size_items = 0;
    }

    pub(crate) fn start_selected_total_size_scan(&mut self) {
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

    pub(crate) fn pump_selected_total_size_progress(&mut self) {
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

    pub(crate) fn selected_total_size_status(&self) -> Option<String> {
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

    pub(crate) fn start_folder_size_scan(&mut self) {
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

    pub(crate) fn clear_current_dir_total_size_state(&mut self) {
        self.current_dir_total_size_scan_id = self.current_dir_total_size_scan_id.wrapping_add(1);
        self.current_dir_total_size_rx = None;
        self.current_dir_total_size_pending = false;
        self.current_dir_total_size_bytes = None;
    }

    pub(crate) fn filesystem_free_space_bytes(path: &PathBuf) -> Option<u64> {
        let output = Command::new("df").args(["-kP"]).arg(path).output().ok()?;
        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.lines().rev().find(|line| !line.trim().is_empty())?;
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            return None;
        }

        let available_kb = u64::from_str(cols[3]).ok()?;
        Some(available_kb.saturating_mul(1024))
    }

    pub(crate) fn refresh_current_dir_free_space(&mut self) {
        self.current_dir_free_bytes = Self::filesystem_free_space_bytes(&self.current_dir);
    }

    pub(crate) fn start_current_dir_total_size_scan(&mut self) {
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

    pub(crate) fn pump_current_dir_total_size_progress(&mut self) {
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

    pub(crate) fn current_dir_total_size_header_suffix(&self) -> Option<String> {
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

    pub(crate) fn reset_folder_size_columns(&mut self) {
        let size_width = 6usize;
        for (idx, entry) in self.entries.iter().enumerate() {
            if entry.path().is_dir() {
                self.entry_render_cache[idx].size_col = format!("{:>width$}", "-", width = size_width);
                self.entry_render_cache[idx].size_bytes = None;
            }
        }
    }

    pub(crate) fn set_folder_size_enabled(&mut self, enabled: bool) {
        if enabled == self.folder_size_enabled {
            return;
        }

        self.folder_size_enabled = enabled;
        self.folder_size_scan_id = self.folder_size_scan_id.wrapping_add(1);
        self.folder_size_rx = None;
        self.reset_folder_size_columns();

        if enabled {
            self.set_status("folder size calculation: on");
            self.start_folder_size_scan();
            self.start_current_dir_total_size_scan();
            self.start_selected_total_size_scan();
        } else {
            self.set_status("folder size calculation: off");
            self.clear_current_dir_total_size_state();
            self.clear_selected_total_size_state();
        }
    }

    pub(crate) fn pump_folder_size_progress(&mut self) {
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
                        self.entry_render_cache[idx].size_col =
                            format!("{:>width$}", Self::format_size(size), width = 6);
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

    pub(crate) fn compute_total_bytes(src: &PathBuf) -> io::Result<u64> {
        Self::compute_total_bytes_inner(src, true)
    }

    pub(crate) fn compute_total_display_bytes(src: &PathBuf) -> io::Result<u64> {
        Self::compute_total_display_bytes_inner(src, false)
    }

    pub(crate) fn compute_total_bytes_inner(src: &PathBuf, follow_symlink_dir: bool) -> io::Result<u64> {
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

    pub(crate) fn compute_total_display_bytes_inner(
        src: &PathBuf,
        follow_symlink_dir: bool,
    ) -> io::Result<u64> {
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

    pub(crate) fn compute_dir_total_bytes(dir: &PathBuf) -> io::Result<u64> {
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

    pub(crate) fn compute_dir_total_display_bytes(dir: &PathBuf) -> io::Result<u64> {
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

    pub(crate) fn display_leaf_size(metadata: &fs::Metadata) -> u64 {
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
}
