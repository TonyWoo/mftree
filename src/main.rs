//! sizetree — WizTree-style disk usage analyzer.
//!
//! Reads the NTFS $MFT directly on Windows; falls back to a directory walk
//! elsewhere. Shows the largest files/folders plus a treemap.

// Hide the console window on Windows release builds (GUI-only app).
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use eframe::egui;
use sizetree::{
    i18n::{self, tr, Lang, S},
    mft, treemap,
    util::{breadcrumb_segs, folder_color, hsv_to_rgb, human, parent_dir, path_under, short_name},
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;

#[derive(Clone)]
struct Item {
    path: String,
    size: u64,
    alloc: u64,
    modified: u64, // unix seconds, 0 = unknown
    is_dir: bool,
}

enum ScanMsg {
    Progress(u64),
    Done(Result<Vec<Item>, String>),
}

#[derive(PartialEq, Clone, Copy)]
enum ListTab {
    Files,
    Folders,
}

#[derive(PartialEq, Clone, Copy)]
enum SortCol {
    Name,
    Size,
    Pct,
    Alloc,
    Modified,
}

/// Top-level view tabs (WizTree-style).
#[derive(PartialEq, Clone, Copy)]
enum ViewTab {
    Tree,
    Files,
    About,
}

#[derive(PartialEq, Clone, Copy)]
enum TreeSortCol {
    Name,
    Pct,
    Size,
    Alloc,
    Items,
    Files,
    Folders,
    Modified,
}

#[derive(PartialEq, Clone, Copy)]
enum ExtSortCol {
    Ext,
    Pct,
    Size,
    Alloc,
    Count,
}

/// Per-extension aggregate for the right-hand stats panel.
struct ExtStat {
    ext: String,
    size: u64,
    alloc: u64,
    count: u64,
}

/// One visible row of the folder tree table.
struct TreeRow {
    idx: usize, // index into `dirs`
    indent: usize,
    parent_size: u64,
}

enum MenuAction {
    Reveal,
    Trash,
    PermDelete,
}

/// Free-space fraction below which a drive is flagged red.
const LOW_FREE_FRAC: f64 = 0.10;

/// Small disk-usage bar. The used portion turns red when free space drops
/// below [`LOW_FREE_FRAC`].
fn usage_bar(ui: &mut egui::Ui, total: u64, free: u64, width: f32) {
    let free_frac = if total > 0 {
        free as f64 / total as f64
    } else {
        1.0
    };
    let used_frac = (1.0 - free_frac).clamp(0.0, 1.0);
    let low = total > 0 && free_frac < LOW_FREE_FRAC;
    let h = 12.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, h), egui::Sense::hover());
    let fill = if low {
        egui::Color32::from_rgb(220, 70, 70)
    } else {
        egui::Color32::from_rgb(76, 141, 255)
    };
    let p = ui.painter();
    p.rect_filled(rect, 3.0, egui::Color32::from_rgb(44, 49, 58));
    let w = rect.width() * used_frac as f32;
    if w >= 2.0 {
        p.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(w, h)),
            3.0,
            fill,
        );
    }
}

/// Truncate `name` with a trailing '…' so its rendered width fits `max_w`,
/// keeping as many characters as possible. Width is measured with the real
/// font, so wide glyphs (e.g. CJK) truncate correctly too.
fn fit_label(painter: &egui::Painter, name: &str, font: &egui::FontId, max_w: f32) -> String {
    if max_w < 10.0 {
        return "…".to_string();
    }
    let full_w = painter
        .layout_no_wrap(name.to_string(), font.clone(), egui::Color32::WHITE)
        .size()
        .x;
    if full_w <= max_w {
        return name.to_string();
    }
    let chars: Vec<char> = name.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        let w = painter
            .layout_no_wrap(s, font.clone(), egui::Color32::WHITE)
            .size()
            .x;
        if w <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        "…".to_string()
    } else {
        chars[..lo].iter().collect::<String>() + "…"
    }
}

/// Reveal a path in the system file manager.
fn reveal(path: &str) {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .args(["/select,", path])
            .spawn()
            .ok();
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", path])
            .spawn()
            .ok();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let dir = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".to_string());
        std::process::Command::new("xdg-open").arg(dir).spawn().ok();
    }
}

fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
}

/// Clickable column header: click to sort by this column, click again to flip direction.
fn sort_header(ui: &mut egui::Ui, col: &mut SortCol, asc: &mut bool, this: SortCol, label: &str) {
    let arrow = if *col == this {
        if *asc {
            " ▲"
        } else {
            " ▼"
        }
    } else {
        ""
    };
    if ui.button(format!("{label}{arrow}")).clicked() {
        if *col == this {
            *asc = !*asc;
        } else {
            *col = this;
            *asc = this == SortCol::Name;
        }
    }
}

/// Generic clickable sort header for the tree / extension tables.
fn sort_header_gen<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    col: &mut T,
    asc: &mut bool,
    this: T,
    default_asc: bool,
    label: &str,
) {
    let arrow = if *col == this {
        if *asc {
            " ▲"
        } else {
            " ▼"
        }
    } else {
        ""
    };
    if ui.button(format!("{label}{arrow}")).clicked() {
        if *col == this {
            *asc = !*asc;
        } else {
            *col = this;
            *asc = default_asc;
        }
    }
}

/// Lowercased extension with leading dot (".dll"), or "" when none.
fn ext_of(path: &str) -> String {
    let name = short_name(path);
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() => format!(".{}", name[i + 1..].to_lowercase()),
        _ => String::new(),
    }
}

/// Human file-type name for an extension (WizTree-style).
fn file_type_name(ext: &str) -> String {
    match ext {
        ".exe" | ".msi" | ".bat" | ".cmd" | ".com" => "Application".to_string(),
        ".dll" | ".sys" | ".ocx" | ".cpl" => "Application ext".to_string(),
        ".jpg" | ".jpeg" => "JPEG File".to_string(),
        ".png" => "PNG File".to_string(),
        ".gif" => "GIF File".to_string(),
        ".bmp" | ".tif" | ".tiff" | ".webp" | ".ico" => "Image File".to_string(),
        ".mp4" | ".mkv" | ".avi" | ".mov" | ".wmv" | ".flv" => "Video File".to_string(),
        ".mp3" | ".wav" | ".flac" | ".aac" | ".ogg" | ".m4a" => "Audio File".to_string(),
        ".pdf" => "PDF File".to_string(),
        ".zip" | ".rar" | ".7z" | ".tar" | ".gz" | ".cab" => "Archive File".to_string(),
        ".txt" | ".log" | ".md" | ".csv" | ".ini" | ".cfg" => "Text File".to_string(),
        ".doc" | ".docx" | ".rtf" | ".odt" => "Document File".to_string(),
        ".xls" | ".xlsx" | ".ods" => "Spreadsheet File".to_string(),
        ".ppt" | ".pptx" => "Presentation File".to_string(),
        ".iso" | ".img" => "Disc Image File".to_string(),
        ".ttf" | ".otf" | ".woff" | ".woff2" => "Font File".to_string(),
        ".html" | ".htm" | ".css" | ".js" | ".ts" | ".json" | ".xml" => "Web File".to_string(),
        ".rs" | ".py" | ".c" | ".h" | ".cpp" | ".java" | ".go" | ".cs" => "Code File".to_string(),
        "" => "File".to_string(),
        _ => format!("{} File", ext.trim_start_matches('.').to_uppercase()),
    }
}

