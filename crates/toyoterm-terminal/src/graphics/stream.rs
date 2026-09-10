//! Strip graphics strings before the text parser, keeping non-graphics bytes intact.
//! Limits apply across PTY reads; overflow discards until the string terminator.
use super::MAX_BYTES;

const MAX_OSC52_SEQUENCE_BYTES: usize = crate::MAX_OSC52_COPY_BYTES.div_ceil(3) * 4 + 5;
const MAX_ITERM_MULTIPART_SEQUENCE_BYTES: usize = 1024 * 1024;

const ITERM_MULTIPART_PREFIXES: [&[u8]; 3] =
    [b"1337;MultipartFile=", b"1337;FilePart=", b"1337;FileEnd"];

fn is_iterm_multipart(bytes: &[u8]) -> bool {
    ITERM_MULTIPART_PREFIXES
        .iter()
        .any(|prefix| bytes.starts_with(prefix) || prefix.starts_with(bytes))
}

#[derive(Default)]
pub(crate) struct Stream {
    state: State,
}
#[derive(Default)]
enum State {
    #[default]
    Ground,
    Escape,
    String {
        kind: u8,
        bytes: Vec<u8>,
        escape: bool,
        overflow: bool,
    },
}
pub(crate) enum Token {
    Text(Vec<u8>),
    Graphic(u8, Vec<u8>),
    Osc1337(Vec<u8>),
    Cancel,
}
impl Stream {
    pub fn advance(&mut self, bytes: &[u8]) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut text = Vec::new();
        for &byte in bytes {
            self.state = match std::mem::take(&mut self.state) {
                State::Ground if byte == 0x1b => State::Escape,
                State::Ground => {
                    text.push(byte);
                    State::Ground
                }
                State::Escape if matches!(byte, b'P' | b'_' | b']' | b'^' | b'X') => {
                    if !text.is_empty() {
                        tokens.push(Token::Text(std::mem::take(&mut text)));
                    }
                    State::String {
                        kind: byte,
                        bytes: Vec::new(),
                        escape: false,
                        overflow: false,
                    }
                }
                State::Escape => {
                    text.push(0x1b);
                    if byte == 0x1b {
                        State::Escape
                    } else {
                        text.push(byte);
                        State::Ground
                    }
                }
                State::String {
                    kind,
                    mut bytes,
                    escape,
                    mut overflow,
                } => {
                    if matches!(byte, 0x18 | 0x1a) {
                        tokens.push(Token::Cancel);
                        State::Ground
                    } else if (escape && byte == b'\\') || (kind == b']' && byte == 7) {
                        if !overflow {
                            let graphic = kind == b'_' && bytes.starts_with(b"G")
                                || kind == b']' && bytes.starts_with(b"1337;File=")
                                || kind == b']'
                                    && (bytes.starts_with(b"1337;MultipartFile=")
                                        || bytes.starts_with(b"1337;FilePart=")
                                        || bytes == b"1337;FileEnd")
                                || kind == b'P'
                                    && bytes.iter().find(|b| (0x40..=0x7e).contains(*b))
                                        == Some(&b'q');
                            if graphic {
                                tokens.push(Token::Graphic(kind, bytes));
                            } else if kind == b']' && bytes.starts_with(b"1337;") {
                                tokens.push(Token::Osc1337(bytes));
                            } else {
                                text.extend_from_slice(&[0x1b, kind]);
                                text.extend_from_slice(&bytes);
                                if byte == 7 {
                                    text.push(7);
                                } else {
                                    text.extend_from_slice(b"\x1b\\");
                                }
                            }
                        } else {
                            tokens.push(Token::Cancel);
                        }
                        State::Ground
                    } else if escape {
                        tokens.push(Token::Cancel);
                        // ESC followed by anything except ST cancels this string.
                        text.push(0x1b);
                        text.push(byte);
                        State::Ground
                    } else {
                        if byte != 0x1b && !overflow {
                            let limit = if kind == b']' && is_iterm_multipart(&bytes) {
                                MAX_ITERM_MULTIPART_SEQUENCE_BYTES
                            } else if kind == b']'
                                && (bytes.starts_with(b"52;") || b"52;".starts_with(&bytes))
                            {
                                MAX_OSC52_SEQUENCE_BYTES
                            } else {
                                MAX_BYTES
                            };
                            if bytes.len() == limit {
                                bytes.clear();
                                overflow = true;
                            } else {
                                bytes.push(byte);
                            }
                        }
                        State::String {
                            kind,
                            bytes,
                            escape: byte == 0x1b,
                            overflow,
                        }
                    }
                }
            };
        }
        if !text.is_empty() {
            tokens.push(Token::Text(text));
        }
        tokens
    }
}
