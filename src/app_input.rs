use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{App, AppMode, PathFilterMode, PathInputFilter};

const TREE_DOUBLE_TAP_WINDOW_MS: u64 = 320;

impl App {
    pub(crate) fn consume_quick_tree_double_tap(&mut self, key: char) -> bool {
        let now = Instant::now();
        let is_double = self
            .tree_last_tap
            .map(|(last_key, last_ts)| {
                last_key == key
                    && now.duration_since(last_ts)
                        <= Duration::from_millis(TREE_DOUBLE_TAP_WINDOW_MS)
            })
            .unwrap_or(false);

        self.tree_last_tap = if is_double { None } else { Some((key, now)) };
        is_double
    }

    fn dir_has_visible_children(&self, path: &PathBuf) -> bool {
        let Ok(read_dir) = fs::read_dir(path) else {
            return false;
        };

        read_dir.filter_map(|entry| entry.ok()).any(|entry| {
            if self.show_hidden {
                true
            } else {
                !entry.file_name().to_string_lossy().starts_with('.')
            }
        })
    }

    fn visible_child_dirs(&self, path: &PathBuf) -> Vec<PathBuf> {
        let Ok(read_dir) = fs::read_dir(path) else {
            return Vec::new();
        };

        read_dir
            .filter_map(|entry| entry.ok())
            .filter(|entry| self.show_hidden || !entry.file_name().to_string_lossy().starts_with('.'))
            .map(|entry| entry.path())
            .filter(|entry_path| entry_path.is_dir())
            .collect()
    }

    fn max_expand_level_for_dir(&self, path: &PathBuf) -> usize {
        fn walk(app: &App, dir: &PathBuf, level: usize, max_level: &mut usize) {
            *max_level = (*max_level).max(level);
            for child in app.visible_child_dirs(dir) {
                walk(app, &child, level.saturating_add(1), max_level);
            }
        }

        let mut max_level = 1usize;
        walk(self, path, 1, &mut max_level);
        max_level
    }

    fn selected_or_marked_dir_paths(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if !self.marked_indices.is_empty() {
            for idx in &self.marked_indices {
                if let Some(entry) = self.entries.get(*idx) {
                    let path = entry.path();
                    if path.is_dir() && self.dir_has_visible_children(&path) {
                        dirs.push(path);
                    }
                }
            }
        } else if let Some(entry) = self.entries.get(self.selected_index) {
            let path = entry.path();
            if path.is_dir() && self.dir_has_visible_children(&path) {
                dirs.push(path);
            }
        }
        dirs.sort();
        dirs.dedup();
        dirs
    }

    pub(crate) fn expand_tree_on_selected_dirs(&mut self, levels: usize) {
        let targets = self.selected_or_marked_dir_paths();
        if targets.is_empty() {
            self.set_status("tree expand: no non-empty selected folders");
            return;
        }
        let step = levels.max(1);
        for path in targets {
            let current = self.tree_expansion_levels.get(&path).copied().unwrap_or(0);
            let max_expand = self.max_expand_level_for_dir(&path);
            self.tree_expansion_levels
                .insert(path, current.saturating_add(step).min(max_expand));
        }
        self.refresh_entries_or_status();
    }

    pub(crate) fn contract_tree_on_selected_dirs(&mut self, levels: usize) {
        let targets = self.selected_or_marked_dir_paths();
        if targets.is_empty() {
            self.set_status("tree contract: no selected folders");
            return;
        }
        for path in targets {
            let current = self.tree_expansion_levels.get(&path).copied().unwrap_or(0);
            let next = current.saturating_sub(levels.max(1));
            if next == 0 {
                self.tree_expansion_levels.remove(&path);
            } else {
                self.tree_expansion_levels.insert(path, next);
            }
        }
        self.refresh_entries_or_status();
    }

    pub(crate) fn collapse_all_tree_expansions(&mut self) {
        self.tree_expansion_levels.clear();
        self.refresh_entries_or_status();
    }

