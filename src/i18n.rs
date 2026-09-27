//! UI strings: centralized Chinese/English resource table.
//!
//! Every user-visible string goes through [`tr`] (or one of the formatting
//! helpers below) so that adding a new language later only touches this file:
//! add a `Lang` variant and fill in one column per string id.

/// Supported UI languages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    /// Switch to the other language.
    pub fn toggle(self) -> Lang {
        match self {
            Lang::Zh => Lang::En,
            Lang::En => Lang::Zh,
        }
    }

    /// Label for the language toggle button: always names the *other*
    /// language, i.e. the one a click will switch to.
    pub fn toggle_label(self) -> &'static str {
        match self {
            Lang::Zh => "🌐 EN",
            Lang::En => "🌐 中文",
        }
    }
}

/// Identifier for every user-visible string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S {
    Drive,
    Scan,
    Cancel,
    Cancelling,
    ExportCsv,
    Files,
    Folders,
    TreeView,
    FilterHint,
    ColName,
    ColSize,
    ColPct,
    NoData,
    NoSubfolders,
    Reveal,
    Trash,
    PermDeleteMenu,
    DeleteTitle,
    DeleteWarning,
    DeleteConfirm,
    FolderWord,
    FileWord,
    TopN,
    PickerTitle,
    PickerHint,
    StatusPickDrive,
    StatusScanCancelled,
}

/// Look up a localized string.
pub fn tr(lang: Lang, id: S) -> &'static str {
    match id {
        S::Drive => match lang {
            Lang::Zh => "盘符",
            Lang::En => "Drive",
        },
        S::Scan => match lang {
            Lang::Zh => "扫描",
            Lang::En => "Scan",
        },
        S::Cancel => match lang {
            Lang::Zh => "取消",
            Lang::En => "Cancel",
        },
        S::Cancelling => match lang {
            Lang::Zh => "正在取消…",
            Lang::En => "Cancelling…",
        },
        S::ExportCsv => match lang {
            Lang::Zh => "导出 CSV",
            Lang::En => "Export CSV",
        },
        S::Files => match lang {
            Lang::Zh => "文件",
            Lang::En => "Files",
        },
        S::Folders => match lang {
            Lang::Zh => "文件夹",
            Lang::En => "Folders",
        },
        S::TreeView => match lang {
            Lang::Zh => "🌲 树形",
            Lang::En => "🌲 Tree",
        },
        S::FilterHint => match lang {
            Lang::Zh => "筛选…",
            Lang::En => "Filter…",
        },
        S::ColName => match lang {
            Lang::Zh => "名称",
            Lang::En => "Name",
        },
        S::ColSize => match lang {
            Lang::Zh => "大小",
            Lang::En => "Size",
        },
        S::ColPct => "%",
        S::NoData => match lang {
            Lang::Zh => "暂无数据，请点击扫描。",
            Lang::En => "No data yet — hit Scan.",
        },
        S::NoSubfolders => match lang {
            Lang::Zh => "没有子文件夹。",
            Lang::En => "No subfolders here.",
        },
        S::Reveal => match lang {
            Lang::Zh => "在资源管理器中显示",
            Lang::En => "Reveal in File Explorer",
        },
        S::Trash => match lang {
            Lang::Zh => "移到回收站",
            Lang::En => "Move to Recycle Bin",
        },
        S::PermDeleteMenu => match lang {
            Lang::Zh => "永久删除…",
            Lang::En => "Delete permanently…",
        },
        S::DeleteTitle => match lang {
            Lang::Zh => "永久删除？",
            Lang::En => "Delete permanently?",
        },
        S::DeleteWarning => match lang {
            Lang::Zh => "此操作不可恢复，文件将直接删除而不进回收站。",
            Lang::En => "This cannot be undone. The item will be deleted directly, not moved to the Recycle Bin.",
        },
        S::DeleteConfirm => match lang {
            Lang::Zh => "永久删除",
            Lang::En => "Delete permanently",
        },
        S::FolderWord => match lang {
            Lang::Zh => "文件夹",
            Lang::En => "Folder",
        },
        S::FileWord => match lang {
            Lang::Zh => "文件",
            Lang::En => "File",
        },
        S::TopN => match lang {
            Lang::Zh => "按大小排列的前 {n} 个文件夹",
            Lang::En => "Top {n} folders by size",
        },
        S::PickerTitle => match lang {
            Lang::Zh => "选择要扫描的盘",
            Lang::En => "Select drive to scan",
        },
        S::PickerHint => match lang {
            Lang::Zh => "点击盘符即可选中。",
            Lang::En => "Click a drive to select it.",
        },
        S::StatusPickDrive => match lang {
            Lang::Zh => "选择盘符并点击扫描。",
            Lang::En => "Pick a drive and hit Scan.",
        },
        S::StatusScanCancelled => match lang {
            Lang::Zh => "扫描已取消。",
            Lang::En => "Scan cancelled.",
        },
    }
}

// ---- Formatted strings (word order differs per language) ----

