use crossterm::style::{style, Attribute, Color as CtColor, Stylize};
use ratatui::style::Modifier;
use std::{
    collections::HashMap,
    env, fs,
    io::{self},
    path::PathBuf,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app_render_cache::{EntryRenderCache, EntryRenderConfig};
use crate::ui::list_render;
use crate::ui::list_temperature;
use crate::{env_flag_true, App};

pub(crate) fn rt_to_ct_color(color: ratatui::style::Color) -> CtColor {
    match color {
        ratatui::style::Color::Black => CtColor::Black,
        ratatui::style::Color::Red => CtColor::Red,
        ratatui::style::Color::Green => CtColor::Green,
        ratatui::style::Color::Yellow => CtColor::Yellow,
        ratatui::style::Color::Blue => CtColor::Blue,
        ratatui::style::Color::Magenta => CtColor::Magenta,
        ratatui::style::Color::Cyan => CtColor::Cyan,
        ratatui::style::Color::Gray => CtColor::Grey,
        ratatui::style::Color::DarkGray => CtColor::DarkGrey,
        ratatui::style::Color::LightRed => CtColor::DarkRed,
        ratatui::style::Color::LightGreen => CtColor::DarkGreen,
        ratatui::style::Color::LightYellow => CtColor::DarkYellow,
        ratatui::style::Color::LightBlue => CtColor::DarkBlue,
        ratatui::style::Color::LightMagenta => CtColor::DarkMagenta,
        ratatui::style::Color::LightCyan => CtColor::DarkCyan,
        ratatui::style::Color::White => CtColor::White,
        ratatui::style::Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
        ratatui::style::Color::Indexed(i) => CtColor::AnsiValue(i),
        ratatui::style::Color::Reset => CtColor::Reset,
    }
}

pub fn list_current_directory(
    include_hidden: bool,
    include_total_size: bool,
    tree_depth: Option<usize>,
    path: Option<&str>,
) -> io::Result<()> {
    let current_dir = if let Some(p) = path {
        PathBuf::from(p)
    } else {
        env::current_dir()?
    };
    let nerd_font_active = env::var("NERD_FONT_ACTIVE").map(|v| v == "1").unwrap_or(false);
    let no_color = env_flag_true(&["NO_COLOR"]);
    let show_icons = env::var("TERMINAL_ICONS").map(|v| v != "0").unwrap_or(true);
    let term_w = crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(120);
    let show_date = term_w >= 90;
    let show_size = term_w >= 70 || include_total_size;
    let show_pct = include_total_size && show_size;
    let show_meta = term_w >= 50;

    let date_width = 16usize;
    let size_width = 6usize;
    let pct_width = 6usize;
    let perms_width = 11usize;

    fn truncate_to(s: &str, max: usize) -> String {
        if s.chars().count() <= max {
            return s.to_string();
        }
        if max <= 1 {
            return "…".to_string();
        }
        let mut out = String::new();
        for ch in s.chars().take(max - 1) {
            out.push(ch);
        }
        out.push('…');
        out
    }

    fn truncate_to_display_width(s: &str, max: usize) -> String {
        if UnicodeWidthStr::width(s) <= max {
            return s.to_string();
        }
        if max <= 1 {
            return "…".to_string();
        }
        let mut out = String::new();
        let mut used = 0usize;
        let target = max - 1;
        for ch in s.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + ch_width > target {
                break;
            }
            out.push(ch);
            used += ch_width;
        }
        out.push('…');
        out
    }

    let tree_rows = tree_depth.map(|depth| {
        crate::ui::tree::collect_tree_rows(
            &current_dir,
            include_hidden,
            if depth == 0 { None } else { Some(depth) },
            crate::SortMode::NameAsc,
            None,
        )
    }).transpose()?;

    let mut direct_entries: Vec<fs::DirEntry> = Vec::new();
    if tree_rows.is_none() {
        direct_entries = fs::read_dir(&current_dir)?
            .filter_map(|res| res.ok())
            .filter(|e| include_hidden || !e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        direct_entries.sort_by_key(|e| (e.path().is_file(), e.file_name()));
    }

    let entries: Vec<&fs::DirEntry> = if let Some(rows) = tree_rows.as_ref() {
        rows.iter().map(|row| &row.entry).collect::<Vec<_>>()
    } else {
        direct_entries.iter().collect::<Vec<_>>()
    };

    let config = EntryRenderConfig { nerd_font_active, show_icons };
    let uid_cache = App::build_uid_cache_refs(&entries);
    let gid_cache = App::build_gid_cache_refs(&entries);

    struct RowData {
        path: PathBuf,
        tree_prefix: String,
        cache: EntryRenderCache,
        entry_total_bytes: Option<u64>,
    }

    let mut rows: Vec<RowData> = Vec::with_capacity(entries.len());
    let mut group_width = 1usize;
    let mut owner_width = 1usize;
    for (idx, entry) in entries.iter().enumerate() {
        let path = entry.path();
        let cache = App::build_entry_render_cache(entry, config, &uid_cache, &gid_cache);
        group_width = group_width.max(cache.group_name.chars().count());
        owner_width = owner_width.max(cache.owner_name.chars().count());
        let tree_prefix = tree_rows
            .as_ref()
            .map(|rows| rows[idx].prefix.clone())
            .unwrap_or_default();
        rows.push(RowData { path, tree_prefix, cache, entry_total_bytes: None });
    }

    group_width = group_width.min(16).max(1);
    owner_width = owner_width.min(20).max(1);

    // Override size columns for all entries when include_total_size=true
    if include_total_size {
        for row in &mut rows {
            let total = App::compute_total_display_bytes(&row.path).unwrap_or(0);
            row.entry_total_bytes = Some(total);
            row.cache.size_col = format!("{:>width$}", App::format_size(total), width = size_width);
            row.cache.size_bytes = Some(total);
        }
    }

    let mut reserved = 0usize;
    if show_meta {
        reserved += perms_width + group_width + owner_width + 3;
    }
    if show_size {
        reserved += size_width + 1;
    }
    if show_pct {
        reserved += pct_width + 1;
    }
    if show_date {
        reserved += date_width + 1;
    }
    let name_width = term_w.saturating_sub(reserved).max(20);

    let total_listing_display_bytes = if include_total_size {
        Some(
            rows.iter()
                .map(|row| row.cache.size_bytes.unwrap_or(0))
                .fold(0u64, |acc, v| acc.saturating_add(v)),
        )
    } else {
        None
    };

    let size_min_max = if show_size {
        list_temperature::size_min_max_from_sizes(rows.iter().map(|row| row.cache.size_bytes))
    } else {
        None
    };
    let date_rank_by_ts: HashMap<u64, f64> = if show_date {
        list_temperature::date_rank_map_from_unix(rows.iter().map(|row| row.cache.modified_unix))
    } else {
        HashMap::new()
    };

    for row in rows {
        let tree_prefix = row.tree_prefix;
        let cache = row.cache;

        // Derive crossterm colours from the ratatui styles stored in the cache
        let mut icon_color = cache.icon_style.fg.map(rt_to_ct_color).unwrap_or(CtColor::Reset);
        let mut name_color = cache.name_style.fg.map(rt_to_ct_color).unwrap_or(CtColor::White);
        if no_color {
            name_color = CtColor::Reset;
            icon_color = CtColor::Reset;
        }

        let tree_color = if no_color {
            CtColor::Reset
        } else {
            CtColor::Rgb { r: 140, g: 140, b: 140 }
        };

        let icon_prefix = if show_icons && !cache.icon_glyph.is_empty() {
            format!("{} ", cache.icon_glyph)
        } else {
            String::new()
        };
        let prefix_width = UnicodeWidthStr::width(tree_prefix.as_str());
        let icon_width = UnicodeWidthStr::width(icon_prefix.as_str());
        let base_name_width = name_width.saturating_sub(prefix_width + icon_width).max(1);
        let rendered_name = truncate_to_display_width(&cache.raw_name, base_name_width);
        let rendered_name_width = UnicodeWidthStr::width(rendered_name.as_str());
        let used_name_width = prefix_width + icon_width + rendered_name_width;
        let trailing_pad = " ".repeat(name_width.saturating_sub(used_name_width));

        let mut styled_name = style(rendered_name).with(name_color);
        if cache.name_style.add_modifier.contains(Modifier::BOLD) {
            styled_name = styled_name.attribute(Attribute::Bold);
        }
        if cache.name_style.add_modifier.contains(Modifier::DIM) {
            styled_name = styled_name.attribute(Attribute::Dim);
        }

        let styled_icon = style(cache.icon_glyph.clone()).with(icon_color);
        let styled_tree = style(tree_prefix.as_str()).with(tree_color);

        if show_meta || show_size || show_date {
            // perms_col is already left-padded to 11 chars by the cache builder
            let perms_col = cache.perms_col.trim_end();
            let group_col = format!(
                "{:>width$}",
                truncate_to(&cache.group_name, group_width),
                width = group_width
            );
            let owner_col = format!(
                "{:<width$}",
                truncate_to(&cache.owner_name, owner_width),
                width = owner_width
            );
            // size_col pre-padded to 6 chars; date_col pre-padded to 16 chars by the cache builder
            let size_col = &cache.size_col;
            let date_col = &cache.date_col;

            let pct_col = if show_pct {
                match (total_listing_display_bytes, cache.size_bytes) {
                    (Some(total), Some(entry_bytes)) if total > 0 => {
                        let pct = (entry_bytes as f64 * 100.0) / (total as f64);
                        format!("{:>5.0}%", pct)
                    }
                    _ => format!("{:>width$}", "-", width = pct_width),
                }
            } else {
                String::new()
            };
            let size_color = if no_color {
                CtColor::Reset
            } else {
                rt_to_ct_color(list_temperature::size_color_for(cache.size_bytes, size_min_max))
            };
            let date_color = if no_color {
                CtColor::Reset
            } else {
                rt_to_ct_color(list_temperature::date_color_for(
                    cache.modified_unix,
                    &date_rank_by_ts,
                ))
            };
            let pct_color = size_color;

            print!("{}", styled_tree);
            if show_icons && !cache.icon_glyph.is_empty() {
                print!("{}", styled_icon);
                print!(" ");
            }
            print!("{}", styled_name);
            print!("{}", trailing_pad);
            if show_meta {
                let perms_segments = list_render::permission_gradient_segments(perms_col, perms_width);
                print!(
                    " "
                );
                for (text, color) in perms_segments {
                    let seg = match (no_color, color) {
                        (false, Some(c)) => style(text).with(rt_to_ct_color(c)),
                        _ => style(text),
                    };
                    print!("{}", seg);
                }
                print!(
                    " {} {}",
                    style(group_col.as_str()).with(CtColor::Rgb { r: 180, g: 150, b: 100 }),
                    style(owner_col.as_str()).with(CtColor::Rgb { r: 180, g: 150, b: 100 })
                );
            }
            if show_size {
                print!(" {}", style(size_col.as_str()).with(size_color));
            }
            if show_pct {
                print!(" {}", style(pct_col.as_str()).with(pct_color));
            }
            if show_date {
                print!(" {}", style(date_col.as_str()).with(date_color));
            }
            println!();
        } else {
            print!("{}", styled_tree);
            if show_icons && !cache.icon_glyph.is_empty() {
                print!("{} ", styled_icon);
            }
            println!("{}{}", styled_name, trailing_pad);
        }
    }

    Ok(())
}

pub(crate) struct ListModeArgs<'a> {
    pub(crate) include_hidden: bool,
    pub(crate) include_total_size: bool,
    pub(crate) tree_depth: Option<usize>,
    pub(crate) path: Option<&'a str>,
}

