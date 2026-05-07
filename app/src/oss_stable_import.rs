//! One-time My Warp import from the official Warp stable channel.
//!
//! The OSS bundle uses its own app ID (`dev.warp.WarpOss`) and therefore its
//! own config/state directories. On first launch of a packaged My Warp build,
//! offer to copy the user's official Warp config and local session database
//! into the OSS namespace before Warp opens its own database.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result};
use chrono::Local;
use warp_core::{
    channel::{Channel, ChannelState},
    paths::{WARP_CONFIG_DIR, data_dir, secure_state_dir, state_dir},
};

const OSS_CONFIG_DIR_NAME: &str = ".warp-oss";
const OFFICIAL_STABLE_STATE_DIR_NAME: &str = "dev.warp.Warp-Stable";
const OSS_STATE_DIR_NAME: &str = "dev.warp.WarpOss";
const MARKER_DIR_NAME: &str = ".my-warp-import";
const IMPORTED_MARKER_NAME: &str = "stable-import-v1-completed";
const DECLINED_MARKER_NAME: &str = "stable-import-v1-declined";

#[derive(Debug, Clone)]
struct StableImportPaths {
    stable_config_dir: PathBuf,
    oss_config_dir: PathBuf,
    stable_state_dir: Option<PathBuf>,
    oss_state_dir: PathBuf,
    backup_root: PathBuf,
}

#[derive(Debug)]
struct ImportReport {
    backup_dir: PathBuf,
    copied_config: bool,
    copied_state: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportPromptChoice {
    Import,
    Skip,
}

/// Prompts once, then imports official Warp config/session data into My Warp if
/// the user chooses to do so.
pub(crate) fn prompt_and_import_from_stable_if_needed() {
    if ChannelState::channel() != Channel::Oss || !ChannelState::is_release_bundle() {
        return;
    }

    let Some(paths) = StableImportPaths::for_current_user() else {
        return;
    };

    if !should_offer_import(&paths) {
        return;
    }

    if is_official_warp_running() {
        show_message(
            "My Warp 安装引导",
            "检测到官方 Warp 仍在运行。\n\n为了安全导入标签页和会话数据库，请先完全退出官方 Warp，然后重新打开 My Warp。本次不会导入任何数据。",
        );
        return;
    }

    match prompt_for_import() {
        Some(ImportPromptChoice::Import) => {
            match import_from_stable(&paths, &Local::now().format("%Y%m%d-%H%M%S").to_string()) {
                Ok(report) => show_import_success(&report),
                Err(err) => {
                    log::warn!("Failed to import official Warp data into My Warp: {err:#}");
                    show_message(
                        "My Warp 导入失败",
                        &format!(
                            "从官方 Warp 导入配置和标签页失败。\n\n错误：{err:#}\n\nMy Warp 会继续启动，官方 Warp 数据没有被修改。"
                        ),
                    );
                }
            }
        }
        Some(ImportPromptChoice::Skip) => {
            if let Err(err) = write_marker(&paths, DECLINED_MARKER_NAME, "declined") {
                log::warn!("Failed to write My Warp import declined marker: {err:#}");
            }
        }
        None => {}
    }
}

impl StableImportPaths {
    fn for_current_user() -> Option<Self> {
        let home = dirs::home_dir()?;
        let stable_config_dir = home.join(WARP_CONFIG_DIR);
        let oss_config_dir = data_dir();
        let stable_state_dir = official_stable_state_dir();
        let oss_state_dir = secure_state_dir().unwrap_or_else(state_dir);
        let backup_root = home.join(".warp-oss-import-backups");

        Some(Self {
            stable_config_dir,
            oss_config_dir,
            stable_state_dir,
            oss_state_dir,
            backup_root,
        })
    }

    #[cfg(test)]
    fn for_test(root: &Path) -> Self {
        Self {
            stable_config_dir: root.join(WARP_CONFIG_DIR),
            oss_config_dir: root.join(OSS_CONFIG_DIR_NAME),
            stable_state_dir: Some(root.join(OFFICIAL_STABLE_STATE_DIR_NAME)),
            oss_state_dir: root.join(OSS_STATE_DIR_NAME),
            backup_root: root.join(".warp-oss-import-backups"),
        }
    }

