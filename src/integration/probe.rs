use std::{
    env,
    process::{Command, Stdio},
};

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
    match key {
        "$EDITOR" => {
            let editor_var = env::var("EDITOR").unwrap_or_else(|_| "(not set)".to_string());
            let editor_cmd = env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
            let (ok, path) = integration_probe(&editor_cmd);
            if ok {
                (true, path)
            } else {
                (false, format!("$EDITOR={}", editor_var))
            }
        }
        "age" => {
            let (age_ok, age_path) = integration_probe("age");
            (age_ok, age_path)
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
            (zip_ok || unzip_ok, detail)
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
            (play_ok || sox_ok, detail)
        }
        "bat" => {
            if let Some(path) = bat_tool() {
                (true, path)
            } else {
                (false, String::new())
            }
        }
        "tar" => integration_probe("tar"),
        "7z" => {
            if let Some(path) = seven_zip_tool() {
                (true, path)
            } else {
                (false, String::new())
            }
        }
        "rar" => {
            if let Some(path) = rar_tool() {
                (true, path)
            } else {
                (false, String::new())
            }
        }
        other => integration_probe(other),
    }
}
