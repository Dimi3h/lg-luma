//! The text path: Inter outlines rasterised on the CPU (256 coverage levels) by a worker
//! thread, kept in a bounded CPU cache, and packed into one single-channel atlas texture that
//! the SDL thread uploads to and draws from, one textured quad per glyph.
use crate::font::Font;
use luma_vector_geometry::raster::{self, Coverage};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

/// Glyph sizes are quantised to a quarter physical pixel: an animated size reuses bitmaps
/// instead of rasterising every frame, and a quarter pixel is never visible in a glyph's size.
const SIZE_STEPS: f32 = 4.;
/// The CPU copies of rasterised glyphs, kept so a full atlas can be repacked by uploads alone.
const BITMAP_BUDGET: usize = 8 * 1024 * 1024;
const BITMAP_ENTRIES: usize = 8192;

/// One glyph of one font instance at one physical size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub font: u16,
    pub glyph: u16,
    /// Physical pixels per em, in quarter pixels.
    pub size: u32,
}

impl GlyphKey {
    pub fn new(font: u16, glyph: u16, physical_px: f32) -> Self {
        Self {
            font,
            glyph,
            size: (physical_px * SIZE_STEPS).round().max(1.) as u32,
        }
    }

    pub fn px(self) -> f32 {
        self.size as f32 / SIZE_STEPS
    }
}

/// A finished glyph from the worker, and how long its rasterisation took there.
pub struct Rasterised {
    pub key: GlyphKey,
    pub coverage: Arc<Coverage>,
    pub ms: f64,
}

/// Rasterises glyphs on its own thread, loading each font instance there on first use.
pub struct Worker {
    requests: Sender<GlyphKey>,
    results: Receiver<Rasterised>,
    pending: HashSet<GlyphKey>,
}

impl Worker {
    pub fn new(directory: PathBuf, load: fn(&Path, u16) -> Result<Font, String>) -> Self {
        let (requests, inbox) = mpsc::channel::<GlyphKey>();
        let (outbox, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("luma-glyphs".into())
            .spawn(move || {
                let mut fonts: HashMap<u16, Option<Font>> = HashMap::new();
                for key in inbox {
                    let started = Instant::now();
                    let font = fonts.entry(key.font).or_insert_with(|| {
                        load(&directory, key.font)
                            .map_err(|error| eprintln!("Luma glyphs: {error}"))
                            .ok()
                    });
                    let coverage = font
                        .as_ref()
                        .and_then(|font| font.glyph_outline(key.glyph))
                        .map(|outline| raster::rasterize(&outline, key.px()))
                        .unwrap_or_default();
                    let result = Rasterised {
                        key,
                        coverage: Arc::new(coverage),
                        ms: started.elapsed().as_secs_f64() * 1000.,
                    };
                    if outbox.send(result).is_err() {
                        break;
                    }
                }
            })
            .expect("glyph worker thread");
        Self {
            requests,
            results,
            pending: HashSet::new(),
        }
    }

    /// Asks for a glyph once; repeated requests while it is being rasterised are ignored.
    pub fn request(&mut self, key: GlyphKey) {
        if self.pending.insert(key) && self.requests.send(key).is_err() {
            self.pending.remove(&key);
        }
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Everything finished so far; with `wait`, blocks up to that long for the first result.
    pub fn receive(&mut self, wait: Option<Duration>) -> Vec<Rasterised> {
        let mut out = Vec::new();
        if let Some(wait) = wait.filter(|_| !self.pending.is_empty()) {
            match self.results.recv_timeout(wait) {
                Ok(result) => out.push(result),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
            }
        }
        out.extend(self.results.try_iter());
        for result in &out {
            self.pending.remove(&result.key);
        }
        out
    }
}

/// The CPU copies of rasterised glyphs, least recently used first out.
#[derive(Default)]
pub struct Bitmaps {
    entries: HashMap<GlyphKey, (Arc<Coverage>, u64)>,
    bytes: usize,
}

impl Bitmaps {
    pub fn insert(&mut self, key: GlyphKey, coverage: Arc<Coverage>, frame: u64) {
        self.bytes += coverage.data.len();
        if let Some((old, _)) = self.entries.insert(key, (coverage, frame)) {
            self.bytes -= old.data.len();
        }
    }

    pub fn get(&mut self, key: &GlyphKey, frame: u64) -> Option<Arc<Coverage>> {
        let (coverage, used) = self.entries.get_mut(key)?;
        *used = frame;
        Some(coverage.clone())
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Drops the least recently used bitmaps not used in `frame` until within budget.
    pub fn trim(&mut self, frame: u64) {
        if self.bytes <= BITMAP_BUDGET && self.entries.len() <= BITMAP_ENTRIES {
            return;
        }
        let mut old: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, (_, used))| *used != frame)
            .map(|(key, (_, used))| (*key, *used))
            .collect();
        old.sort_by_key(|(_, used)| *used);
        for (key, _) in old {
            if self.bytes <= BITMAP_BUDGET && self.entries.len() <= BITMAP_ENTRIES {
                break;
            }
            if let Some((coverage, _)) = self.entries.remove(&key) {
                self.bytes -= coverage.data.len();
            }
        }
    }
}

/// Where a glyph sits in the atlas, in texels, and its bitmap's offset from the pen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
}

