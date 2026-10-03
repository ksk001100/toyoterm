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

    /// Visit consecutive RGBA chunks without allocating a full decoded image.
    /// The final chunk may be shorter. Offsets are in bytes; no copy is cached.
    pub fn for_each_rgba_chunk(
        &self,
        chunk_bytes: usize,
        mut visit: impl FnMut(usize, &[u8]),
    ) -> io::Result<()> {
        if chunk_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty pixel chunk",
            ));
        }
        match &self.storage {
            Storage::Raw(bytes) => {
                for (index, chunk) in bytes.chunks(chunk_bytes).enumerate() {
                    visit(index * chunk_bytes, chunk);
                }
            }
            Storage::Zlib(bytes) => {
                let mut decoder = ZlibDecoder::new(bytes.as_slice());
                let mut chunk = vec![0; chunk_bytes.min(self.decoded_len)];
                let mut offset = 0;
                while offset < self.decoded_len {
                    let len = chunk.len().min(self.decoded_len - offset);
                    decoder.read_exact(&mut chunk[..len])?;
                    visit(offset, &chunk[..len]);
                    offset += len;
                }
                if decoder.read(&mut [0])? != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "background pixel length changed during expansion",
                    ));
                }
            }
        }
        Ok(())
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
    fn chunked_expansion_preserves_offsets_and_partial_final_chunks() {
        for len in [37, 128 * 1024 + 17] {
            let rgba: Vec<_> = (0..len).map(|index| (index % 251) as u8).collect();
            let pixels = ImagePixels::new(rgba.clone());
            for chunk_bytes in [1, 1028, len + 1] {
                let mut restored = Vec::new();
                pixels
                    .for_each_rgba_chunk(chunk_bytes, |offset, chunk| {
                        assert_eq!(offset, restored.len());
                        assert!(chunk.len() <= chunk_bytes);
                        restored.extend_from_slice(chunk);
                    })
                    .unwrap();
                assert_eq!(restored, rgba);
            }
            assert!(pixels.for_each_rgba_chunk(0, |_, _| {}).is_err());
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
        assert!(pixels.for_each_rgba_chunk(3, |_, _| {}).is_err());
        let mut pixels = ImagePixels::new(vec![0; 128 * 1024]);
        pixels.decoded_len = 4;
        assert!(pixels.rgba().is_err());
        assert!(pixels.for_each_rgba_chunk(3, |_, _| {}).is_err());
        pixels.decoded_len = 256 * 1024;
        assert!(pixels.for_each_rgba_chunk(1028, |_, _| {}).is_err());
    }
}
