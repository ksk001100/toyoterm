//! Bounded, in-band terminal graphics. No protocol may read local files.
use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};

pub(crate) mod handler;
mod sixel;
pub(crate) mod stream;

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_STORED_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGES: usize = 128;
const MAX_SIDE: u32 = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalImage {
    /// Unique revision, independent of the client-supplied Kitty image ID.
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub column: u16,
    /// May be negative when an image is partially above the viewport.
    pub row: i32,
    pub display_width: u32,
    pub display_height: u32,
    pub columns: u16,
    pub rows: u16,
}

struct Placement {
    image: TerminalImage,
    kitty_id: u32,
    placement_id: u32,
    alternate: bool,
}

#[derive(Clone)]
struct Pixels {
    width: u32,
    height: u32,
    rgba: Arc<[u8]>,
}

#[derive(Default)]
pub(crate) struct Graphics {
    placements: Vec<Placement>,
    stored: BTreeMap<u32, Pixels>,
    transfer: Option<(BTreeMap<String, String>, Vec<u8>)>,
    serial: u64,
    pub region: Option<(i32, i32)>,
    pub cell_size: (u16, u16),
}

pub(crate) struct GraphicResult {
    pub advance: Option<(u16, u16)>,
    pub reply: Option<String>,
}

impl Graphics {
    pub fn has_placements(&self) -> bool {
        !self.placements.is_empty()
    }
    pub fn new() -> Self {
        Self {
            cell_size: (8, 16),
            ..Self::default()
        }
    }
    pub fn snapshot(&self, alternate: bool, offset: i32, rows: u16) -> Vec<TerminalImage> {
        self.placements
            .iter()
            .filter_map(|p| {
                let mut image = p.image.clone();
                image.row += offset;
                (p.alternate == alternate
                    && image.row < i32::from(rows)
                    && image.row + i32::from(image.rows) > 0)
                    .then_some(image)
            })
            .collect()
    }

    pub fn reset(&mut self) {
        self.placements.clear();
        self.stored.clear();
        self.transfer = None;
        self.region = None;
    }

    pub fn cancel_transfer(&mut self) {
        self.transfer = None;
    }

    pub fn clear(&mut self, alternate: bool, start: i32, end: i32) {
        self.placements.retain(|p| {
            p.alternate != alternate
                || p.image.row >= end
                || p.image.row + i32::from(p.image.rows) <= start
        });
    }

    pub fn scroll(&mut self, alternate: bool, top: i32, bottom: i32, amount: i32, history: usize) {
        self.placements.retain_mut(|p| {
            if p.alternate == alternate
                && p.image.row < bottom
                && (p.image.row >= top || (top == 0 && amount > 0))
            {
                p.image.row -= amount;
                let minimum = if top == 0 && !alternate {
                    -(history as i32)
                } else {
                    top
                };
                return p.image.row + i32::from(p.image.rows) > minimum && p.image.row < bottom;
            }
            true
        });
    }

    fn size(&self) -> (u32, u32) {
        (
            u32::from(self.cell_size.0.max(1)),
            u32::from(self.cell_size.1.max(1)),
        )
    }

    fn place(
        &mut self,
        pixels: Pixels,
        at: (u16, i32),
        cells: (u16, u16),
        ids: (u32, u32),
        alternate: bool,
        display: (u32, u32),
    ) {
        if ids.0 != 0 {
            self.placements.retain(|p| {
                p.kitty_id != ids.0 || p.placement_id != ids.1 || p.alternate != alternate
            });
        }
        self.serial = self.serial.wrapping_add(1);
        self.placements.push(Placement {
            image: TerminalImage {
                id: self.serial,
                width: pixels.width,
                height: pixels.height,
                rgba: pixels.rgba,
                column: at.0,
                row: at.1,
                columns: cells.0,
                rows: cells.1,
                display_width: display.0,
                display_height: display.1,
            },
            kitty_id: ids.0,
            placement_id: ids.1,
            alternate,
        });
        // Count shared pixels conservatively; eviction is deterministic and bounded.
        while self.placements.len() > MAX_IMAGES
            || self
                .placements
                .iter()
                .map(|p| p.image.rgba.len())
                .sum::<usize>()
                > MAX_STORED_BYTES
        {
            self.placements.remove(0);
        }
    }

