//! Integration tests for the small pure helpers in `sizetree::util`.

use eframe::egui::Color32;
use sizetree::util::{
    breadcrumb_segs, folder_color, hsv_to_rgb, human, parent_dir, path_under, short_name, MAIN_SEP,
};

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

#[test]
fn breadcrumbs_edge_cases() {
    assert!(breadcrumb_segs("/").is_empty());
    assert!(breadcrumb_segs("").is_empty());
    let segs = breadcrumb_segs("C:\\");
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].0, "C:");
}

#[test]
fn human_sizes() {
    assert_eq!(human(0), "    0.0 B");
    assert_eq!(human(512), "  512.0 B");
    assert_eq!(human(1023), " 1023.0 B");
    assert_eq!(human(1024), "    1.0 KB");
    assert_eq!(human(1536), "    1.5 KB");
    assert_eq!(human(1024 * 1024), "    1.0 MB");
    assert_eq!(human(5 * 1024 * 1024 * 1024), "    5.0 GB");
    assert_eq!(human(2 * 1024 * 1024 * 1024 * 1024), "    2.0 TB");
    assert!(human(u64::MAX).ends_with("TB"));
}

#[test]
fn short_name_cases() {
    assert_eq!(short_name("C:\\a\\b"), "b");
    assert_eq!(short_name("C:\\a\\"), "");
    assert_eq!(short_name("/a/b"), "b");
    assert_eq!(short_name("C:"), "C:");
    assert_eq!(short_name("file.txt"), "file.txt");
}

#[test]
fn parent_dir_cases() {
    assert_eq!(parent_dir("C:\\Windows\\Sys"), "C:\\Windows");
    assert_eq!(parent_dir("C:\\Windows"), "C:");
    assert_eq!(parent_dir("/a/b"), "/a");
    assert_eq!(parent_dir("/"), "/");
}

#[test]
fn hsv_known_colors() {
    assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), Color32::from_rgb(255, 0, 0));
    assert_eq!(
        hsv_to_rgb(1.0 / 3.0, 1.0, 1.0),
        Color32::from_rgb(0, 255, 0)
    );
    assert_eq!(
        hsv_to_rgb(2.0 / 3.0, 1.0, 1.0),
        Color32::from_rgb(0, 0, 255)
    );
    assert_eq!(hsv_to_rgb(0.5, 1.0, 1.0), Color32::from_rgb(0, 255, 255));
    assert_eq!(hsv_to_rgb(0.0, 0.0, 1.0), Color32::WHITE);
    assert_eq!(hsv_to_rgb(0.0, 0.0, 0.0), Color32::BLACK);
}

#[test]
fn folder_color_distinct_and_stable() {
    // deterministic
    assert_eq!(folder_color(3, 0), folder_color(3, 0));
    // opaque so labels stay readable
    for k in 0..16 {
        assert_eq!(folder_color(k, 0).a(), 255);
    }
    // siblings get pairwise distinct colors
    let cols: Vec<_> = (0..12).map(|k| folder_color(k, 0)).collect();
    for (i, a) in cols.iter().enumerate() {
        for b in &cols[i + 1..] {
            assert_ne!(a, b, "duplicate color at sibling {i}");
        }
    }
    // depth shifts the palette
    assert_ne!(folder_color(0, 0), folder_color(0, 1));
    assert_ne!(folder_color(0, 1), folder_color(0, 2));
}
