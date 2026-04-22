use std::{collections::HashMap, fs, str::FromStr, time::UNIX_EPOCH};

use chrono::{DateTime, Local};
use devicons::{icon_for_file, File as DevFile, Theme};
use ratatui::prelude::*;

use crate::{ui, App, EntryRenderCache, EntryRenderConfig};

impl App {
    pub(crate) fn build_entry_render_cache(
        entry: &fs::DirEntry,
        config: EntryRenderConfig,
        uid_cache: &HashMap<u32, String>,
        gid_cache: &HashMap<u32, String>,
    ) -> EntryRenderCache {
        let path = entry.path();
        let meta = entry.metadata().ok();
        let is_hidden = entry.file_name().to_string_lossy().starts_with('.');
        let is_symlink = entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false);
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let icon_data = if config.nerd_font_active {
            Some(icon_for_file(&DevFile::new(&path), Some(Theme::Dark)))
        } else {
            None
        };

        let (icon_glyph, icon_style) = if !config.show_icons {
            (String::new(), Style::default())
        } else if config.nerd_font_active {
            if is_symlink {
                ("".to_string(), Style::default().fg(Color::Rgb(100, 220, 220)))
            } else if is_dir {
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let dir_style = Style::default()
                    .fg(Color::Rgb(100, 160, 240))
                    .add_modifier(Modifier::BOLD);
                if let Some((glyph, _)) = ui::icons::named_dir_icon(dir_name) {
                    (glyph.to_string(), dir_style)
                } else {
                    ("\u{F07B}".to_string(), dir_style)
                }
            } else if Self::is_age_protected_file(&path) {
                ("".to_string(), Style::default().fg(Color::Rgb(230, 190, 90)))
            } else {
                let icon = icon_data
                    .as_ref()
                    .map(|i| i.icon.to_string())
                    .unwrap_or_else(|| "?".to_string());
                let color = icon_data
                    .as_ref()
                    .and_then(|i| Color::from_str(i.color).ok())
                    .unwrap_or(Color::White);
                (icon, Style::default().fg(color))
            }
        } else if is_dir {
            (
                "📁".to_string(),
                Style::default()
                    .fg(Color::Rgb(100, 160, 240))
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("📄".to_string(), Style::default().fg(Color::White))
        };

        let mut name_style = if is_dir {
            Style::default()
                .fg(Color::Rgb(100, 160, 240))
                .add_modifier(Modifier::BOLD)
        } else if Self::is_age_protected_file(&path) {
            Style::default().fg(Color::Rgb(230, 190, 90))
        } else {
            let file_color = icon_data
                .as_ref()
                .and_then(|i| Color::from_str(i.color).ok())
                .unwrap_or(Color::White);
            Style::default().fg(file_color)
        };

        if is_symlink {
            name_style = Style::default().fg(Color::Rgb(100, 220, 220));
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
                name_style = Style::default().fg(Color::Rgb(120, 220, 120));
            }
        }

        if is_hidden {
            name_style = name_style.add_modifier(Modifier::DIM);
        }

        let perms_width = 11usize;
        let size_width = 6usize;
        let date_width = 16usize;
        let perms = meta
            .as_ref()
            .map(App::parse_permissions)
            .unwrap_or_else(|| "----------".to_string());
        let owner = meta
            .as_ref()
            .map(|m| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    let uid = m.uid();
                    uid_cache
                        .get(&uid)
                        .cloned()
                        .unwrap_or_else(|| uid.to_string())
                }
                #[cfg(not(unix))]
                {
                    "-".to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string());
        let group = meta
            .as_ref()
            .map(|m| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    let gid = m.gid();
                    gid_cache
                        .get(&gid)
                        .cloned()
                        .unwrap_or_else(|| gid.to_string())
                }
                #[cfg(not(unix))]
                {
                    "-".to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string());
        let perms_col = format!("{:<width$}", perms, width = perms_width);
        let size_bytes = meta
            .as_ref()
            .and_then(|m| if m.is_dir() { None } else { Some(Self::display_leaf_size(m)) });
        let size = size_bytes
            .map(App::format_size)
            .unwrap_or_else(|| "-".to_string());
        let size_col = format!("{:>width$}", size, width = size_width);
        let date = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(|t| DateTime::<Local>::from(t).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let date_col = format!("{:>width$}", date, width = date_width);
        let modified_unix = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        EntryRenderCache {
            raw_name: entry.file_name().to_string_lossy().into_owned(),
            icon_glyph,
            icon_style,
            name_style,
            perms_col,
            group_name: group,
            owner_name: owner,
            size_col,
            size_bytes,
            date_col,
            modified_unix,
        }
    }

    pub(crate) fn refresh_meta_identity_widths(&mut self) {
        let mut group_w = 1usize;
        let mut owner_w = 1usize;
        for entry in &self.entry_render_cache {
            group_w = group_w.max(entry.group_name.chars().count());
            owner_w = owner_w.max(entry.owner_name.chars().count());
        }
        self.meta_group_width = group_w.min(16);
        self.meta_owner_width = owner_w.min(20);
    }
}