    fn marker_dir(&self) -> PathBuf {
        self.oss_config_dir.join(MARKER_DIR_NAME)
    }

    fn imported_marker(&self) -> PathBuf {
        self.marker_dir().join(IMPORTED_MARKER_NAME)
    }

    fn declined_marker(&self) -> PathBuf {
        self.marker_dir().join(DECLINED_MARKER_NAME)
    }
}

fn official_stable_state_dir() -> Option<PathBuf> {
    let fallback_state_dir = directories::ProjectDirs::from("dev", "warp", "Warp-Stable")
        .map(|dirs| dirs.data_local_dir().to_owned());

    if let Some(app_group_root) = warp_core::paths::app_group_container_path() {
        let app_group_state_dir = app_group_root
            .join("Library/Application Support")
            .join(OFFICIAL_STABLE_STATE_DIR_NAME);

        if app_group_state_dir.join("warp.sqlite").exists() {
            return Some(app_group_state_dir);
        }

        if fallback_state_dir
            .as_ref()
            .is_some_and(|dir| dir.join("warp.sqlite").exists())
        {
            return fallback_state_dir;
        }

        if app_group_state_dir.exists() {
            return Some(app_group_state_dir);
        }
    }

    fallback_state_dir
}

fn should_offer_import(paths: &StableImportPaths) -> bool {
    if paths.imported_marker().exists() || paths.declined_marker().exists() {
        return false;
    }

    paths.stable_config_dir.exists() || stable_state_has_sqlite(paths)
}

fn stable_state_has_sqlite(paths: &StableImportPaths) -> bool {
    paths
        .stable_state_dir
        .as_ref()
        .is_some_and(|dir| dir.join("warp.sqlite").exists())
}

fn is_official_warp_running() -> bool {
    command_succeeds("pgrep", &["-x", "Warp"])
        || command_succeeds("pgrep", &["-f", "/Warp\\.app/Contents/MacOS/stable"])
        || command_succeeds("pgrep", &["-f", "/Warp\\.app/Contents/MacOS/Warp"])
}

fn command_succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

fn prompt_for_import() -> Option<ImportPromptChoice> {
    let message = "\
检测到你已经安装过官方 Warp。\n\n\
My Warp 可以把官方 Warp 的本地配置、主题、窗口/标签页和会话恢复数据库导入进来。\n\n\
导入前会自动备份当前 My Warp 数据。导入时请确保官方 Warp 已完全退出。";

    let output = Command::new("osascript")
        .env("MY_WARP_IMPORT_MESSAGE", message)
        .args([
            "-e",
            r#"button returned of (display dialog (system attribute "MY_WARP_IMPORT_MESSAGE") buttons {"暂不导入", "从 Warp 导入"} default button "从 Warp 导入" with title "My Warp 安装引导" with icon note)"#,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return Some(ImportPromptChoice::Skip);
    }

    let button = String::from_utf8_lossy(&output.stdout);
    if button.trim() == "从 Warp 导入" {
        Some(ImportPromptChoice::Import)
    } else {
        Some(ImportPromptChoice::Skip)
    }
}

fn show_import_success(report: &ImportReport) {
    let mut imported = Vec::new();
    if report.copied_config {
        imported.push("配置和主题");
    }
    if report.copied_state {
        imported.push("标签页和会话状态");
    }

    let imported = if imported.is_empty() {
        "没有发现可导入的数据".to_owned()
    } else {
        imported.join("、")
    };

    show_message(
        "My Warp 导入完成",
        &format!(
            "已导入：{imported}\n\n当前 My Warp 数据已备份到：\n{}",
            report.backup_dir.display()
        ),
    );
}

fn show_message(title: &str, message: &str) {
    let _ = Command::new("osascript")
        .env("MY_WARP_DIALOG_TITLE", title)
        .env("MY_WARP_DIALOG_MESSAGE", message)
        .args([
            "-e",
            r#"display dialog (system attribute "MY_WARP_DIALOG_MESSAGE") buttons {"好"} default button "好" with title (system attribute "MY_WARP_DIALOG_TITLE") with icon note"#,
        ])
        .status();
}

fn import_from_stable(paths: &StableImportPaths, timestamp: &str) -> Result<ImportReport> {
    let backup_dir = paths.backup_root.join(timestamp);
    fs::create_dir_all(&backup_dir).with_context(|| {
        format!(
            "failed to create My Warp import backup directory {}",
            backup_dir.display()
        )
    })?;

    backup_existing_oss_data(paths, &backup_dir)?;

    let copied_config = if paths.stable_config_dir.exists() {
        fs::create_dir_all(&paths.oss_config_dir).with_context(|| {
            format!(
                "failed to create My Warp config directory {}",
                paths.oss_config_dir.display()
            )
        })?;
        copy_dir_contents(&paths.stable_config_dir, &paths.oss_config_dir).with_context(|| {
            format!(
                "failed to copy {} into {}",
                paths.stable_config_dir.display(),
                paths.oss_config_dir.display()
            )
        })?;
        true
    } else {
        false
    };

    let copied_state = if let Some(stable_state_dir) = &paths.stable_state_dir {
        copy_sqlite_state(stable_state_dir, &paths.oss_state_dir).with_context(|| {
            format!(
                "failed to copy official Warp session database from {} into {}",
                stable_state_dir.display(),
                paths.oss_state_dir.display()
            )
        })?
    } else {
        false
    };

    write_marker(
        paths,
        IMPORTED_MARKER_NAME,
        &format!(
            "completed\nbackup_dir={}\ncopied_config={copied_config}\ncopied_state={copied_state}\n",
            backup_dir.display()
        ),
    )?;

    Ok(ImportReport {
        backup_dir,
        copied_config,
        copied_state,
    })
}

fn backup_existing_oss_data(paths: &StableImportPaths, backup_dir: &Path) -> Result<()> {
    if paths.oss_config_dir.exists() {
        copy_path(&paths.oss_config_dir, &backup_dir.join(OSS_CONFIG_DIR_NAME)).with_context(
            || {
                format!(
                    "failed to back up My Warp config directory {}",
                    paths.oss_config_dir.display()
                )
            },
        )?;
    }

    if paths.oss_state_dir.exists() {
        copy_path(&paths.oss_state_dir, &backup_dir.join(OSS_STATE_DIR_NAME)).with_context(
            || {
                format!(
                    "failed to back up My Warp state directory {}",
                    paths.oss_state_dir.display()
                )
            },
        )?;
    }

    Ok(())
}

fn write_marker(paths: &StableImportPaths, marker_name: &str, contents: &str) -> Result<()> {
    fs::create_dir_all(paths.marker_dir())
        .with_context(|| format!("failed to create {}", paths.marker_dir().display()))?;
    fs::write(paths.marker_dir().join(marker_name), contents).with_context(|| {
        format!(
            "failed to write My Warp import marker {}",
            paths.marker_dir().join(marker_name).display()
        )
    })
}

fn copy_sqlite_state(source_dir: &Path, target_dir: &Path) -> io::Result<bool> {
    let sqlite_files = ["warp.sqlite", "warp.sqlite-wal", "warp.sqlite-shm"];
    if !source_dir.join("warp.sqlite").exists() {
        return Ok(false);
    }

    fs::create_dir_all(target_dir)?;

    for file_name in sqlite_files {
        let target = target_dir.join(file_name);
        if target.exists() || fs::symlink_metadata(&target).is_ok() {
            remove_path(&target)?;
        }
    }

    for file_name in sqlite_files {
        let source = source_dir.join(file_name);
        if source.exists() {
            fs::copy(&source, target_dir.join(file_name))?;
        }
    }

    Ok(true)
}

fn copy_path(source: &Path, target: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        if fs::symlink_metadata(target).is_ok() {
            remove_path(target)?;
        }
        let link_target = fs::read_link(source)?;
        std::os::unix::fs::symlink(link_target, target)
    } else if metadata.is_dir() {
        if let Ok(target_metadata) = fs::symlink_metadata(target) {
            if target_metadata.file_type().is_symlink() || !target_metadata.is_dir() {
                remove_path(target)?;
            }
        }
        fs::create_dir_all(target)?;
        copy_dir_contents(source, target)
    } else {
        if fs::symlink_metadata(target).is_ok() {
            remove_path(target)?;
        }
        fs::copy(source, target)?;
        Ok(())
    }
}

fn copy_dir_contents(source_dir: &Path, target_dir: &Path) -> io::Result<()> {
    fs::create_dir_all(target_dir)?;

    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        if file_name_str == ".DS_Store" || file_name_str.starts_with("._") {
            continue;
        }

        copy_path(&entry.path(), &target_dir.join(file_name))?;
    }

