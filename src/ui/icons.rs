/// Returns `(glyph, (r,g,b))` for the current OS/distro, or `None` if unrecognised.
/// Detection order on Linux: `/etc/os-release` ID exact → ID_LIKE tokens → NAME/ID
/// substring families → generic Tux. Non-Linux OSes are matched via `std::env::consts::OS`.
pub(crate) fn os_nerd_icon() -> Option<(&'static str, (u8, u8, u8))> {
    match std::env::consts::OS {
        "macos" => return Some(("\u{f302}", (255, 255, 255))),
        "windows" => return Some(("\u{f17a}", (0, 120, 212))),
        "freebsd" => return Some(("\u{f30c}", (175, 0, 0))),
        "openbsd" => return Some(("\u{f328}", (253, 200, 0))),
        "netbsd" => return Some(("\u{f328}", (253, 160, 0))),
        "dragonfly" => return Some(("\u{f17c}", (210, 210, 210))),
        "solaris" => return Some(("\u{f185}", (255, 165, 0))),
        _ => {}
    }

    // Linux: parse /etc/os-release
    let content = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    parse_os_release_content(&content).or(Some(("\u{f17c}", (210, 210, 210))))
}

/// Detect OS icon from a remote SSHFS/rclone mount by reading its `/etc/os-release`.
/// Falls back to a generic server icon if the file is absent (e.g. cloud storage).
pub(crate) fn remote_os_nerd_icon(mount_path: &std::path::Path) -> Option<(&'static str, (u8, u8, u8))> {
    let content = std::fs::read_to_string(mount_path.join("etc/os-release"))
        .or_else(|_| std::fs::read_to_string(mount_path.join("usr/lib/os-release")))
        .unwrap_or_default();
    if content.is_empty() {
        return None;
    }
    parse_os_release_content(&content)
}

pub(crate) fn os_nerd_icon_from_os_release_content(content: &str) -> Option<(&'static str, (u8, u8, u8))> {
    parse_os_release_content(content)
}

/// Parse `/etc/os-release` content (as a string) and return the matching distro icon.
fn parse_os_release_content(content: &str) -> Option<(&'static str, (u8, u8, u8))> {

    let mut id = String::new();
    let mut id_like = String::new();
    let mut name = String::new();

    for line in content.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("ID=") {
            id = v.trim_matches('"').to_lowercase();
        } else if let Some(v) = line.strip_prefix("ID_LIKE=") {
            id_like = v.trim_matches('"').to_lowercase();
        } else if let Some(v) = line.strip_prefix("NAME=") {
            name = v.trim_matches('"').to_lowercase();
        }
    }

    // 1. Exact ID match
    if let Some(icon) = distro_icon_for_id(&id) {
        return Some(icon);
    }

    // 2. ID_LIKE tokens (e.g. "ubuntu debian" → try each)
    for token in id_like.split_whitespace() {
        if let Some(icon) = distro_icon_for_id(token) {
            return Some(icon);
        }
    }

    // 3. Substring family match against combined id+name
    let combined = format!("{} {}", id, name);
    if let Some(icon) = distro_icon_by_family(&combined) {
        return Some(icon);
    }

    None
}

