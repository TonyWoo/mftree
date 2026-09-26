//! mftree — WizTree-style disk usage analyzer.
//!
//! Reads the NTFS $MFT directly on Windows; falls back to a directory walk
//! elsewhere. Shows the largest files/folders plus a treemap.

mod mft;
mod treemap;

use eframe::egui;
use std::collections::HashMap;
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

/// Category label + stable color for a path, used by the treemap and legend.
fn category_of(path: &str) -> (&'static str, egui::Color32) {
    let ext = short_name(path)
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" => {
            ("video", egui::Color32::from_rgb(226, 90, 90))
        }
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" => {
            ("audio", egui::Color32::from_rgb(230, 150, 60))
        }
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tiff" | "heic" => {
            ("images", egui::Color32::from_rgb(90, 178, 90))
        }
        "zip" | "rar" | "7z" | "tar" | "gz" | "iso" => {
            ("archives", egui::Color32::from_rgb(214, 188, 70))
        }
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" => {
            ("docs", egui::Color32::from_rgb(90, 140, 228))
        }
        "exe" | "dll" | "msi" | "sys" => ("exe/sys", egui::Color32::from_rgb(158, 110, 218)),
        _ => ("other", egui::Color32::from_rgb(148, 148, 148)),
    }
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

struct App {
    drives: Vec<String>,
    drive: String,
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
}

impl App {
    fn new() -> Self {
        let drives = mft::list_drives();
        let drive = drives.first().cloned().unwrap_or_default();
        Self {
            drives,
            drive,
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
    ) -> (Vec<(usize, &'a Item)>, usize) {
        let q = filter.to_lowercase();
        let mut rows: Vec<(usize, &Item)> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| q.is_empty() || it.path.to_lowercase().contains(&q))
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

    fn draw_list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.list_tab, ListTab::Files, "Files");
            ui.selectable_value(&mut self.list_tab, ListTab::Folders, "Folders");
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter…")
                .desired_width(f32::INFINITY),
        );
        let tab = self.list_tab;
        let mut col = self.sort_col;
        let mut asc = self.sort_asc;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("filelist")
                .striped(true)
                .num_columns(3)
                .min_col_width(56.0)
                .show(ui, |ui| {
                    sort_header(ui, &mut col, &mut asc, SortCol::Name, "Name");
                    sort_header(ui, &mut col, &mut asc, SortCol::Size, "Size");
                    sort_header(ui, &mut col, &mut asc, SortCol::Pct, "%");
                    ui.end_row();
                    let items: &[Item] = match tab {
                        ListTab::Files => &self.files,
                        ListTab::Folders => &self.dirs,
                    };
                    let total_size = self.total_size;
                    let (rows, total) = Self::filter_sort(items, &self.filter, col, asc);
                    for (idx, it) in &rows {
                        let sel = self.selected == Some((tab, *idx));
                        let label = short_name(&it.path).to_string();
                        let resp = ui.selectable_label(sel, label);
                        if resp.clicked() {
                            self.selected = Some((tab, *idx));
                        }
                        resp.on_hover_text(&it.path);
                        ui.monospace(human(it.size));
                        ui.monospace(Self::pct_str(it.size, total_size));
                        ui.end_row();
                    }
                    ui.end_row();
                    ui.label(
                        egui::RichText::new(format!("{} of {total} shown", rows.len()))
                            .small()
                            .weak(),
                    );
                });
        });
        self.sort_col = col;
        self.sort_asc = asc;
    }

    fn draw_treemap(&mut self, ui: &mut egui::Ui) {
        if self.files.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No data yet — hit Scan.");
            });
            return;
        }
        let top: Vec<usize> = (0..self.files.len().min(400)).collect();
        let weights: Vec<f64> = top.iter().map(|&i| self.files[i].size as f64).collect();

        // legend data: total size per category over the top-400
        let mut cat_map: HashMap<&'static str, (u64, egui::Color32)> = HashMap::new();
        for &i in &top {
            let (name, col) = category_of(&self.files[i].path);
            let e = cat_map.entry(name).or_insert((0, col));
            e.0 += self.files[i].size;
        }
        let mut cats: Vec<(&'static str, egui::Color32, u64)> =
            cat_map.into_iter().map(|(n, (s, c))| (n, c, s)).collect();
        cats.sort_by_key(|(_, _, s)| std::cmp::Reverse(*s));
        cats.truncate(8);

        let avail = ui.available_size();
        let tm_h = (avail.y - 34.0).max(60.0);
        let (resp, painter) = ui.allocate_painter(egui::vec2(avail.x, tm_h), egui::Sense::hover());
        let origin = resp.rect.min;
        let rects = treemap::squarify(&weights, 0.0, 0.0, avail.x as f64, tm_h as f64);
        let font_big = egui::FontId::proportional(12.0);
        let font_small = egui::FontId::proportional(11.0);
        for (k, r) in rects.iter().enumerate() {
            if r.w < 2.0 || r.h < 2.0 {
                continue;
            }
            let er = egui::Rect::from_min_size(
                egui::pos2(origin.x + r.x as f32 + 1.0, origin.y + r.y as f32 + 1.0),
                egui::vec2(r.w as f32 - 2.0, r.h as f32 - 2.0),
            );
            let idx = top[k];
            let it = &self.files[idx];
            let (_, base) = category_of(&it.path);
            let id = ui.id().with(("tm", idx));
            let rr = ui.interact(er, id, egui::Sense::click());
            let hot = rr.hovered() || self.selected == Some((ListTab::Files, idx));
            painter.rect_filled(er, 3.0, if hot { base } else { base.gamma_multiply(0.82) });
            if rr.clicked() {
                self.selected = Some((ListTab::Files, idx));
                self.list_tab = ListTab::Files;
            }
            rr.on_hover_text(format!("{}\n{}", it.path, human(it.size)));
            // in-rectangle labels
            let name = short_name(&it.path);
            let max_chars = ((r.w - 14.0) / 7.0) as usize;
            let label: String = if max_chars >= 4 && name.chars().count() > max_chars {
                let mut s: String = name.chars().take(max_chars - 1).collect();
                s.push('…');
                s
            } else {
                name.to_string()
            };
            if r.w > 96.0 && r.h > 44.0 {
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
                    human(it.size).trim_start().to_string(),
                    font_small.clone(),
                    egui::Color32::from_rgb(245, 245, 245),
                );
            } else if r.w > 48.0 && r.h > 20.0 {
                painter.text(
                    er.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    font_small.clone(),
                    egui::Color32::WHITE,
                );
            }
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Legend:").small().weak());
            for (name, col, _) in &cats {
                let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 5.0, *col);
                ui.label(egui::RichText::new(*name).small());
            }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll(ui.ctx());

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
            });
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
            Ok(Box::new(App::new()))
        }),
    )
}
