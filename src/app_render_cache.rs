use std::{collections::HashMap, fs, path::Path, str::FromStr, time::UNIX_EPOCH};

use crate::util::format::format_mtime;
use devicons::{icon_for_file, File as DevFile, Theme};
use ratatui::prelude::*;
use crate::ui::icons::named_file_icon;
use crate::{ui, App};

#[derive(Clone)]
pub(crate) struct EntryRenderCache {
    pub(crate) raw_name: String,
    pub(crate) icon_glyph: String,
    pub(crate) icon_style: Style,
    pub(crate) name_style: Style,
    pub(crate) perms_col: String,
    pub(crate) group_name: String,
    pub(crate) owner_name: String,
    pub(crate) size_col: String,
    pub(crate) size_bytes: Option<u64>,
    pub(crate) date_col: String,
    pub(crate) modified_unix: Option<u64>,
}

#[derive(Clone, Copy)]
pub(crate) struct EntryRenderConfig {
    pub(crate) nerd_font_active: bool,
    pub(crate) show_icons: bool,
}

impl App {
    pub(crate) fn icon_for_name(name: &str, is_dir: bool, show_icons: bool, nerd_font_active: bool, is_symlink: bool) -> (String, Style) {
        if !show_icons {
            return (String::new(), Style::default());
        }

        if is_symlink {
            return ("\u{f1177}".to_string(), Style::default().fg(Color::Rgb(100, 220, 220)));
        }

        if nerd_font_active {
            if is_dir {
                let dir_style = Style::default()
                    .fg(Color::Rgb(100, 160, 240))
                    .add_modifier(Modifier::BOLD);
                if let Some((glyph, _)) = ui::icons::named_dir_icon(name) {
                    (glyph.to_string(), dir_style)
                } else {
                    ("\u{f024b}".to_string(), dir_style)
                }
            } else if name.trim().is_empty()
                || Path::new(name)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.is_empty())
                    .unwrap_or(true)
            {
                // Draft/partial names in interactive prompts (e.g. empty line, '/')
                // can lack a valid filename component; avoid calling devicons in that case.
                ("\u{f15b}".to_string(), Style::default().fg(Color::White))
            } else if Path::new(name)
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("age"))
                .unwrap_or(false)
            {
                ("".to_string(), Style::default().fg(Color::Rgb(230, 190, 90)))
            } else if let Some((custom_icon, (r, g, b))) = named_file_icon(name) {
                (custom_icon.to_string(), Style::default().fg(Color::Rgb(r, g, b)))
            } else {
                let data = icon_for_file(&DevFile::new(Path::new(name)), Some(Theme::Dark));
                let color = Color::from_str(data.color).unwrap_or(Color::White);
                (data.icon.to_string(), Style::default().fg(color))
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
        }
    }

    pub(crate) fn icon_for_path(path: &Path, show_icons: bool, nerd_font_active: bool, is_symlink: bool) -> (String, Style) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        Self::icon_for_name(name, path.is_dir(), show_icons, nerd_font_active, is_symlink)
    }

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
        // icon_data is still needed for name_style color on regular nerd-font files.
        let icon_data = if config.nerd_font_active && !is_symlink && !is_dir {
            Some(icon_for_file(&DevFile::new(&path), Some(Theme::Dark)))
        } else {
            None
        };

        let (icon_glyph, icon_style) = Self::icon_for_path(&path, config.show_icons, config.nerd_font_active, is_symlink);

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
            .map(format_mtime)
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