/// Shelf packing with a one-texel gap around every glyph, so linear filtering never reads a
/// neighbour. Nothing is freed individually: when full, the whole atlas is cleared and the
/// current frame's glyphs are packed again from their CPU copies.
pub struct Packer {
    pub size: u32,
    shelves: Vec<Shelf>,
    bottom: u32,
}

struct Shelf {
    y: u32,
    height: u32,
    x: u32,
}

impl Packer {
    pub fn new(size: u32) -> Self {
        Self {
            size,
            shelves: Vec::new(),
            bottom: 0,
        }
    }

    pub fn clear(&mut self) {
        self.shelves.clear();
        self.bottom = 0;
    }

    /// A free `width` × `height` area's top-left texel, or `None` when the atlas is full.
    pub fn allocate(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        let (w, h) = (width + 1, height + 1);
        if w + 1 > self.size || h + 1 > self.size {
            return None;
        }
        // The lowest shelf it fits without wasting more than a third of the shelf's height.
        let best = self
            .shelves
            .iter_mut()
            .filter(|shelf| shelf.height >= h && shelf.height <= h + h / 3 + 2)
            .filter(|shelf| shelf.x + w < self.size)
            .min_by_key(|shelf| shelf.height);
        if let Some(shelf) = best {
            let at = (shelf.x + 1, shelf.y + 1);
            shelf.x += w;
            return Some(at);
        }
        if self.bottom + h + 1 > self.size {
            return None;
        }
        let shelf = Shelf {
            y: self.bottom,
            height: h,
            x: w,
        };
        self.bottom += h;
        let at = (1, shelf.y + 1);
        self.shelves.push(shelf);
        Some(at)
    }
}

/// The atlas's bookkeeping: which glyphs it holds where, and when each was last drawn.
pub struct Atlas {
    pub packer: Packer,
    pub max_size: u32,
    slots: HashMap<GlyphKey, (Slot, u64)>,
    /// The size most recently packed for each glyph, to draw scaled while another size is
    /// still being rasterised.
    nearest: HashMap<(u16, u16), u32>,
}

impl Atlas {
    pub fn new(size: u32, max_size: u32) -> Self {
        Self {
            packer: Packer::new(size),
            max_size: max_size.max(size),
            slots: HashMap::new(),
            nearest: HashMap::new(),
        }
    }

    pub fn get(&mut self, key: &GlyphKey, frame: u64) -> Option<Slot> {
        let (slot, used) = self.slots.get_mut(key)?;
        *used = frame;
        Some(*slot)
    }

    /// A packed size of the same glyph, for a scaled stand-in.
    pub fn stand_in(&mut self, key: &GlyphKey, frame: u64) -> Option<(GlyphKey, Slot)> {
        let size = *self.nearest.get(&(key.font, key.glyph))?;
        let other = GlyphKey { size, ..*key };
        self.get(&other, frame).map(|slot| (other, slot))
    }

    /// Reserves room for `coverage`; `None` when the atlas is full.
    pub fn insert(&mut self, key: GlyphKey, coverage: &Coverage, frame: u64) -> Option<Slot> {
        let (x, y) = if coverage.width == 0 || coverage.height == 0 {
            (0, 0)
        } else {
            self.packer.allocate(coverage.width, coverage.height)?
        };
        let slot = Slot {
            x,
            y,
            width: coverage.width,
            height: coverage.height,
            left: coverage.left,
            top: coverage.top,
        };
        self.slots.insert(key, (slot, frame));
        self.nearest.insert((key.font, key.glyph), key.size);
        Some(slot)
    }