    pub fn receive(
        &mut self,
        kind: u8,
        payload: &[u8],
        at: (u16, i32),
        screen: (u16, u16),
        alternate: bool,
    ) -> GraphicResult {
        let mut result = GraphicResult {
            advance: None,
            reply: None,
        };
        if kind == b'_' {
            return self.kitty(payload, at, alternate);
        }
        let decoded = if kind == b'P' {
            sixel::decode(payload).map(|p| (p, None))
        } else {
            iterm(payload, self.size(), screen)
        };
        if let Some((pixels, requested)) = decoded {
            let cell = self.size();
            let display = requested.unwrap_or((pixels.width, pixels.height));
            let cells = (
                display.0.div_ceil(cell.0).min(u32::from(u16::MAX)) as u16,
                display.1.div_ceil(cell.1).min(u32::from(u16::MAX)) as u16,
            );
            self.place(pixels, at, cells, (0, 0), alternate, display);
            result.advance = Some(cells);
        }
        result
    }

    fn kitty(&mut self, payload: &[u8], at: (u16, i32), alternate: bool) -> GraphicResult {
        let mut result = GraphicResult {
            advance: None,
            reply: None,
        };
        let Some(payload) = payload.strip_prefix(b"G") else {
            return result;
        };
        let (header, data) = payload.split_once_byte(b';');
        let Ok(header) = std::str::from_utf8(header) else {
            return result;
        };
        let mut keys: BTreeMap<String, String> = header
            .split(',')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let mut data = data.to_vec();
        if let Some((previous, mut bytes)) = self.transfer.take() {
            if bytes.len().saturating_add(data.len()) > MAX_BYTES {
                return result;
            }
            bytes.extend_from_slice(&data);
            data = bytes;
            let more = keys.remove("m");
            keys = previous;
            if let Some(more) = more {
                keys.insert("m".into(), more);
            } else {
                keys.remove("m");
            }
        }
        if value(&keys, "m", "0") == "1" {
            if data.len() <= MAX_BYTES {
                self.transfer = Some((keys, data));
            }
            return result;
        }
        let id = number(&keys, "i", 0);
        let placement = number(&keys, "p", 0);
        let quiet = number(&keys, "q", 0);
        let action = value(&keys, "a", "t");
        let attempt = (|| -> Result<(), &'static str> {
            if action == "d" {
                let delete = value(&keys, "d", "a");
                match delete {
                    "a" | "A" => self.placements.retain(|p| p.alternate != alternate),
                    "i" | "I" => self.placements.retain(|p| {
                        p.alternate != alternate
                            || p.kitty_id != id
                            || (placement != 0 && p.placement_id != placement)
                    }),
                    _ => return Err("ENOTSUP:unsupported delete selector"),
                }
                if delete == "I" {
                    self.stored.remove(&id);
                }
                if delete == "A" {
                    self.stored.clear();
                }
                return Ok(());
            }
            if !matches!(action, "t" | "T" | "p" | "q") {
                return Err("ENOTSUP:unsupported action");
            }
            // Reject advanced placement semantics instead of silently displaying incorrectly.
            if ["U", "P", "Q", "x", "y", "w", "h", "X", "Y", "z", "I"]
                .iter()
                .any(|k| number(&keys, k, 0) != 0)
            {
                return Err("ENOTSUP:unsupported placement");
            }
            let pixels = if action == "p" {
                self.stored
                    .get(&id)
                    .cloned()
                    .ok_or("ENOENT:unknown image")?
            } else {
                if value(&keys, "t", "d") != "d" {
                    return Err("ENOTSUP:direct transmission required");
                }
                let mut bytes = STANDARD
                    .decode(&data)
                    .map_err(|_| "EINVAL:invalid base64")?;
                match value(&keys, "o", "") {
                    "z" => {
                        let mut inflated = Vec::new();
                        flate2::read::ZlibDecoder::new(bytes.as_slice())
                            .take(MAX_BYTES as u64 + 1)
                            .read_to_end(&mut inflated)
                            .map_err(|_| "EINVAL:invalid zlib data")?;
                        if inflated.len() > MAX_BYTES {
                            return Err("E2BIG:image too large");
                        }
                        bytes = inflated;
                    }
                    "" => {}
                    _ => return Err("ENOTSUP:unsupported compression"),
                }
                match number(&keys, "f", 32) {
                    100 => decode_image(&bytes, true).ok_or("EINVAL:invalid PNG")?,
                    format @ (24 | 32) => {
                        let width = number(&keys, "s", 0);
                        let height = number(&keys, "v", 0);
                        let length =
                            pixel_length(width, height).ok_or("E2BIG:invalid dimensions")?;
                        if bytes.len() != length / 4 * (format as usize / 8) {
                            return Err("EINVAL:pixel length mismatch");
                        }
                        let rgba = if format == 24 {
                            bytes
                                .as_chunks::<3>()
                                .0
                                .iter()
                                .flat_map(|p| [p[0], p[1], p[2], 255])
                                .collect()
                        } else {
                            bytes
                        };
                        Pixels {
                            width,
                            height,
                            rgba: Arc::from(rgba),
                        }
                    }
                    _ => return Err("ENOTSUP:unsupported format"),
                }
            };
            if action == "q" {
                return Ok(());
            }
            if action != "p" && id != 0 {
                let bytes: usize = self
                    .stored
                    .iter()
                    .filter(|(key, _)| **key != id)
                    .map(|(_, p)| p.rgba.len())
                    .sum();
                if bytes + pixels.rgba.len() > MAX_STORED_BYTES
                    || (self.stored.len() >= MAX_IMAGES && !self.stored.contains_key(&id))
                {
                    return Err("ENOSPC:image storage full");
                }
                self.stored.insert(id, pixels.clone());
            }
            if action == "t" {
                return Ok(());
            }
            let cell = self.size();
            let columns = number(&keys, "c", pixels.width.div_ceil(cell.0));
            let rows = number(&keys, "r", pixels.height.div_ceil(cell.1));
            if columns == 0
                || rows == 0
                || columns > u32::from(u16::MAX)
                || rows > u32::from(u16::MAX)
            {
                return Err("EINVAL:invalid placement size");
            }
            let display = match (keys.contains_key("c"), keys.contains_key("r")) {
                (false, false) => (pixels.width, pixels.height),
                (true, false) => (
                    columns * cell.0,
                    ((u64::from(pixels.height) * u64::from(columns) * u64::from(cell.0))
                        / u64::from(pixels.width))
                    .min(u64::from(u32::MAX)) as u32,
                ),
                (false, true) => (
                    ((u64::from(pixels.width) * u64::from(rows) * u64::from(cell.1))
                        / u64::from(pixels.height))
                    .min(u64::from(u32::MAX)) as u32,
                    rows * cell.1,
                ),
                (true, true) => (columns * cell.0, rows * cell.1),
            };
            let display = (display.0.max(1), display.1.max(1));
            let cells = (
                display.0.div_ceil(cell.0).min(u32::from(u16::MAX)) as u16,
                display.1.div_ceil(cell.1).min(u32::from(u16::MAX)) as u16,
            );
            self.place(pixels, at, cells, (id, placement), alternate, display);
            if number(&keys, "C", 0) == 0 {
                result.advance = Some(cells);
            }
            Ok(())
        })();
        if quiet < 2 && (attempt.is_err() || quiet == 0) && (id != 0 || attempt.is_err()) {
            let status = attempt.err().unwrap_or("OK");
            let placement = if placement == 0 {
                String::new()
            } else {
                format!(",p={placement}")
            };
            result.reply = Some(format!("\x1b_Gi={id}{placement};{status}\x1b\\"));
        }
        result
    }
}

