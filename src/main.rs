//! mftree — WizTree-style disk usage analyzer.
//!
//! Reads the NTFS $MFT directly on Windows; falls back to a directory walk
//! elsewhere. Shows the largest files/folders plus a treemap.

// Hide the console window on Windows release builds (GUI-only app).
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod mft;
mod treemap;

use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;

#[derive(Clone)]
struct Item {
    path: String,
    size: u64,
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

fn human(n: u64) -> String {
    let mut v = n as f64;
    for u in ["B", "KB", "MB", "GB", "TB"] {
        if v < 1024.0 || u == "TB" {
            return format!("{v:7.1} {u}");
        }
        v /= 1024.0;
    }
    unreachable!()
}

fn short_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Parent directory of a path, e.g. `parent_dir("C:\\Windows\\Sys") == "C:\\Windows"`.
fn parent_dir(path: &str) -> &str {
    let name = short_name(path);
    let p = path[..path.len() - name.len()].trim_end_matches(['\\', '/']);
    if p.is_empty() {
        "/"
    } else {
        p
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Heat color for a 0..1 size fraction: blue (small) -> yellow (mid) -> red (large).
fn heat_color_t(t: f64) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let (r, g, b) = if t < 0.5 {
        let u = t / 0.5;
        (
            lerp(90.0, 235.0, u),
            lerp(150.0, 200.0, u),
            lerp(255.0, 90.0, u),
        )
    } else {
        let u = (t - 0.5) / 0.5;
        (
            lerp(235.0, 225.0, u),
            lerp(200.0, 85.0, u),
            lerp(90.0, 80.0, u),
        )
    };
    egui::Color32::from_rgb(r as u8, g as u8, b as u8)
}

/// Heat color by size on a log scale between `cmin`/`cmax` (ln of sizes).
fn heat_color(size: f64, cmin: f64, cmax: f64) -> egui::Color32 {
    let t = if cmax > cmin {
        (size.max(1.0).ln() - cmin) / (cmax - cmin)
    } else {
        0.5
    };
    heat_color_t(t)
}

#[cfg(windows)]
const MAIN_SEP: char = '\\';
#[cfg(not(windows))]
const MAIN_SEP: char = '/';

/// True if `path` is strictly inside directory `dir`.
fn path_under(path: &str, dir: &str) -> bool {
    path.len() > dir.len()
        && path.starts_with(dir)
        && matches!(path.as_bytes().get(dir.len()), Some(b'\\') | Some(b'/'))
}

/// Split a directory path into breadcrumb segments: (display, full_path).
fn breadcrumb_segs(root: &str) -> Vec<(String, String)> {
    let parts: Vec<&str> = root.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    let mut out = Vec::new();
    let mut acc = String::new();
    for p in parts {
        if acc.is_empty() && root.starts_with('/') {
            acc.push('/');
        }
        if !acc.is_empty() && !acc.ends_with(['\\', '/']) {
            acc.push(MAIN_SEP);
        }
        acc.push_str(p);
        out.push((p.to_string(), acc.clone()));
    }
    out
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

#[derive(Clone)]
struct ConfirmDelete {
    path: String,
    size: u64,
    is_dir: bool,
}

#[derive(PartialEq, Clone)]
struct RowsKey {
    tab: ListTab,
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
    list_tab: ListTab,
    sort_col: SortCol,
    sort_asc: bool,
    filter: String,
    status: String,
    rx: Option<mpsc::Receiver<ScanMsg>>,
    cancel: Option<Arc<AtomicBool>>,
    started: Option<Instant>,
    /// Cached filtered+sorted row indices into `files`/`dirs`; recomputed
    /// only when `rows_key` inputs change (per-frame recompute over millions
    /// of rows stalls resize/drag repaints).
    rows_cache: Vec<usize>,
    rows_total: usize,
    rows_key: Option<RowsKey>,
    data_version: u64,
    /// Folders tab: tree view instead of the flat size-sorted table.
    tree_view: bool,
    /// Expanded folder paths in tree view.
    expanded: HashSet<String>,
    /// parent path -> child dir indices (size-desc); rebuilt on data change.
    dir_children: HashMap<String, Vec<usize>>,
    dir_index_version: u64,
    /// Startup drive-picker dialog (shown once at launch).
    show_drive_picker: bool,
    picker_selected: String,
}

impl App {
    fn new() -> Self {
        let drives = mft::list_drives();
        let drive = drives.first().cloned().unwrap_or_default();
        let mut app = Self {
            drives,
            drive: drive.clone(),
            picker_selected: drive.clone(),
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
            list_tab: ListTab::Files,
            sort_col: SortCol::Size,
            sort_asc: false,
            filter: String::new(),
            status: "Pick a drive and hit Scan.".to_string(),
            rx: None,
            cancel: None,
            started: None,
            rows_cache: Vec::new(),
            rows_total: 0,
            rows_key: None,
            data_version: 0,
            tree_view: false,
            expanded: HashSet::new(),
            dir_children: HashMap::new(),
            dir_index_version: u64::MAX,
            show_drive_picker: true,
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
        self.started = Some(Instant::now());
        let drive = self.drive.clone();
        self.status = format!("Scanning {drive} …");
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
                    self.status = format!(
                        "Done: {nf} files, {nd} folders, {} in {:.1}s",
                        human(self.total_size),
                        self.scan_secs
                    );
                }
                Err(e) if e == "cancelled" => self.status = "Scan cancelled.".to_string(),
                Err(e) => self.status = format!("Scan failed: {e}"),
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
                    self.status = format!("已永久删除：{}", short_name(&cd.path));
                }
                Err(e) => self.status = format!("删除失败：{e}"),
            }
        }
    }

