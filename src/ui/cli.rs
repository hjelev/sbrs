use chrono::{DateTime, Local};
use crossterm::style::{style, Attribute, Color as CtColor, Stylize};
use devicons::{icon_for_file, File as DevFile, Theme};
use std::{
    env, fs,
    io::{self},
    path::PathBuf,
    str::FromStr,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ui::icons::named_dir_icon;
use crate::{env_flag_true, App};

pub fn list_current_directory(
    include_hidden: bool,
    include_total_size: bool,
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

    struct ListRowData {
        entry: fs::DirEntry,
        path: PathBuf,
        meta: Option<fs::Metadata>,
        owner: String,
        group: String,
        total_display_bytes: Option<u64>,
    }

    fn rt_to_ct_color(color: ratatui::style::Color) -> CtColor {
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

    fn pad_to_display_width(s: &str, width: usize) -> String {
        let used = UnicodeWidthStr::width(s);
        if used >= width {
            return s.to_string();
        }
        format!("{}{}", s, " ".repeat(width - used))
    }

    let mut entries: Vec<_> = fs::read_dir(&current_dir)?
        .filter_map(|res| res.ok())
        .filter(|e| include_hidden || !e.file_name().to_string_lossy().starts_with('.'))
        .collect();

    entries.sort_by_key(|e| (e.path().is_file(), e.file_name()));

    let mut rows: Vec<ListRowData> = Vec::with_capacity(entries.len());
    let mut group_width = 1usize;
    let mut owner_width = 1usize;
    for entry in entries {
        let path = entry.path();
        let meta = entry.metadata().ok();
        let owner = meta
            .as_ref()
            .map(App::parse_owner)
            .unwrap_or_else(|| "-".to_string());
        let group = meta
            .as_ref()
            .map(App::parse_group)
            .unwrap_or_else(|| "-".to_string());

        group_width = group_width.max(group.chars().count());
        owner_width = owner_width.max(owner.chars().count());

        let total_display_bytes = if include_total_size {
            Some(App::compute_total_display_bytes(&path).unwrap_or(0))
        } else {
            None
        };

        rows.push(ListRowData {
            entry,
            path,
            meta,
            owner,
            group,
            total_display_bytes,
        });
    }

    group_width = group_width.min(16).max(1);
    owner_width = owner_width.min(20).max(1);

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
                .map(|row| row.total_display_bytes.unwrap_or(0))
                .fold(0u64, |acc, v| acc.saturating_add(v)),
        )
    } else {
        None
    };

    for row in rows {
        let path = row.path;
        let meta = row.meta;
        let owner = row.owner;
        let group = row.group;
        let entry_total_bytes = row.total_display_bytes;
        let entry = row.entry;
        let is_hidden = entry.file_name().to_string_lossy().starts_with('.');
        let is_symlink = entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false);
        let is_dir = path.is_dir();

        let icon_data = if nerd_font_active {
            Some(icon_for_file(&DevFile::new(&path), Some(Theme::Dark)))
        } else {
            None
        };

        let (icon_glyph, mut icon_color) = if !show_icons {
            (String::new(), CtColor::Reset)
        } else if nerd_font_active {
            if is_symlink {
                ("".to_string(), CtColor::Rgb { r: 100, g: 220, b: 220 })
            } else if is_dir {
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if let Some((glyph, (r, g, b))) = named_dir_icon(dir_name) {
                    (glyph.to_string(), CtColor::Rgb { r, g, b })
                } else {
                    ("\u{F07B}".to_string(), CtColor::Rgb { r: 100, g: 160, b: 240 })
                }
            } else if App::is_age_protected_file(&path) {
                ("".to_string(), CtColor::Rgb { r: 230, g: 190, b: 90 })
            } else {
                let icon = icon_data
                    .as_ref()
                    .map(|i| i.icon.to_string())
                    .unwrap_or_else(|| "?".to_string());
                let color = icon_data
                    .as_ref()
                    .and_then(|i| ratatui::style::Color::from_str(i.color).ok())
                    .map(rt_to_ct_color)
                    .unwrap_or(CtColor::White);
                (icon, color)
            }
        } else if is_dir {
            ("📁".to_string(), CtColor::Rgb { r: 100, g: 160, b: 240 })
        } else {
            ("📄".to_string(), CtColor::White)
        };

        let name = entry.file_name().to_string_lossy().into_owned();
        let mut name_color = if is_dir {
            CtColor::Rgb { r: 100, g: 160, b: 240 }
        } else if App::is_age_protected_file(&path) {
            CtColor::Rgb { r: 230, g: 190, b: 90 }
        } else {
            icon_data
                .as_ref()
                .and_then(|i| ratatui::style::Color::from_str(i.color).ok())
                .map(rt_to_ct_color)
                .unwrap_or(CtColor::White)
        };
        if is_symlink {
            name_color = CtColor::Rgb { r: 100, g: 220, b: 220 };
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !is_dir
                && meta
                    .as_ref()
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            {
                name_color = CtColor::Rgb { r: 120, g: 220, b: 120 };
            }
        }
        if no_color {
            name_color = CtColor::Reset;
            icon_color = CtColor::Reset;
        }

        let icon_prefix = if show_icons && !icon_glyph.is_empty() {
            format!("{} ", icon_glyph)
        } else {
            String::new()
        };
        let rendered_name =
            truncate_to_display_width(&format!("{}{}", icon_prefix, name), name_width);
        let rendered_name = pad_to_display_width(&rendered_name, name_width);

        let mut styled_name = style(rendered_name).with(name_color);
        if is_dir {
            styled_name = styled_name.attribute(Attribute::Bold);
        }
        if is_hidden {
            styled_name = styled_name.attribute(Attribute::Dim);
        }

        let styled_icon = style(format!("{}", icon_glyph)).with(icon_color);

        if show_meta || show_size || show_date {
            let perms = meta
                .as_ref()
                .map(App::parse_permissions)
                .unwrap_or_else(|| "----------".to_string());
            let owner = meta
                .as_ref()
                .map(|_| owner.clone())
                .unwrap_or_else(|| "-".to_string());
            let group = meta
                .as_ref()
                .map(|_| group.clone())
                .unwrap_or_else(|| "-".to_string());
            let perms_col = format!("{:<width$}", truncate_to(&perms, perms_width), width = perms_width);
            let group_col = format!("{:>width$}", truncate_to(&group, group_width), width = group_width);
            let owner_col = format!("{:<width$}", truncate_to(&owner, owner_width), width = owner_width);

            let size = if include_total_size {
                App::format_size(entry_total_bytes.unwrap_or(0))
            } else {
                meta.as_ref()
                    .map(|m| if m.is_dir() { "-".to_string() } else { App::format_size(m.len()) })
                    .unwrap_or_else(|| "-".to_string())
            };
            let size_col = format!("{:>width$}", size, width = size_width);

            let pct_col = if show_pct {
                match (total_listing_display_bytes, entry_total_bytes) {
                    (Some(total), Some(entry_bytes)) if total > 0 => {
                        let pct = (entry_bytes as f64 * 100.0) / (total as f64);
                        format!("{:>5.0}%", pct)
                    }
                    _ => format!("{:>width$}", "-", width = pct_width),
                }
            } else {
                String::new()
            };

            let date = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|t| DateTime::<Local>::from(t).format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "-".to_string());
            let date_col = format!("{:<width$}", truncate_to(&date, date_width), width = date_width);

            print!("{}", styled_name);
            if show_meta {
                print!(
                    " {} {} {}",
                    style(perms_col).with(CtColor::Rgb { r: 180, g: 150, b: 100 }),
                    style(group_col).with(CtColor::Rgb { r: 180, g: 150, b: 100 }),
                    style(owner_col).with(CtColor::Rgb { r: 180, g: 150, b: 100 })
                );
            }
            if show_size {
                print!(" {}", style(size_col).with(CtColor::Green));
            }
            if show_pct {
                print!(" {}", style(pct_col).with(CtColor::Rgb { r: 220, g: 200, b: 120 }));
            }
            if show_date {
                print!(" {}", style(date_col).with(CtColor::Rgb { r: 120, g: 190, b: 210 }));
            }
            println!();
        } else {
            println!("{} {}", styled_icon, styled_name);
        }
    }

    Ok(())
}

pub fn parse_list_mode_args<'a>(args: &'a [String]) -> Option<(bool, bool, Option<&'a str>)> {
    let mut list_mode_seen = false;
    let mut include_hidden = false;
    let mut include_total_size = false;
    let mut list_path: Option<&str> = None;

    for arg in args {
        match arg.as_str() {
            "-l" => {
                list_mode_seen = true;
            }
            "-la" => {
                list_mode_seen = true;
                include_hidden = true;
            }
            "--total-size" => {
                include_total_size = true;
            }
            other if !other.starts_with('-') && list_path.is_none() => {
                list_path = Some(other);
            }
            _ => {}
        }
    }

    if list_mode_seen {
        Some((include_hidden, include_total_size, list_path))
    } else {
        None
    }
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
    println!("  -l [PATH]      List folder and exit");
    println!("  -la [PATH]     List folder including hidden files and exit");
    println!("  --total-size   With -l/-la: recursive size + percent columns");
    println!("  -h, --help     Show this help message");
    println!("  -V, --version  Show app name and current version");
}