/// Exact ID → (glyph, rgb). Covers the most common distros.
fn distro_icon_for_id(id: &str) -> Option<(&'static str, (u8, u8, u8))> {
    match id {
        "arch" | "archarm" | "artix"       => Some(("\u{f303}", (23, 147, 209))),
        "debian"                            => Some(("\u{f306}", (215, 10, 83))),
        "ubuntu" | "ubuntu-server"          => Some(("\u{f31b}", (233, 84, 32))),
        "fedora" | "fedora-asahi-remix"     => Some(("\u{f30a}", (41, 162, 228))),
        "opensuse" | "opensuse-leap"
            | "opensuse-tumbleweed" | "suse"=> Some(("\u{f314}", (115, 186, 37))),
        "rhel" | "centos" | "almalinux"
            | "rocky" | "ol" | "scientific" => Some(("\u{f316}", (190, 30, 45))),
        "nixos"                             => Some(("\u{f313}", (126, 186, 228))),
        "linuxmint" | "mint"               => Some(("\u{f30e}", (134, 194, 50))),
        "manjaro"                           => Some(("\u{f312}", (52, 190, 91))),
        "pop" | "pop-os"                   => Some(("\u{f32a}", (72, 185, 199))),
        "kali"                              => Some(("\u{f327}", (38, 139, 210))),
        "alpine"                            => Some(("\u{f300}", (14, 87, 123))),
        "void"                              => Some(("\u{f32e}", (73, 173, 80))),
        "gentoo"                            => Some(("\u{f30d}", (148, 137, 217))),
        // Raspberry Pi OS is Debian-based; use Debian glyph for better Nerd Font compatibility.
        "raspbian" | "raspios"             => Some(("\u{f306}", (196, 0, 40))),
        "slackware"                         => Some(("\u{f30f}", (80, 80, 200))),
        "mageia"                            => Some(("\u{f310}", (40, 120, 220))),
        "elementary" | "elementaryos"      => Some(("\u{f309}", (100, 160, 240))),
        "zorin"                             => Some(("\u{f33e}", (21, 114, 161))),
        "parrot" | "parrotos"              => Some(("\u{f330}", (0, 186, 200))),
        "mx" | "mxlinux"                   => Some(("\u{f11b}", (80, 80, 80))),
        "deepin"                            => Some(("\u{f324}", (0, 142, 207))),
        "guix"                              => Some(("\u{f323}", (255, 200, 0))),
        _                                   => None,
    }
}

/// Substring family fallback for derivatives not listed above.
fn distro_icon_by_family(combined: &str) -> Option<(&'static str, (u8, u8, u8))> {
    if combined.contains("ubuntu")                                      { return Some(("\u{f31b}", (233, 84, 32))); }
    if combined.contains("debian")                                      { return Some(("\u{f306}", (215, 10, 83))); }
    if combined.contains("arch")                                        { return Some(("\u{f303}", (23, 147, 209))); }
    if combined.contains("fedora")                                      { return Some(("\u{f30a}", (41, 162, 228))); }
    if combined.contains("suse")                                        { return Some(("\u{f314}", (115, 186, 37))); }
    if combined.contains("centos") || combined.contains("rhel")
        || combined.contains("redhat") || combined.contains("alma")
        || combined.contains("rocky")                                   { return Some(("\u{f316}", (190, 30, 45))); }
    if combined.contains("gentoo")                                      { return Some(("\u{f30d}", (148, 137, 217))); }
    if combined.contains("mint")                                        { return Some(("\u{f30e}", (134, 194, 50))); }
    if combined.contains("manjaro")                                     { return Some(("\u{f312}", (52, 190, 91))); }
    None
}