    pub(crate) fn expand_tree_to_max_on_selected_dirs(&mut self) {
        let targets = self.selected_or_marked_dir_paths();
        if targets.is_empty() {
            self.set_status("tree expand: no non-empty selected folders");
            return;
        }
        for path in targets {
            let max_expand = self.max_expand_level_for_dir(&path);
            self.tree_expansion_levels.insert(path, max_expand);
        }
        self.refresh_entries_or_status();
    }

    pub(crate) fn parse_path_filter_suffix(raw: &str) -> Option<(String, PathInputFilter)> {
        let trimmed = raw.trim();
        let (base, tail) = trimmed.rsplit_once('/')?;
        if tail.is_empty() {
            return None;
        }

        let base_path = if base.is_empty() && trimmed.starts_with('/') {
            "/".to_string()
        } else {
            base.to_string()
        };

        if let Some(pattern) = tail.strip_prefix('^') {
            return Some((
                base_path,
                PathInputFilter {
                    mode: PathFilterMode::Prefix,
                    pattern: pattern.to_string(),
                },
            ));
        }

        if let Some(pattern) = tail.strip_suffix('$') {
            return Some((
                base_path,
                PathInputFilter {
                    mode: PathFilterMode::Suffix,
                    pattern: pattern.to_string(),
                },
            ));
        }

        if let Some(pattern) = tail.strip_prefix('~') {
            return Some((
                base_path,
                PathInputFilter {
                    mode: PathFilterMode::Contains,
                    pattern: pattern.to_string(),
                },
            ));
        }

        None
    }

    pub(crate) fn begin_input_edit(&mut self, mode: AppMode, initial: String) {
        self.mode = mode;
        self.input_buffer = initial;
        self.input_cursor = self.input_buffer.chars().count();
    }

    pub(crate) fn clear_input_edit(&mut self) {
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

    pub(crate) fn clamp_input_cursor(&mut self) {
        let len = self.input_buffer.chars().count();
        self.input_cursor = self.input_cursor.min(len);
    }

    pub(crate) fn move_selection_delta(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let max_idx = (self.entries.len() - 1) as isize;
        let next = ((self.selected_index as isize) + delta).clamp(0, max_idx) as usize;
        self.selected_index = next;
        self.table_state.select(Some(next));
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

    pub(crate) fn input_insert_char(&mut self, c: char) {
        self.clamp_input_cursor();
        let insert_at = Self::byte_index_for_char(&self.input_buffer, self.input_cursor);
        self.input_buffer.insert(insert_at, c);
        self.input_cursor = self.input_cursor.saturating_add(1);
    }

    pub(crate) fn input_backspace(&mut self) {
        self.clamp_input_cursor();
        if self.input_cursor == 0 {
            return;
        }
        let start = Self::byte_index_for_char(&self.input_buffer, self.input_cursor - 1);
        let end = Self::byte_index_for_char(&self.input_buffer, self.input_cursor);
        self.input_buffer.drain(start..end);
        self.input_cursor -= 1;
    }

    pub(crate) fn input_delete(&mut self) {
        self.clamp_input_cursor();
        let len = self.input_buffer.chars().count();
        if self.input_cursor >= len {
            return;
        }
        let start = Self::byte_index_for_char(&self.input_buffer, self.input_cursor);
        let end = Self::byte_index_for_char(&self.input_buffer, self.input_cursor + 1);
        self.input_buffer.drain(start..end);
    }

    pub(crate) fn input_move_left(&mut self) {
        self.input_cursor = self.input_cursor.saturating_sub(1);
    }

    pub(crate) fn input_move_right(&mut self) {
        let len = self.input_buffer.chars().count();
        self.input_cursor = (self.input_cursor + 1).min(len);
    }

    pub(crate) fn input_move_home(&mut self) {
        self.input_cursor = 0;
    }

    pub(crate) fn input_move_end(&mut self) {
        self.input_cursor = self.input_buffer.chars().count();
    }
}
