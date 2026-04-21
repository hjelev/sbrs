use std::{collections::HashMap, fs};

use crate::App;

impl App {
    pub(crate) fn parse_permissions(meta: &fs::Metadata) -> String {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = meta.permissions().mode();
            let mut p = String::with_capacity(10);
            p.push(if meta.is_dir() { 'd' } else { '-' });
            let chars = ['r', 'w', 'x'];
            for i in (0..9).rev() {
                if mode & (1 << i) != 0 {
                    p.push(chars[2 - (i % 3)]);
                } else {
                    p.push('-');
                }
            }
            p
        }
        #[cfg(not(unix))]
        {
            "----------".to_string()
        }
    }

    pub(crate) fn parse_owner(meta: &fs::Metadata) -> String {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let uid = meta.uid();
            users::get_user_by_uid(uid)
                .map(|user| user.name().to_string_lossy().into_owned())
                .unwrap_or_else(|| uid.to_string())
        }
        #[cfg(not(unix))]
        {
            "-".to_string()
        }
    }

    pub(crate) fn parse_group(meta: &fs::Metadata) -> String {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let gid = meta.gid();
            users::get_group_by_gid(gid)
                .map(|group| group.name().to_string_lossy().into_owned())
                .unwrap_or_else(|| gid.to_string())
        }
        #[cfg(not(unix))]
        {
            "-".to_string()
        }
    }

    pub(crate) fn build_uid_cache(entries: &[fs::DirEntry]) -> HashMap<u32, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let mut map: HashMap<u32, String> = HashMap::new();
            for entry in entries {
                if let Ok(meta) = entry.metadata() {
                    let uid = meta.uid();
                    map.entry(uid).or_insert_with(|| {
                        users::get_user_by_uid(uid)
                            .map(|u| u.name().to_string_lossy().into_owned())
                            .unwrap_or_else(|| uid.to_string())
                    });
                }
            }
            map
        }
        #[cfg(not(unix))]
        {
            HashMap::new()
        }
    }

    pub(crate) fn build_gid_cache(entries: &[fs::DirEntry]) -> HashMap<u32, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let mut map: HashMap<u32, String> = HashMap::new();
            for entry in entries {
                if let Ok(meta) = entry.metadata() {
                    let gid = meta.gid();
                    map.entry(gid).or_insert_with(|| {
                        users::get_group_by_gid(gid)
                            .map(|g| g.name().to_string_lossy().into_owned())
                            .unwrap_or_else(|| gid.to_string())
                    });
                }
            }
            map
        }
        #[cfg(not(unix))]
        {
            HashMap::new()
        }
    }

    pub(crate) fn format_size(bytes: u64) -> String {
        crate::util::format::format_size(bytes)
    }
}