pub fn named_dir_icon(name: &str) -> Option<(&'static str, (u8, u8, u8))> {
    match name.to_lowercase().as_str() {
        // --- XDG User Dirs ---
        "desktop" => Some(("\u{f01c4}", (100, 160, 240))),
        "documents" | "docs" => Some(("\u{f0c82}", (100, 160, 240))),
        "downloads" => Some(("\u{f024d}", (100, 200, 120))),
        "music" => Some(("\u{f1359}", (180, 100, 220))),
        "pictures" | "photos" | "images" | "media" => Some(("\u{f024f}", (255, 200, 60))),
        "videos" | "movies" => Some(("\u{f19fa}", (220, 80, 80))),
        "public" => Some(("\u{f178a}", (80, 180, 220))),
        "templates" => Some(("\u{f0c5}", (180, 180, 180))),
        "home" => Some(("\u{f10b5}", (180, 180, 180))),
        "trash" | ".trash" => Some(("\u{f1f8}", (140, 140, 140))),

        // --- Legal & Licensing ---
        "license" | "licenses" | "copying" => {
            Some(("\u{f0fc3}", (240, 190, 40)))
        }
        "copyright" => Some(("\u{f1f9}", (100, 160, 240))),
        "legal" => Some(("\u{f0e3}", (100, 160, 240))),

        // --- Version Control ---
        ".git" | "git" => Some(("\u{e5fb}", (240, 93, 37))),
        ".github" | "github" => Some(("\u{e5fd}", (220, 220, 220))),
        ".gitlab" | "gitlab" => Some(("\u{e5fb}", (252, 109, 38))),

        // --- Gaming & Media ---
        "games" | "gaming" => Some(("\u{f0eb5}", (80, 220, 80))),
        "steam" | ".steam" | "steamapps" => Some(("\u{F1B7}", (100, 160, 220))),
        "discord" | ".discord" => Some(("\u{f066f}", (88, 101, 242))),
        "obs" | "obs-studio" => Some(("\u{f19fa}", (70, 180, 255))),

        // --- Development & Runtimes ---
        "go" => Some(("\u{f2bf}", (0, 220, 255))),
        "node_modules" => Some(("\u{E718}", (76, 175, 80))),
        "venv" | ".venv" | "env" => Some(("\u{E235}", (59, 153, 11))),
        "python" | "py" | "__pycache__" => Some(("\u{E235}", (255, 212, 59))),
        ".cargo" | "cargo" | "rust" => Some(("\u{f1617}", (222, 165, 132))),
        "java" | "maven" | "gradle" => Some(("\u{f0176}", (200, 130, 60))),
        "ruby" | "gems" => Some(("\u{f0acf}", (170, 20, 1))),


        // --- Project Structure ---
        "plugins" | "extensions" | "addons"  => Some(("\u{f0257}", (255, 153, 0))),
        "local" | "locale" | "i18n" | "l10n" |"translations" => Some(("\u{f024c}", (255, 153, 0))),
        "client" | "server" | "backend" | "frontend" => Some(("\u{f233}", (100, 160, 220))),
        "styles" | "css" | "themes" | "scss" => Some(("\u{e6b8}", (230, 70, 70))),
        "js" | "javascript" | "ts" | "typescript" | "jsx" | "tsx" => Some(("\u{f2ee}", (230, 70, 70))),
        "db" | "data" | "dataset" | "databases" | "sql" => Some(("\u{f12e3}", (100, 160, 240))),
        "api" => Some(("\u{f19ec}", (100, 160, 240))),
        "npm" => Some(("\u{e5fa}", (100, 160, 240))),
        "src" | "source" | "sources" => Some(("\u{f0d09}", (100, 181, 246))),
        "scripts" | "script" => Some(("\u{f0d09}", (255, 183, 77))),
        "include" | "includes" | "headers" => Some(("\u{f0d09}", (77, 208, 225))),
        "test" | "tests" | "spec" | "specs" => Some(("\u{F0C3}", (244, 67, 54))),
        "target" | "build" | "dist" | "out" | "release" | "debug" => {
            Some(("\u{f19fc}", (200, 140, 110)))
        }
        "assets" | "resources" | "res" => Some(("\u{f08de}", (255, 235, 59))),
        "vendor" | "third_party" => Some(("\u{F1B3}", (144, 164, 174))),

        // --- Config & System ---
        "usr" => Some(("\u{f024c}", (255, 183, 77))),
        ".config" | "config" | "conf" | "settings" | "cfg" => Some(("\u{f107d}", (200, 200, 200))),
        ".local" => Some(("\u{f024c}", (160, 160, 160))),
        ".ssh" | "ssh" | "keys" | "certs" => Some(("\u{f08ac}", (255, 183, 77))),
        ".cache" | "cache" => Some(("\u{f197e}", (158, 158, 158))),
        "var" | "tmp" | "temp" => Some(("\u{f0aba}", (210, 105, 30))),
        "logs" | "log" => Some(("\u{F18D}", (160, 160, 160))),
        "snap" => Some(("\u{f0257}", (230, 70, 70))),
        "mnt" | "srv" | "projects" | "workspace" | "sync" => Some(("\u{f126d}", (66, 133, 244))),
        "applications" => Some(("\u{F009}", (66, 133, 244))),
        "android" => Some(("\u{f0032}", (61, 220, 132))),

        // --- Containers & Cloud ---
        ".docker" | "docker" => Some(("\u{f0868}", (13, 110, 253))),
        ".kube" | "kubernetes" | "k8s" => Some(("\u{F30F}", (50, 108, 230))),
        ".aws" | "aws" => Some(("\u{F270}", (255, 153, 0))),
        ".terraform" | "terraform" => Some(("\u{F110}", (92, 78, 229))),

        // --- Tools & Editors ---
        ".vscode" | "vscode" => Some(("\u{E70C}", (0, 120, 212))),
        ".idea" | "intellij" => Some(("\u{E7B5}", (254, 40, 85))),
        ".vim" | "nvim" | "lua" => Some(("\u{E62B}", (87, 158, 58))),

        "backup" | "backups" | "archive" => Some(("\u{f06eb}", (180, 140, 100))), // nf-md-folder_zip
        "private" | "secrets" | "hidden" => Some(("\u{f0250}", (220, 50, 50))),   // nf-md-folder_lock
        "mail" | "emails" => Some(("\u{f01ee}", (200, 200, 100))),               // nf-md-folder_envelope

        // --- Storage & Web ---
        "dropbox" => Some(("\u{F16B}", (0, 97, 255))),
        "onedrive" => Some(("\u{F48F}", (0, 120, 212))),
        "google_drive" | "gdrive" => Some(("\u{F4D8}", (52, 168, 83))),
        "www" | "public_html" | "site" => Some(("\u{F0AC}", (76, 175, 80))),
        "fonts" => Some(("\u{F031}", (255, 200, 100))),

        // ---- Misc & Uncategorized ---
        "swap" => Some(("\u{f0fb6}", (200, 200, 200))),
        "boot" => Some(("\u{f19f0}", (200, 200, 200))),
        "lost+found" => Some(("\u{f0968}", (200, 200, 200))),
        _ if name.contains("bin") => Some(("\u{f107f}", (255, 183, 77))),
        _ if name.contains("masoko") || name.contains("star") => Some(("\u{f069d}", (255, 183, 77))),
        _ if name.contains("love") || name.contains("heart") => Some(("\u{f10ea}", (255, 183, 77))),
        _ if name.contains("lib") => Some(("\u{f0770}", (100, 181, 246))),
        _ if name.starts_with('.') => Some(("\u{f179e}", (120, 120, 120))),
        _ => None,
    }
}

