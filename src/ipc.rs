use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use crate::{Command, NativeCommand, PaneId, SplitDirection};

const MAGIC: &[u8; 4] = b"TYIP";
const VERSION: u16 = 1;
const MAX_MESSAGE: usize = 1024 * 1024;
pub type IpcResponse = mpsc::Sender<Result<String, String>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IpcRequest {
    Eval(String),
    List,
    Reload,
    ListPanes,
    SendText { pane: PaneId, text: String },
    Split { direction: SplitDirection },
    ActivateWorkspace(String),
}

impl IpcRequest {
    pub fn native_command(
        &self,
        current_pane: Option<PaneId>,
    ) -> Result<Option<NativeCommand>, String> {
        Ok(match self {
            Self::Reload => Some(NativeCommand::ReloadConfig),
            Self::SendText { pane, text } => Some(NativeCommand::Mux(Command::SendText {
                pane: *pane,
                text: text.clone(),
            })),
            Self::Split { direction } => Some(NativeCommand::Mux(Command::Split {
                pane: current_pane.ok_or_else(|| "mux has no current pane".to_owned())?,
                direction: *direction,
            })),
            Self::ActivateWorkspace(name) => {
                Some(NativeCommand::Mux(Command::SwitchWorkspace(name.clone())))
            }
            Self::Eval(_) | Self::List | Self::ListPanes => None,
        })
    }
}

#[derive(Debug)]
struct InstanceState {
    id: String,
    pid: u32,
    transport: String,
    endpoint: String,
    token: String,
}

pub struct IpcServer {
    state_path: PathBuf,
    active_path: PathBuf,
    instance_id: String,
    endpoint: String,
    stop: Arc<AtomicBool>,
}

impl IpcServer {
    pub fn start(
        dispatch: impl Fn(IpcRequest, IpcResponse) -> Result<(), String> + Send + 'static,
    ) -> Result<Self, String> {
        let paths = RuntimePaths::new()?;
        let instance_id = requested_instance().unwrap_or_else(|| std::process::id().to_string());
        validate_instance_id(&instance_id)?;
        let state_path = paths.instances.join(format!("{instance_id}.state"));
        remove_stale_instance(&state_path)?;
        let token = random_token()?;
        let (listener, transport, endpoint) = TransportListener::bind(&paths, &instance_id)?;
        let state = InstanceState {
            id: instance_id.clone(),
            pid: std::process::id(),
            transport: transport.into(),
            endpoint: endpoint.clone(),
            token,
        };
        write_private_file(&state_path, serialize_state(&state).as_bytes())?;
        write_private_file(&paths.active, instance_id.as_bytes())?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread_token = state.token.clone();
        thread::Builder::new().name("toyoterm-ipc".into()).spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok(mut stream) => {
                        if thread_stop.load(Ordering::Relaxed) { break; }
                        handle_client(&mut stream, &thread_token, &dispatch);
                    }
                    Err(error) => {
                        if !thread_stop.load(Ordering::Relaxed) { tracing::warn!(target: "toyoterm::ipc", %error, "local IPC listener failed"); }
                        break;
                    }
                }
            }
        }).map_err(|error| format!("spawn local IPC listener: {error}"))?;
        Ok(Self {
            state_path,
            active_path: paths.active,
            instance_id,
            endpoint,
            stop,
        })
    }
    pub fn address(&self) -> &str {
        &self.endpoint
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TransportStream::connect_for_wakeup(&self.endpoint);
        let _ = fs::remove_file(&self.state_path);
        if fs::read_to_string(&self.active_path).is_ok_and(|value| value.trim() == self.instance_id)
        {
            let _ = fs::remove_file(&self.active_path);
        }
        #[cfg(unix)]
        let _ = fs::remove_file(&self.endpoint);
    }
}

pub fn request_remote(request: IpcRequest) -> Result<String, String> {
    let state = load_instance_state()?;
    if state.transport != platform_transport() {
        return Err(format!(
            "instance {} uses unsupported transport {}",
            state.id, state.transport
        ));
    }
    let mut stream = TransportStream::connect(&state.endpoint).map_err(|error| {
        format!(
            "connect to toyoterm instance {} (pid {}): {error}",
            state.id, state.pid
        )
    })?;
    write_request(&mut stream, &state.token, &request).map_err(|error| error.to_string())?;
    read_response(&mut stream).map_err(|error| error.to_string())?
}

pub fn eval_remote(source: &str) -> Result<String, String> {
    request_remote(IpcRequest::Eval(source.into()))
}