    Ok(())
}

fn remove_path(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn offers_import_when_official_config_exists_and_no_marker_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = StableImportPaths::for_test(tmp.path());

        fs::create_dir(&paths.stable_config_dir).unwrap();

        assert!(should_offer_import(&paths));
    }

    #[test]
    fn does_not_offer_import_after_decline_marker_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = StableImportPaths::for_test(tmp.path());

        fs::create_dir(&paths.stable_config_dir).unwrap();
        write_marker(&paths, DECLINED_MARKER_NAME, "declined").unwrap();

        assert!(!should_offer_import(&paths));
    }

    #[test]
    fn imports_config_and_session_database_with_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = StableImportPaths::for_test(tmp.path());

        fs::create_dir(&paths.stable_config_dir).unwrap();
        fs::write(
            paths.stable_config_dir.join("settings.toml"),
            "theme = \"stable\"",
        )
        .unwrap();
        fs::create_dir(paths.stable_config_dir.join("themes")).unwrap();
        fs::write(
            paths.stable_config_dir.join("themes").join("dark.yaml"),
            "theme",
        )
        .unwrap();

        fs::create_dir(&paths.stable_state_dir.clone().unwrap()).unwrap();
        fs::write(
            paths.stable_state_dir.clone().unwrap().join("warp.sqlite"),
            "stable-db",
        )
        .unwrap();
        fs::write(
            paths
                .stable_state_dir
                .clone()
                .unwrap()
                .join("warp.sqlite-wal"),
            "stable-wal",
        )
        .unwrap();

        fs::create_dir(&paths.oss_config_dir).unwrap();
        fs::write(
            paths.oss_config_dir.join("settings.toml"),
            "theme = \"oss\"",
        )
        .unwrap();
        fs::create_dir(&paths.oss_state_dir).unwrap();
        fs::write(paths.oss_state_dir.join("warp.sqlite"), "oss-db").unwrap();

        let report = import_from_stable(&paths, "20260507-120000").unwrap();

        assert!(report.copied_config);
        assert!(report.copied_state);
        assert_eq!(
            fs::read_to_string(paths.oss_config_dir.join("settings.toml")).unwrap(),
            "theme = \"stable\""
        );
        assert_eq!(
            fs::read_to_string(paths.oss_config_dir.join("themes").join("dark.yaml")).unwrap(),
            "theme"
        );
        assert_eq!(
            fs::read_to_string(paths.oss_state_dir.join("warp.sqlite")).unwrap(),
            "stable-db"
        );
        assert_eq!(
            fs::read_to_string(paths.oss_state_dir.join("warp.sqlite-wal")).unwrap(),
            "stable-wal"
        );
        assert_eq!(
            fs::read_to_string(
                report
                    .backup_dir
                    .join(OSS_CONFIG_DIR_NAME)
                    .join("settings.toml")
            )
            .unwrap(),
            "theme = \"oss\""
        );
        assert_eq!(
            fs::read_to_string(
                report
                    .backup_dir
                    .join(OSS_STATE_DIR_NAME)
                    .join("warp.sqlite")
            )
            .unwrap(),
            "oss-db"
        );
        assert!(paths.imported_marker().exists());
    }

    #[test]
    fn imports_config_without_session_database_when_stable_database_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = StableImportPaths::for_test(tmp.path());

        fs::create_dir(&paths.stable_config_dir).unwrap();
        fs::write(
            paths.stable_config_dir.join("settings.toml"),
            "theme = \"stable\"",
        )
        .unwrap();
        fs::create_dir(paths.stable_state_dir.clone().unwrap()).unwrap();

        let report = import_from_stable(&paths, "20260507-120000").unwrap();

        assert!(report.copied_config);
        assert!(!report.copied_state);
        assert_eq!(
            fs::read_to_string(paths.oss_config_dir.join("settings.toml")).unwrap(),
            "theme = \"stable\""
        );
    }
}
