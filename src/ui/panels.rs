use ratatui::prelude::*;

pub fn panel_tab_bar_line(active: u8) -> Line<'static> {
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
        let style = if *idx == active {
            active_style
        } else {
            inactive_style
        };
        spans.push(Span::styled(*label, style));
    }
    Line::from(spans)
}