pub fn parse_list_mode_args<'a>(args: &'a [String]) -> Option<ListModeArgs<'a>> {
    let mut list_mode_seen = false;
    let mut include_hidden = false;
    let mut include_total_size = false;
    let mut list_path: Option<&str> = None;
    let mut tree_depth: Option<usize> = None;

    for arg in args {
        match arg.as_str() {
            "-l" => {
                list_mode_seen = true;
            }
            "-t" => {
                list_mode_seen = true;
                tree_depth = Some(0);
            }
            "-a" => {
                list_mode_seen = true;
                include_hidden = true;
            }
            "-la" => {
                list_mode_seen = true;
                include_hidden = true;
            }
            "--total-size" => {
                include_total_size = true;
            }
            other if other.starts_with("-l") && other.len() > 2 && other[2..].chars().all(|c| c.is_ascii_digit()) => {
                list_mode_seen = true;
                tree_depth = other[2..].parse::<usize>().ok();
            }
            other if other.chars().all(|c| c.is_ascii_digit()) && list_mode_seen && tree_depth.is_none() => {
                tree_depth = other.parse::<usize>().ok();
            }
            other if !other.starts_with('-') && list_path.is_none() => {
                list_path = Some(other);
            }
            _ => {}
        }
    }

    if list_mode_seen {
        Some(ListModeArgs {
            include_hidden,
            include_total_size,
            tree_depth,
            path: list_path,
        })
    } else {
        None
    }
}