/// "{n} records…" progress label while scanning.
pub fn records(lang: Lang, n: u64) -> String {
    match lang {
        Lang::Zh => format!("{n} 条记录…"),
        Lang::En => format!("{n} records…"),
    }
}

/// "{free} free of {total}" disk-space label.
pub fn free_of(lang: Lang, free: &str, total: &str) -> String {
    match lang {
        Lang::Zh => format!("{free} 可用，共 {total}"),
        Lang::En => format!("{free} free of {total}"),
    }
}

/// "Scanning {drive} …" status.
pub fn scanning(lang: Lang, drive: &str) -> String {
    match lang {
        Lang::Zh => format!("正在扫描 {drive}…"),
        Lang::En => format!("Scanning {drive} …"),
    }
}

/// "Done: …" status after a scan.
pub fn done_status(lang: Lang, nf: usize, nd: usize, size: &str, secs: f64) -> String {
    match lang {
        Lang::Zh => format!("完成：{nf} 个文件，{nd} 个文件夹，共 {size}，用时 {secs:.1} 秒"),
        Lang::En => format!("Done: {nf} files, {nd} folders, {size} in {secs:.1}s"),
    }
}

/// "Scan failed: {e}" status.
pub fn scan_failed(lang: Lang, e: &str) -> String {
    match lang {
        Lang::Zh => format!("扫描失败：{e}"),
        Lang::En => format!("Scan failed: {e}"),
    }
}

/// Status after a permanent delete.
pub fn perm_deleted(lang: Lang, name: &str) -> String {
    match lang {
        Lang::Zh => format!("已永久删除：{name}"),
        Lang::En => format!("Deleted permanently: {name}"),
    }
}

/// Status when a permanent delete fails.
pub fn perm_delete_failed(lang: Lang, e: &impl std::fmt::Display) -> String {
    match lang {
        Lang::Zh => format!("删除失败：{e}"),
        Lang::En => format!("Delete failed: {e}"),
    }
}

/// Status after moving an item to the trash.
pub fn trashed(lang: Lang, name: &str) -> String {
    match lang {
        Lang::Zh => format!("已移到回收站：{name}"),
        Lang::En => format!("Moved to Recycle Bin: {name}"),
    }
}

/// Status when moving to trash fails.
pub fn trash_failed(lang: Lang, e: &impl std::fmt::Display) -> String {
    match lang {
        Lang::Zh => format!("移到回收站失败：{e}"),
        Lang::En => format!("Move to Recycle Bin failed: {e}"),
    }
}

/// Status after a CSV export.
pub fn exported(lang: Lang, n: usize, dest: &str) -> String {
    match lang {
        Lang::Zh => format!("已导出 {n} 条记录 → {dest}"),
        Lang::En => format!("Exported {n} records → {dest}"),
    }
}

/// Status when a CSV export fails.
pub fn export_failed(lang: Lang, e: &impl std::fmt::Display) -> String {
    match lang {
        Lang::Zh => format!("导出失败：{e}"),
        Lang::En => format!("Export failed: {e}"),
    }
}

/// "{n} folders" footer under the folder tree.
pub fn folders_count(lang: Lang, n: usize) -> String {
    match lang {
        Lang::Zh => format!("{n} 个文件夹"),
        Lang::En => format!("{n} folders"),
    }
}

/// "{nrows} of {total} shown" footer under the flat table.
pub fn rows_shown(lang: Lang, nrows: usize, total: usize) -> String {
    match lang {
        Lang::Zh => format!("显示 {nrows} / 共 {total} 条"),
        Lang::En => format!("{nrows} of {total} shown"),
    }
}

/// "… {n} more" tree node cap label.
pub fn n_more(lang: Lang, n: usize) -> String {
    match lang {
        Lang::Zh => format!("… 还有 {n} 个"),
        Lang::En => format!("… {n} more"),
    }
}

/// "Top {n} folders by size" treemap footer.
pub fn top_n(lang: Lang, n: usize) -> String {
    tr(lang, S::TopN).replace("{n}", &n.to_string())
}

