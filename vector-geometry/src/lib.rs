//! Shared polygon tessellation and drawable-aware boundary coverage.
pub use lyon_tessellation;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
    math::{Point, point},
    path::{Event, Path, iterator::PathIterator},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub position: Point,
    pub offset: [f32; 2],
    pub coverage: f32,
}

impl Vertex {
    pub fn midpoint(self, other: Self) -> Self {
        Self {
            position: (self.position + other.position.to_vector()) * 0.5,
            offset: [
                (self.offset[0] + other.offset[0]) * 0.5,
                (self.offset[1] + other.offset[1]) * 0.5,
            ],
            coverage: (self.coverage + other.coverage) * 0.5,
        }
    }
}

/// The largest mitre a boundary vertex's offset may take. Sharper corners are bevelled, so an
/// offset can never reach across a narrow notch or crotch (a 4x spike pulled the crotch of a
/// Title "n" 1.9 px into the letter).
pub const MITRE_LIMIT: f32 = 2.;

/// Fill a path and add a one-physical-pixel boundary fringe. The caller scales
/// positions in its shader; offsets remain in physical pixels at every density.
pub fn tessellate(path: &Path, tolerance: f32, rule: FillRule) -> Result<Vec<Vertex>, String> {
    let outline: Vec<_> = path
        .iter()
        .flattened(tolerance)
        .filter_map(|event| match event {
            Event::Line { from, to } => Some((from, to)),
            Event::End {
                last,
                first,
                close: true,
            } => Some((last, first)),
            _ => None,
        })
        .collect();
    let contains = |p: Point| {
        let mut winding = 0i32;
        for &(a, b) in &outline {
            if (a.y <= p.y && b.y > p.y) || (b.y <= p.y && a.y > p.y) {
                let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                if x > p.x {
                    winding += if b.y > a.y { 1 } else { -1 };
                }
            }
        }
        match rule {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => winding % 2 != 0,
        }
    };
    let mut geometry: VertexBuffers<Point, u32> = VertexBuffers::new();
    FillTessellator::new()
        .tessellate_path(
            path,
            &FillOptions::default()
                .with_tolerance(tolerance)
                .with_fill_rule(rule),
            &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| vertex.position()),
        )
        .map_err(|e| format!("Path tessellation failed: {e:?}"))?;
    let mut unique = BTreeMap::new();
    let mut points = Vec::new();
    let mapping: Vec<usize> = geometry
        .vertices
        .iter()
        .map(|p| {
            let key = (
                (if p.x == 0. { 0. } else { p.x }).to_bits(),
                (if p.y == 0. { 0. } else { p.y }).to_bits(),
            );
            *unique.entry(key).or_insert_with(|| {
                points.push(*p);
                points.len() - 1
            })
        })
        .collect();
    let mut triangles = Vec::new();
    let mut edges: BTreeMap<(usize, usize), (usize, usize, usize)> = BTreeMap::new();
    for triangle in geometry.indices.chunks_exact(3) {
        let [a, mut b, mut c] = [
            mapping[triangle[0] as usize],
            mapping[triangle[1] as usize],
            mapping[triangle[2] as usize],
        ];
        if triangles.len() > 300_000 {
            return Err("Path exceeds its geometry budget".into());
        }
        let area = (points[b] - points[a]).cross(points[c] - points[a]);
        // Positions may be in font em units as well as SVG canvas units.
        if area.abs() < 1e-12 {
            continue;
        }
        if area < 0. {
            std::mem::swap(&mut b, &mut c);
        }
        triangles.push([a, b, c]);
        for (a, b) in [(a, b), (b, c), (c, a)] {
            edges
                .entry((a.min(b), a.max(b)))
                .and_modify(|edge| edge.2 += 1)
                .or_insert((a, b, 1));
        }
    }
    let boundary: Vec<_> = edges
        .values()
        .filter(|edge| edge.2 == 1)
        .filter(|&&(a, b, _)| {
            // Do not add fringes to unpaired interior T-junctions.
            let d = points[b] - points[a];
            let length = d.length();
            if length == 0. {
                return false;
            }
            let midpoint = points[a] + d * 0.5;
            let probe =
                point(d.y / length, -d.x / length).to_vector() * (tolerance * 0.25).max(0.0000001);
            contains(midpoint + probe) != contains(midpoint - probe)
        })
        .map(|&(a, b, _)| (a, b))
        .collect();
    // Unpaired interior edges are T-junctions: a vertex on one side lies along a longer edge on
    // the other (Lyon emits these where overlapping contours meet). Moving such a vertex inward
    // opens a crack, so every vertex on or at the end of one keeps its place.
    let on_boundary: std::collections::BTreeSet<(usize, usize)> = boundary
        .iter()
        .map(|&(a, b)| (a.min(b), a.max(b)))
        .collect();
    let mut pinned = vec![false; points.len()];
    for &(a, b, count) in edges.values() {
        if count != 1 || on_boundary.contains(&(a.min(b), a.max(b))) {
            continue;
        }
        pinned[a] = true;
        pinned[b] = true;
        let (pa, d) = (points[a], points[b] - points[a]);
        let length2 = d.square_length();
        if length2 == 0. {
            continue;
        }
        for (index, p) in points.iter().enumerate() {
            let t = (*p - pa).dot(d) / length2;
            if (0. ..=1.).contains(&t) && (pa + d * t - *p).square_length() < length2 * 1e-10 {
                pinned[index] = true;
            }
        }
    }
    let mut normals = vec![[0., 0.]; points.len()];
    let mut first_normal = normals.clone();
    for &(a, b) in &boundary {
        let d = points[b] - points[a];
        let length = d.length();
        if length == 0. {
            continue;
        }
        let n = [d.y / length, -d.x / length];
        for index in [a, b] {
            normals[index][0] += n[0];
            normals[index][1] += n[1];
            first_normal[index] = n;
        }
    }
    for (index, normal) in normals.iter_mut().enumerate() {
        let length = normal[0].hypot(normal[1]);
        if length > 1e-6 {
            let unit = [normal[0] / length, normal[1] / length];
            let miter = 1.
                / (unit[0] * first_normal[index][0] + unit[1] * first_normal[index][1])
                    .max(1. / MITRE_LIMIT);
            *normal = [unit[0] * miter, unit[1] * miter];
        }
    }
    let vertex = |index: usize, outside: bool| {
        // A pinned vertex keeps its fill on the outline; its fringe only reaches outward.
        let inward = if pinned[index] { 0. } else { -0.5 };
        Vertex {
            position: points[index],
            offset: [
                normals[index][0] * if outside { 0.5 } else { inward },
                normals[index][1] * if outside { 0.5 } else { inward },
            ],
            coverage: if outside { 0. } else { 1. },
        }
    };
    let mut vertices = Vec::with_capacity(triangles.len() * 3 + boundary.len() * 6);
    for [a, b, c] in triangles {
        vertices.extend([vertex(a, false), vertex(b, false), vertex(c, false)]);
    }
    for (a, b) in boundary {
        vertices.extend([
            vertex(a, false),
            vertex(a, true),
            vertex(b, false),
            vertex(b, false),
            vertex(a, true),
            vertex(b, true),
        ]);
    }
    if vertices.len() > 1_000_000 {
        return Err("Path exceeds its geometry budget".into());
    }
    Ok(vertices)
}

