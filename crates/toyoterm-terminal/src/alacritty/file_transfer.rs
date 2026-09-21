use std::collections::{HashMap, hash_map::Entry};
use std::io::Read;

use base64::{Engine, engine::general_purpose::STANDARD};

use super::protocol::{FileTransferEntry, FileTransferEntryKind, FileUploadPath, TerminalEvent};

const MAX_SESSIONS: usize = 8;
const MAX_FILES_PER_SESSION: usize = 64;
const MAX_TRANSFER_BYTES: usize = 32 * 1024 * 1024;
const MAX_CHUNK_BYTES: usize = 4 * 1024;
const MAX_ID_BYTES: usize = 128;
const MAX_PATH_BYTES: usize = 4 * 1024;

#[derive(Default)]
pub(super) struct KittyFileTransfers {
    enabled: bool,
    upload_enabled: bool,
    sessions: HashMap<String, Session>,
    receive_sessions: HashMap<String, ReceiveSession>,
}

struct ReceiveSession {
    quiet: u8,
    expected_paths: usize,
    paths: Vec<FileUploadPath>,
    ready: bool,
}

struct Session {
    quiet: u8,
    files: HashMap<String, IncomingFile>,
    completed: Vec<CompletedFile>,
}

struct CompletedFile {
    file_id: String,
    file: IncomingFile,
}

struct IncomingFile {
    name: String,
    parent: Option<String>,
    kind: FileTransferEntryKind,
    expected_size: Option<usize>,
    compressed: bool,
    permissions: Option<u32>,
    modified_ns: Option<u64>,
    data: Vec<u8>,
}

