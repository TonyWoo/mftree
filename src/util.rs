//! Small pure helpers: size formatting, path manipulation, treemap colors.

use eframe::egui::Color32;

pub fn human(n: u64) -> String {
    let mut v = n as f64;
    for u in ["B", "KB", "MB", "GB", "TB"] {
        if v < 1024.0 || u == "TB" {
            return format!("{v:7.1} {u}");
        }
        v /= 1024.0;
    }
    unreachable!()
}

pub fn short_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Parent directory of a path, e.g. `parent_dir("C:\\Windows\\Sys") == "C:\\Windows"`.
pub fn parent_dir(path: &str) -> &str {
    let name = short_name(path);
    let p = path[..path.len() - name.len()].trim_end_matches(['\\', '/']);
    if p.is_empty() {
        "/"
    } else {
        p
    }
}

/// Categorical treemap color: distinct hues per folder so neighbors are easy
/// to tell apart. Size is already encoded by area, so color is not size-based.
/// Golden-ratio hue rotation spreads sibling hues; deeper levels are slightly
/// darker. Saturation/value stay moderate so white labels remain readable.
pub fn folder_color(k: usize, depth: usize) -> Color32 {
    let hue = (k as f32 * 0.618034 + depth as f32 * 0.381966) % 1.0;
    let v = 0.80 - depth.min(2) as f32 * 0.06;
    hsv_to_rgb(hue, 0.52, v)
}

/// Plain HSV (each 0..=1) to RGB.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color32 {
    let h = ((h.fract() * 6.0) % 6.0 + 6.0) % 6.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color32::from_rgb(
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

#[cfg(windows)]
pub const MAIN_SEP: char = '\\';
#[cfg(not(windows))]
pub const MAIN_SEP: char = '/';

/// True if `path` is strictly inside directory `dir`.
pub fn path_under(path: &str, dir: &str) -> bool {
    path.len() > dir.len()
        && path.starts_with(dir)
        && matches!(path.as_bytes().get(dir.len()), Some(b'\\') | Some(b'/'))
}

/// Split a directory path into breadcrumb segments: (display, full_path).
pub fn breadcrumb_segs(root: &str) -> Vec<(String, String)> {
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