/// How well a fringe mesh from `tessellate` matches its outline once drawn at `scale` physical
/// pixels per path unit with the shader's `fringe` width: the most coverage it gives any point
/// clearly outside the outline (`stray`, ideally 0) and the least it gives any point clearly
/// inside (`hole`, ideally 1). "Clearly" means more than `margin` physical pixels from the
/// outline of the filled union (edges between overlapping contours are not outline). Coverage composites as the GPU blends it, 1 - Π(1 - c), so overlapping or folded
/// fringe quads show up as stray coverage or as holes.
#[derive(Debug, Clone, Copy)]
pub struct FringeReport {
    pub stray: f32,
    pub hole: f32,
    /// Where the worst hole is, in physical pixels relative to the path's origin.
    pub hole_at: [f32; 2],
}

pub fn fringe_report(
    path: &Path,
    vertices: &[Vertex],
    scale: f32,
    fringe: f32,
    margin: f32,
) -> FringeReport {
    // The outline in physical pixels, for distances and the inside test.
    let outline: Vec<(Point, Point)> = path
        .iter()
        .flattened(0.0001)
        .filter_map(|event| match event {
            Event::Line { from, to } => Some((from, to)),
            Event::End {
                last,
                first,
                close: true,
            } => Some((last, first)),
            _ => None,
        })
        .map(|(a, b)| (a * scale, b * scale))
        .collect();
    let inside = |p: Point| {
        let mut winding = 0i32;
        for &(a, b) in &outline {
            if (a.y <= p.y && b.y > p.y) || (b.y <= p.y && a.y > p.y) {
                let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                if x > p.x {
                    winding += if b.y > a.y { 1 } else { -1 };
                }
            }
        }
        winding != 0
    };
    // The union's outline: pieces of segments with the fill on one side only. Edges between
    // overlapping contours are interior, and a contour partly covered by another counts only
    // where it is exposed.
    let mut union_outline = Vec::new();
    for &(a, b) in &outline {
        let d = b - a;
        let length = d.length();
        if length == 0. {
            continue;
        }
        let normal = point(d.y / length, -d.x / length).to_vector() * 0.01;
        let pieces = (length / 0.25).ceil().max(1.) as usize;
        for k in 0..pieces {
            let start = a + d * (k as f32 / pieces as f32);
            let end = a + d * ((k + 1) as f32 / pieces as f32);
            let mid = start + (end - start) * 0.5;
            if inside(mid + normal) != inside(mid - normal) {
                union_outline.push((start, end));
            }
        }
    }
    let union_distance = |p: Point| {
        union_outline
            .iter()
            .map(|&(a, b): &(Point, Point)| {
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.square_length().max(1e-12)).clamp(0., 1.);
                (a + ab * t - p).length()
            })
            .fold(f32::MAX, f32::min)
    };
    // Drawn positions, as text.vert places them (fill vertices carry inward offsets too).
    let drawn: Vec<(Point, f32)> = vertices
        .iter()
        .map(|v| {
            (
                point(
                    v.position.x * scale + v.offset[0] * fringe,
                    v.position.y * scale + v.offset[1] * fringe,
                ),
                v.coverage,
            )
        })
        .collect();
    let (mut min, mut max) = (point(f32::MAX, f32::MAX), point(f32::MIN, f32::MIN));
    for (p, _) in &drawn {
        min = point(min.x.min(p.x), min.y.min(p.y));
        max = point(max.x.max(p.x), max.y.max(p.y));
    }
    // Samples every quarter pixel, offset so none sits exactly on a shared edge.
    let step = 0.25;
    let (x0, y0) = (min.x.floor() - 2., min.y.floor() - 2.);
    let width = ((max.x - x0) / step).ceil() as usize + 16;
    let height = ((max.y - y0) / step).ceil() as usize + 16;
    let mut transparency = vec![1f32; width * height];
    for triangle in drawn.chunks_exact(3) {
        let [(a, ca), (b, cb), (c, cc)] = [triangle[0], triangle[1], triangle[2]];
        let area = (b - a).cross(c - a);
        if area.abs() < 1e-9 {
            continue;
        }
        let lo = point(a.x.min(b.x).min(c.x), a.y.min(b.y).min(c.y));
        let hi = point(a.x.max(b.x).max(c.x), a.y.max(b.y).max(c.y));
        let i0 = ((lo.x - x0) / step).floor().max(0.) as usize;
        let i1 = (((hi.x - x0) / step).ceil() as usize).min(width - 1);
        let j0 = ((lo.y - y0) / step).floor().max(0.) as usize;
        let j1 = (((hi.y - y0) / step).ceil() as usize).min(height - 1);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let p = point(x0 + (i as f32 + 0.37) * step, y0 + (j as f32 + 0.61) * step);
                let w0 = (b - p).cross(c - p) / area;
                let w1 = (c - p).cross(a - p) / area;
                let w2 = 1. - w0 - w1;
                if w0 >= 0. && w1 >= 0. && w2 >= 0. {
                    let coverage = (w0 * ca + w1 * cb + w2 * cc).clamp(0., 1.);
                    transparency[j * width + i] *= 1. - coverage;
                }
            }
        }
    }
    let mut report = FringeReport {
        stray: 0.,
        hole: 1.,
        hole_at: [0., 0.],
    };
    for j in 0..height {
        for i in 0..width {
            let p = point(x0 + (i as f32 + 0.37) * step, y0 + (j as f32 + 0.61) * step);
            let coverage = 1. - transparency[j * width + i];
            if coverage == 0. && !inside(p) {
                continue;
            }
            // Only points farther than `margin` from the union's outline count.
            let here = inside(p);
            if union_distance(p) <= margin {
                continue;
            }
            if here {
                if coverage < report.hole {
                    report.hole_at = [p.x, p.y];
                }
                report.hole = report.hole.min(coverage);
            } else {
                report.stray = report.stray.max(coverage);
            }
        }
    }
    report
}

