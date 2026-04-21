use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc,
    thread,
};

use crate::{App, GitInfoCache};

impl App {
    pub(crate) fn pump_git_info(&mut self) {
        let Some(rx) = self.git_info_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok((path, info)) => {
                self.git_info_cache = Some(GitInfoCache { path, info });
                self.git_info_rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.git_info_rx = None;
            }
        }
    }

    pub(crate) fn request_git_info_for_current_dir_once(&mut self) {
        if !self.integration_enabled("git") {
            self.git_info_rx = None;
            self.git_info_cache = None;
            return;
        }
        if self.git_info_rx.is_some() {
            return;
        }
        if self
            .git_info_cache
            .as_ref()
            .map(|cache| cache.path == self.current_dir)
            .unwrap_or(false)
        {
            return;
        }

        // Clear stale data from a previously visited path until the new result arrives.
        self.git_info_cache = None;
        let path = self.current_dir.clone();
        let (tx, rx) = mpsc::channel();
        self.git_info_rx = Some(rx);
        thread::spawn(move || {
            let info = App::get_git_info(&path);
            let _ = tx.send((path, info));
        });
    }

    pub(crate) fn cached_git_info_for_current_dir(&self) -> Option<(&str, bool, Option<(&str, u64)>)> {
        let cache = self.git_info_cache.as_ref()?;
        if cache.path != self.current_dir {
            return None;
        }
        cache.info.as_ref().map(|(branch, dirty, tag)| {
            let tag_info = tag.as_ref().map(|(name, ahead)| (name.as_str(), *ahead));
            (branch.as_str(), *dirty, tag_info)
        })
    }

    pub(crate) fn get_git_info(path: &PathBuf) -> Option<(String, bool, Option<(String, u64)>)> {
        let path_str = path.to_str()?;

        let branch = Command::new("git")
            .args(["-C", path_str, "symbolic-ref", "--short", "-q", "HEAD"])
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if value.is_empty() {
                        None
                    } else {
                        Some(value)
                    }
                } else {
                    None
                }
            })
            .or_else(|| {
                Command::new("git")
                    .args(["-C", path_str, "rev-parse", "--short", "HEAD"])
                    .output()
                    .ok()
                    .and_then(|out| {
                        if out.status.success() {
                            let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
                            if value.is_empty() {
                                None
                            } else {
                                Some(value)
                            }
                        } else {
                            None
                        }
                    })
            })?;

        // Fast tracked-change dirty check: exit code 1 means dirty, 0 means clean.
        let dirty_status = Command::new("git")
            .args(["-C", path_str, "diff-index", "--quiet", "HEAD", "--"])
            .status()
            .ok()?;

        let is_dirty = match dirty_status.code() {
            Some(0) => false,
            Some(1) => true,
            _ => return None,
        };

        let latest_tag = Command::new("git")
            .args([
                "-C",
                path_str,
                "for-each-ref",
                "refs/tags",
                "--sort=-v:refname",
                "--count=1",
                "--format=%(refname:short)",
            ])
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if value.is_empty() {
                        None
                    } else {
                        Some(value)
                    }
                } else {
                    None
                }
            });

        let tag_info = latest_tag.and_then(|tag| {
            let ahead = Command::new("git")
                .args(["-C", path_str, "rev-list", "--count", &format!("{}..HEAD", tag)])
                .output()
                .ok()
                .and_then(|out| {
                    if out.status.success() {
                        String::from_utf8_lossy(&out.stdout)
                            .trim()
                            .parse::<u64>()
                            .ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            Some((tag, ahead))
        });

        Some((branch, is_dirty, tag_info))
    }
}