trait SplitByte {
    fn split_once_byte(&self, byte: u8) -> (&[u8], &[u8]);
}
impl SplitByte for [u8] {
    fn split_once_byte(&self, byte: u8) -> (&[u8], &[u8]) {
        self.iter()
            .position(|b| *b == byte)
            .map_or((self, &[]), |i| (&self[..i], &self[i + 1..]))
    }
}
fn value<'a>(keys: &'a BTreeMap<String, String>, key: &str, default: &'a str) -> &'a str {
    keys.get(key).map_or(default, String::as_str)
}
fn number(keys: &BTreeMap<String, String>, key: &str, default: u32) -> u32 {
    keys.get(key)
        .map_or(default, |v| v.parse().unwrap_or(u32::MAX))
}
fn pixel_length(width: u32, height: u32) -> Option<usize> {
    let length = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    (width > 0 && height > 0 && width <= MAX_SIDE && height <= MAX_SIDE && length <= MAX_BYTES)
        .then_some(length)
}
fn decode_image(bytes: &[u8], png_only: bool) -> Option<Pixels> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if png_only && reader.format() != Some(image::ImageFormat::Png) {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_BYTES as u64);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    pixel_length(image.width(), image.height())?;
    Some(Pixels {
        width: image.width(),
        height: image.height(),
        rgba: Arc::from(image.into_rgba8().into_raw()),
    })
}