impl KittyFileTransfers {
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.sessions.clear();
        }
    }

    pub fn set_upload_enabled(&mut self, enabled: bool) {
        self.upload_enabled = enabled;
        if !enabled {
            self.receive_sessions.clear();
        }
    }

    pub fn handle(&mut self, payload: &[u8]) -> Vec<TerminalEvent> {
        let Some(fields) = parse_fields(payload) else {
            return Vec::new();
        };
        let Some(action) = fields.get("ac").map(String::as_str) else {
            return Vec::new();
        };
        let Some(id) = fields.get("id").filter(|id| valid_id(id)).cloned() else {
            return Vec::new();
        };
        match action {
            "send" => self.start(id, &fields),
            "receive" => self.start_receive(id, &fields),
            "file" if self.receive_sessions.contains_key(&id) => self.receive_file(&id, &fields),
            "file" => self.file(&id, &fields),
            "data" => self.data(&id, &fields, false),
            "end_data" => self.data(&id, &fields, true),
            "cancel" => self.cancel(&id),
            "finish" if self.receive_sessions.contains_key(&id) => self.finish_receive(&id),
            "finished" => self.finish_receive(&id),
            "finish" => self.finish(&id),
            _ => Vec::new(),
        }
    }

    fn start(&mut self, id: String, fields: &HashMap<String, String>) -> Vec<TerminalEvent> {
        let quiet = fields
            .get("q")
            .and_then(|quiet| quiet.parse::<u8>().ok())
            .filter(|quiet| *quiet <= 2)
            .unwrap_or(0);
        if !self.enabled {
            return status(
                &id,
                None,
                quiet,
                true,
                "EPERM:File downloads are disabled",
                None,
            );
        }
        if self.sessions.len() + self.receive_sessions.len() >= MAX_SESSIONS
            && !self.sessions.contains_key(&id)
        {
            return status(
                &id,
                None,
                quiet,
                true,
                "ENOSPC:Too many transfer sessions",
                None,
            );
        }
        self.receive_sessions.remove(&id);
        self.sessions.insert(
            id.clone(),
            Session {
                quiet,
                files: HashMap::new(),
                completed: Vec::new(),
            },
        );
        status(&id, None, quiet, false, "OK", None)
    }

    fn start_receive(
        &mut self,
        id: String,
        fields: &HashMap<String, String>,
    ) -> Vec<TerminalEvent> {
        let quiet = fields
            .get("q")
            .and_then(|quiet| quiet.parse::<u8>().ok())
            .filter(|quiet| *quiet <= 2)
            .unwrap_or(0);
        if !self.upload_enabled {
            return status(
                &id,
                None,
                quiet,
                true,
                "EPERM:File uploads are disabled",
                None,
            );
        }
        let Some(expected_paths) = fields
            .get("sz")
            .and_then(|size| size.parse::<usize>().ok())
            .filter(|size| (1..=MAX_FILES_PER_SESSION).contains(size))
        else {
            return status(
                &id,
                None,
                quiet,
                true,
                "EINVAL:Invalid requested path count",
                None,
            );
        };
        if self.sessions.len() + self.receive_sessions.len() >= MAX_SESSIONS
            && !self.receive_sessions.contains_key(&id)
        {
            return status(
                &id,
                None,
                quiet,
                true,
                "ENOSPC:Too many transfer sessions",
                None,
            );
        }
        self.sessions.remove(&id);
        self.receive_sessions.insert(
            id,
            ReceiveSession {
                quiet,
                expected_paths,
                paths: Vec::with_capacity(expected_paths),
                ready: false,
            },
        );
        Vec::new()
    }

    fn receive_file(&mut self, id: &str, fields: &HashMap<String, String>) -> Vec<TerminalEvent> {
        let Some(session) = self.receive_sessions.get_mut(id) else {
            return Vec::new();
        };
        let quiet = session.quiet;
        let Some(file_id) = fields.get("fid").filter(|file_id| valid_id(file_id)) else {
            return status(id, None, quiet, true, "EINVAL:Invalid file id", None);
        };
        let Some(name) = fields
            .get("n")
            .and_then(|name| decode_text(name, MAX_PATH_BYTES))
            .filter(|name| valid_transfer_path(name))
        else {
            return status(id, Some(file_id), quiet, true, "EINVAL:Invalid path", None);
        };
        if session.ready {
            if fields.get("tt").is_some_and(|kind| kind != "simple")
                || fields
                    .get("zip")
                    .is_some_and(|kind| !matches!(kind.as_str(), "none" | "zlib"))
            {
                return status(
                    id,
                    Some(file_id),
                    quiet,
                    true,
                    "ENOTSUP:Unsupported upload encoding",
                    None,
                );
            }
            return vec![TerminalEvent::FileUploadDataRequest {
                session_id: id.to_owned(),
                file_id: file_id.clone(),
                name,
                compressed: fields.get("zip").is_some_and(|kind| kind == "zlib"),
            }];
        }
        if session.paths.iter().any(|path| path.id == *file_id) {
            return status(
                id,
                Some(file_id),
                quiet,
                true,
                "EEXIST:File id is already in use",
                None,
            );
        }
        session.paths.push(FileUploadPath {
            id: file_id.clone(),
            name,
        });
        if session.paths.len() < session.expected_paths {
            return Vec::new();
        }
        session.ready = true;
        let mut events = status(id, None, quiet, false, "OK", None);
        events.push(TerminalEvent::FileUploadRequest {
            session_id: id.to_owned(),
            quiet,
            paths: session.paths.clone(),
        });
        events
    }

    fn file(&mut self, id: &str, fields: &HashMap<String, String>) -> Vec<TerminalEvent> {
        let Some(session) = self.sessions.get_mut(id) else {
            return Vec::new();
        };
        let quiet = session.quiet;
        let Some(file_id) = fields.get("fid").filter(|file_id| valid_id(file_id)) else {
            return status(id, None, quiet, true, "EINVAL:Invalid file id", None);
        };
        let kind = match fields.get("ft").map(String::as_str).unwrap_or("regular") {
            "regular" => FileTransferEntryKind::Regular,
            "directory" => FileTransferEntryKind::Directory,
            "symlink" => FileTransferEntryKind::Symlink,
            "link" => FileTransferEntryKind::HardLink,
            _ => {
                return status(
                    id,
                    Some(file_id),
                    quiet,
                    true,
                    "ENOTSUP:Unsupported file type",
                    None,
                );
            }
        };
        if fields.get("tt").is_some_and(|kind| kind != "simple")
            || fields
                .get("zip")
                .is_some_and(|kind| !matches!(kind.as_str(), "none" | "zlib"))
            || kind == FileTransferEntryKind::Directory && fields.contains_key("zip")
        {
            return status(
                id,
                Some(file_id),
                quiet,
                true,
                "ENOTSUP:Unsupported transfer encoding",
                None,
            );
        }
        let Some(name) = fields
            .get("n")
            .and_then(|name| decode_text(name, MAX_PATH_BYTES))
            .filter(|name| valid_transfer_path(name))
        else {
            return status(id, Some(file_id), quiet, true, "EINVAL:Invalid path", None);
        };
        let expected_size = match fields.get("sz") {
            Some(size) => match size.parse::<usize>() {
                Ok(size) if size <= MAX_TRANSFER_BYTES => Some(size),
                _ => {
                    return status(
                        id,
                        Some(file_id),
                        quiet,
                        true,
                        "EFBIG:File exceeds transfer limit",
                        None,
                    );
                }
            },
            None => None,
        };
        let parent = match fields.get("pr") {
            Some(parent)
                if valid_id(parent)
                    && parent != file_id
                    && session.completed.iter().any(|entry| {
                        entry.file_id == *parent
                            && entry.file.kind == FileTransferEntryKind::Directory
                    }) =>
            {
                Some(parent.clone())
            }
            Some(_) => {
                return status(
                    id,
                    Some(file_id),
                    quiet,
                    true,
                    "EINVAL:Parent must be an earlier directory",
                    None,
                );
            }
            None => None,
        };
        if session.files.contains_key(file_id.as_str())
            || session
                .completed
                .iter()
                .any(|file| file.file_id == *file_id)
        {
            return status(
                id,
                Some(file_id),
                quiet,
                true,
                "EEXIST:File id is already in use",
                None,
            );
        }
        if session.files.len() + session.completed.len() >= MAX_FILES_PER_SESSION {
            return status(
                id,
                Some(file_id),
                quiet,
                true,
                "ENOSPC:Too many files in session",
                None,
            );
        }
        let file = IncomingFile {
            name,
            parent,
            kind,
            expected_size,
            compressed: fields.get("zip").is_some_and(|kind| kind == "zlib"),
            permissions: fields
                .get("prm")
                .and_then(|permissions| permissions.parse::<u32>().ok())
                .map(|permissions| permissions & 0o777),
            modified_ns: fields
                .get("mod")
                .and_then(|modified| modified.parse::<u64>().ok()),
            data: Vec::new(),
        };
        if kind == FileTransferEntryKind::Directory {
            if expected_size.is_some_and(|size| size != 0) {
                return status(
                    id,
                    Some(file_id),
                    quiet,
                    true,
                    "EINVAL:Directory cannot contain data",
                    None,
                );
            }
            session.completed.push(CompletedFile {
                file_id: file_id.clone(),
                file,
            });
            status(id, Some(file_id), quiet, false, "OK", None)
        } else {
            session.files.insert(file_id.clone(), file);
            status(id, Some(file_id), quiet, false, "STARTED", Some(0))
        }
    }

    fn data(
        &mut self,
        id: &str,
        fields: &HashMap<String, String>,
        finished: bool,
    ) -> Vec<TerminalEvent> {
        let buffered_bytes = self
            .sessions
            .values()
            .map(|session| {
                session
                    .files
                    .values()
                    .map(|file| file.data.len())
                    .chain(session.completed.iter().map(|file| file.file.data.len()))
                    .sum::<usize>()
            })
            .sum::<usize>();
        let Some(session) = self.sessions.get_mut(id) else {
            return Vec::new();
        };
        let quiet = session.quiet;
        let Some(file_id) = fields.get("fid").filter(|file_id| valid_id(file_id)) else {
            return status(id, None, quiet, true, "EINVAL:Invalid file id", None);
        };
        let chunk = match fields.get("d") {
            Some(data) => match STANDARD.decode(data) {
                Ok(data) if data.len() <= MAX_CHUNK_BYTES => data,
                _ => {
                    session.files.remove(file_id.as_str());
                    return status(
                        id,
                        Some(file_id),
                        quiet,
                        true,
                        "EINVAL:Invalid data chunk",
                        None,
                    );
                }
            },
            None => Vec::new(),
        };
        let Entry::Occupied(mut entry) = session.files.entry(file_id.clone()) else {
            return Vec::new();
        };
        if entry.get().data.len().saturating_add(chunk.len()) > MAX_TRANSFER_BYTES
            || buffered_bytes.saturating_add(chunk.len()) > MAX_TRANSFER_BYTES
        {
            entry.remove();
            return status(
                id,
                Some(file_id),
                quiet,
                true,
                "EFBIG:File exceeds transfer limit",
                None,
            );
        }
        entry.get_mut().data.extend_from_slice(&chunk);
        let size = entry.get().data.len();
        if !finished {
            return status(id, Some(file_id), quiet, false, "PROGRESS", Some(size));
        }
        let mut file = entry.remove();
        if file.compressed {
            let mut decoded = Vec::new();
            let mut decoder = flate2::read::ZlibDecoder::new(file.data.as_slice())
                .take(MAX_TRANSFER_BYTES as u64 + 1);
            if decoder.read_to_end(&mut decoded).is_err() || decoded.len() > MAX_TRANSFER_BYTES {
                return status(
                    id,
                    Some(file_id),
                    quiet,
                    true,
                    "EINVAL:Invalid compressed data",
                    None,
                );
            }
            file.data = decoded;
        }
        let size = file.data.len();
        if file.expected_size.is_some_and(|expected| expected != size) {
            return status(
                id,
                Some(file_id),
                quiet,
                true,
                "EINVAL:File size mismatch",
                Some(size),
            );
        }
        session.completed.push(CompletedFile {
            file_id: file_id.clone(),
            file,
        });
        status(id, Some(file_id), quiet, false, "OK", Some(size))
    }

    fn cancel(&mut self, id: &str) -> Vec<TerminalEvent> {
        if let Some(session) = self.receive_sessions.remove(id) {
            let mut events = vec![TerminalEvent::FileUploadCancel(id.to_owned())];
            events.extend(status(id, None, session.quiet, false, "CANCELED", None));
            return events;
        }
        let Some(session) = self.sessions.remove(id) else {
            return Vec::new();
        };
        status(id, None, session.quiet, false, "CANCELED", None)
    }

    fn finish_receive(&mut self, id: &str) -> Vec<TerminalEvent> {
        if self.receive_sessions.remove(id).is_some() {
            vec![TerminalEvent::FileUploadCancel(id.to_owned())]
        } else {
            Vec::new()
        }
    }

    fn finish(&mut self, id: &str) -> Vec<TerminalEvent> {
        let Some(session) = self.sessions.get(id) else {
            return Vec::new();
        };
        if !session.files.is_empty() {
            return status(
                id,
                None,
                session.quiet,
                true,
                "EINVAL:Transfer has incomplete files",
                None,
            );
        }
        let session = self.sessions.remove(id).expect("session was just checked");
        if let Err(error) = validate_completed_files(&session.completed) {
            return status(id, None, session.quiet, true, error, None);
        }
        let entries = session
            .completed
            .into_iter()
            .map(|completed| FileTransferEntry {
                id: completed.file_id,
                parent: completed.file.parent,
                name: completed.file.name,
                kind: completed.file.kind,
                data: completed.file.data,
                permissions: completed.file.permissions,
                modified_ns: completed.file.modified_ns,
            })
            .collect::<Vec<_>>();
        let mut events = vec![TerminalEvent::FileTransferCommit(entries)];
        events.extend(status(id, None, session.quiet, false, "OK", None));
        events
    }
}

