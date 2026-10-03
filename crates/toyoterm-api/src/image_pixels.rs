use std::borrow::Cow;
use std::io::{self, Read, Write};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};

/// Immutable, lossless RGBA storage shared by configuration and GPU uploads.
/// Large compressible images need no persistent decoded CPU copy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImagePixels {
    storage: Storage,
    decoded_len: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Storage {
    Raw(Vec<u8>),
    Zlib(Vec<u8>),
}

impl ImagePixels {
    pub fn new(rgba: Vec<u8>) -> Self {
        let decoded_len = rgba.len();
        // Avoid compression overhead for tiny images and retain raw storage
        // when the savings would be negligible (or encoding fails).
        if decoded_len >= 64 * 1024 {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
            if encoder.write_all(&rgba).is_ok()
                && let Ok(mut compressed) = encoder.finish()
                && compressed.len() <= decoded_len - decoded_len / 8
            {
                compressed.shrink_to_fit();
                return Self {
                    storage: Storage::Zlib(compressed),
                    decoded_len,
                };
            }
        }
        Self {
            storage: Storage::Raw(rgba),
            decoded_len,
        }
    }

    pub fn stored_len(&self) -> usize {
        match &self.storage {
            Storage::Raw(bytes) | Storage::Zlib(bytes) => bytes.len(),
        }
    }

    /// Borrow raw pixels or expand compressed pixels for a single GPU upload.
    /// No decoded copy is cached; subsequent uploads reconstruct identical bytes.
    pub fn rgba(&self) -> io::Result<Cow<'_, [u8]>> {
        match &self.storage {
            Storage::Raw(bytes) => Ok(Cow::Borrowed(bytes)),
            Storage::Zlib(bytes) => {
                let decoder = ZlibDecoder::new(bytes.as_slice());
                let mut rgba = Vec::with_capacity(self.decoded_len);
                decoder
                    .take(self.decoded_len as u64 + 1)
                    .read_to_end(&mut rgba)?;
                if rgba.len() != self.decoded_len {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "background pixel length changed during expansion",
                    ));
                }
                Ok(Cow::Owned(rgba))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_pixels_preserve_color_alpha_and_repeated_uploads() {
        let rgba = [13, 77, 209, 128, 255, 0, 93, 0].repeat(128 * 1024);
        let pixels = ImagePixels::new(rgba.clone());
        assert!(pixels.stored_len() < rgba.len() / 8);
        for _ in 0..3 {
            assert_eq!(pixels.rgba().unwrap().as_ref(), rgba);
        }
    }

    #[test]
    fn small_and_incompressible_pixels_stay_raw() {
        let small = ImagePixels::new(vec![255, 0, 0, 128]);
        assert!(matches!(small.rgba().unwrap(), Cow::Borrowed(_)));
        let mut state = 1_u32;
        let bytes = (0..128 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect::<Vec<_>>();
        let pixels = ImagePixels::new(bytes.clone());
        assert_eq!(pixels.stored_len(), bytes.len());
        assert!(matches!(pixels.rgba().unwrap(), Cow::Borrowed(_)));
        assert_eq!(pixels.rgba().unwrap().as_ref(), bytes);
    }

    #[test]
    fn expansion_rejects_corrupt_or_wrong_length_data() {
        let pixels = ImagePixels {
            storage: Storage::Zlib(vec![0; 16]),
            decoded_len: 4,
        };
        assert!(pixels.rgba().is_err());
        let mut pixels = ImagePixels::new(vec![0; 128 * 1024]);
        pixels.decoded_len = 4;
        assert!(pixels.rgba().is_err());
    }
}
