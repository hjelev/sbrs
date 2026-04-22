#[derive(Clone)]
pub struct IntegrationSpec {
    pub key: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub required: bool,
}

pub fn integration_catalog() -> Vec<IntegrationSpec> {
    vec![
        IntegrationSpec { key: "git", description: "branch & dirty status in header", category: "vcs", required: false },
        IntegrationSpec { key: "less", description: "view files (Enter fallback)", category: "viewer", required: true },
        IntegrationSpec { key: "$EDITOR", description: "edit files (e / F4)", category: "editor", required: true },
        IntegrationSpec { key: "bat", description: "syntax-highlighted view on Enter", category: "viewer", required: false },
        IntegrationSpec { key: "glow", description: "Markdown preview on Enter", category: "viewer", required: false },
        IntegrationSpec { key: "mmdflux", description: "Mermaid diagram preview on Enter (.mmd)", category: "viewer", required: false },
        IntegrationSpec { key: "links", description: "HTML preview on Enter", category: "viewer", required: false },
        IntegrationSpec { key: "pdftotext", description: "PDF text preview on Enter", category: "preview", required: false },
        IntegrationSpec { key: "asciinema", description: "terminal recording playback (.cast) on Enter (q/Esc to stop)", category: "preview", required: false },
        IntegrationSpec { key: "age", description: "password-protect/decrypt files (.age) with p/Enter/e", category: "security", required: false },
        IntegrationSpec { key: "jnv", description: "interactive JSON preview on Enter", category: "preview", required: false },
        IntegrationSpec { key: "csvlens", description: "interactive delimited preview (.csv/.tsv/.tab/.psv/.dsv/.ssv)", category: "preview", required: false },
        IntegrationSpec { key: "delta", description: "side-by-side colored compare (C: marked file vs cursor)", category: "diff", required: false },
        IntegrationSpec { key: "hexyl", description: "hex view for binary files on Enter", category: "preview", required: false },
        IntegrationSpec { key: "hexedit", description: "hex edit for binary files (e / F4)", category: "editor", required: false },
        IntegrationSpec { key: "vidir", description: "bulk rename when >1 marked (F2/r)", category: "rename", required: false },
        IntegrationSpec { key: "zip", description: "create/extract archives (Z)", category: "archive", required: false },
        IntegrationSpec { key: "tar", description: "extract tar/tar.gz/tar.xz/... archives", category: "archive", required: false },
        IntegrationSpec { key: "7z", description: "extract .7z archives", category: "archive", required: false },
        IntegrationSpec { key: "rar", description: "extract .rar archives", category: "archive", required: false },
        IntegrationSpec { key: "fuse-zip", description: "browse zip-based archives as folders", category: "archive", required: false },
        IntegrationSpec { key: "archivemount", description: "browse tar/zip archives as folders (Enter)", category: "archive", required: false },
        IntegrationSpec { key: "sox", description: "play audio files on Enter", category: "preview", required: false },
        IntegrationSpec { key: "viu", description: "image preview on Enter (preferred)", category: "preview", required: false },
        IntegrationSpec { key: "chafa", description: "image preview on Enter", category: "preview", required: false },
        IntegrationSpec { key: "sshfs", description: "mount SSH hosts via S picker", category: "network", required: false },
        IntegrationSpec { key: "rclone", description: "mount rclone remotes via S picker", category: "network", required: false },
        IntegrationSpec { key: "tmux", description: "split shell + less preview (i), editor (E)", category: "terminal", required: false },
        IntegrationSpec { key: "rg", description: "content search, fzf preview if avail (g)", category: "search", required: false },
        IntegrationSpec { key: "fzf", description: "fuzzy file search (f)", category: "search", required: false },
        IntegrationSpec { key: "wl-copy", description: "Wayland clipboard backend used by Ctrl+c full-path copy", category: "clipboard", required: false },
        IntegrationSpec { key: "xclip", description: "X11 clipboard backend used by Ctrl+c full-path copy", category: "clipboard", required: false },
        IntegrationSpec { key: "xsel", description: "X11 clipboard backend used by Ctrl+c full-path copy", category: "clipboard", required: false },
        IntegrationSpec { key: "pbcopy", description: "macOS clipboard backend used by Ctrl+c full-path copy", category: "clipboard", required: false },
    ]
}

pub fn integration_brew_package(key: &str) -> Option<&'static str> {
    match key {
        "__all_optional__" | "$EDITOR" | "less" | "pbcopy" => None,
        "git" => Some("git"),
        "bat" => Some("bat"),
        "glow" => Some("glow"),
        "mmdflux" => Some("kevinswiber/mmdflux/mmdflux"),
        "links" => Some("links"),
        "7z" => Some("p7zip"),
        "zip" => Some("zip"),
        "tar" => Some("gnu-tar"),
        "rar" => Some("rar"),
        "asciinema" => Some("asciinema"),
        "age" => Some("age"),
        "jnv" => Some("jnv"),
        "csvlens" => Some("csvlens"),
        "delta" => Some("git-delta"),
        "hexyl" => Some("hexyl"),
        "hexedit" => Some("hexedit"),
        "vidir" => Some("moreutils"),
        "fuse-zip" => {
            #[cfg(target_os = "macos")]
            {
                Some("fuse-zip-mac")
            }
            #[cfg(not(target_os = "macos"))]
            {
                Some("fuse-zip")
            }
        }
        "archivemount" => {
            #[cfg(target_os = "macos")]
            {
                Some("gromgit/fuse/archivemount")
            }
            #[cfg(not(target_os = "macos"))]
            {
                Some("archivemount")
            }
        }
        "sox" => Some("sox"),
        "viu" => Some("viu"),
        "chafa" => Some("chafa"),
        "sshfs" => Some("sshfs"),
        "rclone" => Some("rclone"),
        "tmux" => Some("tmux"),
        "rg" => Some("ripgrep"),
        "fzf" => Some("fzf"),
        "wl-copy" => Some("wl-clipboard"),
        "xclip" => Some("xclip"),
        "xsel" => Some("xsel"),
        "pdftotext" => Some("poppler"),
        _ => None,
    }
}
