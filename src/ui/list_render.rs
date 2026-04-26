use ratatui::style::Color;

pub(crate) fn permission_gradient_segments(
    perms_text: &str,
    perms_width: usize,
) -> Vec<(String, Option<Color>)> {
    let left_pad = perms_width.saturating_sub(perms_text.chars().count());
    let chars: Vec<char> = perms_text.chars().collect();
    let steps = chars.len().saturating_sub(1).max(1) as f32;

    let mut segments: Vec<(String, Option<Color>)> = Vec::new();
    if left_pad > 0 {
        segments.push((" ".repeat(left_pad), None));
    }
    for (i, ch) in chars.iter().enumerate() {
        let t = 1.0 - (i as f32 / steps);
        let r = (196.0 + (255.0 - 196.0) * t).round() as u8;
        let g = (150.0 + (255.0 - 150.0) * t).round() as u8;
        let b = (96.0 + (255.0 - 96.0) * t).round() as u8;
        segments.push((ch.to_string(), Some(Color::Rgb(r, g, b))));
    }

    segments
}