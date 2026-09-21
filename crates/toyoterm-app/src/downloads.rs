use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread;

use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use toyoterm_api::PaneId;
use toyoterm_config::BackgroundImage;
use toyoterm_terminal::{FileTransferEntry, FileTransferEntryKind, FileUploadPath};
use winit::event_loop::EventLoopProxy;

use super::AppEvent;

const MAX_PENDING_DOWNLOADS: usize = 16;
const MAX_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILENAME_BYTES: usize = 240;
const MAX_UPLOAD_ENTRIES: usize = 64;
const MAX_UPLOAD_PATH_BYTES: usize = 4 * 1024;
const UPLOAD_CHUNK_BYTES: usize = 4 * 1024;

enum DownloadJob {
    Single {
        directory: PathBuf,
        name: String,
        data: Vec<u8>,
        permissions: Option<u32>,
        modified_ns: Option<u64>,
    },
    Transfer {
        directory: PathBuf,
        entries: Vec<FileTransferEntry>,
    },
    UploadListing {
        pane: PaneId,
        root: PathBuf,
        session_id: String,
        quiet: u8,
        paths: Vec<FileUploadPath>,
        proxy: EventLoopProxy<AppEvent>,
    },
    UploadData {
        pane: PaneId,
        session_id: String,
        file_id: String,
        name: String,
        compressed: bool,
        proxy: EventLoopProxy<AppEvent>,
    },
    UploadCancel {
        pane: PaneId,
        session_id: String,
    },
    UploadCancelPane {
        pane: PaneId,
    },
    BackgroundImage {
        root: PathBuf,
        path: Option<String>,
        proxy: EventLoopProxy<AppEvent>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UploadKind {
    Regular,
    Directory,
    Symlink,
}

struct UploadEntry {
    original_id: String,
    actual_id: String,
    host_path: PathBuf,
    kind: UploadKind,
}

struct UploadSession {
    root: PathBuf,
    entries: HashMap<String, UploadEntry>,
}

static NEXT_TRANSFER_ID: AtomicU64 = AtomicU64::new(1);

static DOWNLOAD_SENDER: OnceLock<SyncSender<DownloadJob>> = OnceLock::new();

pub fn queue(
    directory: &Path,
    name: Option<&str>,
    data: Vec<u8>,
    permissions: Option<u32>,
    modified_ns: Option<u64>,
) -> Result<(), String> {
    if !directory.is_absolute() {
        return Err("OSC download directory must be absolute".into());
    }
    if data.len() > MAX_DOWNLOAD_BYTES {
        return Err("OSC download exceeds the 32 MiB limit".into());
    }
    let job = DownloadJob::Single {
        directory: directory.to_owned(),
        name: safe_filename(name.unwrap_or("Unnamed file")),
        data,
        permissions,
        modified_ns,
    };
    download_sender()
        .try_send(job)
        .map_err(|error| match error {
            TrySendError::Full(_) => "OSC download queue is full".into(),
            TrySendError::Disconnected(_) => "OSC download worker stopped".into(),
        })
}

fn download_sender() -> &'static SyncSender<DownloadJob> {
    DOWNLOAD_SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<DownloadJob>(MAX_PENDING_DOWNLOADS);
        thread::Builder::new()
            .name("toyoterm-osc-download".into())
            .spawn(move || {
                let mut uploads = HashMap::<(PaneId, String), UploadSession>::new();
                while let Ok(job) = receiver.recv() {
                    let result = match job {
                        DownloadJob::Single {
                            directory,
                            name,
                            data,
                            permissions,
                            modified_ns,
                        } => write_download(
                            &directory,
                            &name,
                            &data,
                            permissions,
                            modified_ns,
                        )
                        .map(|_| ()),
                        DownloadJob::Transfer { directory, entries } => {
                            write_transfer(&directory, &entries).map(|_| ())
                        }
                        DownloadJob::UploadListing {
                            pane,
                            root,
                            session_id,
                            quiet,
                            paths,
                            proxy,
                        } => {
                            let (session, responses) = build_upload_listing(
                                &root,
                                &session_id,
                                quiet,
                                &paths,
                            );
                            if let Some(session) = session {
                                uploads.insert((pane, session_id), session);
                            }
                            let _ = proxy.send_event(AppEvent::FileTransferResponse {
                                pane,
                                responses,
                            });
                            Ok(())
                        }
                        DownloadJob::UploadData {
                            pane,
                            session_id,
                            file_id,
                            name,
                            compressed,
                            proxy,
                        } => {
                            let responses = match uploads.get(&(pane, session_id.clone())) {
                                Some(session) => upload_data_responses(
                                    session,
                                    &session_id,
                                    &file_id,
                                    &name,
                                    compressed,
                                ),
                                None => vec![upload_status(
                                    &session_id,
                                    Some(&file_id),
                                    "ENOENT:Unknown upload session",
                                )],
                            };
                            let _ = proxy.send_event(AppEvent::FileTransferResponse {
                                pane,
                                responses,
                            });
                            Ok(())
                        }
                        DownloadJob::UploadCancel { pane, session_id } => {
                            uploads.remove(&(pane, session_id));
                            Ok(())
                        }
                        DownloadJob::UploadCancelPane { pane } => {
                            uploads.retain(|(session_pane, _), _| *session_pane != pane);
                            Ok(())
                        }
                        DownloadJob::BackgroundImage { root, path, proxy } => {
                            let result = path
                                .as_deref()
                                .map(|path| load_background_image(&root, path))
                                .transpose();
                            let _ = proxy.send_event(AppEvent::BackgroundImageLoaded {
                                root,
                                result,
                            });
                            Ok(())
                        }
                    };
                    if let Err(error) = result {
                        tracing::warn!(target: "toyoterm::download", %error, "write OSC download failed");
                    }
                }
            })
            .expect("spawn OSC download worker");
        sender
    })
}

