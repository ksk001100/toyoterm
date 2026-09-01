use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

const MAX_MESSAGE: usize = 1024 * 1024;

pub type EvalResponse = mpsc::Sender<Result<String, String>>;

pub struct IpcServer {
    state_path: PathBuf,
    address: String,
    stop: Arc<AtomicBool>,
}

impl IpcServer {
    pub fn start(
        dispatch: impl Fn(String, EvalResponse) -> Result<(), String> + Send + 'static,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("start Ruby console listener: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .to_string();
        let state_path = state_path();
        write_state(&state_path, &address)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        thread::Builder::new().name("toyoterm-ruby-ipc".into()).spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => handle_client(stream, &dispatch),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(error) => {
                        tracing::warn!(target: "toyoterm::script", %error, "Ruby console listener failed");
                        break;
                    }
                }
            }
        }).map_err(|error| format!("spawn Ruby console listener: {error}"))?;
        Ok(Self {
            state_path,
            address,
            stop,
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ =
            TcpStream::connect(&self.address).and_then(|stream| stream.shutdown(Shutdown::Both));
        let _ = fs::remove_file(&self.state_path);
    }
}

pub fn eval_remote(source: &str) -> Result<String, String> {
    let address = fs::read_to_string(state_path())
        .map_err(|_| "no running toyoterm GUI was found".to_owned())?;
    let mut stream = TcpStream::connect(address.trim())
        .map_err(|error| format!("connect to running toyoterm GUI: {error}"))?;
    write_frame(&mut stream, source.as_bytes()).map_err(|error| error.to_string())?;
    let mut status = [0_u8; 1];
    stream
        .read_exact(&mut status)
        .map_err(|error| error.to_string())?;
    let body = read_frame(&mut stream).map_err(|error| error.to_string())?;
    let text = String::from_utf8(body).map_err(|_| "GUI returned invalid UTF-8".to_owned())?;
    if status[0] == 0 { Ok(text) } else { Err(text) }
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
        .map_err(|error| error.to_string())?;
        stdout.flush().map_err(|error| error.to_string())?;
        let mut line = String::new();
        if input
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            break;
        }
        let trimmed = line.trim_end();
        if source.is_empty() && matches!(trimmed, "exit" | "quit" | ":q") {
            break;
        }
        if source.is_empty() && trimmed == ":history" {
            for (index, entry) in history.iter().enumerate() {
                println!("{:>4}  {}", index + 1, entry);
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
    mut stream: TcpStream,
    dispatch: &impl Fn(String, EvalResponse) -> Result<(), String>,
) {
    let response = read_frame(&mut stream)
        .map_err(|error| error.to_string())
        .and_then(|body| String::from_utf8(body).map_err(|_| "request is not UTF-8".into()))
        .and_then(|source| {
            let (sender, receiver) = mpsc::channel();
            dispatch(source, sender)?;
            receiver
                .recv()
                .map_err(|_| "GUI closed before evaluating Ruby".to_owned())?
        });
    let (status, body) = match response {
        Ok(value) => (0, value),
        Err(error) => (1, error),
    };
    let _ = stream.write_all(&[status]);
    let _ = write_frame(&mut stream, body.as_bytes());
}

fn write_frame(stream: &mut TcpStream, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "message is too large",
        ));
    }
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(body)
}

fn read_frame(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message is too large",
        ));
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(body)
}

fn state_path() -> PathBuf {
    std::env::var_os("TOYOTERM_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("toyoterm-ruby.addr")
}

fn history_path() -> PathBuf {
    std::env::var_os("TOYOTERM_HISTORY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("toyoterm-ruby-history"))
}

fn write_state(path: &PathBuf, address: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("write Ruby console state: {error}"))?;
    file.write_all(address.as_bytes())
        .map_err(|error| error.to_string())
}

fn load_history() -> Vec<String> {
    fs::read_to_string(history_path())
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn save_history(history: &[String]) {
    let _ = fs::write(history_path(), history.join("\n"));
}

fn input_incomplete(source: &str) -> bool {
    let mut depth = 0_i32;
    let mut quote = None;
    let mut escaped = false;
    for character in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_common_multiline_input() {
        assert!(input_incomplete("[1,\n"));
        assert!(input_incomplete("\"hello"));
        assert!(!input_incomplete("[1, 2]\n"));
        assert!(is_incomplete_ruby_error(
            "syntax error, unexpected end of file"
        ));
    }
}
