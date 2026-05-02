use crate::integration::rows::IntegrationRow;
use crate::SortMode;
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

const PANEL_TABS: &[(&str, u8)] = &[
    (" Help ", 0),
    (" Search ", 1),
    (" Bookmarks ", 2),
    (" Remote Mounts ", 3),
    (" Sorting ", 4),
    (" Integrations ", 5),
];

pub fn panel_tab_bar_line(active: u8) -> Line<'static> {
    let active_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(Color::Rgb(100, 100, 100));
    let sep_style = Style::default().fg(Color::Rgb(80, 200, 180));
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, (label, idx)) in PANEL_TABS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("│", sep_style));
        }
        let style = if *idx == active {
            active_style
        } else {
            inactive_style
        };
        spans.push(Span::styled(*label, style));
    }
    Line::from(spans)
}

pub fn panel_tab_hit_test(relative_x: u16) -> Option<u8> {
    let mut cursor = 0u16;

    for (index, (label, tab)) in PANEL_TABS.iter().enumerate() {
        if index > 0 {
            if relative_x == cursor {
                return None;
            }
            cursor = cursor.saturating_add(1);
        }

        let width = label.chars().count() as u16;
        if relative_x >= cursor && relative_x < cursor.saturating_add(width) {
            return Some(*tab);
        }
        cursor = cursor.saturating_add(width);
    }

    None
}

pub fn shortcut_footer_line(entries: &[(&'static str, &'static str)]) -> Line<'static> {
    let shortcut_style = Style::default().fg(Color::White);
    let desc_style = Style::default().fg(Color::DarkGray);
    let sep_style = Style::default().fg(Color::DarkGray);
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];

    for (idx, (shortcut, description)) in entries.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled("  ", sep_style));
        }
        spans.push(Span::styled(*shortcut, shortcut_style));
        spans.push(Span::styled(":", shortcut_style));
        spans.push(Span::styled(*description, desc_style));
    }

    Line::from(spans)
}

pub fn shortcut_footer_lines(entries: &[(&'static str, &'static str)]) -> Vec<Line<'static>> {
    vec![Line::from(""), shortcut_footer_line(entries)]
}

pub fn render_integrations_overlay<F>(
    f: &mut Frame,
    area: Rect,
    tab_overlay_anchor: Rect,
    panel_tab: u8,
    integrations: &[IntegrationRow],
    integration_selected: usize,
    is_integration_enabled: F,
)
where
    F: Fn(&str) -> bool,
{
    let int_w = (area.width * 5 / 6).max(70).min(tab_overlay_anchor.width);
    let int_content_w = int_w.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = vec![Line::from("")];
    for (i, row) in integrations.iter().enumerate() {
        let is_selected = i == integration_selected;
        let status_text = if row.required || (is_integration_enabled(&row.key) && row.available) {
            " ✓ ".to_string()
        } else if is_integration_enabled(&row.key) && row.partially_supported {
            " ✓ ".to_string()
        } else {
            " ✕ ".to_string()
        };
        let status_style = if row.required || (is_integration_enabled(&row.key) && row.available)
        {
            Style::default().fg(Color::Rgb(100, 220, 120))
        } else if is_integration_enabled(&row.key) && row.partially_supported {
            Style::default().fg(Color::Rgb(245, 200, 90))
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
            } else if !row.available && !row.partially_supported {
                base_style.fg(Color::Rgb(220, 80, 80))
            } else if is_integration_enabled(&row.key) && row.partially_supported {
                base_style.fg(Color::Rgb(245, 200, 90))
            } else if is_integration_enabled(&row.key) {
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
                spans.push(Span::styled(" ".repeat(int_content_w - used_w), base_style));
            }
        }

        lines.push(Line::from(spans));
    }
    let int_h = (lines.len() as u16 + 4).min(tab_overlay_anchor.height);
    let int_area = Rect::new(tab_overlay_anchor.x, tab_overlay_anchor.y, int_w, int_h);
    f.render_widget(Clear, int_area);
    let int_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(panel_tab_bar_line(panel_tab))
        .title_style(Style::default().fg(Color::White))
        .border_style(Style::default().fg(Color::Rgb(80, 200, 180)));
    let int_inner = int_block.inner(int_area);
    f.render_widget(int_block, int_area);
    f.render_widget(
        Paragraph::new(Span::styled(
            "x",
            Style::default().fg(Color::Rgb(170, 170, 170)),
        )),
        Rect::new(
            int_area.x + int_area.width.saturating_sub(2),
            int_area.y,
            1,
            1,
        ),
    );
    let int_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(int_inner);
    let visible_rows = int_chunks[0].height as usize;
    let total_rows = lines.len();
    let max_scroll = total_rows.saturating_sub(visible_rows);
    let selected_line = integration_selected + 1;
    let int_scroll = if selected_line + 1 <= visible_rows {
        0usize
    } else {
        selected_line + 1 - visible_rows
    }
    .min(max_scroll);
    let can_draw_scrollbar = int_chunks[0].width > 2 && total_rows > visible_rows;

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
        Paragraph::new(indented_lines).scroll((int_scroll as u16, 0)),
        int_chunks[0],
    );
    if can_draw_scrollbar {
        let sb_area = Rect::new(
            int_area.x + int_area.width.saturating_sub(1),
            int_chunks[0].y,
            1,
            int_chunks[0].height,
        );
        let track_h = sb_area.height as usize;
        if track_h > 0 {
            let thumb_h = ((visible_rows * track_h + total_rows.saturating_sub(1)) / total_rows)
                .max(1)
                .min(track_h);
            let scroll_space = track_h.saturating_sub(thumb_h);
            let thumb_y = if max_scroll == 0 {
                0
            } else {
                (int_scroll * scroll_space + (max_scroll / 2)) / max_scroll
            };

            let mut sb_lines: Vec<Line> = Vec::with_capacity(track_h);
            for row in 0..track_h {
                let in_thumb = row >= thumb_y && row < thumb_y + thumb_h;
                let (ch, color) = if in_thumb {
                    ("┃", Color::Rgb(120, 240, 220))
                } else {
                    ("│", Color::Rgb(80, 200, 180))
                };
                sb_lines.push(Line::from(Span::styled(ch, Style::default().fg(color))));
            }
            f.render_widget(Paragraph::new(sb_lines), sb_area);
        }
    }
    f.render_widget(
        Paragraph::new(shortcut_footer_lines(&[
            ("↑↓", "navigate"),
            ("Space", "toggle"),
            ("Enter", "install missing"),
            ("Tab", "switch tabs"),
            ("Esc", "close"),
        ])),
        int_chunks[1],
    );
}