fn write_transfer(directory: &Path, entries: &[FileTransferEntry]) -> Result<PathBuf, String> {
    let metadata = fs::metadata(directory).map_err(|error| error.to_string())?;
    if !metadata.is_dir() {
        return Err("OSC download destination is not a directory".into());
    }
    if entries.is_empty() {
        return Err("OSC file transfer is empty".into());
    }

    let transfer_id = NEXT_TRANSFER_ID.fetch_add(1, Ordering::Relaxed);
    let staging = directory.join(format!(
        ".toyoterm-transfer-{}-{transfer_id}",
        std::process::id()
    ));
    fs::create_dir(&staging).map_err(|error| error.to_string())?;
    let result = build_transfer_tree(&staging, entries).and_then(|()| {
        for suffix in 0..=999_u16 {
            let destination = directory.join(unique_name("Transferred files", suffix));
            match fs::rename(&staging, &destination) {
                Ok(()) => return Ok(destination),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("OSC transfer destination collision limit reached".into())
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn build_transfer_tree(staging: &Path, entries: &[FileTransferEntry]) -> Result<(), String> {
    let mut paths = HashMap::<String, (PathBuf, FileTransferEntryKind)>::new();
    let mut links = Vec::new();
    let mut directories = Vec::new();

    for entry in entries {
        if paths.contains_key(&entry.id) {
            return Err("OSC transfer contains duplicate file ids".into());
        }
        let parent = match entry.parent.as_deref() {
            Some(parent_id) => match paths.get(parent_id) {
                Some((path, FileTransferEntryKind::Directory)) => path,
                Some(_) => return Err("OSC transfer parent is not a directory".into()),
                None => return Err("OSC transfer parent is missing or out of order".into()),
            },
            None => staging,
        };
        let path = parent.join(safe_filename(&entry.name));
        if paths.values().any(|(existing, _)| existing == &path) {
            return Err("OSC transfer contains duplicate destination paths".into());
        }
        paths.insert(entry.id.clone(), (path.clone(), entry.kind));

        match entry.kind {
            FileTransferEntryKind::Directory => {
                if !entry.data.is_empty() {
                    return Err("OSC transfer directory contains data".into());
                }
                fs::create_dir(&path).map_err(|error| error.to_string())?;
                directories.push((path, entry.permissions, entry.modified_ns));
            }
            FileTransferEntryKind::Regular => {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|error| error.to_string())?;
                file.write_all(&entry.data)
                    .and_then(|()| file.sync_all())
                    .map_err(|error| error.to_string())?;
                apply_metadata(&file, entry.permissions, entry.modified_ns);
            }
            FileTransferEntryKind::Symlink | FileTransferEntryKind::HardLink => {
                links.push(entry);
            }
        }
    }

    for entry in links {
        let (path, _) = paths
            .get(&entry.id)
            .ok_or_else(|| "OSC transfer link path is missing".to_string())?;
        match entry.kind {
            FileTransferEntryKind::HardLink => {
                let target_id = std::str::from_utf8(&entry.data)
                    .map_err(|_| "OSC hard-link target is not UTF-8".to_string())?;
                let (target, kind) = paths
                    .get(target_id)
                    .ok_or_else(|| "OSC hard-link target is missing".to_string())?;
                if *kind != FileTransferEntryKind::Regular {
                    return Err("OSC hard-link target is not a regular file".into());
                }
                fs::hard_link(target, path).map_err(|error| error.to_string())?;
            }
            FileTransferEntryKind::Symlink => {
                let target = symlink_target(staging, path, &entry.data, &paths)?;
                create_symlink(&target.0, path, target.1)?;
            }
            FileTransferEntryKind::Regular | FileTransferEntryKind::Directory => unreachable!(),
        }
    }

    for (path, permissions, modified_ns) in directories.into_iter().rev() {
        apply_path_metadata(&path, permissions, modified_ns);
    }
    Ok(())
}

fn symlink_target(
    staging: &Path,
    link: &Path,
    data: &[u8],
    paths: &HashMap<String, (PathBuf, FileTransferEntryKind)>,
) -> Result<(PathBuf, FileTransferEntryKind), String> {
    let value = std::str::from_utf8(data)
        .map_err(|_| "OSC symbolic-link target is not UTF-8".to_string())?;
    if let Some(target_id) = value
        .strip_prefix("fid:")
        .or_else(|| value.strip_prefix("fid_abs:"))
    {
        let (target, kind) = paths
            .get(target_id)
            .ok_or_else(|| "OSC symbolic-link target is missing".to_string())?;
        let parent = link
            .parent()
            .ok_or_else(|| "OSC symbolic-link parent is missing".to_string())?;
        return Ok((relative_path(parent, target), *kind));
    }
    let Some(target) = value.strip_prefix("path:") else {
        return Err("OSC symbolic-link target has an invalid prefix".into());
    };
    let target = Path::new(target);
    if target.is_absolute()
        || target
            .components()
            .any(|part| matches!(part, Component::Prefix(_)))
    {
        return Err("OSC symbolic-link target outside the transfer is not allowed".into());
    }
    let parent = link
        .parent()
        .ok_or_else(|| "OSC symbolic-link parent is missing".to_string())?;
    let mut depth = parent
        .strip_prefix(staging)
        .map_err(|_| "OSC symbolic-link parent escaped staging".to_string())?
        .components()
        .count();
    for component in target.components() {
        match component {
            Component::ParentDir if depth == 0 => {
                return Err("OSC symbolic-link target escapes the transfer".into());
            }
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err("OSC symbolic-link target is not relative".into());
            }
        }
    }
    Ok((target.to_owned(), FileTransferEntryKind::Regular))
}

fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let from = from.components().collect::<Vec<_>>();
    let to = to.components().collect::<Vec<_>>();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut result = PathBuf::new();
    for _ in common..from.len() {
        result.push("..");
    }
    for component in &to[common..] {
        result.push(component.as_os_str());
    }
    result
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path, _kind: FileTransferEntryKind) -> Result<(), String> {
    std::os::unix::fs::symlink(target, link).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn create_symlink(target: &Path, link: &Path, kind: FileTransferEntryKind) -> Result<(), String> {
    match kind {
        FileTransferEntryKind::Directory => std::os::windows::fs::symlink_dir(target, link),
        _ => std::os::windows::fs::symlink_file(target, link),
    }
    .map_err(|error| error.to_string())
}

fn apply_path_metadata(path: &Path, permissions: Option<u32>, modified_ns: Option<u64>) {
    if let Some(permissions) = permissions {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(error) =
                fs::set_permissions(path, fs::Permissions::from_mode(permissions & 0o777))
            {
                tracing::warn!(target: "toyoterm::download", %error, "apply OSC directory permissions failed");
            }
        }
        #[cfg(windows)]
        if let Ok(metadata) = fs::metadata(path) {
            let mut native = metadata.permissions();
            native.set_readonly(permissions & 0o200 == 0);
            if let Err(error) = fs::set_permissions(path, native) {
                tracing::warn!(target: "toyoterm::download", %error, "apply OSC directory permissions failed");
            }
        }
    }
    if let Some(modified_ns) = modified_ns
        && let Some(modified) =
            std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_nanos(modified_ns))
        && let Ok(file) = std::fs::File::open(path)
        && let Err(error) = file.set_times(std::fs::FileTimes::new().set_modified(modified))
    {
        tracing::warn!(target: "toyoterm::download", %error, "apply OSC directory modification time failed");
    }
}

pub fn queue_transfer(directory: &Path, mut entries: Vec<FileTransferEntry>) -> Result<(), String> {
    if !directory.is_absolute() {
        return Err("OSC download directory must be absolute".into());
    }
    if entries.len() > 64
        || entries.iter().map(|entry| entry.data.len()).sum::<usize>() > MAX_DOWNLOAD_BYTES
    {
        return Err("OSC file transfer exceeds its bounded limits".into());
    }
    let job = if entries.len() == 1
        && entries[0].kind == FileTransferEntryKind::Regular
        && entries[0].parent.is_none()
    {
        let entry = entries.pop().expect("one transfer entry was checked");
        DownloadJob::Single {
            directory: directory.to_owned(),
            name: safe_filename(&entry.name),
            data: entry.data,
            permissions: entry.permissions,
            modified_ns: entry.modified_ns,
        }
    } else {
        DownloadJob::Transfer {
            directory: directory.to_owned(),
            entries,
        }
    };
    download_sender()
        .try_send(job)
        .map_err(|error| match error {
            TrySendError::Full(_) => "OSC download queue is full".into(),
            TrySendError::Disconnected(_) => "OSC download worker stopped".into(),
        })
}

pub fn queue_upload_listing(
    pane: PaneId,
    root: &Path,
    session_id: String,
    quiet: u8,
    paths: Vec<FileUploadPath>,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<(), String> {
    if !root.is_absolute() {
        return Err("OSC upload directory must be absolute".into());
    }
    send_job(DownloadJob::UploadListing {
        pane,
        root: root.to_owned(),
        session_id,
        quiet,
        paths,
        proxy,
    })
}

pub fn queue_upload_data(
    pane: PaneId,
    session_id: String,
    file_id: String,
    name: String,
    compressed: bool,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<(), String> {
    send_job(DownloadJob::UploadData {
        pane,
        session_id,
        file_id,
        name,
        compressed,
        proxy,
    })
}

pub fn queue_upload_cancel(pane: PaneId, session_id: String) -> Result<(), String> {
    send_job(DownloadJob::UploadCancel { pane, session_id })
}

pub fn queue_upload_cancel_pane(pane: PaneId) -> Result<(), String> {
    send_job(DownloadJob::UploadCancelPane { pane })
}

pub fn queue_background_image(
    root: &Path,
    path: Option<String>,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<(), String> {
    if !root.is_absolute() {
        return Err("OSC background-image directory must be absolute".into());
    }
    send_job(DownloadJob::BackgroundImage {
        root: root.to_owned(),
        path,
        proxy,
    })
}

fn load_background_image(root: &Path, requested: &str) -> Result<BackgroundImage, String> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|_| "OSC background-image root does not exist".to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("OSC background-image root must be a non-symbolic-link directory".into());
    }
    let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let relative = requested
        .strip_prefix("~/")
        .or_else(|| requested.strip_prefix('/'))
        .unwrap_or(requested);
    if relative.is_empty() {
        return Err("OSC background-image path is empty".into());
    }
    let mut candidate = root.clone();
    for component in relative.split('/') {
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.contains(['\\', ':'])
            || component.chars().any(char::is_control)
        {
            return Err("OSC background-image path is outside the configured root".into());
        }
        candidate.push(component);
    }
    let candidate = fs::canonicalize(candidate)
        .map_err(|_| "OSC background-image file does not exist".to_string())?;
    if !candidate.starts_with(&root) || !candidate.is_file() {
        return Err("OSC background-image path is outside the configured root".into());
    }
    BackgroundImage::load(&candidate).map_err(|error| error.to_string())
}

fn send_job(job: DownloadJob) -> Result<(), String> {
    download_sender()
        .try_send(job)
        .map_err(|error| match error {
            TrySendError::Full(_) => "OSC file transfer queue is full".into(),
            TrySendError::Disconnected(_) => "OSC file transfer worker stopped".into(),
        })
}

fn build_upload_listing(
    root: &Path,
    session_id: &str,
    quiet: u8,
    paths: &[FileUploadPath],
) -> (Option<UploadSession>, Vec<String>) {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return (
                None,
                upload_error_responses(session_id, None, quiet, "EPERM:Invalid upload root"),
            );
        }
        Err(_) => {
            return (
                None,
                upload_error_responses(session_id, None, quiet, "ENOENT:Upload root not found"),
            );
        }
    }

    let root = match fs::canonicalize(root) {
        Ok(root) => root,
        Err(_) => {
            return (
                None,
                upload_error_responses(session_id, None, quiet, "EACCES:Cannot open upload root"),
            );
        }
    };
    let mut entries = HashMap::new();
    let mut responses = Vec::new();
    let mut next_id = 1_u64;
    let mut total_bytes = 0_u64;
    for requested in paths {
        let Some((host_path, virtual_path)) = resolve_upload_path(&root, &requested.name) else {
            responses.extend(upload_error_responses(
                session_id,
                Some(&requested.id),
                quiet,
                "EPERM:Requested path is outside the upload root",
            ));
            continue;
        };
        let response_start = responses.len();
        let total_start = total_bytes;
        let result = collect_upload_entry(
            session_id,
            &requested.id,
            &root,
            &host_path,
            &virtual_path,
            None,
            &mut next_id,
            &mut total_bytes,
            &mut entries,
            &mut responses,
        );
        if let Err(message) = result {
            responses.truncate(response_start);
            total_bytes = total_start;
            entries.retain(|_, entry| entry.original_id != requested.id);
            responses.extend(upload_error_responses(
                session_id,
                Some(&requested.id),
                quiet,
                &message,
            ));
        }
    }
    if entries.is_empty() {
        return (None, responses);
    }
    if quiet == 0 {
        responses.push(format!(
            "\x1b]5113;ac=status;id={session_id};st={};n={}\x1b\\",
            STANDARD.encode("OK"),
            STANDARD.encode("/")
        ));
    }
    (Some(UploadSession { root, entries }), responses)
}

