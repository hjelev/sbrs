use std::collections::HashMap;

use ratatui::style::Color;

pub(crate) fn size_min_max_from_sizes<I>(sizes: I) -> Option<(u64, u64)>
where
    I: IntoIterator<Item = Option<u64>>,
{
    let mut min_size: Option<u64> = None;
    let mut max_size: u64 = 0;
    for size in sizes.into_iter().flatten() {
        min_size = Some(min_size.map_or(size, |current| current.min(size)));
        max_size = max_size.max(size);
    }
    min_size.map(|min| (min, max_size))
}

pub(crate) fn size_color_for(entry_bytes: Option<u64>, min_max: Option<(u64, u64)>) -> Color {
    let shade_color = |t: f64| {
        let t = t.clamp(0.0, 1.0);
        let r = (255.0 * t).round() as u8;
        let g = 255u8;
        let b = (255.0 * t).round() as u8;
        Color::Rgb(r, g, b)
    };

    match (min_max, entry_bytes) {
        (Some((min_size, max_size)), Some(bytes)) if max_size > min_size => {
            let min_log = (min_size as f64 + 1.0).ln();
            let max_log = (max_size as f64 + 1.0).ln();
            let entry_log = (bytes as f64 + 1.0).ln();
            let t = ((entry_log - min_log) / (max_log - min_log)).clamp(0.0, 1.0);
            shade_color(t)
        }
        (Some(_), Some(_)) => shade_color(0.0),
        _ => Color::Green,
    }
}

pub(crate) fn date_rank_map_from_unix<I>(timestamps: I) -> HashMap<u64, f64>
where
    I: IntoIterator<Item = Option<u64>>,
{
    let mut values: Vec<u64> = timestamps.into_iter().flatten().collect();
    values.sort_unstable();
    values.dedup();

    if values.len() <= 1 {
        values.into_iter().map(|ts| (ts, 1.0)).collect()
    } else {
        let denom = (values.len() - 1) as f64;
        values
            .into_iter()
            .enumerate()
            .map(|(idx, ts)| (ts, idx as f64 / denom))
            .collect()
    }
}

pub(crate) fn date_color_for(modified_unix: Option<u64>, rank_map: &HashMap<u64, f64>) -> Color {
    let fade_color = |age_t: f64| {
        let age_t = age_t.clamp(0.0, 1.0);
        let base = (116.0, 178.0, 205.0);
        let white = (255.0, 255.0, 255.0);
        let r = (white.0 + (base.0 - white.0) * age_t).round() as u8;
        let g = (white.1 + (base.1 - white.1) * age_t).round() as u8;
        let b = (white.2 + (base.2 - white.2) * age_t).round() as u8;
        Color::Rgb(r, g, b)
    };

    modified_unix
        .and_then(|ts| rank_map.get(&ts).copied())
        .map(|rank_t| fade_color(1.0 - rank_t))
        .unwrap_or(Color::Rgb(116, 178, 205))
}