fn validate_completed_files(files: &[CompletedFile]) -> Result<(), &'static str> {
    let kinds = files
        .iter()
        .map(|entry| (entry.file_id.as_str(), entry.file.kind))
        .collect::<HashMap<_, _>>();
    for entry in files {
        match entry.file.kind {
            FileTransferEntryKind::HardLink => {
                let target = std::str::from_utf8(&entry.file.data)
                    .map_err(|_| "EINVAL:Hard-link target is not UTF-8")?;
                if kinds.get(target) != Some(&FileTransferEntryKind::Regular) {
                    return Err("EINVAL:Hard-link target is not a transferred regular file");
                }
            }
            FileTransferEntryKind::Symlink => {
                let target = std::str::from_utf8(&entry.file.data)
                    .map_err(|_| "EINVAL:Symbolic-link target is not UTF-8")?;
                if let Some(target) = target
                    .strip_prefix("fid:")
                    .or_else(|| target.strip_prefix("fid_abs:"))
                {
                    if !kinds.contains_key(target) {
                        return Err("EINVAL:Symbolic-link target is not transferred");
                    }
                } else if target.starts_with("path:") {
                    return Err("EPERM:External symbolic-link targets are disabled");
                } else {
                    return Err("EINVAL:Symbolic-link target has an invalid prefix");
                }
            }
            FileTransferEntryKind::Regular | FileTransferEntryKind::Directory => {}
        }
    }
    Ok(())
}

