use ratatui::prelude::*;

pub fn search_spans_with_ranges(
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
