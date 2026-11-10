//! CPU glyph rasterisation in 256 coverage levels. Each pixel row is sampled by 32
//! sub-scanlines; on each, the true non-zero winding rule gives the spans inside the outline,
//! and every span adds its exact horizontal share to the pixels it covers. Overlapping
//! contours (Inter's variable-font stems and arches) therefore fill as one shape: no seam where
//! they meet and no double coverage where their edges coincide. The error is at most 1/64 of
//! a pixel's coverage for horizontal edges and smaller for slanted ones.
use lyon_tessellation::{
    math::{Point, point},
    path::{Event, Path, iterator::PathIterator},
};

/// A glyph's coverage bitmap. `left`/`top` place its top-left pixel relative to the outline's
/// origin (the pen position on the baseline), in whole pixels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Curve flattening tolerance in physical pixels.
const FLATTENING_PX: f32 = 0.02;
/// Sub-scanlines per pixel row.
pub const SUBSCANLINES: usize = 32;
/// No glyph bitmap is larger than this many pixels (a 2048 px Title would be 4 Mpx).
pub const MAX_PIXELS: u64 = 1 << 22;

/// The outline's segments in pixels (closing every contour, as a fill does).
fn segments(path: &Path, scale: f32) -> Vec<(Point, Point)> {
    let tolerance = FLATTENING_PX / scale.max(1e-6);
    path.iter()
        .flattened(tolerance)
        .filter_map(|event| match event {
            Event::Line { from, to } => Some((from, to)),
            Event::End { last, first, .. } => Some((last, first)),
            _ => None,
        })
        .map(|(a, b)| {
            (
                point(a.x * scale, a.y * scale),
                point(b.x * scale, b.y * scale),
            )
        })
        .filter(|(a, b)| a != b)
        .collect()
}

/// Rasterises `path` (in em units, y down) at `scale` pixels per em with its origin on a pixel
/// corner. An empty outline (a space) gives an empty bitmap.
pub fn rasterize(path: &Path, scale: f32) -> Coverage {
    let lines = segments(path, scale);
    let Some((min, max)) = bounds(&lines) else {
        return Coverage::default();
    };
    let left = min.x.floor() as i32;
    let top = min.y.floor() as i32;
    let width = (max.x.ceil() as i32 - left).max(1) as usize;
    let height = (max.y.ceil() as i32 - top).max(1) as usize;
    if (width as u64) * (height as u64) > MAX_PIXELS {
        return Coverage::default();
    }
    let sub = SUBSCANLINES as f32;
    let rows = height * SUBSCANLINES;
    // Every edge's crossing of every sub-scanline centre it spans: (sub-scanline, x, winding).
    let mut crossings: Vec<(u32, f32, i8)> = Vec::new();
    for (a, b) in lines {
        let (a, b) = (
            point(a.x - left as f32, a.y - top as f32),
            point(b.x - left as f32, b.y - top as f32),
        );
        if a.y == b.y {
            continue;
        }
        let (winding, lo, hi) = if a.y < b.y { (1, a, b) } else { (-1, b, a) };
        let dxdy = (hi.x - lo.x) / (hi.y - lo.y);
        // Sub-scanline k samples y = (k + 0.5) / sub; take lo.y <= y < hi.y.
        let first = (lo.y * sub - 0.5).ceil().max(0.) as usize;
        let last = ((hi.y * sub - 0.5).ceil().max(0.) as usize).min(rows);
        for k in first..last {
            let y = (k as f32 + 0.5) / sub;
            crossings.push((k as u32, lo.x + (y - lo.y) * dxdy, winding));
        }
    }
    crossings.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    // Span starts add and span ends subtract, split exactly between the two pixels at x.
    let stride = width + 2;
    let mut cells = vec![0f32; stride * height];
    let weight = 1. / sub;
    let mut add = |row: usize, x: f32, amount: f32| {
        let x = x.clamp(0., width as f32);
        let cell = (x.floor() as usize).min(width);
        let fraction = x - cell as f32;
        cells[row * stride + cell] += amount * (1. - fraction);
        cells[row * stride + cell + 1] += amount * fraction;
    };
    let mut index = 0;
    while index < crossings.len() {
        let line = crossings[index].0;
        let row = line as usize / SUBSCANLINES;
        let mut winding = 0i32;
        let mut start = 0.;
        while index < crossings.len() && crossings[index].0 == line {
            let (_, x, direction) = crossings[index];
            let before = winding;
            winding += i32::from(direction);
            if before == 0 && winding != 0 {
                start = x;
            } else if before != 0 && winding == 0 {
                add(row, start, weight);
                add(row, x, -weight);
            }
            index += 1;
        }
    }
    let mut data = Vec::with_capacity(width * height);
    for row in cells.chunks_exact(stride) {
        let mut sum = 0f32;
        for &cell in &row[..width] {
            sum += cell;
            data.push((sum.clamp(0., 1.) * 255. + 0.5) as u8);
        }
    }
    Coverage {
        left,
        top,
        width: width as u32,
        height: height as u32,
        data,
    }
}

fn bounds(lines: &[(Point, Point)]) -> Option<(Point, Point)> {
    let mut points = lines.iter().flat_map(|(a, b)| [*a, *b]);
    let first = points.next()?;
    Some(points.fold((first, first), |(lo, hi), p| {
        (
            point(lo.x.min(p.x), lo.y.min(p.y)),
            point(hi.x.max(p.x), hi.y.max(p.y)),
        )
    }))
}