pub fn render_help_overlay(
    f: &mut Frame,
    tab_overlay_anchor: Rect,
    panel_tab: u8,
    help_scroll_offset: u16,
) -> (u16, u16) {
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
        .title(panel_tab_bar_line(panel_tab))
        .title_style(Style::default().fg(Color::White))
        .border_style(Style::default().fg(Color::Rgb(80, 200, 180)));
    let help_inner = help_block.inner(help_area);
    f.render_widget(help_block, help_area);
    f.render_widget(
        Paragraph::new(Span::styled(
            "x",
            Style::default().fg(Color::Rgb(170, 170, 170)),
        )),
        Rect::new(
            help_area.x + help_area.width.saturating_sub(2),
            help_area.y,
            1,
            1,
        ),
    );
    let help_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(help_inner);
    let help_content_area = help_chunks[0];
    let help_footer_area = help_chunks[1];

    let needs_scroll = lines.len() > help_content_area.height as usize;
    let can_draw_scrollbar = help_content_area.width > 2 && needs_scroll;
    let help_text_area = help_content_area;

    let visible_lines = help_text_area.height as usize;
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(visible_lines);
    let max_offset = max_scroll as u16;
    let clamped_offset = (help_scroll_offset as usize).min(max_scroll) as u16;
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
        help_text_area,
    );
    if can_draw_scrollbar {
        let sb_area = Rect::new(
            help_area.x + help_area.width.saturating_sub(1),
            help_content_area.y,
            1,
            help_content_area.height,
        );
        let track_h = sb_area.height as usize;
        if track_h > 0 {
            let offset = clamped_offset as usize;
            let thumb_h = ((visible_lines * track_h + total_lines.saturating_sub(1)) / total_lines)
                .max(1)
                .min(track_h);
            let scroll_space = track_h.saturating_sub(thumb_h);
            let thumb_y = if max_scroll == 0 {
                0
            } else {
                (offset * scroll_space + (max_scroll / 2)) / max_scroll
            };

            let mut sb_lines: Vec<Line> = Vec::with_capacity(track_h);
            for row in 0..track_h {
                let in_thumb = row >= thumb_y && row < thumb_y + thumb_h;
                let (ch, color) = if in_thumb {
                    ("┃", Color::Rgb(120, 240, 220))
                } else {
                    ("│", Color::Rgb(80, 200, 180))
                };
                sb_lines.push(Line::from(Span::styled(ch, Style::default().fg(color))));
            }
            f.render_widget(Paragraph::new(sb_lines), sb_area);
        }
    }
    f.render_widget(
        Paragraph::new(shortcut_footer_lines(&[
            ("↑↓/PgUp/PgDn/Home/End", "scroll"),
            ("Tab", "switch tabs"),
            ("Esc", "close"),
        ])),
        help_footer_area,
    );

    (max_offset, clamped_offset)
}

