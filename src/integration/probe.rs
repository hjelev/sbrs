use std::{
    env,
    process::{Command, Stdio},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalImageProtocol {
    Kitty,
    Iterm2Inline,
    Sixel,
    Unsupported,
}

impl TerminalImageProtocol {
    pub fn label(self) -> &'static str {
        match self {
            TerminalImageProtocol::Kitty => "kitty",
            TerminalImageProtocol::Iterm2Inline => "iterm2-inline",
            TerminalImageProtocol::Sixel => "sixel",
            TerminalImageProtocol::Unsupported => "unsupported",
        }
    }
}

pub fn terminal_image_protocol() -> (TerminalImageProtocol, String) {
    let term = env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    let term_program = env::var("TERM_PROGRAM").unwrap_or_default().to_ascii_lowercase();

    if env::var_os("KITTY_WINDOW_ID").is_some()
        || env::var_os("KONSOLE_VERSION").is_some()
        || env::var_os("GHOSTTY_RESOURCES_DIR").is_some()
        || term.contains("kitty")
        || term.contains("konsole")
        || term_program.contains("warp")
    {
        return (TerminalImageProtocol::Kitty, "env-detected kitty protocol".to_string());
    }

    if term_program.contains("iterm") || term_program.contains("wezterm") {
        return (
            TerminalImageProtocol::Iterm2Inline,
            "env-detected iTerm2 inline protocol".to_string(),
        );
    }

    if env::var_os("WT_SESSION").is_some() || term.contains("foot") {
        return (TerminalImageProtocol::Sixel, "env-detected sixel protocol".to_string());
    }

    (
        TerminalImageProtocol::Unsupported,
        "no known image protocol detected".to_string(),
    )
}

pub fn integration_probe(cmd: &str) -> (bool, String) {
    if let Ok(out) = Command::new("which").arg(cmd).output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return (true, path);
            }
        }
    }

    // Fallback for environments where `which` is unavailable or shell setup differs.
    match Command::new(cmd)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(_) => (true, cmd.to_string()),
        Err(_) => (false, String::new()),
    }
}

pub fn seven_zip_tool() -> Option<String> {
    for cmd in ["7z", "7zz", "7zr"] {
        if let (true, path) = integration_probe(cmd) {
            return Some(path);
        }
    }
    None
}

pub fn rar_tool() -> Option<String> {
    if let (true, path) = integration_probe("unrar") {
        return Some(path);
    }
    if let (true, path) = integration_probe("rar") {
        return Some(path);
    }
    None
}

pub fn bat_tool() -> Option<String> {
    if let (true, path) = integration_probe("bat") {
        return Some(path);
    }
    if let (true, path) = integration_probe("batcat") {
        return Some(path);
    }
    None
}

pub fn integration_availability_and_detail(key: &str) -> (bool, String) {
    let (available, _, detail) = integration_support_and_detail(key);
    (available, detail)
}

pub fn integration_support_and_detail(key: &str) -> (bool, bool, String) {
    match key {
        "$EDITOR" => {
            let editor_var = env::var("EDITOR").unwrap_or_else(|_| "(not set)".to_string());
            let editor_cmd = env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
            let (ok, path) = integration_probe(&editor_cmd);
            if ok {
                (true, false, path)
            } else {
                (false, false, format!("$EDITOR={}", editor_var))
            }
        }
        "age" => {
            let (age_ok, age_path) = integration_probe("age");
            (age_ok, false, age_path)
        }
        "zip" => {
            let (zip_ok, zip_path) = integration_probe("zip");
            let (unzip_ok, unzip_path) = integration_probe("unzip");
            let detail = if zip_ok && unzip_ok {
                format!("{} | {}", zip_path, unzip_path)
            } else if zip_ok {
                zip_path
            } else if unzip_ok {
                unzip_path
            } else {
                String::new()
            };
            (zip_ok || unzip_ok, false, detail)
        }
        "sox" => {
            let (play_ok, play_path) = integration_probe("play");
            let (sox_ok, sox_path) = integration_probe("sox");
            let detail = if play_ok {
                play_path
            } else if sox_ok {
                sox_path
            } else {
                String::new()
            };
            (play_ok || sox_ok, false, detail)
        }
        "bat" => {
            if let Some(path) = bat_tool() {
                (true, false, path)
            } else {
                (false, false, String::new())
            }
        }
        "tar" => {
            let (ok, detail) = integration_probe("tar");
            (ok, false, detail)
        }
        "7z" => {
            if let Some(path) = seven_zip_tool() {
                (true, false, path)
            } else {
                (false, false, String::new())
            }
        }
        "rar" => {
            if let Some(path) = rar_tool() {
                (true, false, path)
            } else {
                (false, false, String::new())
            }
        }
        "image-native" => {
            let (protocol, detail) = terminal_image_protocol();
            if protocol != TerminalImageProtocol::Unsupported {
                (true, false, detail)
            } else {
                (
                    false,
                    true,
                    "no native protocol detected (halfblock fallback available)".to_string(),
                )
            }
        }
        other => {
            let (ok, detail) = integration_probe(other);
            (ok, false, detail)
        }
    }
}
