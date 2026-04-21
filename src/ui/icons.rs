pub fn named_dir_icon(name: &str) -> Option<(&'static str, (u8, u8, u8))> {
    match name.to_lowercase().as_str() {
        // --- XDG User Dirs ---
        "desktop" => Some(("\u{F108}", (100, 160, 240))),
        "documents" | "docs" => Some(("\u{F02D}", (100, 160, 240))),
        "downloads" => Some(("\u{F019}", (100, 200, 120))),
        "music" => Some(("\u{F001}", (180, 100, 220))),
        "pictures" | "photos" | "images" => Some(("\u{F03E}", (255, 200, 60))),
        "videos" | "movies" => Some(("\u{F03D}", (220, 80, 80))),
        "public" => Some(("\u{F0C0}", (80, 180, 220))),
        "templates" => Some(("\u{F0C5}", (180, 180, 180))),
        "trash" | ".trash" => Some(("\u{F014}", (140, 140, 140))),

        // --- Legal & Licensing ---
        "license" | "licenses" | "legal" | "copying" | "copyright" => {
            Some(("\u{F423}", (240, 190, 40)))
        }

        // --- Version Control ---
        ".git" | "git" => Some(("\u{E702}", (240, 93, 37))),
        ".github" | "github" => Some(("\u{F09B}", (220, 220, 220))),
        ".gitlab" | "gitlab" => Some(("\u{F296}", (252, 109, 38))),

        // --- Development & Runtimes ---
        "go" => Some(("\u{E724}", (0, 173, 216))),
        "node_modules" => Some(("\u{E718}", (76, 175, 80))),
        "venv" | ".venv" | "env" => Some(("\u{E235}", (59, 153, 11))),
        "python" | "py" | "__pycache__" => Some(("\u{E235}", (255, 212, 59))),
        ".cargo" | "cargo" | "rust" => Some(("\u{E7A8}", (222, 165, 132))),
        "java" | "maven" | "gradle" => Some(("\u{E738}", (231, 10, 26))),
        "ruby" | "gems" => Some(("\u{E739}", (170, 20, 1))),
        "php" => Some(("\u{E73D}", (119, 123, 179))),

        // --- Project Structure ---
        "src" | "source" | "sources" => Some(("\u{F121}", (100, 181, 246))),
        "lib" | "libs" | "library" => Some(("\u{F1B2}", (100, 181, 246))),
        "bin" | "sbin" => Some(("\u{F489}", (255, 183, 77))),
        "scripts" | "script" => Some(("\u{F085}", (255, 183, 77))),
        "include" | "includes" | "headers" => Some(("\u{F121}", (77, 208, 225))),
        "test" | "tests" | "spec" | "specs" => Some(("\u{F0C3}", (244, 67, 54))),
        "target" | "build" | "dist" | "out" | "release" | "debug" => {
            Some(("\u{F0AD}", (200, 140, 110)))
        }
        "assets" | "resources" | "res" => Some(("\u{F044}", (255, 235, 59))),
        "vendor" | "third_party" => Some(("\u{F1B3}", (144, 164, 174))),

        // --- Config & System ---
        ".config" | "config" | "conf" => Some(("\u{F013}", (200, 200, 200))),
        ".local" => Some(("\u{F07B}", (160, 160, 160))),
        ".ssh" | "ssh" | "keys" | "certs" => Some(("\u{F023}", (255, 183, 77))),
        ".cache" | "cache" => Some(("\u{F4A1}", (158, 158, 158))),
        "var" | "tmp" | "temp" => Some(("\u{F017}", (210, 105, 30))),
        "logs" | "log" => Some(("\u{F18D}", (160, 160, 160))),
        "snap" => Some(("\u{F17C}", (230, 70, 70))),
        "applications" => Some(("\u{F009}", (66, 133, 244))),
        "android" => Some(("\u{F17B}", (61, 220, 132))),

        // --- Containers & Cloud ---
        ".docker" | "docker" => Some(("\u{F308}", (13, 110, 253))),
        ".kube" | "kubernetes" | "k8s" => Some(("\u{F30F}", (50, 108, 230))),
        ".aws" | "aws" => Some(("\u{F270}", (255, 153, 0))),
        ".terraform" | "terraform" => Some(("\u{F110}", (92, 78, 229))),

        // --- Tools & Editors ---
        ".vscode" | "vscode" => Some(("\u{E70C}", (0, 120, 212))),
        ".idea" | "intellij" => Some(("\u{E7B5}", (254, 40, 85))),
        ".vim" | "nvim" | "lua" => Some(("\u{E62B}", (87, 158, 58))),

        // --- Storage & Web ---
        "dropbox" => Some(("\u{F16B}", (0, 97, 255))),
        "onedrive" => Some(("\u{F48F}", (0, 120, 212))),
        "google_drive" | "gdrive" => Some(("\u{F4D8}", (52, 168, 83))),
        "www" | "public_html" | "site" => Some(("\u{F0AC}", (76, 175, 80))),
        "fonts" => Some(("\u{F031}", (255, 200, 100))),

        _ => None,
    }
}
