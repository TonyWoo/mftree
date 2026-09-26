//! Squarified treemap layout (Bruls et al.).

#[derive(Clone, Copy, Debug)]
pub struct TreemapRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Lay out `weights` (must be non-negative, sorted descending for best results)
/// into the rectangle (x, y, w, h). Returns one rect per weight, in order.
pub fn squarify(weights: &[f64], x: f64, y: f64, w: f64, h: f64) -> Vec<TreemapRect> {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 || w <= 0.0 || h <= 0.0 {
        return Vec::new();
    }
    let scale = w * h / total;
    let mut items: Vec<(usize, f64)> = weights
        .iter()
        .enumerate()
        .map(|(i, &wt)| (i, (wt * scale).max(0.0)))
        .collect();
    // keep input order but squarify works best descending; caller sorts.
    let mut rects = vec![
        TreemapRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        weights.len()
    ];

    let mut cx = x;
    let mut cy = y;
    let mut cw = w;
    let mut ch = h;
    let mut row: Vec<(usize, f64)> = Vec::new();

    // worst aspect ratio of a row placed along the short side
    fn worst(row: &[(usize, f64)], side: f64) -> f64 {
        let s: f64 = row.iter().map(|(_, a)| a).sum();
        if s <= 0.0 {
            return f64::INFINITY;
        }
        let (mut mx, mut mn) = (0.0f64, f64::INFINITY);
        for (_, a) in row {
            mx = mx.max(*a);
            mn = mn.min(*a);
        }
        let s2 = s * s;
        (side * side * mx / s2).max(s2 / (side * side * mn))
    }

    fn layout_row(
        row: &[(usize, f64)],
        cx: &mut f64,
        cy: &mut f64,
        cw: &mut f64,
        ch: &mut f64,
        rects: &mut [TreemapRect],
    ) {
        let s: f64 = row.iter().map(|(_, a)| a).sum();
        if s <= 0.0 {
            return;
        }
        if *cw >= *ch {
            // row along the top, full width slice of height s / cw
            let rh = s / *cw;
            let mut rx = *cx;
            for &(i, a) in row {
                let rw = a / rh;
                rects[i] = TreemapRect { x: rx, y: *cy, w: rw, h: rh };
                rx += rw;
            }
            *cy += rh;
            *ch -= rh;
        } else {
            let rw = s / *ch;
            let mut ry = *cy;
            for &(i, a) in row {
                let rh = a / rw;
                rects[i] = TreemapRect { x: *cx, y: ry, w: rw, h: rh };
                ry += rh;
            }
            *cx += rw;
            *cw -= rw;
        }
    }

    let mut idx = 0;
    while idx < items.len() {
        let side = cw.min(ch);
        row.push(items[idx]);
        idx += 1;
        // try adding the next item; keep it if it doesn't worsen the ratio
        while idx < items.len() {
            let cur = worst(&row, side);
            row.push(items[idx]);
            let nxt = worst(&row, side);
            if nxt <= cur {
                idx += 1;
            } else {
                row.pop();
                break;
            }
        }
        layout_row(&row, &mut cx, &mut cy, &mut cw, &mut ch, &mut rects);
        row.clear();
        if cw <= 0.0 || ch <= 0.0 {
            break;
        }
    }
    // any leftovers get zero rects (already default)
    let _ = &mut items;
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