/// Reference coverage for tests: `samples` × `samples` point samples per pixel with the
/// non-zero rule, over the bitmap `rasterize` would produce.
pub fn reference_coverage(path: &Path, scale: f32, bitmap: &Coverage, samples: u32) -> Vec<f32> {
    let lines = segments(path, scale);
    let inside = |p: Point| {
        let mut winding = 0i32;
        for &(a, b) in &lines {
            if (a.y <= p.y && b.y > p.y) || (b.y <= p.y && a.y > p.y) {
                let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                if x > p.x {
                    winding += if b.y > a.y { 1 } else { -1 };
                }
            }
        }
        winding != 0
    };
    let mut out = Vec::with_capacity((bitmap.width * bitmap.height) as usize);
    for y in 0..bitmap.height as i32 {
        for x in 0..bitmap.width as i32 {
            let mut count = 0;
            for j in 0..samples {
                for i in 0..samples {
                    count += u32::from(inside(point(
                        (bitmap.left + x) as f32 + (i as f32 + 0.5) / samples as f32,
                        (bitmap.top + y) as f32 + (j as f32 + 0.5) / samples as f32,
                    )));
                }
            }
            out.push(count as f32 / (samples * samples) as f32);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangles(rects: &[[f32; 4]], clockwise: &[bool]) -> Path {
        let mut builder = Path::builder();
        for (rect, &cw) in rects.iter().zip(clockwise) {
            let corners = [
                point(rect[0], rect[1]),
                point(rect[2], rect[1]),
                point(rect[2], rect[3]),
                point(rect[0], rect[3]),
            ];
            builder.begin(corners[0]);
            if cw {
                for corner in &corners[1..] {
                    builder.line_to(*corner);
                }
            } else {
                for corner in corners[1..].iter().rev() {
                    builder.line_to(*corner);
                }
            }
            builder.end(true);
        }
        builder.build()
    }

    /// The largest difference from a 16 x 16 point-sampled reference. The reference alone can
    /// be 1/32 off at a horizontal edge and the rasteriser 1/64, so 0.05 is the bound.
    fn worst(path: &Path, scale: f32) -> f32 {
        let bitmap = rasterize(path, scale);
        let reference = reference_coverage(path, scale, &bitmap, 16);
        bitmap
            .data
            .iter()
            .zip(reference)
            .map(|(&got, want)| (got as f32 / 255. - want).abs())
            .fold(0., f32::max)
    }

    #[test]
    fn coverage_is_the_exact_area_including_partial_pixels() {
        // 0.25 .. 2.75 by 0.5 .. 1.5 at scale 1: edge pixels are a quarter and a half covered.
        let path = rectangles(&[[0.25, 0.5, 2.75, 1.5]], &[true]);
        let bitmap = rasterize(&path, 1.);
        assert_eq!(
            (bitmap.left, bitmap.top, bitmap.width, bitmap.height),
            (0, 0, 3, 2)
        );
        let level = |x: usize, y: usize| bitmap.data[y * 3 + x];
        assert_eq!(level(0, 0), (0.75f32 * 0.5 * 255. + 0.5) as u8);
        assert_eq!(level(1, 0), (0.5f32 * 255. + 0.5) as u8);
        assert_eq!(level(2, 1), (0.75f32 * 0.5 * 255. + 0.5) as u8);
    }

    #[test]
    fn overlapping_contours_fill_as_one_shape_without_a_seam() {
        // A stem and an arch drawn as overlapping contours, as Inter draws "n", sharing edges
        // inside partly covered pixels; a contour wound the other way cuts a counter.
        let n = rectangles(
            &[
                [0.0, 0.0, 0.1, 0.7],
                [0.06, 0.08, 0.5, 0.2],
                [0.38, 0.08, 0.5, 0.7],
            ],
            &[true, true, true],
        );
        for size in [23., 29., 62.7, 84.] {
            let error = worst(&n, size);
            assert!(error < 0.05, "{size} px: {error}");
        }
        let counter = rectangles(
            &[[0.0, 0.0, 1.0, 1.0], [0.3, 0.3, 0.7, 0.7]],
            &[true, false],
        );
        let bitmap = rasterize(&counter, 10.);
        assert_eq!(bitmap.data[5 * 10 + 5], 0, "the counter stays open");
        assert_eq!(bitmap.data[10 + 1], 255);
    }

    #[test]
    fn diagonals_and_curves_match_area_sampling() {
        let mut builder = Path::builder();
        builder.begin(point(0.1, 0.9));
        builder.line_to(point(0.5, 0.05));
        builder.quadratic_bezier_to(point(0.95, 0.5), point(0.8, 0.95));
        builder.cubic_bezier_to(point(0.6, 0.7), point(0.3, 1.1), point(0.1, 0.9));
        builder.end(true);
        let path = builder.build();
        for size in [11.5, 25., 57., 152.] {
            let error = worst(&path, size);
            assert!(error < 0.05, "{size} px: {error}");
        }
    }

    #[test]
    fn empty_and_oversized_outlines_give_no_bitmap() {
        assert_eq!(
            rasterize(&Path::builder().build(), 30.),
            Coverage::default()
        );
        let huge = rectangles(&[[0., 0., 1., 1.]], &[true]);
        assert_eq!(rasterize(&huge, 4096.), Coverage::default());
    }
}