pub fn run_console() -> Result<(), String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::stdout();
    let mut history = load_history();
    let mut source = String::new();
    loop {
        write!(
            stdout,
            "{}",
            if source.is_empty() {
                "toyoterm> "
            } else {
                "        | "
            }
        )
        .map_err(|e| e.to_string())?;
        stdout.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if source.is_empty() && matches!(trimmed, "exit" | "quit" | ":q") {
            break;
        }
        if source.is_empty() && trimmed == ":history" {
            for (i, entry) in history.iter().enumerate() {
                println!("{:>4}  {}", i + 1, entry);
            }
            continue;
        }
        source.push_str(&line);
        if input_incomplete(&source) {
            continue;
        }
        let submitted = source.trim_end().to_owned();
        if submitted.is_empty() {
            source.clear();
            continue;
        }
        match eval_remote(&submitted) {
            Ok(result) => println!("=> {result}"),
            Err(error) if is_incomplete_ruby_error(&error) => continue,
            Err(error) => eprintln!("{error}"),
        }
        history.push(submitted.replace('\n', "\\n"));
        if history.len() > 500 {
            history.remove(0);
        }
        save_history(&history);
        source.clear();
    }
    Ok(())
}

fn handle_client(
    stream: &mut TransportStream,
    token: &str,
    dispatch: &impl Fn(IpcRequest, IpcResponse) -> Result<(), String>,
) {
    let response = read_request(stream, token)
        .and_then(|request| {
            let (sender, receiver) = mpsc::channel();
            dispatch(request, sender).map_err(io::Error::other)?;
            receiver
                .recv()
                .map_err(|_| io::Error::other("GUI closed before answering IPC request"))?
                .map_err(io::Error::other)
        })
        .map_err(|error| error.to_string());
    let _ = write_response(stream, &response);
}

fn write_request(stream: &mut impl Write, token: &str, request: &IpcRequest) -> io::Result<()> {
    let mut body = Vec::new();
    body.extend_from_slice(MAGIC);
    body.extend_from_slice(&VERSION.to_be_bytes());
    put_string(&mut body, token)?;
    match request {
        IpcRequest::Eval(value) => {
            body.push(0);
            put_string(&mut body, value)?;
        }
        IpcRequest::List => body.push(1),
        IpcRequest::Reload => body.push(2),
        IpcRequest::ListPanes => body.push(3),
        IpcRequest::SendText { pane, text } => {
            body.push(4);
            body.extend_from_slice(&pane.0.to_be_bytes());
            put_string(&mut body, text)?;
        }
        IpcRequest::Split { direction } => {
            body.push(5);
            body.push(encode_direction(*direction));
        }
        IpcRequest::ActivateWorkspace(value) => {
            body.push(6);
            put_string(&mut body, value)?;
        }
    }
    write_frame(stream, &body)
}

fn read_request(stream: &mut impl Read, expected_token: &str) -> io::Result<IpcRequest> {
    let body = read_frame(stream)?;
    let mut cursor = io::Cursor::new(body);
    read_header(&mut cursor)?;
    let token = get_string(&mut cursor)?;
    if !constant_time_eq(token.as_bytes(), expected_token.as_bytes()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "IPC authentication failed",
        ));
    }
    let request = match read_u8(&mut cursor)? {
        0 => IpcRequest::Eval(get_string(&mut cursor)?),
        1 => IpcRequest::List,
        2 => IpcRequest::Reload,
        3 => IpcRequest::ListPanes,
        4 => IpcRequest::SendText {
            pane: PaneId(read_u64(&mut cursor)?),
            text: get_string(&mut cursor)?,
        },
        5 => IpcRequest::Split {
            direction: decode_direction(read_u8(&mut cursor)?)?,
        },
        6 => IpcRequest::ActivateWorkspace(get_string(&mut cursor)?),
        value => return Err(invalid_data(format!("unknown IPC request type {value}"))),
    };
    if cursor.position() != cursor.get_ref().len() as u64 {
        return Err(invalid_data("trailing bytes in IPC request"));
    }
    Ok(request)
}

