use std::{
    fs, io,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::{App, ArchiveKind, ArchiveMount};

impl App {
    pub(crate) fn create_archive_mount_path(&self) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("sbrs_zip_{}_{}", std::process::id(), stamp))
    }

    pub(crate) fn try_mount_archive(&mut self, archive_path: PathBuf) -> bool {
        self.try_mount_archive_with(archive_path, "fuse-zip")
    }

    pub(crate) fn try_mount_archive_with(&mut self, archive_path: PathBuf, tool: &str) -> bool {
        if !self.integration_active(tool) {
            self.set_status(&format!("{} not installed", tool));
            return false;
        }

        if let Some(existing_idx) = self
            .archive_mounts
            .iter()
            .position(|m| m.archive_path == archive_path && m.mount_path.is_dir())
        {
            let archive_name = archive_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| archive_path.to_string_lossy().into_owned());
            let mount_path = self.archive_mounts[existing_idx].mount_path.clone();
            self.archive_mounts[existing_idx].return_dir = self.current_dir.clone();
            self.archive_mounts[existing_idx].archive_name = archive_name;
            self.try_enter_dir(mount_path);
            return true;
        }

        let mount_path = self.create_archive_mount_path();
        if fs::create_dir_all(&mount_path).is_err() {
            self.set_status("failed to create archive mount directory");
            return false;
        }

        match Command::new(tool).arg(&archive_path).arg(&mount_path).status() {
            Ok(status) if status.success() => {
                let archive_name = archive_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| archive_path.to_string_lossy().into_owned());
                let return_dir = self.current_dir.clone();
                self.archive_mounts.push(ArchiveMount {
                    archive_path,
                    mount_path: mount_path.clone(),
                    return_dir,
                    archive_name,
                });
                self.try_enter_dir(mount_path);
                true
            }
            _ => {
                let _ = fs::remove_dir(&mount_path);
                self.set_status(&format!("failed to mount archive with {}", tool));
                false
            }
        }
    }

    pub(crate) fn preview_archive_contents(&mut self, archive_path: &PathBuf) -> bool {
        let archive_name = archive_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| archive_path.to_string_lossy().into_owned());

        let mut cmd = match Self::archive_kind(archive_path) {
            Some(ArchiveKind::Zip)
                if self.integration_enabled("zip") && Self::integration_probe("unzip").0 =>
            {
                let mut c = Command::new("unzip");
                c.arg("-l").arg(archive_path);
                c
            }
            Some(ArchiveKind::Tar) if self.integration_active("tar") => {
                let mut c = Command::new("tar");
                c.arg("-tvf").arg(archive_path);
                c
            }
            Some(ArchiveKind::SevenZip)
                if self.integration_enabled("7z") && Self::seven_zip_tool().is_some() =>
            {
                let tool = Self::seven_zip_tool().unwrap_or_else(|| "7z".to_string());
                let mut c = Command::new(tool);
                c.arg("l").arg(archive_path);
                c
            }
            Some(ArchiveKind::Rar)
                if self.integration_enabled("rar") && Self::rar_tool().is_some() =>
            {
                let tool = Self::rar_tool().unwrap_or_else(|| "unrar".to_string());
                let mut c = Command::new(tool);
                c.arg("l").arg(archive_path);
                c
            }
            _ => {
                self.set_status(format!(
                    "no archive preview tool available for {}",
                    archive_name
                ));
                return false;
            }
        };

        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);

        let mut shown = false;
        if let Ok(mut child) = cmd.stdout(Stdio::piped()).spawn() {
            if let Some(stdout) = child.stdout.take() {
                shown = Command::new("less")
                    .arg("-R")
                    .stdin(stdout)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            }
            let _ = child.wait();
        }

        let _ = execute!(io::stdout(), EnterAlternateScreen);
        let _ = enable_raw_mode();

        if shown {
            self.set_status(format!("previewed archive listing: {}", archive_name));
        } else {
            self.set_status(format!("failed to preview archive: {}", archive_name));
        }

        shown
    }

    pub(crate) fn unmount_archive_path(path: &PathBuf) {
        let _ = Command::new("fusermount")
            .args(["-u", path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("fusermount3")
            .args(["-u", path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("fusermount")
            .args(["-uz", path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("fusermount3")
            .args(["-uz", path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("umount")
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("umount")
            .args(["-l", path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    pub(crate) fn try_leave_archive(&mut self) -> bool {
        let Some(mount_idx) = self
            .archive_mounts
            .iter()
            .rposition(|mount| mount.mount_path == self.current_dir)
        else {
            return false;
        };

        self.remember_current_selection();
        let return_dir = self.archive_mounts[mount_idx].return_dir.clone();
        let archive_name = self.archive_mounts[mount_idx].archive_name.clone();
        self.current_dir = return_dir;
        if self.refresh_entries_or_status() {
            self.select_entry_named(&archive_name);
        }
        true
    }

    pub(crate) fn cleanup_archive_mounts(&mut self) {
        // If current_dir is inside an archive mount, switch back to that mount's
        // return directory before unmounting so shell integration doesn't keep
        // a now-removed temp path.
        if let Some(mount) = self
            .archive_mounts
            .iter()
            .rev()
            .find(|m| self.current_dir == m.mount_path || self.current_dir.starts_with(&m.mount_path))
        {
            self.current_dir = mount.return_dir.clone();
        }

        while let Some(mount) = self.archive_mounts.pop() {
            let _ = mount.archive_path;
            Self::unmount_archive_path(&mount.mount_path);
            let _ = fs::remove_dir(&mount.mount_path);
        }
    }

    pub(crate) fn unmount_archive_mount_by_path(&mut self, mount_path: &PathBuf) -> bool {
        let Some(idx) = self
            .archive_mounts
            .iter()
            .rposition(|m| &m.mount_path == mount_path)
        else {
            return false;
        };

        let mount = self.archive_mounts.remove(idx);
        let was_inside = self.current_dir == mount.mount_path || self.current_dir.starts_with(&mount.mount_path);
        if was_inside {
            self.current_dir = mount.return_dir.clone();
            if self.refresh_entries_or_status() {
                self.select_entry_named(&mount.archive_name);
            }
        }
        Self::unmount_archive_path(&mount.mount_path);
        let _ = fs::remove_dir(&mount.mount_path);
        true
    }
}