#[allow(clippy::too_many_arguments)]
fn collect_upload_entry(
    session_id: &str,
    original_id: &str,
    root: &Path,
    host_path: &Path,
    virtual_path: &str,
    parent_id: Option<&str>,
    next_id: &mut u64,
    total_bytes: &mut u64,
    entries: &mut HashMap<String, UploadEntry>,
    responses: &mut Vec<String>,
) -> Result<(), String> {
    if entries.len() >= MAX_UPLOAD_ENTRIES {
        return Err("ENOSPC:Upload contains too many entries".into());
    }
    let metadata = fs::symlink_metadata(host_path)
        .map_err(|_| "ENOENT:Requested upload path not found".to_string())?;
    let kind = if metadata.file_type().is_symlink() {
        validate_upload_symlink(root, host_path)?;
        UploadKind::Symlink
    } else if metadata.is_dir() {
        UploadKind::Directory
    } else if metadata.is_file() {
        UploadKind::Regular
    } else {
        return Err("ENOTSUP:Unsupported upload file type".into());
    };
    if kind == UploadKind::Regular {
        *total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| "EFBIG:Upload exceeds the 32 MiB limit".to_string())?;
        if *total_bytes > MAX_DOWNLOAD_BYTES as u64 {
            return Err("EFBIG:Upload exceeds the 32 MiB limit".into());
        }
    }
    let actual_id = format!("t{next_id}");
    *next_id += 1;
    let file_type = match kind {
        UploadKind::Regular => "regular",
        UploadKind::Directory => "directory",
        UploadKind::Symlink => "symlink",
    };
    let parent = parent_id.map_or(String::new(), |parent| format!(";pr={parent}"));
    let size = (kind == UploadKind::Regular).then(|| format!(";sz={}", metadata.len()));
    let size = size.unwrap_or_default();
    let permissions = upload_permissions(&metadata);
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
        .map_or(String::new(), |value| format!(";mod={value}"));
    responses.push(format!(
        "\x1b]5113;ac=file;id={session_id};fid={original_id};st={actual_id};ft={file_type}{parent};prm={permissions}{modified}{size};n={}\x1b\\",
        STANDARD.encode(virtual_path)
    ));
    entries.insert(
        virtual_path.to_owned(),
        UploadEntry {
            original_id: original_id.to_owned(),
            actual_id: actual_id.clone(),
            host_path: host_path.to_owned(),
            kind,
        },
    );

    if kind == UploadKind::Directory {
        let mut children = fs::read_dir(host_path)
            .map_err(|_| "EACCES:Cannot read upload directory".to_string())?
            .take(MAX_UPLOAD_ENTRIES + 1)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "EACCES:Cannot read upload directory".to_string())?;
        if children.len() > MAX_UPLOAD_ENTRIES {
            return Err("ENOSPC:Upload directory contains too many entries".into());
        }
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let name = child
                .file_name()
                .into_string()
                .map_err(|_| "EINVAL:Upload path is not UTF-8".to_string())?;
            if name.len() > 255 || name.chars().any(char::is_control) {
                return Err("EINVAL:Invalid upload path component".into());
            }
            let child_virtual = format!("{}/{}", virtual_path.trim_end_matches('/'), name);
            if child_virtual.len() > MAX_UPLOAD_PATH_BYTES {
                return Err("ENAMETOOLONG:Upload path is too long".into());
            }
            collect_upload_entry(
                session_id,
                original_id,
                root,
                &child.path(),
                &child_virtual,
                Some(&actual_id),
                next_id,
                total_bytes,
                entries,
                responses,
            )?;
        }
    }
    Ok(())
}

