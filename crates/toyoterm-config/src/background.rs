use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Decoded on the script thread; immutable pixels are shared with the renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackgroundImage {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl BackgroundImage {
    pub fn load(path: &Path) -> Result<Self, image::ImageError> {
        let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        // Within wgpu's default maximum 2D texture dimension on all backends.
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let pixels = reader.decode()?.into_rgba8();
        Ok(Self {
            path: path.to_owned(),
            width: pixels.width(),
            height: pixels.height(),
            rgba: pixels.into_raw().into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_png_and_jpeg_and_rejects_invalid_images() {
        let directory = std::env::temp_dir().join(format!(
            "toyoterm-image-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let image = image::RgbImage::from_pixel(2, 3, image::Rgb([255, 0, 0]));
        for extension in ["png", "jpg"] {
            let path = directory.join(format!("wallpaper.{extension}"));
            image.save(&path).unwrap();
            let loaded = BackgroundImage::load(&path).unwrap();
            assert_eq!((loaded.width, loaded.height), (2, 3));
            assert_eq!(loaded.rgba.len(), 24);
            assert_eq!(loaded.rgba[3], 255);
        }
        let oversized = directory.join("oversized.png");
        image::RgbImage::new(8193, 1).save(&oversized).unwrap();
        assert!(BackgroundImage::load(&oversized).is_err());
        let invalid = directory.join("invalid.png");
        std::fs::write(&invalid, b"not an image").unwrap();
        assert!(BackgroundImage::load(&invalid).is_err());
        assert!(BackgroundImage::load(&directory.join("missing.png")).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