fn iterm(
    payload: &[u8],
    cell: (u32, u32),
    screen: (u16, u16),
) -> Option<(Pixels, Option<(u32, u32)>)> {
    let payload = payload.strip_prefix(b"1337;File=")?;
    let (header, data) = payload.split_once_byte(b':');
    let header = std::str::from_utf8(header).ok()?;
    let keys: BTreeMap<String, String> = header
        .split(';')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    if value(&keys, "inline", "0") != "1" {
        return None;
    }
    let bytes = STANDARD.decode(data).ok()?;
    if keys.contains_key("size") && number(&keys, "size", 0) as usize != bytes.len() {
        return None;
    }
    let pixels = decode_image(&bytes, false)?;
    let dimension = |key, native: u32, cell: u32, screen: u16| -> Option<u32> {
        let val = value(&keys, key, "auto");
        if val == "auto" {
            Some(native)
        } else if let Some(px) = val.strip_suffix("px") {
            px.parse().ok()
        } else if let Some(percent) = val.strip_suffix('%') {
            Some(percent.parse::<u32>().ok()?.min(100) * u32::from(screen) * cell / 100)
        } else {
            val.parse::<u32>().ok()?.checked_mul(cell)
        }
    };
    let mut width = dimension("width", pixels.width, cell.0, screen.0)?;
    let mut height = dimension("height", pixels.height, cell.1, screen.1)?;
    if value(&keys, "preserveAspectRatio", "1") != "0" {
        let scale = match (
            value(&keys, "width", "auto") == "auto",
            value(&keys, "height", "auto") == "auto",
        ) {
            (false, true) => f64::from(width) / f64::from(pixels.width),
            (true, false) => f64::from(height) / f64::from(pixels.height),
            _ => (f64::from(width) / f64::from(pixels.width))
                .min(f64::from(height) / f64::from(pixels.height)),
        };
        if scale > 0.0 {
            width = ((f64::from(pixels.width) * scale) as u32).max(1);
            height = ((f64::from(pixels.height) * scale) as u32).max(1);
        }
    }
    if width == 0 || height == 0 {
        return None;
    }
    Some((pixels, Some((width, height))))
}