fn resolve_upload_path(root: &Path, requested: &str) -> Option<(PathBuf, String)> {
    let relative = requested
        .strip_prefix("~/")
        .or_else(|| requested.strip_prefix('/'))?;
    if relative.is_empty() {
        return Some((root.to_owned(), "/".into()));
    }
    let mut host = root.to_owned();
    let mut virtual_parts = Vec::new();
    for component in relative.split('/') {
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.contains(['\\', ':'])
            || component.chars().any(char::is_control)
        {
            return None;
        }
        host.push(component);
        virtual_parts.push(component);
    }
    Some((host, format!("/{}", virtual_parts.join("/"))))
}

fn validate_upload_symlink(root: &Path, path: &Path) -> Result<(), String> {
    let target =
        fs::read_link(path).map_err(|_| "EACCES:Cannot read symbolic-link target".to_string())?;
    if target.is_absolute() {
        return Err("EPERM:Absolute symbolic-link uploads are not allowed".into());
    }
    let mut depth = path
        .parent()
        .and_then(|parent| parent.strip_prefix(root).ok())
        .ok_or_else(|| "EPERM:Symbolic-link is outside the upload root".to_string())?
        .components()
        .count();
    for component in target.components() {
        match component {
            Component::ParentDir if depth == 0 => {
                return Err("EPERM:Symbolic-link target escapes the upload root".into());
            }
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err("EPERM:Invalid symbolic-link target".into());
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn upload_permissions(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(windows)]
fn upload_permissions(metadata: &fs::Metadata) -> u32 {
    if metadata.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

fn upload_data_responses(
    session: &UploadSession,
    session_id: &str,
    file_id: &str,
    name: &str,
    compressed: bool,
) -> Vec<String> {
    let Some(entry) = session
        .entries
        .get(name)
        .filter(|entry| entry.actual_id == file_id || entry.original_id == file_id)
    else {
        return vec![upload_status(
            session_id,
            Some(file_id),
            "ENOENT:Unknown upload file",
        )];
    };
    let result = match entry.kind {
        UploadKind::Directory => Err("EISDIR:Cannot transfer directory data".into()),
        UploadKind::Regular => {
            let canonical = fs::canonicalize(&entry.host_path)
                .map_err(|_| "ENOENT:Upload file no longer exists".to_string());
            match canonical {
                Ok(path) if path.starts_with(&session.root) => read_bounded(&path),
                Ok(_) => Err("EPERM:Upload file escaped the configured root".into()),
                Err(message) => Err(message),
            }
        }
        UploadKind::Symlink => fs::read_link(&entry.host_path)
            .map_err(|_| "EACCES:Cannot read symbolic-link target".to_string())
            .and_then(|target| {
                target
                    .to_str()
                    .map(str::as_bytes)
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| "EINVAL:Symbolic-link target is not UTF-8".into())
            }),
    };
    let mut data = match result {
        Ok(data) => data,
        Err(message) => return vec![upload_status(session_id, Some(file_id), &message)],
    };
    if compressed {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        if encoder.write_all(&data).is_err() {
            return vec![upload_status(
                session_id,
                Some(file_id),
                "EIO:Cannot compress upload data",
            )];
        }
        data = match encoder.finish() {
            Ok(data) => data,
            Err(_) => {
                return vec![upload_status(
                    session_id,
                    Some(file_id),
                    "EIO:Cannot compress upload data",
                )];
            }
        };
    }
    let mut responses = Vec::new();
    for chunk in data.chunks(UPLOAD_CHUNK_BYTES) {
        responses.push(format!(
            "\x1b]5113;ac=data;id={session_id};fid={file_id};d={}\x1b\\",
            STANDARD.encode(chunk)
        ));
    }
    responses.push(format!(
        "\x1b]5113;ac=end_data;id={session_id};fid={file_id}\x1b\\"
    ));
    responses
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "ENOENT:Upload file no longer exists".to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_DOWNLOAD_BYTES as u64 {
        return Err("EFBIG:Upload file exceeds the 32 MiB limit".into());
    }
    let mut file =
        fs::File::open(path).map_err(|_| "EACCES:Cannot read upload file".to_string())?;
    let mut data = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(MAX_DOWNLOAD_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|_| "EIO:Cannot read upload file".to_string())?;
    if data.len() > MAX_DOWNLOAD_BYTES {
        return Err("EFBIG:Upload file exceeds the 32 MiB limit".into());
    }
    Ok(data)
}

fn upload_error_responses(
    session_id: &str,
    file_id: Option<&str>,
    quiet: u8,
    message: &str,
) -> Vec<String> {
    (quiet != 2)
        .then(|| upload_status(session_id, file_id, message))
        .into_iter()
        .collect()
}

fn upload_status(session_id: &str, file_id: Option<&str>, message: &str) -> String {
    let file_id = file_id.map_or(String::new(), |file_id| format!(";fid={file_id}"));
    format!(
        "\x1b]5113;ac=status;id={session_id}{file_id};st={}\x1b\\",
        STANDARD.encode(message)
    )
}

fn safe_filename(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut safe = String::new();
    for character in leaf.chars() {
        if character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
        {
            safe.push('_');
        } else if safe.len() + character.len_utf8() <= MAX_FILENAME_BYTES {
            safe.push(character);
        }
    }
    while safe.ends_with([' ', '.']) {
        safe.pop();
    }
    let stem = safe.split('.').next().unwrap_or_default();
    if safe.is_empty() || is_windows_reserved_name(stem) {
        "Unnamed file".into()
    } else {
        safe
    }
}

fn is_windows_reserved_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || upper
            .strip_prefix("COM")
            .or_else(|| upper.strip_prefix("LPT"))
            .is_some_and(|number| {
                matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
}

fn write_download(
    directory: &Path,
    name: &str,
    data: &[u8],
    permissions: Option<u32>,
    modified_ns: Option<u64>,
) -> Result<PathBuf, String> {
    let metadata = fs::metadata(directory).map_err(|error| error.to_string())?;
    if !metadata.is_dir() {
        return Err("OSC download destination is not a directory".into());
    }
    for suffix in 0..=999_u16 {
        let candidate = directory.join(unique_name(name, suffix));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        };
        if let Err(error) = file.write_all(data).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&candidate);
            return Err(error.to_string());
        }
        apply_metadata(&file, permissions, modified_ns);
        return Ok(candidate);
    }
    Err("OSC download name collision limit reached".into())
}

fn apply_metadata(file: &std::fs::File, permissions: Option<u32>, modified_ns: Option<u64>) {
    if let Some(permissions) = permissions {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(error) =
                file.set_permissions(fs::Permissions::from_mode(permissions & 0o777))
            {
                tracing::warn!(target: "toyoterm::download", %error, "apply OSC download permissions failed");
            }
        }
        #[cfg(windows)]
        {
            if let Ok(metadata) = file.metadata() {
                let mut native = metadata.permissions();
                native.set_readonly(permissions & 0o200 == 0);
                if let Err(error) = file.set_permissions(native) {
                    tracing::warn!(target: "toyoterm::download", %error, "apply OSC download permissions failed");
                }
            }
        }
    }
    if let Some(modified_ns) = modified_ns
        && let Some(modified) =
            std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_nanos(modified_ns))
        && let Err(error) = file.set_times(std::fs::FileTimes::new().set_modified(modified))
    {
        tracing::warn!(target: "toyoterm::download", %error, "apply OSC download modification time failed");
    }
}