fn parse_fields(payload: &[u8]) -> Option<HashMap<String, String>> {
    let text = std::str::from_utf8(payload.strip_prefix(b"5113;")?).ok()?;
    let mut fields = HashMap::new();
    for field in text.split(';') {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        let (key, value) = field.split_once('=')?;
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return None;
        }
        fields.insert(key.into(), value.into());
    }
    Some(fields)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'.' | b'/' | b'@' | b'-')
        })
}

fn decode_text(value: &str, maximum: usize) -> Option<String> {
    let bytes = STANDARD.decode(value).ok()?;
    (bytes.len() <= maximum)
        .then(|| String::from_utf8(bytes).ok())
        .flatten()
}

fn valid_transfer_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH_BYTES
        && !path.chars().any(char::is_control)
        && (path.starts_with('/') || path.starts_with("~/"))
        && path.split('/').all(|component| component.len() <= 255)
}

fn status(
    id: &str,
    file_id: Option<&str>,
    quiet: u8,
    error: bool,
    message: &str,
    size: Option<usize>,
) -> Vec<TerminalEvent> {
    if quiet == 2 || quiet == 1 && !error {
        return Vec::new();
    }
    let file_id = file_id.map_or(String::new(), |file_id| format!(";fid={file_id}"));
    let size = size.map_or(String::new(), |size| format!(";sz={size}"));
    vec![TerminalEvent::PtyWrite(format!(
        "\x1b]5113;ac=status;id={id}{file_id};st={}{size}\x1b\\",
        STANDARD.encode(message)
    ))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn commits_regular_files_only_when_the_session_finishes() {
        let mut transfers = KittyFileTransfers::default();
        transfers.set_enabled(true);
        assert_eq!(transfers.handle(b"5113;ac=send;id=test").len(), 1);
        assert_eq!(
            transfers
                .handle(b"5113;ac=file;id=test;fid=f1;n=L3RtcC9maWxlLnR4dA==;sz=3;prm=384;mod=1000000000")
                .len(),
            1
        );
        assert_eq!(
            transfers
                .handle(b"5113;ac=data;id=test;fid=f1;d=YQ==")
                .len(),
            1
        );
        let events = transfers.handle(b"5113;ac=end_data;id=test;fid=f1;d=YmM=");
        assert_eq!(events.len(), 1);
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, TerminalEvent::FileTransferCommit(_)))
        );
        let events = transfers.handle(b"5113;ac=finish;id=test");
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            TerminalEvent::FileTransferCommit(vec![FileTransferEntry {
                id: "f1".into(),
                parent: None,
                name: "/tmp/file.txt".into(),
                kind: FileTransferEntryKind::Regular,
                data: b"abc".to_vec(),
                permissions: Some(0o600),
                modified_ns: Some(1_000_000_000),
            }])
        );
        assert!(transfers.handle(b"5113;ac=cancel;id=test").is_empty());
    }

    #[test]
    fn cancellation_discards_completed_files_and_finish_rejects_incomplete_files() {
        let mut transfers = KittyFileTransfers::default();
        transfers.set_enabled(true);
        transfers.handle(b"5113;ac=send;id=test");
        transfers.handle(b"5113;ac=file;id=test;fid=f1;n=L3RtcC9maWxl;sz=1");
        assert_eq!(transfers.handle(b"5113;ac=finish;id=test").len(), 1);
        transfers.handle(b"5113;ac=end_data;id=test;fid=f1;d=YQ==");
        let events = transfers.handle(b"5113;ac=cancel;id=test");
        assert_eq!(events.len(), 1);
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, TerminalEvent::FileTransferCommit(_)))
        );
    }

    #[test]
    fn refuses_disabled_oversized_and_out_of_order_transfers() {
        let mut transfers = KittyFileTransfers::default();
        assert_eq!(transfers.handle(b"5113;ac=send;id=test").len(), 1);
        assert!(
            transfers
                .handle(b"5113;ac=data;id=test;fid=f1;d=YQ==")
                .is_empty()
        );
        transfers.set_enabled(true);
        transfers.handle(b"5113;ac=send;id=test");
        let events =
            transfers.handle(b"5113;ac=file;id=test;fid=f1;n=L3RtcC9maWxl;sz=99999999999999999999");
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn receives_zlib_compressed_regular_files() {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"compressed text").unwrap();
        let encoded = STANDARD.encode(encoder.finish().unwrap());

        let mut transfers = KittyFileTransfers::default();
        transfers.set_enabled(true);
        transfers.handle(b"5113;ac=send;id=test");
        transfers.handle(b"5113;ac=file;id=test;fid=f1;n=L3RtcC9maWxl;sz=15;zip=zlib");
        transfers.handle(format!("5113;ac=end_data;id=test;fid=f1;d={encoded}").as_bytes());
        let events = transfers.handle(b"5113;ac=finish;id=test");
        assert_eq!(
            events[0],
            TerminalEvent::FileTransferCommit(vec![FileTransferEntry {
                id: "f1".into(),
                parent: None,
                name: "/tmp/file".into(),
                kind: FileTransferEntryKind::Regular,
                data: b"compressed text".to_vec(),
                permissions: None,
                modified_ns: None,
            }])
        );
    }

    #[test]
    fn preserves_directory_and_link_metadata_until_commit() {
        let mut transfers = KittyFileTransfers::default();
        transfers.set_enabled(true);
        transfers.handle(b"5113;ac=send;id=test");
        transfers.handle(b"5113;ac=file;id=test;fid=d;n=L3RtcC9kaXI=;ft=directory;prm=448");
        transfers.handle(b"5113;ac=file;id=test;fid=f;n=L3RtcC9kaXIvZmlsZQ==;ft=regular;pr=d;sz=1");
        transfers.handle(b"5113;ac=end_data;id=test;fid=f;d=YQ==");
        transfers.handle(b"5113;ac=file;id=test;fid=h;n=L3RtcC9kaXIvaGFyZA==;ft=link;pr=d");
        transfers.handle(b"5113;ac=end_data;id=test;fid=h;d=Zg==");
        transfers.handle(b"5113;ac=file;id=test;fid=s;n=L3RtcC9kaXIvc3lt;ft=symlink;pr=d");
        transfers.handle(b"5113;ac=end_data;id=test;fid=s;d=ZmlkOmY=");

        let events = transfers.handle(b"5113;ac=finish;id=test");
        let TerminalEvent::FileTransferCommit(entries) = &events[0] else {
            panic!("expected a file transfer commit");
        };
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].kind, FileTransferEntryKind::Directory);
        assert_eq!(entries[1].parent.as_deref(), Some("d"));
        assert_eq!(entries[2].kind, FileTransferEntryKind::HardLink);
        assert_eq!(entries[2].data, b"f");
        assert_eq!(entries[3].kind, FileTransferEntryKind::Symlink);
        assert_eq!(entries[3].data, b"fid:f");
    }

    #[test]
    fn receive_requests_are_opt_in_and_collected_before_listing() {
        let mut transfers = KittyFileTransfers::default();
        let events = transfers.handle(b"5113;ac=receive;id=get;sz=1");
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], TerminalEvent::PtyWrite(value) if value.contains("ac=status"))
        );

        transfers.set_upload_enabled(true);
        assert!(transfers.handle(b"5113;ac=receive;id=get;sz=2").is_empty());
        assert!(
            transfers
                .handle(b"5113;ac=file;id=get;fid=a;n=L2E=")
                .is_empty()
        );
        let events = transfers.handle(b"5113;ac=file;id=get;fid=b;n=L2I=");
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], TerminalEvent::PtyWrite(value) if value.contains("T0s=")));
        assert_eq!(
            events[1],
            TerminalEvent::FileUploadRequest {
                session_id: "get".into(),
                quiet: 0,
                paths: vec![
                    FileUploadPath {
                        id: "a".into(),
                        name: "/a".into(),
                    },
                    FileUploadPath {
                        id: "b".into(),
                        name: "/b".into(),
                    },
                ],
            }
        );
    }

    #[test]
    fn receive_data_requests_and_cancellation_are_forwarded() {
        let mut transfers = KittyFileTransfers::default();
        transfers.set_upload_enabled(true);
        transfers.handle(b"5113;ac=receive;id=get;sz=1;q=1");
        let events = transfers.handle(b"5113;ac=file;id=get;fid=a;n=L2E=");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TerminalEvent::FileUploadRequest { .. }));

        assert_eq!(
            transfers.handle(b"5113;ac=file;id=get;fid=t1;n=L2E=;zip=zlib"),
            vec![TerminalEvent::FileUploadDataRequest {
                session_id: "get".into(),
                file_id: "t1".into(),
                name: "/a".into(),
                compressed: true,
            }]
        );
        let events = transfers.handle(b"5113;ac=cancel;id=get");
        assert!(matches!(events[0], TerminalEvent::FileUploadCancel(ref id) if id == "get"));
        assert!(
            transfers
                .handle(b"5113;ac=file;id=get;fid=t1;n=L2E=")
                .is_empty()
        );
    }

    #[test]
    fn receive_rejects_invalid_counts_duplicate_ids_and_encodings() {
        let mut transfers = KittyFileTransfers::default();
        transfers.set_upload_enabled(true);
        assert_eq!(transfers.handle(b"5113;ac=receive;id=get;sz=0").len(), 1);
        transfers.handle(b"5113;ac=receive;id=get;sz=2");
        transfers.handle(b"5113;ac=file;id=get;fid=a;n=L2E=");
        assert_eq!(
            transfers.handle(b"5113;ac=file;id=get;fid=a;n=L2I=").len(),
            1
        );
        transfers.handle(b"5113;ac=file;id=get;fid=b;n=L2I=");
        assert_eq!(
            transfers
                .handle(b"5113;ac=file;id=get;fid=t1;n=L2E=;zip=bzip2")
                .len(),
            1
        );
    }
}
