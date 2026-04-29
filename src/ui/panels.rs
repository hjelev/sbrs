use ratatui::prelude::*;

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