pub fn validate_cli_args(args: &[String]) -> Result<(), String> {
    let mut positional_count = 0usize;

    for arg in args {
        match arg.as_str() {
            "-h" | "--help" | "-V" | "--version" | "-l" | "-a" | "-la" | "-e" | "-t" | "--total-size" => {}
            other if other.starts_with("-l") && other.len() > 2 && other[2..].chars().all(|c| c.is_ascii_digit()) => {}
            other if other.chars().all(|c| c.is_ascii_digit()) => {}
            other if other.starts_with('-') => {
                return Err(format!("unrecognized option: {}", other));
            }
            _ => {
                positional_count += 1;
                if positional_count > 1 {
                    return Err("too many positional arguments".to_string());
                }
            }
        }
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectFileMode {
    ViewNoPager,
    ViewWithPager,
    Edit,
}

pub fn parse_direct_file_mode_args<'a>(args: &'a [String]) -> Option<(DirectFileMode, &'a str)> {
    let mut edit_mode = false;
    let mut pager_mode = false;
    let mut has_hidden_list_mode = false;
    let mut has_unknown_flag = false;
    let mut path: Option<&str> = None;

    for arg in args {
        match arg.as_str() {
            "-e" => {
                edit_mode = true;
            }
            "-l" => {
                pager_mode = true;
            }
            "-a" => {
                has_hidden_list_mode = true;
            }
            "-la" => {
                has_hidden_list_mode = true;
            }
            "--total-size" => {}
            other if !other.starts_with('-') && path.is_none() => {
                path = Some(other);
            }
            other if other.starts_with('-') => {
                has_unknown_flag = true;
            }
            _ => {}
        }
    }

    let path = path?;

    // Keep -a/-la reserved for list mode semantics.
    if has_hidden_list_mode || has_unknown_flag {
        return None;
    }

    let mode = if edit_mode {
        DirectFileMode::Edit
    } else if pager_mode {
        DirectFileMode::ViewWithPager
    } else {
        DirectFileMode::ViewNoPager
    };

    Some((mode, path))
}

pub fn print_version() {
    let name = "Shell Buddy (sb)";
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "{} {}",
        style(name).attribute(Attribute::Bold),
        style(format!("v{}", version))
    );
}