/// Stable color per extension; the treemap and the stats panel share it.
fn ext_color(ext: &str) -> egui::Color32 {
    let mut h: u32 = 0;
    for b in ext.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u32);
    }
    hsv_to_rgb((h % 360) as f32 / 360.0, 0.62, 0.82)
}

/// Unix seconds -> "YYYY-MM-DD HH:MM" in local time; "—" when unknown.
fn fmt_modified(unix: u64) -> String {
    if unix == 0 {
        return "—".to_string();
    }
    chrono::DateTime::from_timestamp(unix as i64, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".to_string())
}

/// Small horizontal percentage bar + label, WizTree-style.
fn pct_bar(ui: &mut egui::Ui, frac: f64, label: &str) {
    let frac = frac.clamp(0.0, 1.0);
    let (resp, painter) = ui.allocate_painter(egui::vec2(64.0, 10.0), egui::Sense::hover());
    let r = resp.rect;
    painter.rect_filled(r, 2.0, egui::Color32::from_rgb(52, 58, 66));
    if frac > 0.0 {
        let fill = egui::Rect::from_min_size(
            r.min,
            egui::vec2((r.width() * frac as f32).max(2.0), r.height()),
        );
        painter.rect_filled(fill, 2.0, egui::Color32::from_rgb(64, 130, 200));
    }
    ui.monospace(label);
}

#[derive(Clone)]
struct ConfirmDelete {
    path: String,
    size: u64,
    is_dir: bool,
}

#[derive(PartialEq, Clone)]
struct RowsKey {
    filter: String,
    sort_col: SortCol,
    sort_asc: bool,
    view_root: Option<String>,
    data_version: u64,
}

struct App {
    drives: Vec<String>,
    drive: String,
    last_drive: String,
    disk_total: u64,
    disk_free: u64,
    view_root: Option<String>,
    scroll_to_sel: bool,
    confirm_delete: Option<ConfirmDelete>,
    scanning: bool,
    progress: u64,
    files: Vec<Item>,
    dirs: Vec<Item>,
    total_size: u64,
    scan_secs: f64,
    selected: Option<(ListTab, usize)>,
    /// Top-level view tab.
    view: ViewTab,
    sort_col: SortCol,
    sort_asc: bool,
    filter: String,
    status: String,
    rx: Option<mpsc::Receiver<ScanMsg>>,
    cancel: Option<Arc<AtomicBool>>,
    started: Option<Instant>,
    /// Cached filtered+sorted row indices into `files`; recomputed
    /// only when `rows_key` inputs change (per-frame recompute over millions
    /// of rows stalls resize/drag repaints).
    rows_cache: Vec<usize>,
    rows_total: usize,
    rows_key: Option<RowsKey>,
    data_version: u64,
    /// Expanded folder paths in the folder tree.
    expanded: HashSet<String>,
    /// parent path -> child dir indices (size-desc); rebuilt on data change.
    dir_children: HashMap<String, Vec<usize>>,
    dir_index_version: u64,
    /// UI language (toggle in the top bar).
    lang: Lang,
    /// Folder tree: scroll the selected row into view (one-shot).
    tree_scroll_to_sel: bool,
    /// Folder tree table sorting.
    tree_sort_col: TreeSortCol,
    tree_sort_asc: bool,
    /// Extension stats panel (cached per data_version).
    ext_stats: Vec<ExtStat>,
    ext_version: u64,
    ext_sort_col: ExtSortCol,
    ext_sort_asc: bool,
    sel_ext: Option<String>,
    /// dir path -> (recursive file count, recursive subfolder count).
    dir_counts: HashMap<String, (u64, u64)>,
    dir_counts_version: u64,
    /// top-level treemap folder -> dominant extension (for ext coloring).
    top_ext: HashMap<String, String>,
    top_ext_version: u64,
}

impl App {
    fn new() -> Self {
        let drives = mft::list_drives();
        let drive = drives.first().cloned().unwrap_or_default();
        let mut app = Self {
            drives,
            drive: drive.clone(),
            last_drive: drive,
            disk_total: 0,
            disk_free: 0,
            view_root: None,
            scroll_to_sel: false,
            confirm_delete: None,
            scanning: false,
            progress: 0,
            files: Vec::new(),
            dirs: Vec::new(),
            total_size: 0,
            scan_secs: 0.0,
            selected: None,
            view: ViewTab::Tree,
            sort_col: SortCol::Size,
            sort_asc: false,
            filter: String::new(),
            status: tr(Lang::Zh, S::StatusPickDrive).to_string(),
            rx: None,
            cancel: None,
            started: None,
            rows_cache: Vec::new(),
            rows_total: 0,
            rows_key: None,
            data_version: 0,
            expanded: HashSet::new(),
            dir_children: HashMap::new(),
            dir_index_version: u64::MAX,
            lang: Lang::Zh,
            tree_scroll_to_sel: false,
            tree_sort_col: TreeSortCol::Size,
            tree_sort_asc: false,
            ext_stats: Vec::new(),
            ext_version: u64::MAX,
            ext_sort_col: ExtSortCol::Size,
            ext_sort_asc: false,
            sel_ext: None,
            dir_counts: HashMap::new(),
            dir_counts_version: u64::MAX,
            top_ext: HashMap::new(),
            top_ext_version: u64::MAX,
        };
        app.refresh_disk_space();
        app
    }

    fn refresh_disk_space(&mut self) {
        match mft::disk_space(&self.drive) {
            Ok((total, free)) => {
                self.disk_total = total;
                self.disk_free = free;
            }
            Err(_) => {
                self.disk_total = 0;
                self.disk_free = 0;
            }
        }
    }

    fn start_scan(&mut self) {
        if self.scanning {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.scanning = true;
        self.progress = 0;
        self.view_root = None;
        self.files.clear();
        self.dirs.clear();
        self.selected = None;
        self.sel_ext = None;
        // start with the drive root expanded in the folder tree
        self.expanded.clear();
        self.expanded.insert(self.drive_root());
        self.started = Some(Instant::now());
        let drive = self.drive.clone();
        self.status = i18n::scanning(self.lang, &drive);
        std::thread::spawn(move || {
            let r = mft::scan(&drive, &|n| {
                let _ = tx.send(ScanMsg::Progress(n));
                !cancel.load(Ordering::Relaxed)
            });
            let items = r.map(|v| {
                v.into_iter()
                    .map(|e| Item {
                        path: e.path,
                        size: e.size,
                        alloc: e.alloc,
                        modified: e.modified,
                        is_dir: e.is_dir,
                    })
                    .collect::<Vec<_>>()
            });
            let _ = tx.send(ScanMsg::Done(items));
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let mut done = None;
        if let Some(rx) = &self.rx {
            while let Ok(m) = rx.try_recv() {
                match m {
                    ScanMsg::Progress(n) => self.progress = n,
                    ScanMsg::Done(r) => {
                        done = Some(r);
                        break;
                    }
                }
            }
        }
        if let Some(r) = done {
            self.scanning = false;
            self.rx = None;
            self.cancel = None;
            self.refresh_disk_space();
            self.scan_secs = self
                .started
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or(0.0);
            match r {
                Ok(mut items) => {
                    items.sort_by_key(|a| std::cmp::Reverse(a.size));
                    self.total_size = items.iter().filter(|i| !i.is_dir).map(|i| i.size).sum();
                    self.dirs = items.iter().filter(|i| i.is_dir).cloned().collect();
                    self.files = items.into_iter().filter(|i| !i.is_dir).collect();
                    self.data_version += 1;
                    let nf = self.files.len();
                    let nd = self.dirs.len();
                    self.status = i18n::done_status(
                        self.lang,
                        nf,
                        nd,
                        human(self.total_size).trim(),
                        self.scan_secs,
                    );
                }
                Err(e) if e == "cancelled" => {
                    self.status = tr(self.lang, S::StatusScanCancelled).to_string()
                }
                Err(e) => self.status = i18n::scan_failed(self.lang, &e),
            }
        } else if self.scanning {
            ctx.request_repaint();
        }
    }

    fn pct_str(size: u64, total: u64) -> String {
        if total == 0 {
            "  0.0%".to_string()
        } else {
            format!("{:5.1}%", size as f64 / total as f64 * 100.0)
        }
    }

    /// Filtered + sorted rows as (original index, item); also returns the
    /// total match count before the 2000-row display cap.
    fn filter_sort<'a>(
        items: &'a [Item],
        filter: &str,
        col: SortCol,
        asc: bool,
        root: Option<&str>,
    ) -> (Vec<(usize, &'a Item)>, usize) {
        let q = filter.to_lowercase();
        let mut rows: Vec<(usize, &Item)> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                (q.is_empty() || it.path.to_lowercase().contains(&q))
                    && root.map(|r| path_under(&it.path, r)).unwrap_or(true)
            })
            .collect();
        let total = rows.len();
        match col {
            SortCol::Name => {
                rows.sort_by(|a, b| {
                    short_name(&a.1.path)
                        .to_lowercase()
                        .cmp(&short_name(&b.1.path).to_lowercase())
                });
            }
            SortCol::Size | SortCol::Pct => rows.sort_by_key(|(_, it)| it.size),
            SortCol::Alloc => rows.sort_by_key(|(_, it)| it.alloc),
            SortCol::Modified => rows.sort_by_key(|(_, it)| it.modified),
        }
        if !asc {
            rows.reverse();
        }
        rows.truncate(2000);
        (rows, total)
    }

    /// Remove an item (and, for dirs, everything under it) from the in-memory
    /// lists after a successful delete. No rescan needed.
    fn remove_item(&mut self, path: &str, is_dir: bool) {
        let mut freed = 0u64;
        if is_dir {
            self.files.retain(|i| {
                let gone = i.path == path || path_under(&i.path, path);
                if gone {
                    freed += i.size;
                }
                !gone
            });
            self.dirs
                .retain(|i| !(i.path == path || path_under(&i.path, path)));
        } else if let Some(pos) = self.files.iter().position(|i| i.path == path) {
            freed = self.files[pos].size;
            self.files.remove(pos);
        }
        self.total_size = self.total_size.saturating_sub(freed);
        self.selected = None;
        self.data_version += 1;
    }

    fn permanent_delete(&mut self) {
        if let Some(cd) = self.confirm_delete.take() {
            let r = if cd.is_dir {
                std::fs::remove_dir_all(&cd.path)
            } else {
                std::fs::remove_file(&cd.path)
            };
            match r {
                Ok(_) => {
                    self.remove_item(&cd.path, cd.is_dir);
                    self.status = i18n::perm_deleted(self.lang, short_name(&cd.path));
                }
                Err(e) => self.status = i18n::perm_delete_failed(self.lang, &e),
            }
        }
    }

    fn move_to_trash(&mut self, path: &str, is_dir: bool) {
        match trash::delete(path) {
            Ok(_) => {
                self.remove_item(path, is_dir);
                self.status = i18n::trashed(self.lang, short_name(path));
            }
            Err(e) => self.status = i18n::trash_failed(self.lang, &e),
        }
    }

    fn export_csv(&mut self) {
        let (kind, items): (&str, &[Item]) = match self.view {
            ViewTab::Files => ("file", &self.files),
            ViewTab::Tree => ("folder", &self.dirs),
            ViewTab::About => return,
        };
        let root = self.view_root.clone();
        let mut out = String::from(
            "Path,Size (bytes),Allocated (bytes),Size,Allocated,% of disk,Modified,Type\n",
        );
        let mut n = 0usize;
        for it in items {
            if let Some(r) = &root {
                if !path_under(&it.path, r) {
                    continue;
                }
            }
            n += 1;
            let pct = if self.total_size == 0 {
                0.0
            } else {
                it.size as f64 / self.total_size as f64 * 100.0
            };
            out.push_str(&format!(
                "\"{}\",{},{},{},{},{:.1}%,{},{}\n",
                it.path.replace('"', "\"\""),
                it.size,
                it.alloc,
                human(it.size).trim(),
                human(it.alloc).trim(),
                pct,
                fmt_modified(it.modified),
                kind
            ));
        }
        let drive_tag: String = self
            .drive
            .trim_end_matches(':')
            .replace(['\\', '/'], "_")
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        let fname = format!("sizetree_{drive_tag}.csv");
        let dest = home_dir()
            .map(|h| format!("{h}/{fname}"))
            .unwrap_or(fname.clone());
        match std::fs::write(&dest, out) {
            Ok(_) => self.status = i18n::exported(self.lang, n, &dest),
            Err(e) => self.status = i18n::export_failed(self.lang, &e),
        }
    }

    fn delete_modal(&mut self, ctx: &egui::Context) {
        let cd = match self.confirm_delete.clone() {
            Some(cd) => cd,
            None => return,
        };
        let mut close = false;
        let mut confirmed = false;
        egui::Window::new(tr(self.lang, S::DeleteTitle))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&cd.path);
                ui.label(
                    egui::RichText::new(i18n::delete_meta(
                        self.lang,
                        human(cd.size).trim(),
                        cd.is_dir,
                    ))
                    .weak(),
                );
                ui.label(
                    egui::RichText::new(tr(self.lang, S::DeleteWarning))
                        .color(egui::Color32::from_rgb(255, 150, 150)),
                );
                ui.horizontal(|ui| {
                    if ui.button(tr(self.lang, S::Cancel)).clicked() {
                        close = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(tr(self.lang, S::DeleteConfirm))
                                .fill(egui::Color32::from_rgb(180, 60, 60)),
                        )
                        .clicked()
                    {
                        confirmed = true;
                    }
                });
            });
        if confirmed {
            self.permanent_delete();
        } else if close {
            self.confirm_delete = None;
        }
    }

    /// Breadcrumb for the drilled-in folder (shared by tree and file views).
    /// Returns true if the view root changed.
    fn draw_breadcrumb(&mut self, ui: &mut egui::Ui) -> bool {
        if self.view_root.is_none() {
            return false;
        }
        let segs = breadcrumb_segs(self.view_root.as_deref().unwrap_or(""));
        let drive_root = self.drive.trim_end_matches(':').to_string();
        let mut go: Option<Option<String>> = None;
        ui.horizontal(|ui| {
            let n = segs.len();
            for (i, (name, full)) in segs.iter().enumerate() {
                if ui.small_button(name).clicked() {
                    go = Some(if full.trim_end_matches(':') == drive_root {
                        None
                    } else {
                        Some(full.clone())
                    });
                }
                if i + 1 < n {
                    ui.label(egui::RichText::new("›").weak());
                }
            }
            if ui.small_button("✕").clicked() {
                go = Some(None);
            }
        });
        if let Some(g) = go {
            match g {
                Some(p) => {
                    self.view_root = Some(p.clone());
                    self.sync_tree_to(&p, None);
                }
                None => {
                    self.view_root = None;
                    self.selected = None;
                }
            }
            ui.separator();
            return true;
        }
        ui.separator();
        false
    }

    /// File view: flat table of files (WizTree "File view" tab).
    fn draw_file_view(&mut self, ui: &mut egui::Ui) {
        self.draw_breadcrumb(ui);
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text(tr(self.lang, S::FilterHint))
                .desired_width(f32::INFINITY),
        );
        let mut col = self.sort_col;
        let mut asc = self.sort_asc;
        let root = self.view_root.clone();
        let scanning = self.scanning;
        let key = RowsKey {
            filter: self.filter.clone(),
            sort_col: col,
            sort_asc: asc,
            view_root: root.clone(),
            data_version: self.data_version,
        };
        if self.rows_key.as_ref() != Some(&key) {
            let (rows, total) =
                Self::filter_sort(&self.files, &self.filter, col, asc, root.as_deref());
            self.rows_cache = rows.into_iter().map(|(i, _)| i).collect();
            self.rows_total = total;
            self.rows_key = Some(key);
        }
        let mut menu_hit: Option<(MenuAction, String, u64, bool)> = None;
        let total_size = self.total_size;
        let total = self.rows_total;
        let nrows = self.rows_cache.len();
        let lang = self.lang;
        // Reserve room for the footer label below the table.
        let table_h = (ui.available_height() - 22.0).max(80.0);
        egui_extras::TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(egui_extras::Column::initial(400.0).clip(true))
            .column(egui_extras::Column::initial(110.0).clip(true))
            .column(egui_extras::Column::initial(90.0).clip(true))
            .column(egui_extras::Column::initial(90.0).clip(true))
            .column(egui_extras::Column::initial(130.0).clip(true))
            .max_scroll_height(table_h)
            .header(20.0, |mut header| {
                header.col(|ui| {
                    sort_header(ui, &mut col, &mut asc, SortCol::Name, tr(lang, S::ColName))
                });
                header.col(|ui| {
                    sort_header(ui, &mut col, &mut asc, SortCol::Size, tr(lang, S::ColSize))
                });
                header.col(|ui| {
                    sort_header(
                        ui,
                        &mut col,
                        &mut asc,
                        SortCol::Alloc,
                        tr(lang, S::ColAlloc),
                    )
                });
                header.col(|ui| {
                    sort_header(ui, &mut col, &mut asc, SortCol::Pct, tr(lang, S::ColPct))
                });
                header.col(|ui| {
                    sort_header(
                        ui,
                        &mut col,
                        &mut asc,
                        SortCol::Modified,
                        tr(lang, S::ColModified),
                    )
                });
            })
            .body(|body| {
                body.rows(18.0, nrows, |mut row| {
                    let idx = self.rows_cache[row.index()];
                    let it = &self.files[idx];
                    let sel = self.selected == Some((ListTab::Files, idx));
                    row.col(|ui| {
                        let label = short_name(&it.path);
                        let resp = ui.selectable_label(sel, label);
                        if resp.clicked() {
                            self.selected = Some((ListTab::Files, idx));
                        }
                        if sel && self.scroll_to_sel {
                            ui.scroll_to_rect(resp.rect, Some(egui::Align::Center));
                        }
                        let path = it.path.clone();
                        let size = it.size;
                        let mut action = None;
                        resp.context_menu(|ui| {
                            if ui.button(tr(lang, S::Reveal)).clicked() {
                                action = Some(MenuAction::Reveal);
                            }
                            ui.separator();
                            if ui
                                .add_enabled(!scanning, egui::Button::new(tr(lang, S::Trash)))
                                .clicked()
                            {
                                action = Some(MenuAction::Trash);
                            }
                            if ui
                                .add_enabled(
                                    !scanning,
                                    egui::Button::new(tr(lang, S::PermDeleteMenu)),
                                )
                                .clicked()
                            {
                                action = Some(MenuAction::PermDelete);
                            }
                        });
                        resp.on_hover_text(&path);
                        if let Some(a) = action {
                            menu_hit = Some((a, path, size, false));
                        }
                    });
                    row.col(|ui| {
                        ui.monospace(human(it.size));
                    });
                    row.col(|ui| {
                        ui.monospace(human(it.alloc));
                    });
                    row.col(|ui| {
                        ui.monospace(Self::pct_str(it.size, total_size));
                    });
                    row.col(|ui| {
                        ui.monospace(fmt_modified(it.modified));
                    });
                });
            });
        ui.label(
            egui::RichText::new(i18n::rows_shown(self.lang, nrows, total))
                .small()
                .weak(),
        );
        self.scroll_to_sel = false;
        if let Some((a, path, size, is_dir)) = menu_hit {
            match a {
                MenuAction::Reveal => reveal(&path),
                MenuAction::Trash => self.move_to_trash(&path, is_dir),
                MenuAction::PermDelete => {
                    self.confirm_delete = Some(ConfirmDelete { path, size, is_dir })
                }
            }
        }
        self.sort_col = col;
        self.sort_asc = asc;
    }

    /// Tree view: folder hierarchy with WizTree columns (left pane).
    fn draw_tree_view(&mut self, ui: &mut egui::Ui) {
        if self.dirs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(tr(self.lang, S::NoData));
            });
            return;
        }
        self.ensure_dir_index();
        self.ensure_dir_counts();
        self.draw_breadcrumb(ui);

        // Flatten the visible (expanded) rows. WizTree-style: the drive root
        // itself is not a row; its children are the top level.
        let root = self.drive_root();
        let mut rows: Vec<TreeRow> = Vec::new();
        if let Some(ridx) = self.dir_idx(&root) {
            let root_size = self.dirs[ridx].size;
            self.collect_tree_rows(&root, 0, root_size, &mut rows);
        }

        let mut tcol = self.tree_sort_col;
        let mut tasc = self.tree_sort_asc;
        let mut drill: Option<(String, usize)> = None;
        let mut menu_hit: Option<(MenuAction, String, u64, bool)> = None;
        let lang = self.lang;
        let scanning = self.scanning;
        let nrows = rows.len();
        // Reserve room for the footer label below the table.
        let table_h = (ui.available_height() - 22.0).max(80.0);
        egui_extras::TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(
                egui_extras::Column::initial(150.0)
                    .clip(true)
                    .resizable(true),
            )
            .column(egui_extras::Column::initial(90.0).clip(true))
            .column(egui_extras::Column::initial(65.0).clip(true))
            .column(egui_extras::Column::initial(65.0).clip(true))
            .column(egui_extras::Column::initial(45.0).clip(true))
            .column(egui_extras::Column::initial(45.0).clip(true))
            .column(egui_extras::Column::initial(45.0).clip(true))
            .column(egui_extras::Column::initial(90.0).clip(true))
            .max_scroll_height(table_h)
            .header(20.0, |mut header| {
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut tcol,
                        &mut tasc,
                        TreeSortCol::Name,
                        true,
                        tr(lang, S::ColName),
                    )
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut tcol,
                        &mut tasc,
                        TreeSortCol::Pct,
                        false,
                        tr(lang, S::ColParentPct),
                    )
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut tcol,
                        &mut tasc,
                        TreeSortCol::Size,
                        false,
                        tr(lang, S::ColSize),
                    )
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut tcol,
                        &mut tasc,
                        TreeSortCol::Alloc,
                        false,
                        tr(lang, S::ColAlloc),
                    )
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut tcol,
                        &mut tasc,
                        TreeSortCol::Items,
                        false,
                        tr(lang, S::ColItems),
                    )
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut tcol,
                        &mut tasc,
                        TreeSortCol::Files,
                        false,
                        tr(lang, S::Files),
                    )
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut tcol,
                        &mut tasc,
                        TreeSortCol::Folders,
                        false,
                        tr(lang, S::Folders),
                    )
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut tcol,
                        &mut tasc,
                        TreeSortCol::Modified,
                        false,
                        tr(lang, S::ColModified),
                    )
                });
            })
            .body(|body| {
                body.rows(20.0, nrows, |mut row| {
                    let r = &rows[row.index()];
                    let d = &self.dirs[r.idx];
                    let (fc, dc) = self.dir_counts.get(&d.path).copied().unwrap_or((0, 0));
                    let sel = self.selected == Some((ListTab::Folders, r.idx));
                    row.col(|ui| {
                        ui.add_space(r.indent as f32 * 14.0);
                        let has_kids = self
                            .dir_children
                            .get(&d.path)
                            .map(|v| !v.is_empty())
                            .unwrap_or(false);
                        let is_exp = self.expanded.contains(&d.path);
                        if has_kids {
                            if ui.small_button(if is_exp { "▼" } else { "▶" }).clicked() {
                                if is_exp {
                                    self.expanded.remove(&d.path);
                                } else {
                                    self.expanded.insert(d.path.clone());
                                }
                            }
                        } else {
                            ui.add_space(18.0);
                        }
                        let resp = ui.selectable_label(sel, short_name(&d.path));
                        if resp.clicked() {
                            self.selected = Some((ListTab::Folders, r.idx));
                        }
                        if resp.double_clicked() {
                            drill = Some((d.path.clone(), r.idx));
                        }
                        if sel && self.tree_scroll_to_sel {
                            ui.scroll_to_rect(resp.rect, Some(egui::Align::Center));
                        }
                        let path = d.path.clone();
                        let size = d.size;
                        let mut action = None;
                        resp.context_menu(|ui| {
                            if ui.button(tr(lang, S::Reveal)).clicked() {
                                action = Some(MenuAction::Reveal);
                            }
                            ui.separator();
                            if ui
                                .add_enabled(!scanning, egui::Button::new(tr(lang, S::Trash)))
                                .clicked()
                            {
                                action = Some(MenuAction::Trash);
                            }
                            if ui
                                .add_enabled(
                                    !scanning,
                                    egui::Button::new(tr(lang, S::PermDeleteMenu)),
                                )
                                .clicked()
                            {
                                action = Some(MenuAction::PermDelete);
                            }
                        });
                        resp.on_hover_text(&path);
                        if let Some(a) = action {
                            menu_hit = Some((a, path, size, true));
                        }
                    });
                    row.col(|ui| {
                        let frac = d.size as f64 / r.parent_size.max(1) as f64;
                        pct_bar(ui, frac, &Self::pct_str(d.size, r.parent_size));
                    });
                    row.col(|ui| {
                        ui.monospace(human(d.size));
                    });
                    row.col(|ui| {
                        ui.monospace(human(d.alloc));
                    });
                    row.col(|ui| {
                        ui.monospace(format!("{}", fc + dc));
                    });
                    row.col(|ui| {
                        ui.monospace(format!("{}", fc));
                    });
                    row.col(|ui| {
                        ui.monospace(format!("{}", dc));
                    });
                    row.col(|ui| {
                        ui.monospace(fmt_modified(d.modified));
                    });
                });
            });
        ui.label(
            egui::RichText::new(i18n::folders_count(self.lang, self.dirs.len()))
                .small()
                .weak(),
        );
        self.tree_scroll_to_sel = false;
        if let Some((d, idx)) = drill {
            self.drill_into(d, Some(idx));
        }
        if let Some((a, path, size, is_dir)) = menu_hit {
            match a {
                MenuAction::Reveal => reveal(&path),
                MenuAction::Trash => self.move_to_trash(&path, is_dir),
                MenuAction::PermDelete => {
                    self.confirm_delete = Some(ConfirmDelete { path, size, is_dir })
                }
            }
        }
        self.tree_sort_col = tcol;
        self.tree_sort_asc = tasc;
    }

    /// Recursively flatten expanded children of `parent` into `rows`,
    /// sorting siblings per the current tree sort column.
    fn collect_tree_rows(
        &self,
        parent: &str,
        indent: usize,
        parent_size: u64,
        rows: &mut Vec<TreeRow>,
    ) {
        let Some(kids) = self.dir_children.get(parent) else {
            return;
        };
        let mut kids: Vec<usize> = kids.clone();
        let col = self.tree_sort_col;
        let asc = self.tree_sort_asc;
        kids.sort_by(|&a, &b| {
            let (da, db) = (&self.dirs[a], &self.dirs[b]);
            let ord = match col {
                TreeSortCol::Name => short_name(&da.path)
                    .to_lowercase()
                    .cmp(&short_name(&db.path).to_lowercase()),
                TreeSortCol::Size | TreeSortCol::Pct => da.size.cmp(&db.size),
                TreeSortCol::Alloc => da.alloc.cmp(&db.alloc),
                TreeSortCol::Items => {
                    let ka = self
                        .dir_counts
                        .get(&da.path)
                        .map(|(f, d)| f + d)
                        .unwrap_or(0);
                    let kb = self
                        .dir_counts
                        .get(&db.path)
                        .map(|(f, d)| f + d)
                        .unwrap_or(0);
                    ka.cmp(&kb)
                }
                TreeSortCol::Files => {
                    let ka = self.dir_counts.get(&da.path).map(|c| c.0).unwrap_or(0);
                    let kb = self.dir_counts.get(&db.path).map(|c| c.0).unwrap_or(0);
                    ka.cmp(&kb)
                }
                TreeSortCol::Folders => {
                    let ka = self.dir_counts.get(&da.path).map(|c| c.1).unwrap_or(0);
                    let kb = self.dir_counts.get(&db.path).map(|c| c.1).unwrap_or(0);
                    ka.cmp(&kb)
                }
                TreeSortCol::Modified => da.modified.cmp(&db.modified),
            };
            if asc {
                ord
            } else {
                ord.reverse()
            }
        });
        for idx in kids {
            let d = &self.dirs[idx];
            rows.push(TreeRow {
                idx,
                indent,
                parent_size,
            });
            if self.expanded.contains(&d.path) {
                self.collect_tree_rows(&d.path, indent + 1, d.size, rows);
            }
        }
    }

    /// Extension stats panel (right pane of the tree view).
    fn draw_ext_view(&mut self, ui: &mut egui::Ui) {
        if self.files.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(tr(self.lang, S::NoData));
            });
            return;
        }
        self.ensure_ext_stats();
        let mut col = self.ext_sort_col;
        let mut asc = self.ext_sort_asc;
        let mut order: Vec<usize> = (0..self.ext_stats.len()).collect();
        order.sort_by(|&a, &b| {
            let (ea, eb) = (&self.ext_stats[a], &self.ext_stats[b]);
            let ord = match col {
                ExtSortCol::Ext => ea.ext.cmp(&eb.ext),
                ExtSortCol::Pct | ExtSortCol::Size => ea.size.cmp(&eb.size),
                ExtSortCol::Alloc => ea.alloc.cmp(&eb.alloc),
                ExtSortCol::Count => ea.count.cmp(&eb.count),
            };
            if asc {
                ord
            } else {
                ord.reverse()
            }
        });
        let lang = self.lang;
        let total = self.total_size;
        let nrows = order.len();
        let table_h = (ui.available_height() - 22.0).max(80.0);
        egui_extras::TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(egui_extras::Column::initial(70.0).clip(true))
            .column(egui_extras::Column::initial(85.0).clip(true))
            .column(egui_extras::Column::initial(100.0).clip(true))
            .column(egui_extras::Column::initial(60.0).clip(true))
            .column(egui_extras::Column::initial(55.0).clip(true))
            .column(egui_extras::Column::initial(45.0).clip(true))
            .max_scroll_height(table_h)
            .header(20.0, |mut header| {
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut col,
                        &mut asc,
                        ExtSortCol::Ext,
                        true,
                        tr(lang, S::ColExt),
                    )
                });
                header.col(|ui| {
                    ui.label(tr(lang, S::ColFileType));
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut col,
                        &mut asc,
                        ExtSortCol::Pct,
                        false,
                        tr(lang, S::ColPct),
                    )
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut col,
                        &mut asc,
                        ExtSortCol::Size,
                        false,
                        tr(lang, S::ColSize),
                    )
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut col,
                        &mut asc,
                        ExtSortCol::Alloc,
                        false,
                        tr(lang, S::ColAlloc),
                    )
                });
                header.col(|ui| {
                    sort_header_gen(
                        ui,
                        &mut col,
                        &mut asc,
                        ExtSortCol::Count,
                        false,
                        tr(lang, S::Files),
                    )
                });
            })
            .body(|body| {
                body.rows(20.0, nrows, |mut row| {
                    let st = &self.ext_stats[order[row.index()]];
                    let sel = self.sel_ext.as_deref() == Some(st.ext.as_str());
                    row.col(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2.0, ext_color(&st.ext));
                        let ext_label = if st.ext.is_empty() {
                            tr(lang, S::NoExt).to_string()
                        } else {
                            st.ext.clone()
                        };
                        let resp = ui.selectable_label(sel, ext_label);
                        if resp.clicked() {
                            self.sel_ext = Some(st.ext.clone());
                        }
                        resp.on_hover_text(file_type_name(&st.ext));
                    });
                    row.col(|ui| {
                        ui.label(file_type_name(&st.ext));
                    });
                    row.col(|ui| {
                        let frac = st.size as f64 / total.max(1) as f64;
                        pct_bar(ui, frac, &Self::pct_str(st.size, total));
                    });
                    row.col(|ui| {
                        ui.monospace(human(st.size));
                    });
                    row.col(|ui| {
                        ui.monospace(human(st.alloc));
                    });
                    row.col(|ui| {
                        ui.monospace(format!("{}", st.count));
                    });
                });
            });
        ui.label(
            egui::RichText::new(format!("{nrows} {}", tr(self.lang, S::ColExt)))
                .small()
                .weak(),
        );
        self.ext_sort_col = col;
        self.ext_sort_asc = asc;
    }

    /// About tab.
    fn draw_about(&self, ui: &mut egui::Ui) {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.heading("sizetree");
                ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
                ui.add_space(8.0);
                ui.label(match self.lang {
                    Lang::Zh => "WizTree 风格的磁盘空间分析工具",
                    Lang::En => "WizTree-style disk space analyzer",
                });
                ui.add_space(8.0);
                ui.hyperlink_to(
                    "github.com/TonyWoo/sizetree",
                    "https://github.com/TonyWoo/sizetree",
                );
                ui.add_space(8.0);
                ui.label(match self.lang {
                    Lang::Zh => {
                        "Windows 下直读 NTFS $MFT 实现毫秒级扫描；\n其他平台使用目录递归扫描。\nRust + egui 构建。"
                    }
                    Lang::En => {
                        "Reads the NTFS $MFT directly on Windows for millisecond scans;\nfalls back to a directory walk elsewhere.\nBuilt with Rust + egui."
                    }
                });
            });
        });
    }

    /// Rebuild the parent -> children folder index when scan data changed.
    fn ensure_dir_index(&mut self) {
        if self.dir_index_version == self.data_version {
            return;
        }
        let mut map: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, d) in self.dirs.iter().enumerate() {
            let p = parent_dir(&d.path);
            if p == d.path {
                continue; // drive root is its own parent; skip
            }
            map.entry(p.to_string()).or_default().push(i);
        }
        // self.dirs is size-desc from the scan, so each child list is too.
        self.dir_children = map;
        self.dir_index_version = self.data_version;
    }

    /// Current view root as a plain path ("C:" at drive root).
    fn treemap_root(&self) -> String {
        match &self.view_root {
            Some(r) => r.clone(),
            None => self.drive_root(),
        }
    }

    /// Drive root path ("C:" on Windows, "/" elsewhere). The folder tree is
    /// always rooted here so a treemap drill can reveal the folder's place
    /// in the full hierarchy.
    fn drive_root(&self) -> String {
        let d = self.drive.trim_end_matches(':');
        if d == "/" || d.is_empty() {
            "/".to_string()
        } else {
            format!("{d}:")
        }
    }

    /// Linear index lookup of a folder path in `self.dirs` (size-desc).
    fn dir_idx(&self, path: &str) -> Option<usize> {
        self.dirs.iter().position(|d| d.path == path)
    }

    /// Sync the folder tree to `path`: expand the ancestor chain, select the
    /// folder and scroll it into view.
    fn sync_tree_to(&mut self, path: &str, idx: Option<usize>) {
        let idx = idx.or_else(|| self.dir_idx(path));
        self.selected = idx.map(|i| (ListTab::Folders, i));
        self.ensure_dir_index();
        let root = self.drive_root();
        let mut p = path.to_string();
        loop {
            self.expanded.insert(p.clone());
            if p == root {
                break;
            }
            let up = parent_dir(&p).to_string();
            if up == p {
                break; // safety; shouldn't happen
            }
            p = up;
        }
        self.tree_scroll_to_sel = true;
    }

    /// Drill into a folder: re-root the treemap/list views there and sync
    /// the folder tree to it.
    fn drill_into(&mut self, path: String, idx: Option<usize>) {
        self.view_root = Some(path.clone());
        self.view = ViewTab::Tree;
        self.sync_tree_to(&path, idx);
    }

    /// (files, subfolders) counts per dir, recursive; rebuilt per scan.
    fn ensure_dir_counts(&mut self) {
        if self.dir_counts_version == self.data_version {
            return;
        }
        let mut counts: HashMap<String, (u64, u64)> = HashMap::new();
        let root = self.drive_root();
        for f in &self.files {
            let mut p = parent_dir(&f.path).to_string();
            loop {
                let e = counts.entry(p.clone()).or_insert((0, 0));
                e.0 += 1;
                if p == root {
                    break;
                }
                let up = parent_dir(&p).to_string();
                if up == p {
                    break;
                }
                p = up;
            }
        }
        for d in &self.dirs {
            if d.path == root {
                continue;
            }
            let mut p = parent_dir(&d.path).to_string();
            loop {
                let e = counts.entry(p.clone()).or_insert((0, 0));
                e.1 += 1;
                if p == root {
                    break;
                }
                let up = parent_dir(&p).to_string();
                if up == p {
                    break;
                }
                p = up;
            }
        }
        self.dir_counts = counts;
        self.dir_counts_version = self.data_version;
    }

    /// Extension aggregates over all files; rebuilt per scan.
    fn ensure_ext_stats(&mut self) {
        if self.ext_version == self.data_version {
            return;
        }
        let mut map: HashMap<String, (u64, u64, u64)> = HashMap::new();
        for f in &self.files {
            let e = ext_of(&f.path);
            let v = map.entry(e).or_insert((0, 0, 0));
            v.0 += f.size;
            v.1 += f.alloc;
            v.2 += 1;
        }
        self.ext_stats = map
            .into_iter()
            .map(|(ext, (size, alloc, count))| ExtStat {
                ext,
                size,
                alloc,
                count,
            })
            .collect();
        self.ext_version = self.data_version;
    }

    /// Dominant (largest-size) extension per top-level treemap folder, so
    /// the treemap can share the extension stats panel's colors.
    fn ensure_top_ext(&mut self) {
        if self.top_ext_version == self.data_version {
            return;
        }
        let root = self.treemap_root();
        let mut per_dir: HashMap<String, HashMap<String, u64>> = HashMap::new();
        for f in &self.files {
            if !path_under(&f.path, &root) {
                continue;
            }
            let rel = &f.path[root.len().min(f.path.len())..];
            let rel = rel.trim_start_matches(['\\', '/']);
            let top_seg = rel.split(['\\', '/']).next().unwrap_or("");
            if top_seg.is_empty() {
                continue;
            }
            let sep = if root.contains('\\') { '\\' } else { '/' };
            let top = if root.ends_with(sep) {
                format!("{root}{top_seg}")
            } else {
                format!("{root}{sep}{top_seg}")
            };
            // only folders (treemap shows folders); skip bare files at root
            if self.dir_idx(&top).is_none() {
                continue;
            }
            *per_dir
                .entry(top)
                .or_default()
                .entry(ext_of(&f.path))
                .or_insert(0) += f.size;
        }
        self.top_ext = per_dir
            .into_iter()
            .filter_map(|(dir, m)| {
                m.into_iter()
                    .max_by_key(|(_, s)| *s)
                    .map(|(ext, _)| (dir, ext))
            })
            .collect();
        self.top_ext_version = self.data_version;
    }

    /// Nested treemap of folders: child folders of the current view root,
    /// sized by total size, with subfolders nested inside (up to 3 levels).
    /// Color encodes size (blue -> yellow -> red). Click drills in;
    /// right-click opens the folder menu.
    fn draw_treemap(&mut self, ui: &mut egui::Ui) {
        if self.dirs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(tr(self.lang, S::NoData));
            });
            return;
        }
        self.ensure_dir_index();
        self.ensure_top_ext();
        let root = self.treemap_root();
        let top: Vec<usize> = self
            .dir_children
            .get(&root)
            .map(|v| v.iter().take(200).copied().collect())
            .unwrap_or_default();
        if top.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(tr(self.lang, S::NoSubfolders));
            });
            return;
        }
        let avail = ui.available_size();
        let tm_h = (avail.y - 34.0).max(60.0);
        let (resp, painter) = ui.allocate_painter(egui::vec2(avail.x, tm_h), egui::Sense::hover());
        let origin = resp.rect.min;
        let painter = painter.clone();
        let mut menu_hit: Option<(MenuAction, String, u64, bool)> = None;
        let mut drill: Option<(String, usize)> = None;
        self.draw_treemap_level(
            ui,
            &painter,
            origin,
            &top,
            0.0,
            0.0,
            avail.x as f64,
            tm_h as f64,
            0,
            &mut menu_hit,
            &mut drill,
        );
        if let Some((d, idx)) = drill {
            self.drill_into(d, Some(idx));
        }
        if let Some((a, path, size, is_dir)) = menu_hit {
            match a {
                MenuAction::Reveal => reveal(&path),
                MenuAction::Trash => self.move_to_trash(&path, is_dir),
                MenuAction::PermDelete => {
                    self.confirm_delete = Some(ConfirmDelete { path, size, is_dir })
                }
            }
        }

        // how many folders are shown in the treemap
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(i18n::top_n(self.lang, top.len()))
                    .small()
                    .weak(),
            );
        });
    }

    /// Draw one treemap level; recurses into subfolders for big-enough rects.
    /// Colors are scaled per level, so siblings are always blue -> red by
    /// their relative sizes within the current folder.
    #[allow(clippy::too_many_arguments)]
    fn draw_treemap_level(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        origin: egui::Pos2,
        indices: &[usize],
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        depth: usize,
        menu_hit: &mut Option<(MenuAction, String, u64, bool)>,
        drill: &mut Option<(String, usize)>,
    ) {
        if indices.is_empty() || w < 4.0 || h < 4.0 {
            return;
        }
        let weights: Vec<f64> = indices.iter().map(|&i| self.dirs[i].size as f64).collect();
        let rects = treemap::squarify(&weights, x, y, w, h);
        let font_big = egui::FontId::proportional(12.0);
        let font_small = egui::FontId::proportional(11.0);
        let scanning = self.scanning;
        for (k, r) in rects.iter().enumerate() {
            if r.w < 2.0 || r.h < 2.0 {
                continue;
            }
            let idx = indices[k];
            let (path, size) = {
                let d = &self.dirs[idx];
                (d.path.clone(), d.size)
            };
            let er = egui::Rect::from_min_size(
                egui::pos2(origin.x + r.x as f32 + 1.0, origin.y + r.y as f32 + 1.0),
                egui::vec2(r.w as f32 - 2.0, r.h as f32 - 2.0),
            );
            let col = if depth == 0 {
                self.top_ext
                    .get(&path)
                    .map(|e| ext_color(e))
                    .unwrap_or_else(|| folder_color(k, depth))
            } else {
                folder_color(k, depth)
            };
            let id = ui.id().with(("tm", depth, idx));
            let rr = ui.interact(er, id, egui::Sense::click());
            let hot = rr.hovered() || self.selected == Some((ListTab::Folders, idx));
            painter.rect_filled(er, 3.0, col);
            if hot {
                painter.rect_stroke(
                    er,
                    3.0,
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
            }
            if rr.clicked() {
                *drill = Some((path.clone(), idx));
            }
            let mut action = None;
            rr.context_menu(|ui| {
                if ui.button(tr(self.lang, S::Reveal)).clicked() {
                    action = Some(MenuAction::Reveal);
                }
                ui.separator();
                if ui
                    .add_enabled(!scanning, egui::Button::new(tr(self.lang, S::Trash)))
                    .clicked()
                {
                    action = Some(MenuAction::Trash);
                }
                if ui
                    .add_enabled(
                        !scanning,
                        egui::Button::new(tr(self.lang, S::PermDeleteMenu)),
                    )
                    .clicked()
                {
                    action = Some(MenuAction::PermDelete);
                }
            });
            if let Some(a) = action {
                *menu_hit = Some((a, path.clone(), size, true));
            }
            rr.on_hover_text(format!("{}\n{}", path, human(size)));
            // Labels for large-enough rects; nested children go below the label.
            // Names are truncated by measured width so they show as completely
            // as possible; the full path is always in the hover tooltip.
            let label_h = if r.w > 96.0 && r.h > 44.0 {
                let label = fit_label(painter, short_name(&path), &font_big, r.w as f32 - 14.0);
                painter.text(
                    er.min + egui::vec2(6.0, 4.0),
                    egui::Align2::LEFT_TOP,
                    label,
                    font_big.clone(),
                    egui::Color32::WHITE,
                );
                painter.text(
                    er.min + egui::vec2(6.0, 22.0),
                    egui::Align2::LEFT_TOP,
                    human(size).trim_start().to_string(),
                    font_small.clone(),
                    egui::Color32::from_rgb(245, 245, 245),
                );
                36.0
            } else if r.w > 48.0 && r.h > 20.0 {
                let label = fit_label(painter, short_name(&path), &font_small, r.w as f32 - 14.0);
                painter.text(
                    er.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    font_small.clone(),
                    egui::Color32::WHITE,
                );
                0.0
            } else {
                0.0
            };
            // Nested subfolders (up to 3 levels deep).
            if depth < 2 && r.w > 120.0 && r.h > 90.0 {
                let kids: Vec<usize> = self
                    .dir_children
                    .get(&path)
                    .map(|v| v.iter().take(40).copied().collect())
                    .unwrap_or_default();
                if !kids.is_empty() {
                    let pad = 3.0;
                    self.draw_treemap_level(
                        ui,
                        painter,
                        origin,
                        &kids,
                        r.x + pad,
                        r.y + pad + label_h,
                        r.w - pad * 2.0,
                        r.h - pad * 2.0 - label_h,
                        depth + 1,
                        menu_hit,
                        drill,
                    );
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll(ui.ctx());

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(self.lang.toggle_label()).clicked() {
                    self.lang = self.lang.toggle();
                }
                ui.heading("sizetree");
                egui::ComboBox::from_label(tr(self.lang, S::Drive))
                    .selected_text(&self.drive)
                    .show_ui(ui, |ui| {
                        for d in self.drives.clone() {
                            ui.selectable_value(&mut self.drive, d.clone(), d);
                        }
                    });
                if self.drive != self.last_drive {
                    self.last_drive = self.drive.clone();
                    self.view_root = None;
                    self.selected = None;
                    self.sel_ext = None;
                    let root = self.drive_root();
                    self.expanded.clear();
                    self.expanded.insert(root);
                    self.refresh_disk_space();
                }
                if self.scanning {
                    if ui
                        .add(
                            egui::Button::new(format!("⏹ {}", tr(self.lang, S::Cancel)))
                                .fill(egui::Color32::from_rgb(150, 60, 60)),
                        )
                        .clicked()
                    {
                        if let Some(c) = &self.cancel {
                            c.store(true, Ordering::Relaxed);
                        }
                        self.status = tr(self.lang, S::Cancelling).to_string();
                    }
                    ui.spinner();
                    ui.label(i18n::records(self.lang, self.progress));
                } else {
                    let btn = ui.add_enabled(
                        !self.scanning,
                        egui::Button::new(format!("🔍 {}", tr(self.lang, S::Scan))),
                    );
                    if btn.clicked() {
                        self.start_scan();
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(format!("⬇ {}", tr(self.lang, S::ExportCsv)))
                        .clicked()
                    {
                        self.export_csv();
                    }
                });
            });
            // disk space bar (red when free space < 10%)
            if self.disk_total > 0 {
                let (total, free) = (self.disk_total, self.disk_free);
                let low = (free as f64) / (total.max(1) as f64) < LOW_FREE_FRAC;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&self.drive).small().weak());
                    usage_bar(ui, total, free, ui.available_width() - 220.0);
                    let txt = egui::RichText::new(i18n::free_of(
                        self.lang,
                        human(free).trim(),
                        human(total).trim(),
                    ))
                    .small();
                    ui.label(if low {
                        txt.color(egui::Color32::from_rgb(235, 110, 110)).strong()
                    } else {
                        txt.weak()
                    });
                });
            }
        });

        // view tabs (WizTree-style)
        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.view, ViewTab::Tree, tr(self.lang, S::TabTree));
                ui.selectable_value(&mut self.view, ViewTab::Files, tr(self.lang, S::TabFiles));
                ui.selectable_value(&mut self.view, ViewTab::About, tr(self.lang, S::TabAbout));
            });
        });

        // status bar at the very bottom
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let sel_txt: Option<String> = match self.selected {
                        Some((ListTab::Files, i)) => self.files.get(i).map(|it| {
                            format!("{} ({})", short_name(&it.path), human(it.size).trim())
                        }),
                        Some((ListTab::Folders, i)) => self.dirs.get(i).map(|it| {
                            format!("{} ({})", short_name(&it.path), human(it.size).trim())
                        }),
                        None => None,
                    };
                    if let Some(t) = sel_txt {
                        ui.label(egui::RichText::new(t).small().weak());
                    }
                });
            });
        });

        // treemap docked above the status bar
        egui::Panel::bottom("treemap")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| {
                self.draw_treemap(ui);
            });

        match self.view {
            ViewTab::Tree => {
                egui::Panel::left("tree")
                    .resizable(true)
                    .default_size(600.0)
                    .max_size(640.0)
                    .show(ui, |ui| {
                        self.draw_tree_view(ui);
                    });
                egui::CentralPanel::default().show(ui, |ui| {
                    self.draw_ext_view(ui);
                });
            }
            ViewTab::Files => {
                egui::CentralPanel::default().show(ui, |ui| {
                    self.draw_file_view(ui);
                });
            }
            ViewTab::About => {
                egui::CentralPanel::default().show(ui, |ui| {
                    self.draw_about(ui);
                });
            }
        }

        self.delete_modal(ui.ctx());
    }
}

