pub fn format_eta(total_seconds: u64) -> String {
    let mins = total_seconds / 60;
    let secs = total_seconds % 60;
    if mins > 0 {
        format!("{}m{:02}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}

pub fn format_size(bytes: u64) -> String {
    let units = ["B", "K", "M", "G", "T"];
    let mut size = bytes as f64;
    let mut unit_idx = 0usize;
    while size >= 1024.0 && unit_idx < units.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{}{}", bytes, units[unit_idx])
    } else if size >= 10.0 {
        format!("{:.0}{}", size, units[unit_idx])
    } else {
        format!("{:.1}{}", size, units[unit_idx])
    }
}