    fn move_to_trash(&mut self, path: &str, is_dir: bool) {
        match trash::delete(path) {
            Ok(_) => {
                self.remove_item(path, is_dir);
                self.status = format!("已移到回收站：{}", short_name(path));
            }
            Err(e) => self.status = format!("移到回收站失败：{e}"),
        }
    }

    fn export_csv(&mut self) {
        let kind: &str = match self.list_tab {
            ListTab::Files => "file",
            ListTab::Folders => "folder",
        };
        let items: &[Item] = match self.list_tab {
            ListTab::Files => &self.files,
            ListTab::Folders => &self.dirs,
        };
        let root = self.view_root.clone();
        let mut out = String::from("Path,Size (bytes),Size,% of disk,Type\n");
        for it in items {
            if let Some(r) = &root {
                if !path_under(&it.path, r) {
                    continue;
                }
            }
            let pct = if self.total_size == 0 {
                0.0
            } else {
                it.size as f64 / self.total_size as f64 * 100.0
            };
            out.push_str(&format!(
                "\"{}\",{},{},{:.1}%,{}\n",
                it.path.replace('"', "\"\""),
                it.size,
                human(it.size).trim(),
                pct,
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
        let fname = format!("mftree_{drive_tag}.csv");
        let dest = home_dir()
            .map(|h| format!("{h}/{fname}"))
            .unwrap_or(fname.clone());
        match std::fs::write(&dest, out) {
            Ok(_) => self.status = format!("已导出 {} 条记录 → {dest}", items.len()),
            Err(e) => self.status = format!("导出失败：{e}"),
        }
    }

    fn delete_modal(&mut self, ctx: &egui::Context) {
        let cd = match self.confirm_delete.clone() {
            Some(cd) => cd,
            None => return,
        };
        let mut close = false;
        let mut confirmed = false;
        egui::Window::new("永久删除？")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&cd.path);
                ui.label(
                    egui::RichText::new(format!(
                        "{} · {}",
                        human(cd.size).trim(),
                        if cd.is_dir { "文件夹" } else { "文件" }
                    ))
                    .weak(),
                );
                ui.label(
                    egui::RichText::new("此操作不可恢复，文件将直接删除而不进回收站。")
                        .color(egui::Color32::from_rgb(255, 150, 150)),
                );
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        close = true;
                    }
                    if ui
                        .add(
                            egui::Button::new("永久删除")
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

    fn draw_list(&mut self, ui: &mut egui::Ui) {
        // breadcrumb (only when drilled into a folder)
        if self.view_root.is_some() {
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
                self.view_root = g;
                self.selected = None;
            }
            ui.separator();
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.list_tab, ListTab::Files, "Files");
            ui.selectable_value(&mut self.list_tab, ListTab::Folders, "Folders");
            if self.list_tab == ListTab::Folders {
                ui.checkbox(&mut self.tree_view, "🌲 树形");
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter…")
                .desired_width(f32::INFINITY),
        );
        let tab = self.list_tab;
        let mut col = self.sort_col;
        let mut asc = self.sort_asc;
        let root = self.view_root.clone();
        let scanning = self.scanning;
        // Recompute the filtered+sorted rows only when the inputs changed;
        // doing it every frame over millions of rows stalls resize repaints.
        let key = RowsKey {
            tab,
            filter: self.filter.clone(),
            sort_col: col,
            sort_asc: asc,
            view_root: root.clone(),
            data_version: self.data_version,
        };
        if self.rows_key.as_ref() != Some(&key) {
            let (rows, total) = {
                let items: &[Item] = match tab {
                    ListTab::Files => &self.files,
                    ListTab::Folders => &self.dirs,
                };
                Self::filter_sort(items, &self.filter, col, asc, root.as_deref())
            };
            self.rows_cache = rows.into_iter().map(|(i, _)| i).collect();
            self.rows_total = total;
            self.rows_key = Some(key);
        }
        let mut menu_hit: Option<(MenuAction, String, u64, bool)> = None;
        let mut drill: Option<String> = None;
        if tab == ListTab::Folders && self.tree_view {
            let table_h = (ui.available_height() - 22.0).max(80.0);
            egui::ScrollArea::vertical()
                .max_height(table_h)
                .show(ui, |ui| {
                    self.draw_folder_tree(ui, &mut menu_hit, &mut drill);
                });
            ui.label(
                egui::RichText::new(format!("{} folders", self.dirs.len()))
                    .small()
                    .weak(),
            );
        } else {
            let total_size = self.total_size;
            let total = self.rows_total;
            let nrows = self.rows_cache.len();
            // Reserve room for the footer label below the table.
            let table_h = (ui.available_height() - 22.0).max(80.0);
            egui_extras::TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(egui_extras::Column::remainder().clip(true))
                .column(egui_extras::Column::auto())
                .column(egui_extras::Column::auto())
                .max_scroll_height(table_h)
                .header(20.0, |mut header| {
                    header.col(|ui| sort_header(ui, &mut col, &mut asc, SortCol::Name, "Name"));
                    header.col(|ui| sort_header(ui, &mut col, &mut asc, SortCol::Size, "Size"));
                    header.col(|ui| sort_header(ui, &mut col, &mut asc, SortCol::Pct, "%"));
                })
                .body(|body| {
                    body.rows(18.0, nrows, |mut row| {
                        let idx = self.rows_cache[row.index()];
                        let it: &Item = match tab {
                            ListTab::Files => &self.files[idx],
                            ListTab::Folders => &self.dirs[idx],
                        };
                        let sel = self.selected == Some((tab, idx));
                        row.col(|ui| {
                            let label = short_name(&it.path);
                            let resp = ui.selectable_label(sel, label);
                            if resp.clicked() {
                                self.selected = Some((tab, idx));
                            }
                            if tab == ListTab::Folders && resp.double_clicked() {
                                drill = Some(it.path.clone());
                            }
                            if sel && self.scroll_to_sel {
                                ui.scroll_to_rect(resp.rect, Some(egui::Align::Center));
                            }
                            let path = it.path.clone();
                            let is_dir = it.is_dir;
                            let size = it.size;
                            let mut action = None;
                            resp.context_menu(|ui| {
                                if ui.button("在资源管理器中显示").clicked() {
                                    action = Some(MenuAction::Reveal);
                                }
                                ui.separator();
                                if ui
                                    .add_enabled(!scanning, egui::Button::new("移到回收站"))
                                    .clicked()
                                {
                                    action = Some(MenuAction::Trash);
                                }
                                if ui
                                    .add_enabled(!scanning, egui::Button::new("永久删除…"))
                                    .clicked()
                                {
                                    action = Some(MenuAction::PermDelete);
                                }
                            });
                            resp.on_hover_text(&path);
                            if let Some(a) = action {
                                menu_hit = Some((a, path, size, is_dir));
                            }
                        });
                        row.col(|ui| {
                            ui.monospace(human(it.size));
                        });
                        row.col(|ui| {
                            ui.monospace(Self::pct_str(it.size, total_size));
                        });
                    });
                });
            ui.label(
                egui::RichText::new(format!("{nrows} of {total} shown"))
                    .small()
                    .weak(),
            );
        }
        if let Some(d) = drill {
            self.view_root = Some(d);
            self.selected = None;
        }
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

    /// Startup dialog: pick a drive to scan. Drives with <10% free space
    /// are flagged red.
    fn draw_drive_picker(&mut self, ctx: &egui::Context) {
        let mut open = self.show_drive_picker;
        egui::Window::new("Select drive to scan")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.label("Choose a drive, then hit Scan.");
                ui.add_space(6.0);
                let mut picked: Option<String> = None;
                for drive in self.drives.clone() {
                    let (total, free) = mft::disk_space(&drive).unwrap_or((0, 0));
                    let low = total > 0 && (free as f64) / (total as f64) < LOW_FREE_FRAC;
                    let selected = self.picker_selected == drive;
                    ui.horizontal(|ui| {
                        let name = if low {
                            egui::RichText::new(format!("⚠ {drive}"))
                                .color(egui::Color32::from_rgb(235, 110, 110))
                                .strong()
                        } else {
                            egui::RichText::new(&drive).strong()
                        };
                        if ui.selectable_label(selected, name).clicked() {
                            picked = Some(drive.clone());
                        }
                        if total > 0 {
                            usage_bar(ui, total, free, 120.0);
                            let t = egui::RichText::new(format!(
                                "{} free of {}",
                                human(free).trim(),
                                human(total).trim()
                            ))
                            .small();
                            ui.label(if low {
                                t.color(egui::Color32::from_rgb(235, 110, 110))
                            } else {
                                t.weak()
                            });
                        }
                    });
                }
                if let Some(d) = picked {
                    self.picker_selected = d;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("🔍 Scan").clicked() {
                        self.drive = self.picker_selected.clone();
                        self.refresh_disk_space();
                        self.show_drive_picker = false;
                        self.start_scan();
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_drive_picker = false;
                    }
                });
            });
        self.show_drive_picker = open;
    }