/// Per-pixel error of a fringe mesh against the exact box-filtered coverage of its outline, at
/// pixel centres as the GPU samples them, after `exponent` (the shader's coverage boost):
/// the largest excess (too bright for light text) and the largest shortfall, and where the
/// excess is. `origin` places the path in pixels.
pub fn coverage_error(
    path: &Path,
    vertices: &[Vertex],
    scale: f32,
    fringe: f32,
    exponent: f32,
    origin: [f32; 2],
) -> (f32, f32, [i32; 2]) {
    let outline: Vec<(Point, Point)> = path
        .iter()
        .flattened(0.0001)
        .filter_map(|event| match event {
            Event::Line { from, to } => Some((from, to)),
            Event::End {
                last,
                first,
                close: true,
            } => Some((last, first)),
            _ => None,
        })
        .map(|(a, b)| {
            (
                point(a.x * scale + origin[0], a.y * scale + origin[1]),
                point(b.x * scale + origin[0], b.y * scale + origin[1]),
            )
        })
        .collect();
    let inside = |p: Point| {
        let mut winding = 0i32;
        for &(a, b) in &outline {
            if (a.y <= p.y && b.y > p.y) || (b.y <= p.y && a.y > p.y) {
                let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                if x > p.x {
                    winding += if b.y > a.y { 1 } else { -1 };
                }
            }
        }
        winding != 0
    };
    let drawn: Vec<(Point, f32)> = vertices
        .iter()
        .map(|v| {
            (
                point(
                    v.position.x * scale + origin[0] + v.offset[0] * fringe,
                    v.position.y * scale + origin[1] + v.offset[1] * fringe,
                ),
                v.coverage,
            )
        })
        .collect();
    let (mut lo, mut hi) = ([i32::MAX; 2], [i32::MIN; 2]);
    for (p, _) in &drawn {
        lo = [lo[0].min(p.x.floor() as i32), lo[1].min(p.y.floor() as i32)];
        hi = [hi[0].max(p.x.ceil() as i32), hi[1].max(p.y.ceil() as i32)];
    }
    let (mut excess, mut shortfall, mut at) = (0f32, 0f32, [0, 0]);
    for y in lo[1] - 1..=hi[1] + 1 {
        for x in lo[0] - 1..=hi[0] + 1 {
            let centre = point(x as f32 + 0.5, y as f32 + 0.5);
            let mut transparency = 1.;
            for triangle in drawn.chunks_exact(3) {
                let [(a, ca), (b, cb), (c, cc)] = [triangle[0], triangle[1], triangle[2]];
                let area = (b - a).cross(c - a);
                if area.abs() < 1e-9 {
                    continue;
                }
                let w0 = (b - centre).cross(c - centre) / area;
                let w1 = (c - centre).cross(a - centre) / area;
                let w2 = 1. - w0 - w1;
                if w0 >= 0. && w1 >= 0. && w2 >= 0. {
                    let coverage: f32 = (w0 * ca + w1 * cb + w2 * cc).clamp(0., 1.);
                    transparency *= 1. - coverage.powf(exponent);
                }
            }
            let rendered = 1. - transparency;
            let mut count = 0;
            for j in 0..8 {
                for i in 0..8 {
                    count += usize::from(inside(point(
                        x as f32 + (i as f32 + 0.5) / 8.,
                        y as f32 + (j as f32 + 0.5) / 8.,
                    )));
                }
            }
            let exact = (count as f32 / 64.).powf(exponent);
            if rendered - exact > excess {
                excess = rendered - exact;
                at = [x, y];
            }
            shortfall = shortfall.max(exact - rendered);
        }
    }
    (excess, shortfall, at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyon_tessellation::path::Path;

    /// A stem and an arch-like bar drawn as two overlapping contours, as Inter draws "n".
    fn overlapping() -> Path {
        let mut builder = Path::builder();
        for rect in [
            [0.0f32, 0.0, 0.1, 0.7],
            [0.06, 0.08, 0.5, 0.2],
            [0.38, 0.08, 0.5, 0.7],
        ] {
            builder.begin(point(rect[0], rect[1]));
            builder.line_to(point(rect[2], rect[1]));
            builder.line_to(point(rect[2], rect[3]));
            builder.line_to(point(rect[0], rect[3]));
            builder.end(true);
        }
        builder.build()
    }

    #[test]
    fn overlapping_contours_leave_no_seam_inside_and_nothing_outside() {
        let path = overlapping();
        let mesh = tessellate(&path, 0.15 / 256., FillRule::NonZero).unwrap();
        for size in [29., 62.7, 84.] {
            let report = fringe_report(&path, &mesh, size, 1., 0.75);
            assert!(report.stray < 0.01, "{size} px: {report:?}");
            assert!(report.hole > 0.99, "{size} px: {report:?}");
            let (excess, shortfall, at) = coverage_error(&path, &mesh, size, 1., 1., [3.3, 2.7]);
            assert!(
                excess < 0.35 && shortfall < 0.25,
                "{size} px: {excess} {shortfall} {at:?}"
            );
        }
    }

    #[test]
    fn mitres_are_capped_so_sharp_corners_cannot_spike() {
        // A narrow V notch: without a cap its offsets would reach 4 px across.
        let mut builder = Path::builder();
        builder.begin(point(0., 0.));
        builder.line_to(point(0.45, 0.6));
        builder.line_to(point(0.5, 0.));
        builder.line_to(point(1., 0.));
        builder.line_to(point(1., 1.));
        builder.line_to(point(0., 1.));
        builder.end(true);
        let path = builder.build();
        let mesh = tessellate(&path, 0.001, FillRule::NonZero).unwrap();
        for vertex in &mesh {
            let length = vertex.offset[0].hypot(vertex.offset[1]);
            // Never more than one physical pixel (with the shader's one-pixel ramp).
            assert!(length <= 1. + 1e-4, "offset {length}");
        }
        let report = fringe_report(&path, &mesh, 60., 1., 1.5);
        assert!(report.stray < 0.01, "{report:?}");
    }
}