/// egui's built-in font has no CJK glyphs, so Chinese text (e.g. the context
/// menu) renders as tofu boxes. Load a system CJK font at startup instead of
/// bundling a ~16MB font with the binary.
fn setup_cjk_font(ctx: &egui::Context) {
    let mut candidates: Vec<String> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        for name in ["msyh.ttc", "msyhbd.ttc", "simsun.ttc"] {
            candidates.push(format!(r"{sysroot}\Fonts\{name}"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push("/System/Library/Fonts/PingFang.ttc".to_string());
    }
    #[cfg(target_os = "linux")]
    {
        candidates.extend(
            [
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    }
    for path in candidates {
        if let Ok(data) = std::fs::read(&path) {
            let mut fonts = egui::FontDefinitions::default();
            // .ttc is a font collection; index 0 is the regular face.
            fonts.font_data.insert(
                "cjk".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(data)),
            );
            if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                list.insert(0, "cjk".to_owned());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1120.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "sizetree — disk usage analyzer",
        opts,
        Box::new(|cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = egui::Color32::from_rgb(27, 29, 33);
            visuals.window_fill = egui::Color32::from_rgb(34, 37, 43);
            visuals.extreme_bg_color = egui::Color32::from_rgb(20, 22, 25);
            visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(44, 49, 58);
            visuals.selection.bg_fill = egui::Color32::from_rgb(76, 141, 255);
            cc.egui_ctx.set_visuals(visuals);
            setup_cjk_font(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}