    /// Whether every packed glyph was drawn in `frame` (then only a larger atlas helps).
    pub fn all_used_in(&self, frame: u64) -> bool {
        self.slots.values().all(|(_, used)| *used == frame)
    }

    pub fn clear(&mut self) {
        self.packer.clear();
        self.slots.clear();
        self.nearest.clear();
    }

    /// Doubles the atlas (clearing it) while below its maximum; false at the maximum.
    pub fn grow(&mut self) -> bool {
        if self.packer.size >= self.max_size {
            return false;
        }
        self.packer = Packer::new((self.packer.size * 2).min(self.max_size));
        self.slots.clear();
        self.nearest.clear();
        true
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap(width: u32, height: u32) -> Coverage {
        Coverage {
            left: -1,
            top: -(height as i32),
            width,
            height,
            data: vec![255; (width * height) as usize],
        }
    }

    #[test]
    fn packed_glyphs_never_overlap_or_touch_and_the_atlas_reports_full() {
        let mut packer = Packer::new(64);
        let mut placed: Vec<[u32; 4]> = Vec::new();
        while let Some((x, y)) = packer.allocate(7, 9 + placed.len() as u32 % 3) {
            let h = 9 + placed.len() as u32 % 3;
            assert!(x >= 1 && y >= 1 && x + 7 < 64 && y + h < 64);
            for other in &placed {
                // At least one texel between any two glyphs.
                let apart = x > other[0] + other[2]
                    || other[0] > x + 7
                    || y > other[1] + other[3]
                    || other[1] > y + h;
                assert!(apart, "{:?} touches {other:?}", [x, y, 7, h]);
            }
            placed.push([x, y, 7, h]);
        }
        assert!(placed.len() > 20, "{} glyphs", placed.len());
        assert!(packer.allocate(100, 4).is_none(), "wider than the atlas");
        packer.clear();
        assert_eq!(packer.allocate(7, 9), Some((1, 1)));
    }

    #[test]
    fn a_full_atlas_grows_once_then_clears_and_sizes_round_to_quarter_pixels() {
        let mut atlas = Atlas::new(32, 64);
        let key = |glyph| GlyphKey::new(400, glyph, 29.06);
        assert_eq!(key(1).px(), 29.0);
        assert_eq!(GlyphKey::new(400, 1, 29.13).px(), 29.25);
        let mut glyph = 0;
        while atlas.insert(key(glyph), &bitmap(10, 10), 1).is_some() {
            glyph += 1;
        }
        assert!(atlas.all_used_in(1));
        assert!(atlas.grow());
        assert_eq!(atlas.packer.size, 64);
        assert_eq!(atlas.len(), 0);
        assert!(!atlas.grow(), "bounded");
        // Spaces take no room; a different size of a packed glyph stands in meanwhile.
        assert!(atlas.insert(key(100), &Coverage::default(), 2).is_some());
        let slot = atlas.insert(key(5), &bitmap(10, 10), 2).unwrap();
        let wanted = GlyphKey::new(400, 5, 40.);
        assert_eq!(atlas.stand_in(&wanted, 3), Some((key(5), slot)));
        assert!(!atlas.all_used_in(3));
        atlas.clear();
        assert!(atlas.get(&key(5), 4).is_none());
    }

    #[test]
    fn the_bitmap_cache_keeps_the_current_frame_and_drops_the_oldest() {
        let mut bitmaps = Bitmaps::default();
        let big = Arc::new(bitmap(1024, 1024));
        for glyph in 0..12 {
            bitmaps.insert(GlyphKey::new(400, glyph, 30.), big.clone(), glyph as u64);
        }
        bitmaps.trim(11);
        assert!(bitmaps.bytes() <= BITMAP_BUDGET);
        assert!(bitmaps.get(&GlyphKey::new(400, 11, 30.), 12).is_some());
        assert!(bitmaps.get(&GlyphKey::new(400, 0, 30.), 12).is_none());
    }
}
