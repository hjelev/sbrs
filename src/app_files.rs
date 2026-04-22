use std::{
    env, fs,
    io::{self, Read},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{App, ArchiveKind, ZIP_BASED_EXTENSIONS};

impl App {
    pub(crate) fn is_supported_archive(path: &PathBuf) -> bool {
        let lower_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();

        let tar_like = lower_name.ends_with(".tar")
            || lower_name.ends_with(".tar.gz")
            || lower_name.ends_with(".tgz")
            || lower_name.ends_with(".tar.bz2")
            || lower_name.ends_with(".tbz")
            || lower_name.ends_with(".tbz2")
            || lower_name.ends_with(".tar.xz")
            || lower_name.ends_with(".txz")
            || lower_name.ends_with(".tar.zst")
            || lower_name.ends_with(".tzst");

        let seven_zip = lower_name.ends_with(".7z");
        let rar = lower_name.ends_with(".rar");

        let ext_supported = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ZIP_BASED_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false);

        ext_supported || tar_like || seven_zip || rar || Self::has_zip_signature(path)
    }

    pub(crate) fn is_fuse_zip_archive(path: &PathBuf) -> bool {
        matches!(Self::archive_kind(path), Some(ArchiveKind::Zip))
    }

    pub(crate) fn is_archivemount_archive(path: &PathBuf) -> bool {
        matches!(Self::archive_kind(path), Some(ArchiveKind::Tar) | Some(ArchiveKind::Zip))
    }

    pub(crate) fn archive_kind(path: &PathBuf) -> Option<ArchiveKind> {
        let lower_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();

        let is_zip = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ZIP_BASED_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
            || Self::has_zip_signature(path);
        if is_zip {
            return Some(ArchiveKind::Zip);
        }

        if lower_name.ends_with(".tar")
            || lower_name.ends_with(".tar.gz")
            || lower_name.ends_with(".tgz")
            || lower_name.ends_with(".tar.bz2")
            || lower_name.ends_with(".tbz")
            || lower_name.ends_with(".tbz2")
            || lower_name.ends_with(".tar.xz")
            || lower_name.ends_with(".txz")
            || lower_name.ends_with(".tar.zst")
            || lower_name.ends_with(".tzst")
        {
            return Some(ArchiveKind::Tar);
        }
        if lower_name.ends_with(".7z") {
            return Some(ArchiveKind::SevenZip);
        }
        if lower_name.ends_with(".rar") {
            return Some(ArchiveKind::Rar);
        }
        None
    }

    pub(crate) fn is_image_file(path: &PathBuf) -> bool {
        const IMAGE_EXTENSIONS: &[&str] = &[
            "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "avif", "heic", "ico",
        ];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| IMAGE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    pub(crate) fn is_audio_file(path: &PathBuf) -> bool {
        const AUDIO_EXTENSIONS: &[&str] = &[
            "mp3", "flac", "wav", "ogg", "opus", "m4a", "aac", "wma", "aiff", "aif", "alac", "mid", "midi",
        ];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| AUDIO_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    pub(crate) fn is_json_file(path: &PathBuf) -> bool {
        const JSON_EXTENSIONS: &[&str] = &["json", "jsonc", "jsonl", "ndjson", "geojson"];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| JSON_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    pub(crate) fn is_markdown_file(path: &PathBuf) -> bool {
        const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd", "mkdn"];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| MARKDOWN_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    pub(crate) fn is_html_file(path: &PathBuf) -> bool {
        const HTML_EXTENSIONS: &[&str] = &["html", "htm", "xhtml"];
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| HTML_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    pub(crate) fn is_mermaid_file(path: &PathBuf) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("mmd"))
            .unwrap_or(false)
    }

    pub(crate) fn is_pdf_file(path: &PathBuf) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
    }

    pub(crate) fn is_cast_file(path: &PathBuf) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("cast"))
            .unwrap_or(false)
    }

    pub(crate) fn is_age_protected_file(path: &PathBuf) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("age"))
            .unwrap_or(false)
    }

    pub(crate) fn age_protected_output_path(path: &PathBuf) -> PathBuf {
        PathBuf::from(format!("{}.age", path.to_string_lossy()))
    }

    pub(crate) fn age_plain_output_path(path: &PathBuf) -> PathBuf {
        let mut out = path.clone();
        out.set_extension("");
        if out == *path {
            path.with_extension("decrypted")
        } else {
            out
        }
    }

    pub(crate) fn age_temp_decrypt_paths(path: &PathBuf, purpose: &str) -> io::Result<(PathBuf, PathBuf)> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp_dir = env::temp_dir().join(format!(
            "sbrs_age_{}_{}_{}",
            purpose,
            std::process::id(),
            stamp
        ));
        fs::create_dir_all(&tmp_dir)?;

        let plain_name = Self::age_plain_output_path(path)
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_else(|| "decrypted.bin".into());
        let tmp_path = tmp_dir.join(plain_name);
        Ok((tmp_dir, tmp_path))
    }

    pub(crate) fn is_delimited_text_file(path: &PathBuf) -> bool {
        const DELIMITED_EXTENSIONS: &[&str] = &["csv", "tsv", "tab", "psv", "dsv", "ssv"];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| DELIMITED_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            .unwrap_or(false)
    }

    pub(crate) fn is_binary_file(path: &PathBuf) -> bool {
        let Ok(mut file) = fs::File::open(path) else {
            return false;
        };
        let mut buf = [0u8; 8192];
        let Ok(n) = file.read(&mut buf) else {
            return false;
        };
        buf[..n].contains(&0u8)
    }

    pub(crate) fn has_zip_signature(path: &PathBuf) -> bool {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(_) => return false,
        };

        let mut magic = [0u8; 4];
        match file.read(&mut magic) {
            Ok(read) if read >= 4 => {
                magic == [0x50, 0x4B, 0x03, 0x04]
                    || magic == [0x50, 0x4B, 0x05, 0x06]
                    || magic == [0x50, 0x4B, 0x07, 0x08]
            }
            _ => false,
        }
    }
}
