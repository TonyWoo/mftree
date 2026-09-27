//! Integration tests for the bilingual UI strings.

use sizetree::i18n::*;

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