pub fn render_bookmarks_overlay(
    f: &mut Frame,
    tab_overlay_anchor: Rect,
    panel_tab: u8,
    bookmarks: &[(usize, Option<PathBuf>)],
    bookmark_selected: usize,
) {
    let mut lines: Vec<Line> = vec![Line::from("")];
    let bm_w = tab_overlay_anchor.width;
    let bm_content_w = bm_w.saturating_sub(2) as usize;
    for (row_idx, (i, path)) in bookmarks.iter().enumerate() {
        let is_selected = row_idx == bookmark_selected;
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
        .title(panel_tab_bar_line(panel_tab))
        .title_style(Style::default().fg(Color::White))
        .border_style(Style::default().fg(Color::Rgb(80, 200, 180)));
    let bm_inner = bm_block.inner(bm_area);
    f.render_widget(bm_block, bm_area);
    f.render_widget(
        Paragraph::new(Span::styled(
            "x",
            Style::default().fg(Color::Rgb(170, 170, 170)),
        )),
        Rect::new(
            bm_area.x + bm_area.width.saturating_sub(2),
            bm_area.y,
            1,
            1,
        ),
    );
    let bm_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(bm_inner);
    f.render_widget(Paragraph::new(lines), bm_chunks[0]);
    f.render_widget(
        Paragraph::new(shortcut_footer_lines(&[
            ("↑↓", "navigate"),
            ("Enter/0-9", "jump"),
            ("Tab", "switch tabs"),
            ("Esc", "close"),
        ])),
        bm_chunks[1],
    );
}

pub fn render_sort_overlay(
    f: &mut Frame,
    tab_overlay_anchor: Rect,
    panel_tab: u8,
    options: &[SortMode],
    sort_menu_selected: usize,
    current_sort_mode: SortMode,
    nerd_font_active: bool,
) {
    let sort_w = tab_overlay_anchor.width;
    let sort_content_w = sort_w.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = vec![Line::from("")];
    for (idx, mode) in options.iter().enumerate() {
        let is_selected = idx == sort_menu_selected;
        let is_current = *mode == current_sort_mode;
        let (nerd_icon, fallback_icon) = match mode {
            SortMode::NameAsc => ("\u{f15d}", "[A-Z]"),
            SortMode::NameDesc => ("\u{f15e}", "[Z-A]"),
            SortMode::ExtensionAsc => ("\u{f1c9}", "[EXT]"),
            SortMode::SizeAsc => ("\u{f160}", "[SZ+]"),
            SortMode::SizeDesc => ("\u{f161}", "[SZ-]"),
            SortMode::ModifiedNewest => ("\u{f017}", "[NEW]"),
            SortMode::ModifiedOldest => ("\u{f1da}", "[OLD]"),
        };
        let sort_icon = if nerd_font_active {
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
        .title(panel_tab_bar_line(panel_tab))
        .title_style(Style::default().fg(Color::White))
        .border_style(Style::default().fg(Color::Rgb(80, 200, 180)));
    let sort_inner = sort_block.inner(sort_area);
    f.render_widget(sort_block, sort_area);
    f.render_widget(
        Paragraph::new(Span::styled(
            "x",
            Style::default().fg(Color::Rgb(170, 170, 170)),
        )),
        Rect::new(
            sort_area.x + sort_area.width.saturating_sub(2),
            sort_area.y,
            1,
            1,
        ),
    );
    let sort_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(sort_inner);
    f.render_widget(Paragraph::new(lines), sort_chunks[0]);
    f.render_widget(
        Paragraph::new(shortcut_footer_lines(&[
            ("↑↓", "navigate"),
            ("Enter", "apply"),
            ("Tab", "switch tabs"),
            ("Esc", "close"),
        ])),
        sort_chunks[1],
    );
}
