//! Immutable jobs for the host's file worker. The VT parser performs no file I/O.
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::{Engine, engine::general_purpose::STANDARD};

use super::{MAX_BYTES, Pixels, decode_pixels, number, value};

pub(crate) const FILE_ERROR: &str = "EBADF:Failed to read image file";
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageFileRequest {
    pub(crate) id: u64,
    path: PathBuf,
    offset: u64,
    size: u64,
    format: u32,
    width: u32,
    height: u32,
    compression: String,
}

#[derive(Debug)]
pub struct ImageFileResult {
    pub(crate) id: u64,
    pub(crate) pixels: Result<Pixels, &'static str>,
}

impl ImageFileRequest {
    pub(super) fn new(keys: &BTreeMap<String, String>, data: &[u8]) -> Result<Self, &'static str> {
        let compression = match value(keys, "o", "") {
            compression @ ("" | "z") => compression.to_owned(),
            _ => return Err("ENOTSUP:unsupported compression"),
        };
        if data.len() > 4096usize.div_ceil(3) * 4 {
            return Err(FILE_ERROR);
        }
        let bytes = STANDARD.decode(data).map_err(|_| FILE_ERROR)?;
        let path = std::str::from_utf8(&bytes).map_err(|_| FILE_ERROR)?;
        if path.len() > 4096 || path.contains('\0') || !Path::new(path).is_absolute() {
            return Err(FILE_ERROR);
        }
        let integer = |key| {
            keys.get(key)
                .map_or(Ok(0), |v| v.parse::<u64>().map_err(|_| FILE_ERROR))
        };
        Ok(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            path: PathBuf::from(path),
            offset: integer("O")?,
            size: integer("S")?,
            format: number(keys, "f", 32),
            width: number(keys, "s", 0),
            height: number(keys, "v", 0),
            compression,
        })
    }

    /// Run on a background worker, never on the terminal/event-loop thread.
    pub fn load(self) -> ImageFileResult {
        let pixels = self.read().map_err(|error| {
            tracing::debug!(target: "toyoterm::image", %error, "read Kitty image file failed");
            FILE_ERROR
        });
        ImageFileResult {
            id: self.id,
            pixels,
        }
    }

    /// Reject an overloaded or stopped worker without revealing filesystem state.
    pub fn failure(&self) -> ImageFileResult {
        ImageFileResult {
            id: self.id,
            pixels: Err(FILE_ERROR),
        }
    }

    fn read(&self) -> Result<Pixels, String> {
        let error = |e: std::io::Error| e.to_string();
        check_path(&self.path)?;
        let path = fs::canonicalize(&self.path).map_err(error)?;
        check_path(&path)?;
        if !fs::metadata(&path).map_err(error)?.is_file() {
            return Err("not a regular file".into());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Also reject a replaced final symlink and avoid blocking on a FIFO
            // substituted after the pre-open regular-file check.
            options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_NOFOLLOW);
        }
        let mut file = options.open(&path).map_err(error)?;
        let metadata = file.metadata().map_err(error)?;
        if !metadata.is_file() {
            return Err("not a regular file".into());
        }
        let available = metadata
            .len()
            .checked_sub(self.offset)
            .ok_or("invalid offset")?;
        let size = if self.size == 0 { available } else { self.size };
        if size == 0 || size > MAX_BYTES as u64 || size > available {
            return Err("invalid file range".into());
        }
        file.seek(SeekFrom::Start(self.offset)).map_err(error)?;
        let mut bytes = Vec::new();
        file.take(size).read_to_end(&mut bytes).map_err(error)?;
        if bytes.len() as u64 != size {
            return Err("file changed while reading".into());
        }
        decode_pixels(
            self.format,
            self.width,
            self.height,
            &self.compression,
            bytes,
        )
        .map_err(str::to_owned)
    }
}

fn check_path(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    if ["/proc", "/sys", "/dev"]
        .iter()
        .any(|root| path.starts_with(root))
    {
        return Err("sensitive file location".into());
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(path.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return Err("only local disk paths are supported".into());
        }
    }
    Ok(())
}