pub fn named_file_icon(name: &str) -> Option<(&'static str, (u8, u8, u8))> {
    let name_low = name.to_lowercase();
    
    match name_low.as_str() {
        "desktop" => Some(("\u{f10b5}", (100, 160, 240))),
        "documents" | "docs" => Some(("\u{f0c82}", (100, 160, 240))),
        "downloads" => Some(("\u{f024d}", (100, 200, 120))),
        "music" => Some(("\u{f075a}", (180, 100, 220))),
        "legal" => Some(("\u{f08ea}", (255, 183, 77))),
        "makefile" => Some(("\u{f1323}", (158, 158, 158))),
        _ if name_low.contains("lib") => Some(("\u{f0770}", (100, 181, 246))),
        
        _ if name_low.contains("masoko") || name_low.contains("star") => Some(("\u{f04ce}", (255, 183, 77))),
        _ if name_low.contains("love") || name_low.contains("heart") || name_low.contains("adult") => Some(("\u{f02d1}", (244, 67, 54))),

        _ if name_low.starts_with('.') => Some(("\u{f0613}", (120, 120, 120))),
        _ if name_low.starts_with("license") => Some(("\u{f0fc3}", (120, 120, 120))),
        _ if name_low.ends_with(".mmd") => Some(("\u{f154f}", (120, 120, 120))),
        _ => None,
    }
}