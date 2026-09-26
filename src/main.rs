//! mftree — WizTree-style disk usage analyzer.
//!
//! Reads the NTFS $MFT directly on Windows; falls back to a directory walk
//! elsewhere. Shows the largest files/folders plus a treemap.

mod mft;
mod treemap;

use eframe::egui;
use std::sync::mpsc;
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

fn color_for(path: &str) -> egui::Color32 {
    let ext = short_name(path).rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" => {
            egui::Color32::from_rgb(226, 90, 90)
        }
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" => {
            egui::Color32::from_rgb(230, 150, 60)
        }
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tiff" | "heic" => {
            egui::Color32::from_rgb(90, 178, 90)
        }
        "zip" | "rar" | "7z" | "tar" | "gz" | "iso" => egui::Color32::from_rgb(214, 188, 70),
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" => {
            egui::Color32::from_rgb(90, 140, 228)
        }
        "exe" | "dll" | "msi" | "sys" => egui::Color32::from_rgb(158, 110, 218),
        _ => egui::Color32::from_rgb(148, 148, 148),
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
    filter: String,
    status: String,
    rx: Option<mpsc::Receiver<ScanMsg>>,
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
            filter: String::new(),
            status: "Pick a drive and hit Scan.".to_string(),
            rx: None,
            started: None,
        }
    }

    fn start_scan(&mut self) {
        if self.scanning {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.scanning = true;
        self.progress = 0;
        self.files.clear();
        self.dirs.clear();
        self.selected = None;
        self.started = Some(Instant::now());
        let drive = self.drive.clone();
        self.status = format!("Scanning {drive} …");
        std::thread::spawn(move || {
            let txp = tx.clone();
            let r = mft::scan(&drive, &|n| {
                let _ = txp.send(ScanMsg::Progress(n));
            });
            let items = r.map(|v| {
                v.into_iter()
                    .map(|e| Item { path: e.path, size: e.size, is_dir: e.is_dir })
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
            self.scan_secs = self.started.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
            match r {
                Ok(mut items) => {
                    items.sort_by(|a, b| b.size.cmp(&a.size));
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
                Err(e) => self.status = format!("Scan failed: {e}"),
            }
        } else if self.scanning {
            ctx.request_repaint();
        }
    }

    fn filtered<'a>(&self, items: &'a [Item]) -> Vec<(usize, &'a Item)> {
        let q = self.filter.to_lowercase();
        items
            .iter()
            .enumerate()
            .filter(|(_, it)| q.is_empty() || it.path.to_lowercase().contains(&q))
            .take(2000)
            .collect()
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
        let items: &[Item] = match self.list_tab {
            ListTab::Files => &self.files,
            ListTab::Folders => &self.dirs,
        };
        let rows = self.filtered(items);
        let tab = self.list_tab;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("filelist")
                .striped(true)
                .num_columns(2)
                .min_col_width(60.0)
                .show(ui, |ui| {
                    ui.strong("Name");
                    ui.strong("Size");
                    ui.end_row();
                    for (idx, it) in rows {
                        let sel = self.selected == Some((tab, idx));
                        let label = short_name(&it.path).to_string();
                        let resp = ui.selectable_label(sel, label);
                        if resp.clicked() {
                            self.selected = Some((tab, idx));
                        }
                        resp.on_hover_text(&it.path);
                        ui.monospace(human(it.size));
                        ui.end_row();
                    }
                });
        });
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
        let avail = ui.available_size();
        let (resp, painter) = ui.allocate_painter(avail, egui::Sense::hover());
        let origin = resp.rect.min;
        let rects = treemap::squarify(&weights, 0.0, 0.0, avail.x as f64, avail.y as f64);
        let font = egui::FontId::proportional(11.0);
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
            let base = color_for(&it.path);
            let id = ui.id().with(("tm", idx));
            let rr = ui.interact(er, id, egui::Sense::click());
            let hot = rr.hovered() || self.selected == Some((ListTab::Files, idx));
            painter.rect_filled(er, 3.0, if hot { base } else { base.gamma_multiply(0.82) });
            if rr.clicked() {
                self.selected = Some((ListTab::Files, idx));
                self.list_tab = ListTab::Files;
            }
            rr.on_hover_text(format!("{}\n{}", it.path, human(it.size)));
            if r.w > 70.0 && r.h > 26.0 {
                painter.text(
                    er.center(),
                    egui::Align2::CENTER_CENTER,
                    short_name(&it.path),
                    font.clone(),
                    egui::Color32::WHITE,
                );
            }
        }
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
                let btn = ui.add_enabled(!self.scanning, egui::Button::new("🔍 Scan"));
                if btn.clicked() {
                    self.start_scan();
                }
                if self.scanning {
                    ui.spinner();
                    ui.label(format!("{} records…", self.progress));
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
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
