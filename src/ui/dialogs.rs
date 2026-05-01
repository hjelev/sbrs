use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};
use std::path::{Path, PathBuf};

pub fn confirm_integration_install_msg_lines(
    key: &str,
    package: &str,
    brew_display: &str,
    brew_missing: bool,
) -> Vec<String> {
    let mut msg_lines: Vec<String> = vec![
        "Install missing integration?".to_string(),
        String::new(),
        format!(" Integration: {}", key),
        format!(" Package:     {}", package),
        format!(" Command:     {} install {}", brew_display, package),
        String::new(),
    ];

    if brew_missing {
        msg_lines.push("Homebrew is not installed; setup guidance will be shown first.".to_string());
        msg_lines.push(String::new());
    }

    msg_lines.push("  Enter: activate selected button   ←/→/Tab: switch".to_string());
    msg_lines
}

pub fn confirm_integration_install_dialog_area(area: Rect, msg_lines: &[String]) -> Rect {
    let content_w = msg_lines
        .iter()
        .map(|line| line.chars().count() as u16)
        .max()
        .unwrap_or(36);
    let content_h = msg_lines.len() as u16;
    let max_w = area.width.saturating_sub(4).max(1);
    let max_h = area.height.saturating_sub(4).max(1);
    let dialog_w = (content_w + 2).max(56).min(max_w);
    let dialog_h = (content_h + 4).max(10).min(max_h);
    Rect::new(
        (area.width.saturating_sub(dialog_w)) / 2,
        (area.height.saturating_sub(dialog_h)) / 2,
        dialog_w,
        dialog_h,
    )
}

pub fn confirm_ok_cancel_button_layout(area: Rect) -> Option<(Rect, u16, u16, u16, u16)> {
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if inner.width == 0 || inner.height == 0 {
        return None;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let button_area = sections[1];

    let prefix_w = 2u16;
    let ok_w = "  OK  ".chars().count() as u16;
    let gap_w = 4u16;
    let cancel_w = "  Cancel  ".chars().count() as u16;
    let total_w = prefix_w + ok_w + gap_w + cancel_w;
    if button_area.width < total_w {
        return None;
    }

    let start_x = button_area.x + (button_area.width - total_w) / 2;
    let ok_start = start_x + prefix_w;
    let cancel_start = ok_start + ok_w + gap_w;
    Some((button_area, ok_start, ok_w, cancel_start, cancel_w))
}

pub fn render_confirm_integration_install_dialog(
    f: &mut Frame,
    msg: String,
    confirm_area: Rect,
    button_focus: u8,
    nerd_font_active: bool,
) {
    f.render_widget(Clear, confirm_area);

    let title = if nerd_font_active {
        " \u{f01da} Install Integration "
    } else {
        " Install Integration "
    };

    f.render_widget(
        Paragraph::new(msg)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(Color::Rgb(140, 200, 255)))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(title)
                    .title_style(Style::default().fg(Color::White)),
            ),
        confirm_area,
    );

    if let Some((button_area, _, _, _, _)) = confirm_ok_cancel_button_layout(confirm_area) {
        let ok_focused = button_focus == 0;
        let cancel_focused = !ok_focused;
        let ok_style = if ok_focused {
            Style::default()
                .fg(Color::Rgb(20, 20, 30))
                .bg(Color::Rgb(120, 220, 140))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(200, 220, 200))
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
            Span::styled("  OK  ", ok_style),
            Span::styled("    ", Style::default()),
            Span::styled("  Cancel  ", cancel_style),
        ]);

        f.render_widget(
            Paragraph::new(button_line).alignment(Alignment::Center),
            button_area,
        );
    }
}

pub fn confirm_delete_title(file_count: usize, folder_count: usize) -> String {
    let plural = |count: usize, singular: &str, plural: &str| -> String {
        if count == 1 {
            singular.to_string()
        } else {
            plural.to_string()
        }
    };

    if file_count > 0 && folder_count > 0 {
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
    }
}