fn write_response(stream: &mut impl Write, response: &Result<String, String>) -> io::Result<()> {
    let mut body = Vec::new();
    body.extend_from_slice(MAGIC);
    body.extend_from_slice(&VERSION.to_be_bytes());
    let (status, text) = match response {
        Ok(value) => (0, value),
        Err(value) => (1, value),
    };
    body.push(status);
    put_string(&mut body, text)?;
    write_frame(stream, &body)
}
fn read_response(stream: &mut impl Read) -> io::Result<Result<String, String>> {
    let body = read_frame(stream)?;
    let mut cursor = io::Cursor::new(body);
    read_header(&mut cursor)?;
    let status = read_u8(&mut cursor)?;
    let text = get_string(&mut cursor)?;
    if cursor.position() != cursor.get_ref().len() as u64 {
        return Err(invalid_data("trailing bytes in IPC response"));
    }
    match status {
        0 => Ok(Ok(text)),
        1 => Ok(Err(text)),
        _ => Err(invalid_data("invalid IPC response status")),
    }
}
fn read_header(reader: &mut impl Read) -> io::Result<()> {
    let mut magic = [0; 4];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(invalid_data("invalid IPC protocol magic"));
    }
    let version = read_u16(reader)?;
    if version != VERSION {
        return Err(invalid_data(format!(
            "unsupported IPC protocol version {version}; expected {VERSION}"
        )));
    }
    Ok(())
}
fn write_frame(stream: &mut impl Write, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "message is too large",
        ));
    }
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(body)
}
fn read_frame(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let length = read_u32(stream)? as usize;
    if length > MAX_MESSAGE {
        return Err(invalid_data("message is too large"));
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(body)
}
fn put_string(output: &mut Vec<u8>, value: &str) -> io::Result<()> {
    let length = u32::try_from(value.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "string is too large"))?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}
fn get_string(reader: &mut impl Read) -> io::Result<String> {
    let length = read_u32(reader)? as usize;
    if length > MAX_MESSAGE {
        return Err(invalid_data("string is too large"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| invalid_data("IPC string is not UTF-8"))
}
fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}
fn read_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}
fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}
fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_be_bytes(b))
}
fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn encode_direction(value: SplitDirection) -> u8 {
    match value {
        SplitDirection::Left => 0,
        SplitDirection::Right => 1,
        SplitDirection::Up => 2,
        SplitDirection::Down => 3,
    }
}
fn decode_direction(value: u8) -> io::Result<SplitDirection> {
    match value {
        0 => Ok(SplitDirection::Left),
        1 => Ok(SplitDirection::Right),
        2 => Ok(SplitDirection::Up),
        3 => Ok(SplitDirection::Down),
        _ => Err(invalid_data("invalid split direction")),
    }
}
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && left.iter().zip(right).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}

struct RuntimePaths {
    instances: PathBuf,
    active: PathBuf,
}
impl RuntimePaths {
    fn new() -> Result<Self, String> {
        let paths = Self::from_environment();
        create_private_dir(
            paths
                .instances
                .parent()
                .expect("instances directory has a runtime root"),
        )?;
        create_private_dir(&paths.instances)?;
        Ok(paths)
    }