pub fn print_help() {
    let logo = [
        " ┌─┐┬ ┬┌─┐┬  ┬    ┌┐ ┬ ┬┌┬┐┌┬┐┬ ┬",
        " └─┐├─┤├┤ │  │    ├┴┐│ │ ││ ││└┬┘",
        " └─┘┴ ┴└─┘┴─┘┴─┘  └─┘└─┘─┴┘─┴┘ ┴",
    ];

    for (i, line) in logo.iter().enumerate() {
        let color = match i {
            0 => CtColor::Rgb {
                r: 125,
                g: 205,
                b: 255,
            },
            1 => CtColor::Rgb {
                r: 110,
                g: 190,
                b: 245,
            },
            _ => CtColor::Rgb {
                r: 95,
                g: 175,
                b: 235,
            },
        };
        println!("{}", style(*line).with(color).attribute(Attribute::Bold));
    }

    println!(
        "{}",
        style("Bringing your tools together")
            .with(CtColor::Rgb {
                r: 185,
                g: 185,
                b: 185
            })
            .attribute(Attribute::Italic)
    );
    println!();

    println!(
        "{}",
        style("Usage:")
            .with(CtColor::Rgb {
                r: 125,
                g: 205,
                b: 255
            })
            .attribute(Attribute::Bold)
    );
    println!("  sb [OPTIONS]");
    println!();
    println!(
        "{}",
        style("Options:")
            .with(CtColor::Rgb {
                r: 125,
                g: 205,
                b: 255
            })
            .attribute(Attribute::Bold)
    );
    println!("  -l [PATH]      List folder and exit; with FILE uses pager mode");
    println!("  -l2 / -l 2     Tree list mode limited to depth 2");
    println!("  -t [PATH]      Unlimited recursive tree list mode");
    println!("  -a [PATH]      List folder including hidden files and exit");
    println!("  -la [PATH]     Same as -a");
    println!("  -e [FILE]      Open file in $EDITOR (fallback: nano)");
    println!("  --total-size   With -l/-a/-la/-t: recursive size + percent columns");
    println!("  -h, --help     Show this help message");
    println!("  -V, --version  Show app name and current version");
}
