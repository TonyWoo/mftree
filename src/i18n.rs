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
    TabTree,
    TabFiles,
    TabAbout,
    ColParentPct,
    ColAlloc,
    ColItems,
    ColModified,
    ColExt,
    ColFileType,
    NoExt,
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
        S::TabTree => match lang {
            Lang::Zh => "树查看",
            Lang::En => "Tree view",
        },
        S::TabFiles => match lang {
            Lang::Zh => "文件查看",
            Lang::En => "File view",
        },
        S::TabAbout => match lang {
            Lang::Zh => "关于",
            Lang::En => "About",
        },
        S::ColParentPct => match lang {
            Lang::Zh => "父级百分比",
            Lang::En => "% of parent",
        },
        S::ColAlloc => match lang {
            Lang::Zh => "分配",
            Lang::En => "Allocated",
        },
        S::ColItems => match lang {
            Lang::Zh => "项目",
            Lang::En => "Items",
        },
        S::ColModified => match lang {
            Lang::Zh => "修改时间",
            Lang::En => "Modified",
        },
        S::ColExt => match lang {
            Lang::Zh => "扩展名",
            Lang::En => "Extension",
        },
        S::ColFileType => match lang {
            Lang::Zh => "文件类型",
            Lang::En => "File type",
        },
        S::NoExt => match lang {
            Lang::Zh => "(无扩展名)",
            Lang::En => "(no extension)",
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

/// Status for a scan stage after raw record enumeration.
pub fn scan_phase(lang: Lang, phase: crate::mft::ScanPhase) -> String {
    match (lang, phase) {
        (Lang::Zh, crate::mft::ScanPhase::ReadingRecords) => "正在读取 MFT 记录…".to_string(),
        (Lang::Zh, crate::mft::ScanPhase::IndexingFolders) => "正在建立文件夹索引…".to_string(),
        (Lang::Zh, crate::mft::ScanPhase::AggregatingFolders) => "正在汇总文件夹大小…".to_string(),
        (Lang::Zh, crate::mft::ScanPhase::BuildingPaths) => "正在生成文件路径…".to_string(),
        (Lang::Zh, crate::mft::ScanPhase::PreparingResults) => "正在整理扫描结果…".to_string(),
        (Lang::En, crate::mft::ScanPhase::ReadingRecords) => "Reading MFT records…".to_string(),
        (Lang::En, crate::mft::ScanPhase::IndexingFolders) => "Building folder index…".to_string(),
        (Lang::En, crate::mft::ScanPhase::AggregatingFolders) => {
            "Calculating folder sizes…".to_string()
        }
        (Lang::En, crate::mft::ScanPhase::BuildingPaths) => "Building file paths…".to_string(),
        (Lang::En, crate::mft::ScanPhase::PreparingResults) => {
            "Preparing scan results…".to_string()
        }
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
