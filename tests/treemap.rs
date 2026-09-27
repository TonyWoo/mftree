//! Integration tests for the squarified treemap layout.

use sizetree::treemap::squarify;

#[test]
fn area_preserved() {
    let w = vec![50.0, 30.0, 15.0, 5.0];
    let r = squarify(&w, 0.0, 0.0, 100.0, 100.0);
    let area: f64 = r.iter().map(|x| x.w * x.h).sum();
    assert!((area - 10000.0).abs() < 1.0, "area={area}");
    for x in &r {
        assert!(x.x >= -1e-6 && x.y >= -1e-6);
        assert!(x.x + x.w <= 100.0 + 1e-6 && x.y + x.h <= 100.0 + 1e-6);
    }
}

#[test]
fn single_item() {
    let r = squarify(&[10.0], 5.0, 5.0, 20.0, 30.0);
    assert_eq!(r.len(), 1);
    assert!((r[0].w - 20.0).abs() < 1e-6 && (r[0].h - 30.0).abs() < 1e-6);
}

#[test]
fn empty_weights() {
    assert!(squarify(&[], 0.0, 0.0, 100.0, 100.0).is_empty());
}

#[test]
fn zero_total_returns_empty() {
    assert!(squarify(&[0.0, 0.0], 0.0, 0.0, 100.0, 100.0).is_empty());
}

#[test]
fn areas_proportional_to_weights() {
    let r = squarify(&[2.0, 1.0, 1.0], 0.0, 0.0, 100.0, 100.0);
    assert_eq!(r.len(), 3);
    let areas: Vec<f64> = r.iter().map(|x| x.w * x.h).collect();
    assert!((areas[0] - 5000.0).abs() < 1.0, "areas={areas:?}");
    assert!((areas[1] - 2500.0).abs() < 1.0, "areas={areas:?}");
    assert!((areas[2] - 2500.0).abs() < 1.0, "areas={areas:?}");
}

#[test]
fn zero_and_negative_weights_safe() {
    let r = squarify(&[10.0, 0.0, -3.0, 5.0], 0.0, 0.0, 100.0, 100.0);
    assert_eq!(r.len(), 4);
    for x in &r {
        assert!(x.w.is_finite() && x.h.is_finite(), "rect={x:?}");
        assert!(x.w >= 0.0 && x.h >= 0.0, "rect={x:?}");
    }
    // zero/negative weights collapse to zero area; positives share the space
    assert!(r[1].w * r[1].h < 1e-6);
    assert!(r[2].w * r[2].h < 1e-6);
}

#[test]
fn rects_within_offset_bounds() {
    let r = squarify(&[30.0, 20.0, 10.0, 40.0], 10.0, 20.0, 200.0, 100.0);
    assert_eq!(r.len(), 4);
    for x in &r {
        assert!(x.x >= 10.0 - 1e-6 && x.y >= 20.0 - 1e-6, "rect={x:?}");
        assert!(
            x.x + x.w <= 210.0 + 1e-6 && x.y + x.h <= 120.0 + 1e-6,
            "rect={x:?}"
        );
    }
}

#[test]
fn no_overlap() {
    let r = squarify(&[40.0, 30.0, 20.0, 10.0], 0.0, 0.0, 100.0, 100.0);
    for (i, a) in r.iter().enumerate() {
        for b in &r[i + 1..] {
            let overlap = a.x < b.x + b.w - 1e-9
                && b.x < a.x + a.w - 1e-9
                && a.y < b.y + b.h - 1e-9
                && b.y < a.y + a.h - 1e-9;
            assert!(!overlap, "rects {i} and {b:?} overlap");
        }
    }
}

#[test]
fn many_items() {
    let w: Vec<f64> = (1..=200).map(|i| i as f64).collect();
    let r = squarify(&w, 0.0, 0.0, 800.0, 600.0);
    assert_eq!(r.len(), 200);
    let area: f64 = r.iter().map(|x| x.w * x.h).sum();
    assert!((area - 480000.0).abs() < 100.0, "area={area}");
    for x in &r {
        assert!(x.w.is_finite() && x.h.is_finite(), "rect={x:?}");
        assert!(x.x >= -1e-6 && x.y >= -1e-6, "rect={x:?}");
        assert!(
            x.x + x.w <= 800.0 + 1e-6 && x.y + x.h <= 600.0 + 1e-6,
            "rect={x:?}"
        );
    }
}