/// "{size} · 文件/文件夹" line in the delete confirmation dialog.
pub fn delete_meta(lang: Lang, size: &str, is_dir: bool) -> String {
    let kind = tr(lang, if is_dir { S::FolderWord } else { S::FileWord });
    format!("{size} · {kind}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every string id must resolve to a non-empty string in both languages.
    /// Update this list when adding variants to `S`.
    const ALL_IDS: [S; 27] = [
        S::Drive,
        S::Scan,
        S::Cancel,
        S::Cancelling,
        S::ExportCsv,
        S::Files,
        S::Folders,
        S::TreeView,
        S::FilterHint,
        S::ColName,
        S::ColSize,
        S::ColPct,
        S::NoData,
        S::NoSubfolders,
        S::Reveal,
        S::Trash,
        S::PermDeleteMenu,
        S::DeleteTitle,
        S::DeleteWarning,
        S::DeleteConfirm,
        S::FolderWord,
        S::FileWord,
        S::TopN,
        S::PickerTitle,
        S::PickerHint,
        S::StatusPickDrive,
        S::StatusScanCancelled,
    ];

    #[test]
    fn every_string_localized_and_nonempty() {
        assert_eq!(ALL_IDS.len(), 27);
        for id in ALL_IDS {
            let zh = tr(Lang::Zh, id);
            let en = tr(Lang::En, id);
            assert!(!zh.is_empty(), "empty Zh string for {id:?}");
            assert!(!en.is_empty(), "empty En string for {id:?}");
        }
    }

    #[test]
    fn toggle_label_names_other_language() {
        assert_eq!(Lang::Zh.toggle_label(), "🌐 EN");
        assert_eq!(Lang::En.toggle_label(), "🌐 中文");
    }

    #[test]
    fn formatters_zh() {
        let zh = Lang::Zh;
        assert_eq!(records(zh, 42), "42 条记录…");
        assert_eq!(
            free_of(zh, "10.0 GB", "100.0 GB"),
            "10.0 GB 可用，共 100.0 GB"
        );
        assert_eq!(scanning(zh, "C:"), "正在扫描 C:…");
        assert_eq!(
            done_status(zh, 10, 5, "1.0 GB", 2.5),
            "完成：10 个文件，5 个文件夹，共 1.0 GB，用时 2.5 秒"
        );
        assert_eq!(scan_failed(zh, "boom"), "扫描失败：boom");
        assert_eq!(perm_deleted(zh, "a.txt"), "已永久删除：a.txt");
        assert_eq!(perm_delete_failed(zh, &"boom"), "删除失败：boom");
        assert_eq!(trashed(zh, "a.txt"), "已移到回收站：a.txt");
        assert_eq!(trash_failed(zh, &"boom"), "移到回收站失败：boom");
        assert_eq!(exported(zh, 3, "out.csv"), "已导出 3 条记录 → out.csv");
        assert_eq!(export_failed(zh, &"boom"), "导出失败：boom");
        assert_eq!(folders_count(zh, 7), "7 个文件夹");
        assert_eq!(rows_shown(zh, 50, 200), "显示 50 / 共 200 条");
        assert_eq!(n_more(zh, 5), "… 还有 5 个");
        assert_eq!(top_n(zh, 200), "按大小排列的前 200 个文件夹");
        assert_eq!(delete_meta(zh, "1.0 MB", true), "1.0 MB · 文件夹");
        assert_eq!(delete_meta(zh, "1.0 MB", false), "1.0 MB · 文件");
    }

    #[test]
    fn formatters_en() {
        let en = Lang::En;
        assert_eq!(records(en, 42), "42 records…");
        assert_eq!(
            free_of(en, "10.0 GB", "100.0 GB"),
            "10.0 GB free of 100.0 GB"
        );
        assert_eq!(scanning(en, "C:"), "Scanning C: …");
        assert_eq!(
            done_status(en, 10, 5, "1.0 GB", 2.5),
            "Done: 10 files, 5 folders, 1.0 GB in 2.5s"
        );
        assert_eq!(scan_failed(en, "boom"), "Scan failed: boom");
        assert_eq!(perm_deleted(en, "a.txt"), "Deleted permanently: a.txt");
        assert_eq!(perm_delete_failed(en, &"boom"), "Delete failed: boom");
        assert_eq!(trashed(en, "a.txt"), "Moved to Recycle Bin: a.txt");
        assert_eq!(
            trash_failed(en, &"boom"),
            "Move to Recycle Bin failed: boom"
        );
        assert_eq!(exported(en, 3, "out.csv"), "Exported 3 records → out.csv");
        assert_eq!(export_failed(en, &"boom"), "Export failed: boom");
        assert_eq!(folders_count(en, 7), "7 folders");
        assert_eq!(rows_shown(en, 50, 200), "50 of 200 shown");
        assert_eq!(n_more(en, 5), "… 5 more");
        assert_eq!(top_n(en, 200), "Top 200 folders by size");
        assert_eq!(delete_meta(en, "1.0 MB", true), "1.0 MB · Folder");
        assert_eq!(delete_meta(en, "1.0 MB", false), "1.0 MB · File");
    }

    #[test]
    fn languages_differ() {
        // Every formatted string must actually be translated, not copied.
        assert_ne!(records(Lang::Zh, 1), records(Lang::En, 1));
        assert_ne!(scanning(Lang::Zh, "C:"), scanning(Lang::En, "C:"));
        assert_ne!(
            done_status(Lang::Zh, 1, 2, "1 B", 0.5),
            done_status(Lang::En, 1, 2, "1 B", 0.5)
        );
        // And every plain string id resolves differently per language,
        // except a few that are legitimately language-neutral.
        const LANG_NEUTRAL: [S; 1] = [S::ColPct]; // "%"
        for id in ALL_IDS {
            if LANG_NEUTRAL.contains(&id) {
                continue;
            }
            assert_ne!(
                tr(Lang::Zh, id),
                tr(Lang::En, id),
                "untranslated string for {id:?}"
            );
        }
    }
}
