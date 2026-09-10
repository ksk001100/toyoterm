use super::{MAX_SIDE, Pixels, pixel_length};
use std::sync::Arc;

pub(super) fn decode(payload: &[u8], terminal_background: [u8; 4]) -> Option<Pixels> {
    let header = payload.iter().position(|b| *b == b'q')?;
    if !payload[..header]
        .iter()
        .all(|b| b.is_ascii_digit() || *b == b';')
    {
        return None;
    }
    let transparent = payload[..header].split(|b| *b == b';').nth(1) == Some(b"1".as_slice());
    let data = &payload[header + 1..];
    let mut palette = [[0, 0, 0, 255]; 256];
    for (i, color) in palette.iter_mut().enumerate() {
        let level = i as u8;
        *color = [level, level, level, 255];
    }
    palette[..16].copy_from_slice(&[
        [0, 0, 0, 255],
        [51, 51, 204, 255],
        [204, 33, 33, 255],
        [51, 204, 51, 255],
        [204, 51, 204, 255],
        [51, 204, 204, 255],
        [204, 204, 51, 255],
        [135, 135, 135, 255],
        [66, 66, 66, 255],
        [84, 84, 153, 255],
        [153, 66, 66, 255],
        [84, 153, 84, 255],
        [153, 84, 153, 255],
        [84, 153, 153, 255],
        [153, 153, 84, 255],
        [204, 204, 204, 255],
    ]);
    // Sparse rows avoid allocating an attacker-controlled stride on every raster command.
    let mut raster: Vec<Vec<[u8; 4]>> = Vec::new();
    let mut x = 0u32;
    let mut y = 0u32;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut color = 0usize;
    let mut index = 0;
    let background = if transparent {
        [0, 0, 0, 0]
    } else {
        terminal_background
    };
    let mut declared_height = None;
    while index < data.len() {
        let command = data[index];
        index += 1;
        match command {
            b'"' | b'#' => {
                let mut params = Vec::new();
                loop {
                    params.push(integer(data, &mut index)?);
                    if data.get(index) != Some(&b';') {
                        break;
                    }
                    index += 1;
                    if params.len() >= 5 {
                        return None;
                    }
                }
                if command == b'"' {
                    if !matches!(params.len(), 2 | 4) {
                        return None;
                    }
                    if params.len() == 4 {
                        width = width.max(params[2]);
                        height = height.max(params[3]);
                        declared_height = Some(params[3]);
                        pixel_length(width.max(1), height.max(1))?;
                    }
                } else {
                    color = params[0] as usize;
                    if color >= palette.len() {
                        return None;
                    }
                    if params.len() == 5 {
                        palette[color] = match params[1] {
                            2 if params[2..].iter().all(|p| *p <= 100) => [
                                (params[2] * 255 / 100) as u8,
                                (params[3] * 255 / 100) as u8,
                                (params[4] * 255 / 100) as u8,
                                255,
                            ],
                            1 if params[2] <= 360 && params[3] <= 100 && params[4] <= 100 => {
                                hls(params[2], params[3], params[4])
                            }
                            _ => return None,
                        };
                    } else if params.len() != 1 {
                        return None;
                    }
                }
            }
            b'$' => x = 0,
            b'-' => {
                x = 0;
                y = y.checked_add(6)?;
                if y > MAX_SIDE {
                    return None;
                }
            }
            b'!' | b'?'..=b'~' => {
                let (count, bits) = if command == b'!' {
                    let count = integer(data, &mut index)?.max(1);
                    let b = *data.get(index)?;
                    index += 1;
                    if !(b'?'..=b'~').contains(&b) {
                        return None;
                    }
                    (count, b - b'?')
                } else {
                    (1, command - b'?')
                };
                let end = x.checked_add(count)?;
                width = width.max(end);
                // A sixel advances a full six-pixel band. A declared raster height can
                // describe a shorter final band, but does not limit later data.
                let band_end = y.checked_add(6)?;
                let data_height = declared_height
                    .filter(|declared| *declared > y && *declared < band_end)
                    .unwrap_or(band_end);
                height = height.max(data_height);
                pixel_length(width, height)?;
                raster.resize_with(height as usize, Vec::new);
                for bit in 0..6 {
                    if y + bit >= height {
                        break;
                    }
                    let row = &mut raster[(y + bit) as usize];
                    if row.len() < end as usize {
                        row.resize(end as usize, background);
                    }
                    if bits & (1 << bit) != 0 {
                        row[x as usize..end as usize].fill(palette[color]);
                    }
                }
                x = end;
            }
            b'\r' | b'\n' => {}
            _ => return None,
        }
    }
    let len = pixel_length(width, height)?;
    let mut rgba = Vec::with_capacity(len);
    for row in 0..height as usize {
        for col in 0..width as usize {
            rgba.extend_from_slice(
                raster
                    .get(row)
                    .and_then(|r| r.get(col))
                    .unwrap_or(&background),
            );
        }
    }
    Some(Pixels {
        width,
        height,
        rgba: Arc::from(rgba),
    })
}
fn integer(data: &[u8], index: &mut usize) -> Option<u32> {
    let start = *index;
    let mut n = 0u32;
    while let Some(b) = data.get(*index).filter(|b| b.is_ascii_digit()) {
        n = n.checked_mul(10)?.checked_add(u32::from(*b - b'0'))?;
        *index += 1;
    }
    (*index > start).then_some(n)
}
fn hls(hue: u32, light: u32, saturation: u32) -> [u8; 4] {
    // DEC HLS: blue=0, red=120, green=240.
    let hue = f64::from((hue + 240) % 360) / 60.0;
    let light = f64::from(light) / 100.0;
    let chroma = (1.0 - (2.0 * light - 1.0).abs()) * f64::from(saturation) / 100.0;
    let x = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
    let rgb = match hue as u32 {
        0 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        _ => [chroma, 0.0, x],
    };
    let m = light - chroma / 2.0;
    [
        ((rgb[0] + m) * 255.0).round() as u8,
        ((rgb[1] + m) * 255.0).round() as u8,
        ((rgb[2] + m) * 255.0).round() as u8,
        255,
    ]
}