    /// Treemap of folders: direct child folders of the current view root,
    /// sized by total folder size. Click a folder to drill into it.
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
            None => {
                let d = self.drive.trim_end_matches(':');
                if d == "/" || d.is_empty() {
                    "/".to_string()
                } else {
                    format!("{d}:")
                }
            }
        }
    }

    /// Nested treemap of folders: child folders of the current view root,
    /// sized by total size, with subfolders nested inside (up to 3 levels).
    /// Color encodes size (blue -> yellow -> red). Click drills in;
    /// right-click opens the folder menu.
    fn draw_treemap(&mut self, ui: &mut egui::Ui) {
        if self.dirs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No data yet — hit Scan.");
            });
            return;
        }
        self.ensure_dir_index();
        let root = self.treemap_root();
        let top: Vec<usize> = self
            .dir_children
            .get(&root)
            .map(|v| v.iter().take(200).copied().collect())
            .unwrap_or_default();
        if top.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No subfolders here.");
            });
            return;
        }
        let max_s = self.dirs[top[0]].size as f64;
        let min_s = self.dirs[*top.last().unwrap()].size as f64;
        let (cmin, cmax) = (min_s.max(1.0).ln(), max_s.max(1.0).ln());

        let avail = ui.available_size();
        let tm_h = (avail.y - 34.0).max(60.0);
        let (resp, painter) = ui.allocate_painter(egui::vec2(avail.x, tm_h), egui::Sense::hover());
        let origin = resp.rect.min;
        let painter = painter.clone();
        let mut menu_hit: Option<(MenuAction, String, u64, bool)> = None;
        let mut drill: Option<String> = None;
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
            cmin,
            cmax,
            &mut menu_hit,
            &mut drill,
        );
        if let Some(d) = drill {
            self.view_root = Some(d);
            self.selected = None;
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

        // legend: size gradient
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("small").small().weak());
            let (rrect, _) = ui.allocate_exact_size(egui::vec2(120.0, 10.0), egui::Sense::hover());
            {
                let p = ui.painter();
                let steps = 24;
                for i in 0..steps {
                    let c = heat_color_t(i as f64 / (steps - 1) as f64);
                    let x0 = rrect.min.x + rrect.width() * i as f32 / steps as f32;
                    p.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(x0, rrect.min.y),
                            egui::vec2(rrect.width() / steps as f32 + 1.0, rrect.height()),
                        ),
                        0.0,
                        c,
                    );
                }
            }
            ui.label(egui::RichText::new("large").small().weak());
            ui.label(
                egui::RichText::new(format!("Top {} folders by size", top.len()))
                    .small()
                    .weak(),
            );
        });
    }

    /// Draw one treemap level; recurses into subfolders for big-enough rects.
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
        cmin: f64,
        cmax: f64,
        menu_hit: &mut Option<(MenuAction, String, u64, bool)>,
        drill: &mut Option<String>,
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
            let col = heat_color(size as f64, cmin, cmax);
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
                *drill = Some(path.clone());
            }
            let mut action = None;
            rr.context_menu(|ui| {
                if ui.button("在资源管理器中显示").clicked() {
                    action = Some(MenuAction::Reveal);
                }
                ui.separator();
                if ui
                    .add_enabled(!scanning, egui::Button::new("移到回收站"))
                    .clicked()
                {
                    action = Some(MenuAction::Trash);
                }
                if ui
                    .add_enabled(!scanning, egui::Button::new("永久删除…"))
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
            let label_h = if r.w > 96.0 && r.h > 44.0 {
                let name = short_name(&path);
                let max_chars = ((r.w - 14.0) / 7.0) as usize;
                let label: String = if max_chars >= 4 && name.chars().count() > max_chars {
                    let mut s: String = name.chars().take(max_chars - 1).collect();
                    s.push('…');
                    s
                } else {
                    name.to_string()
                };
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
                let name = short_name(&path);
                let max_chars = ((r.w - 14.0) / 7.0) as usize;
                let label: String = if max_chars >= 4 && name.chars().count() > max_chars {
                    let mut s: String = name.chars().take(max_chars - 1).collect();
                    s.push('…');
                    s
                } else {
                    name.to_string()
                };
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
                        cmin,
                        cmax,
                        menu_hit,
                        drill,
                    );
                }
            }
        }
    }

    /// Folder tree view: hierarchical expand/collapse, children size-desc.
    fn draw_folder_tree(
        &mut self,
        ui: &mut egui::Ui,
        menu_hit: &mut Option<(MenuAction, String, u64, bool)>,
        drill: &mut Option<String>,
    ) {
        if self.dirs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No data yet — hit Scan.");
            });
            return;
        }
        self.ensure_dir_index();
        let root = self.treemap_root();
        self.draw_tree_node(ui, &root, 0, menu_hit, drill);
    }

    fn draw_tree_node(
        &mut self,
        ui: &mut egui::Ui,
        parent: &str,
        indent: usize,
        menu_hit: &mut Option<(MenuAction, String, u64, bool)>,
        drill: &mut Option<String>,
    ) {
        let total_kids = self.dir_children.get(parent).map(|v| v.len()).unwrap_or(0);
        let kids: Vec<usize> = self
            .dir_children
            .get(parent)
            .map(|v| v.iter().take(500).copied().collect())
            .unwrap_or_default();
        let scanning = self.scanning;
        let shown = kids.len();
        for idx in &kids {
            let idx = *idx;
            let (path, size, name) = {
                let d = &self.dirs[idx];
                (d.path.clone(), d.size, short_name(&d.path).to_string())
            };
            let has_kids = self
                .dir_children
                .get(&path)
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            let is_exp = self.expanded.contains(&path);
            ui.horizontal(|ui| {
                ui.add_space(indent as f32 * 16.0);
                if has_kids {
                    if ui.small_button(if is_exp { "▼" } else { "▶" }).clicked() {
                        if is_exp {
                            self.expanded.remove(&path);
                        } else {
                            self.expanded.insert(path.clone());
                        }
                    }
                } else {
                    ui.add_space(20.0);
                }
                let sel = self.selected == Some((ListTab::Folders, idx));
                let resp = ui.selectable_label(sel, &name);
                if resp.clicked() {
                    self.selected = Some((ListTab::Folders, idx));
                }
                if resp.double_clicked() {
                    *drill = Some(path.clone());
                }
                let mut action = None;
                resp.context_menu(|ui| {
                    if ui.button("在资源管理器中显示").clicked() {
                        action = Some(MenuAction::Reveal);
                    }
                    ui.separator();
                    if ui
                        .add_enabled(!scanning, egui::Button::new("移到回收站"))
                        .clicked()
                    {
                        action = Some(MenuAction::Trash);
                    }
                    if ui
                        .add_enabled(!scanning, egui::Button::new("永久删除…"))
                        .clicked()
                    {
                        action = Some(MenuAction::PermDelete);
                    }
                });
                resp.on_hover_text(&path);
                if let Some(a) = action {
                    *menu_hit = Some((a, path.clone(), size, true));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.monospace(human(size));
                });
            });
            if is_exp {
                self.draw_tree_node(ui, &path, indent + 1, menu_hit, drill);
            }
        }
        if total_kids > shown {
            ui.horizontal(|ui| {
                ui.add_space(indent as f32 * 16.0 + 20.0);
                ui.label(
                    egui::RichText::new(format!("… {} more", total_kids - shown))
                        .small()
                        .weak(),
                );
            });
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll(ui.ctx());

        if self.show_drive_picker {
            self.draw_drive_picker(ui.ctx());
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("mftree");
                egui::ComboBox::from_label("Drive")
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
                    self.refresh_disk_space();
                }
                if self.scanning {
                    if ui
                        .add(
                            egui::Button::new("⏹ Cancel")
                                .fill(egui::Color32::from_rgb(150, 60, 60)),
                        )
                        .clicked()
                    {
                        if let Some(c) = &self.cancel {
                            c.store(true, Ordering::Relaxed);
                        }
                        self.status = "Cancelling…".to_string();
                    }
                    ui.spinner();
                    ui.label(format!("{} records…", self.progress));
                } else {
                    let btn = ui.add_enabled(!self.scanning, egui::Button::new("🔍 Scan"));
                    if btn.clicked() {
                        self.start_scan();
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⬇ Export CSV").clicked() {
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
                    let txt = egui::RichText::new(format!(
                        "{} free of {}",
                        human(free).trim(),
                        human(total).trim()
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

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
            });
        });

        egui::Panel::left("list")
            .resizable(true)
            .default_size(430.0)
            .show(ui, |ui| {
                self.draw_list(ui);
            });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.draw_treemap(ui);
        });

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
        "mftree — disk usage analyzer",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_under_windows() {
        assert!(path_under("C:\\Users\\Tony\\a.txt", "C:\\Users\\Tony"));
        assert!(!path_under("C:\\Users\\Tony2\\a.txt", "C:\\Users\\Tony"));
        assert!(!path_under("C:\\Users\\Tony", "C:\\Users\\Tony"));
    }

    #[test]
    fn path_under_unix() {
        assert!(path_under("/a/b/c", "/a/b"));
        assert!(!path_under("/ab/c", "/a"));
    }

    #[test]
    fn breadcrumbs() {
        let segs = breadcrumb_segs("C:\\Users\\Tony");
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], ("C:".to_string(), "C:".to_string()));
        assert_eq!(segs[2].1, format!("C:{s}Users{s}Tony", s = MAIN_SEP));

        let segs = breadcrumb_segs("/a/b");
        assert_eq!(segs.len(), 2);
        assert!(segs[0].1.ends_with("a"));
    }
}