pub fn confirm_delete_dialog_area(area: Rect, title: &str) -> Rect {
    let content_w = title.chars().count().max(42) as u16;
    let content_h = area.height.saturating_sub(8).max(7);
    let max_w = area.width.saturating_sub(4).max(1);
    let max_h = area.height.saturating_sub(4).max(1);
    let dialog_w = (content_w + 2).max(48).min(max_w);
    let full_dialog_h = (content_h + 2).max(10).min(max_h);
    let dialog_h = (full_dialog_h / 2).max(8).min(max_h);
    Rect::new(
        (area.width.saturating_sub(dialog_w)) / 2,
        (area.height.saturating_sub(dialog_h)) / 2,
        dialog_w,
        dialog_h,
    )
}

pub fn confirm_delete_button_layout(area: Rect) -> Option<(Rect, u16, u16, u16, u16)> {
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if inner.width == 0 || inner.height == 0 {
        return None;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let button_area = sections[1];

    let prefix_w = 2u16;
    let confirm_w = "  Confirm  ".chars().count() as u16;
    let gap_w = 4u16;
    let cancel_w = "  Cancel  ".chars().count() as u16;
    let total_w = prefix_w + confirm_w + gap_w + cancel_w;
    if button_area.width < total_w {
        return None;
    }

    let start_x = button_area.x + (button_area.width - total_w) / 2;
    let confirm_start = start_x + prefix_w;
    let cancel_start = confirm_start + confirm_w + gap_w;
    Some((button_area, confirm_start, confirm_w, cancel_start, cancel_w))
}

pub fn render_confirm_delete_buttons(f: &mut Frame, button_area: Rect, confirm_focused: bool) {
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
    f.render_widget(
        Paragraph::new(button_line).alignment(Alignment::Center),
        button_area,
    );
}

pub struct ConfirmDeleteRenderState {
    pub max_offset: u16,
    pub clamped_offset: u16,
}

pub fn render_confirm_delete_dialog<F>(
    f: &mut Frame,
    area: Rect,
    title: &str,
    to_delete: &[PathBuf],
    scroll_offset: u16,
    confirm_focused: bool,
    show_icons: bool,
    mut icon_for_path: F,
) -> ConfirmDeleteRenderState
where
    F: FnMut(&Path, bool) -> (String, Style),
{
    let confirm_area = confirm_delete_dialog_area(area, title);
    f.render_widget(Clear, confirm_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .title_style(Style::default().fg(Color::White))
        .border_style(Style::default().fg(Color::Rgb(255, 100, 100)));
    let inner = block.inner(confirm_area);
    f.render_widget(block, confirm_area);

    if inner.width <= 2 || inner.height <= 2 {
        return ConfirmDeleteRenderState {
            max_offset: 0,
            clamped_offset: 0,
        };
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(90, 90, 90)));
    let list_frame_area = sections[0];
    let list_inner = list_block.inner(list_frame_area);
    f.render_widget(list_block, list_frame_area);

    let needs_scroll = to_delete.len() > list_inner.height as usize;
    let can_draw_scrollbar = list_inner.width > 2 && needs_scroll;
    let list_area = list_inner;
    let visible_rows = list_area.height.max(1) as usize;
    let max_scroll = to_delete.len().saturating_sub(visible_rows);
    let offset = (scroll_offset as usize).min(max_scroll);

    let mut list_lines: Vec<Line> = Vec::new();
    if to_delete.is_empty() {
        list_lines.push(Line::from(Span::styled(
            "No selected item",
            Style::default().fg(Color::Rgb(210, 170, 170)),
        )));
    } else {
        let row_name_max = list_area.width.saturating_sub(2) as usize;
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
            let path_is_symlink = path
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            let (icon_glyph, icon_style) = icon_for_path(path, path_is_symlink);
            let mut spans: Vec<Span> = Vec::new();
            if show_icons && !icon_glyph.is_empty() {
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
            list_frame_area.x + list_frame_area.width.saturating_sub(1),
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
                    ("┃", Color::Rgb(120, 120, 120))
                } else {
                    ("│", Color::Rgb(90, 90, 90))
                };
                sb_lines.push(Line::from(Span::styled(ch, Style::default().fg(color))));
            }
            f.render_widget(Paragraph::new(sb_lines), sb_area);
        }
    }

    render_confirm_delete_buttons(f, sections[1], confirm_focused);

    ConfirmDeleteRenderState {
        max_offset: max_scroll as u16,
        clamped_offset: offset as u16,
    }
}