    fn from_environment() -> Self {
        let root = runtime_root();
        let instances = root.join("instances");
        Self {
            active: root.join("active"),
            instances,
        }
    }
}
fn runtime_root() -> PathBuf {
    if let Some(path) = std::env::var_os("TOYOTERM_RUNTIME_DIR") {
        return PathBuf::from(path);
    }
    #[cfg(unix)]
    {
        if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR") {
            return PathBuf::from(path).join("toyoterm");
        }
        std::env::temp_dir().join(format!("toyoterm-{}", effective_user_id()))
    }
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("toyoterm")
            .join("runtime")
    }
}
fn requested_instance() -> Option<String> {
    std::env::var("TOYOTERM_INSTANCE")
        .ok()
        .filter(|v| !v.is_empty())
}
fn validate_instance_id(id: &str) -> Result<(), String> {
    if id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        Ok(())
    } else {
        Err("TOYOTERM_INSTANCE must contain only ASCII letters, digits, '-' or '_' (maximum 64 characters)".into())
    }
}
fn load_instance_state() -> Result<InstanceState, String> {
    let paths = RuntimePaths::from_environment();
    let id = match requested_instance() {
        Some(id) => {
            validate_instance_id(&id)?;
            id
        }
        None => fs::read_to_string(&paths.active)
            .map_err(|_| "no running toyoterm GUI was found".to_owned())?
            .trim()
            .to_owned(),
    };
    let contents = fs::read_to_string(paths.instances.join(format!("{id}.state")))
        .map_err(|_| format!("toyoterm instance `{id}` is not running"))?;
    parse_state(&contents)
}
fn serialize_state(s: &InstanceState) -> String {
    format!(
        "version={VERSION}\nid={}\npid={}\ntransport={}\nendpoint={}\ntoken={}\n",
        s.id, s.pid, s.transport, s.endpoint, s.token
    )
}
fn remove_stale_instance(path: &Path) -> Result<(), String> {
    let Ok(contents) = fs::read_to_string(path) else {
        return Ok(());
    };
    if let Ok(state) = parse_state(&contents) {
        if process_is_alive(state.pid) {
            return Err(format!(
                "toyoterm instance `{}` is already running with pid {}",
                state.id, state.pid
            ));
        }
        #[cfg(unix)]
        if state.transport == "unix" {
            let _ = fs::remove_file(state.endpoint);
        }
    }
    fs::remove_file(path).map_err(|error| format!("remove stale IPC instance state: {error}"))
}
fn parse_state(contents: &str) -> Result<InstanceState, String> {
    let value = |name: &str| {
        contents
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .map(str::to_owned)
            .ok_or_else(|| format!("invalid instance state: missing {name}"))
    };
    let version = value("version")?
        .parse::<u16>()
        .map_err(|_| "invalid instance protocol version".to_owned())?;
    if version != VERSION {
        return Err(format!(
            "instance uses IPC protocol version {version}; client expects {VERSION}"
        ));
    }
    Ok(InstanceState {
        id: value("id")?,
        pid: value("pid")?
            .parse()
            .map_err(|_| "invalid instance pid".to_owned())?,
        transport: value("transport")?,
        endpoint: value("endpoint")?,
        token: value("token")?,
    })
}
fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("create IPC runtime directory: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("secure IPC runtime directory: {e}"))?;
    }
    Ok(())
}
fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("write IPC instance state: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("secure IPC instance state: {e}"))?;
    }
    file.write_all(contents).map_err(|e| e.to_string())
}
fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    fill_random(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn history_path() -> PathBuf {
    std::env::var_os("TOYOTERM_HISTORY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("toyoterm-ruby-history"))
}
fn load_history() -> Vec<String> {
    fs::read_to_string(history_path())
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}
fn save_history(h: &[String]) {
    let _ = fs::write(history_path(), h.join("\n"));
}
fn input_incomplete(source: &str) -> bool {
    let (mut depth, mut quote, mut escaped) = (0i32, None, false);
    for c in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if c == active {
                quote = None
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    quote.is_some()
        || depth > 0
        || source
            .lines()
            .last()
            .is_some_and(|line| line.trim_end().ends_with('\\'))
}
pub(crate) fn is_incomplete_ruby_error(error: &str) -> bool {
    error.contains("syntax error") && error.contains("unexpected end of file")
}

#[cfg(unix)]
mod transport {
    use super::*;
    use std::os::unix::net::{UnixListener, UnixStream};
    pub struct Listener(UnixListener);
    pub struct Stream(UnixStream);
    impl Read for Stream {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0.read(b)
        }
    }
    impl Write for Stream {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.write(b)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl Listener {
        pub fn bind(
            paths: &RuntimePaths,
            id: &str,
        ) -> Result<(Self, &'static str, String), String> {
            let path = paths.instances.join(format!("{id}.sock"));
            if path.exists() {
                let _ = fs::remove_file(&path);
            }
            let listener =
                UnixListener::bind(&path).map_err(|e| format!("bind Unix IPC socket: {e}"))?;
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("secure Unix IPC socket: {e}"))?;
            Ok((Self(listener), "unix", path.display().to_string()))
        }
        pub fn accept(&self) -> io::Result<Stream> {
            self.0.accept().map(|(s, _)| Stream(s))
        }
    }
    impl Stream {
        pub fn connect(endpoint: &str) -> io::Result<Self> {
            UnixStream::connect(endpoint).map(Self)
        }
        pub fn connect_for_wakeup(endpoint: &str) -> io::Result<()> {
            Self::connect(endpoint).map(|_| ())
        }
    }
}

#[cfg(windows)]
mod transport {
    use super::*;
    use std::ffi::{OsStr, c_void};
    use std::fs::File;
    use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
    type Handle = *mut c_void;
    const INVALID: Handle = -1isize as Handle;
    const ERROR_PIPE_CONNECTED: i32 = 535;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateNamedPipeW(
            n: *const u16,
            o: u32,
            m: u32,
            x: u32,
            a: u32,
            b: u32,
            t: u32,
            s: *const c_void,
        ) -> Handle;
        fn ConnectNamedPipe(h: Handle, o: *mut c_void) -> i32;
        fn CreateFileW(
            n: *const u16,
            a: u32,
            s: u32,
            p: *const c_void,
            c: u32,
            f: u32,
            t: Handle,
        ) -> Handle;
    }
    pub struct Listener {
        name: String,
    }
    pub struct Stream(File);
    impl Read for Stream {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0.read(b)
        }
    }
    impl Write for Stream {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.write(b)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    fn wide(v: &str) -> Vec<u16> {
        OsStr::new(v).encode_wide().chain(Some(0)).collect()
    }
    impl Listener {
        pub fn bind(_: &RuntimePaths, id: &str) -> Result<(Self, &'static str, String), String> {
            let name = format!(r"\\.\pipe\toyoterm-{id}");
            Ok((Self { name: name.clone() }, "named-pipe", name))
        }
        pub fn accept(&self) -> io::Result<Stream> {
            let h = unsafe {
                CreateNamedPipeW(
                    wide(&self.name).as_ptr(),
                    3,
                    0x00000008, // PIPE_REJECT_REMOTE_CLIENTS, byte mode, blocking mode.
                    255,
                    65536,
                    65536,
                    0,
                    std::ptr::null(),
                )
            };
            if h == INVALID {
                return Err(io::Error::last_os_error());
            }
            let ok = unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) };
            if ok == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED) {
                    unsafe { drop(File::from_raw_handle(h)) };
                    return Err(error);
                }
            }
            Ok(Stream(unsafe { File::from_raw_handle(h) }))
        }
    }
    impl Stream {
        pub fn connect(e: &str) -> io::Result<Self> {
            let h = unsafe {
                CreateFileW(
                    wide(e).as_ptr(),
                    0xC0000000,
                    0,
                    std::ptr::null(),
                    3,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if h == INVALID {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(unsafe { File::from_raw_handle(h) }))
            }
        }
        pub fn connect_for_wakeup(e: &str) -> io::Result<()> {
            Self::connect(e).map(|_| ())
        }
    }
}
use transport::{Listener as TransportListener, Stream as TransportStream};
fn platform_transport() -> &'static str {
    if cfg!(unix) { "unix" } else { "named-pipe" }
}
#[cfg(unix)]
fn fill_random(bytes: &mut [u8]) -> Result<(), String> {
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(bytes))
        .map_err(|e| format!("generate IPC authentication token: {e}"))
}
#[cfg(unix)]
fn effective_user_id() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}
#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    i32::try_from(pid).is_ok_and(|pid| unsafe { kill(pid, 0) } == 0)
}
#[cfg(windows)]
fn fill_random(bytes: &mut [u8]) -> Result<(), String> {
    #[link(name = "advapi32")]
    unsafe extern "system" {
        #[link_name = "SystemFunction036"]
        fn random(b: *mut u8, l: u32) -> u8;
    }
    if unsafe { random(bytes.as_mut_ptr(), bytes.len() as u32) } == 0 {
        Err("generate IPC authentication token: RtlGenRandom failed".into())
    } else {
        Ok(())
    }
}
#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let handle = unsafe { OpenProcess(0x1000, 0, pid) };
    if handle.is_null() {
        false
    } else {
        unsafe { CloseHandle(handle) };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_round_trips() {
        let requests = [
            IpcRequest::Eval("1+2".into()),
            IpcRequest::List,
            IpcRequest::Reload,
            IpcRequest::ListPanes,
            IpcRequest::SendText {
                pane: PaneId(42),
                text: "hi\n".into(),
            },
            IpcRequest::Split {
                direction: SplitDirection::Left,
            },
            IpcRequest::ActivateWorkspace("dev".into()),
        ];
        for request in requests {
            let mut bytes = Vec::new();
            write_request(&mut bytes, "secret", &request).unwrap();
            assert_eq!(
                read_request(&mut io::Cursor::new(bytes), "secret").unwrap(),
                request
            )
        }
    }
    #[test]
    fn rejects_wrong_token() {
        let mut bytes = Vec::new();
        write_request(&mut bytes, "secret", &IpcRequest::List).unwrap();
        assert_eq!(
            read_request(&mut io::Cursor::new(bytes), "wrong")
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        )
    }
    #[test]
    fn mutations_use_native_commands() {
        let pane = PaneId(9);
        assert_eq!(
            IpcRequest::Reload.native_command(Some(pane)).unwrap(),
            Some(NativeCommand::ReloadConfig)
        );
        assert_eq!(
            IpcRequest::Split {
                direction: SplitDirection::Down
            }
            .native_command(Some(pane))
            .unwrap(),
            Some(NativeCommand::Mux(Command::Split {
                pane,
                direction: SplitDirection::Down
            }))
        );
        assert_eq!(IpcRequest::List.native_command(Some(pane)).unwrap(), None)
    }
    #[test]
    fn detects_multiline() {
        assert!(input_incomplete("[1,\n"));
        assert!(!input_incomplete("[1,2]\n"));
        assert!(is_incomplete_ruby_error(
            "syntax error, unexpected end of file"
        ));
    }
}
