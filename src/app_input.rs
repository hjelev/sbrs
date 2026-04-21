use crate::{App, AppMode};

impl App {
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