fn unique_name(name: &str, suffix: u16) -> String {
    if suffix == 0 {
        return name.into();
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("file");
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if !extension.is_empty() => format!("{stem} ({suffix}).{extension}"),
        _ => format!("{stem} ({suffix})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_download_names_and_avoids_overwrites() {
        assert_eq!(safe_filename("../../bad<name>.txt"), "bad_name_.txt");
        assert_eq!(safe_filename("CON.txt"), "Unnamed file");

        let directory = std::env::temp_dir().join(format!(
            "toyoterm-download-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let first = write_download(&directory, "file.txt", b"one", Some(0o600), None).unwrap();
        let second = write_download(&directory, "file.txt", b"two", None, None).unwrap();
        assert_eq!(first.file_name().unwrap(), "file.txt");
        assert_eq!(second.file_name().unwrap(), "file (1).txt");
        assert_eq!(fs::read(first).unwrap(), b"one");
        assert_eq!(fs::read(second).unwrap(), b"two");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn commits_directory_trees_and_hard_links_inside_a_session_container() {
        let directory = test_directory("tree");
        fs::create_dir_all(&directory).unwrap();
        let entries = vec![
            FileTransferEntry {
                id: "d".into(),
                parent: None,
                name: "/remote/tree".into(),
                kind: FileTransferEntryKind::Directory,
                data: Vec::new(),
                permissions: Some(0o700),
                modified_ns: None,
            },
            FileTransferEntry {
                id: "f".into(),
                parent: Some("d".into()),
                name: "/remote/tree/file.txt".into(),
                kind: FileTransferEntryKind::Regular,
                data: b"payload".to_vec(),
                permissions: Some(0o600),
                modified_ns: None,
            },
            FileTransferEntry {
                id: "h".into(),
                parent: Some("d".into()),
                name: "/remote/tree/copy.txt".into(),
                kind: FileTransferEntryKind::HardLink,
                data: b"f".to_vec(),
                permissions: None,
                modified_ns: None,
            },
        ];
        let destination = write_transfer(&directory, &entries).unwrap();
        assert_eq!(
            fs::read(destination.join("tree/file.txt")).unwrap(),
            b"payload"
        );
        assert_eq!(
            fs::read(destination.join("tree/copy.txt")).unwrap(),
            b"payload"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_out_of_order_parents_and_escaping_symlink_targets() {
        let directory = test_directory("reject");
        fs::create_dir_all(&directory).unwrap();
        let child = FileTransferEntry {
            id: "f".into(),
            parent: Some("missing".into()),
            name: "/remote/file".into(),
            kind: FileTransferEntryKind::Regular,
            data: Vec::new(),
            permissions: None,
            modified_ns: None,
        };
        assert!(write_transfer(&directory, &[child]).is_err());

        let entries = vec![
            FileTransferEntry {
                id: "d".into(),
                parent: None,
                name: "/remote/tree".into(),
                kind: FileTransferEntryKind::Directory,
                data: Vec::new(),
                permissions: None,
                modified_ns: None,
            },
            FileTransferEntry {
                id: "s".into(),
                parent: Some("d".into()),
                name: "/remote/tree/link".into(),
                kind: FileTransferEntryKind::Symlink,
                data: b"path:../../outside".to_vec(),
                permissions: None,
                modified_ns: None,
            },
        ];
        assert!(write_transfer(&directory, &entries).is_err());
        assert!(fs::read_dir(&directory).unwrap().next().is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn lists_and_reads_uploads_only_from_the_configured_root() {
        let directory = test_directory("upload");
        fs::create_dir_all(directory.join("folder")).unwrap();
        fs::write(directory.join("folder/file.txt"), b"upload payload").unwrap();
        let paths = vec![FileUploadPath {
            id: "request".into(),
            name: "/folder".into(),
        }];

        let (session, responses) = build_upload_listing(&directory, "session", 0, &paths);
        let session = session.expect("valid upload session");
        assert_eq!(session.entries.len(), 2);
        assert!(responses.iter().any(|response| {
            response.contains("ft=directory") && response.contains("n=L2ZvbGRlcg==")
        }));
        assert!(responses.iter().any(|response| {
            response.contains("ft=regular") && response.contains("n=L2ZvbGRlci9maWxlLnR4dA==")
        }));

        let responses =
            upload_data_responses(&session, "session", "request", "/folder/file.txt", false);
        assert!(responses[0].contains(&STANDARD.encode(b"upload payload")));
        assert!(responses.last().unwrap().contains("ac=end_data"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_upload_traversal_and_unknown_file_requests() {
        let directory = test_directory("upload-reject");
        fs::create_dir_all(&directory).unwrap();
        assert!(resolve_upload_path(&directory, "/../secret").is_none());
        assert!(resolve_upload_path(&directory, "/C:/secret").is_none());

        let paths = vec![FileUploadPath {
            id: "request".into(),
            name: "/missing".into(),
        }];
        let (session, responses) = build_upload_listing(&directory, "session", 0, &paths);
        assert!(session.is_none());
        assert!(
            responses
                .iter()
                .any(|response| response.contains("ac=status"))
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn loads_background_images_only_below_the_fixed_root() {
        let directory = test_directory("background");
        fs::create_dir_all(directory.join("nested")).unwrap();
        let path = directory.join("nested/wallpaper.png");
        image::RgbImage::from_pixel(2, 3, image::Rgb([1, 2, 3]))
            .save(&path)
            .unwrap();

        let loaded = load_background_image(&directory, "/nested/wallpaper.png").unwrap();
        assert_eq!((loaded.width, loaded.height), (2, 3));
        assert!(load_background_image(&directory, "../outside.png").is_err());
        assert!(load_background_image(&directory, "C:/outside.png").is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    fn test_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "toyoterm-download-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
