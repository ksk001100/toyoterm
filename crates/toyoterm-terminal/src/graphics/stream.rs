//! Strip graphics strings before the text parser, keeping non-graphics bytes intact.
//! Limits apply across PTY reads; overflow discards until the string terminator.
use super::MAX_BYTES;
use std::borrow::Cow;

const MAX_OSC52_SEQUENCE_BYTES: usize = crate::MAX_OSC52_COPY_BYTES.div_ceil(3) * 4 + 5;
const MAX_ITERM_MULTIPART_SEQUENCE_BYTES: usize = 1024 * 1024;

const ITERM_MULTIPART_PREFIXES: [&[u8]; 3] =
    [b"1337;MultipartFile=", b"1337;FilePart=", b"1337;FileEnd"];

fn is_iterm_multipart(bytes: &[u8]) -> bool {
    ITERM_MULTIPART_PREFIXES
        .iter()
        .any(|prefix| bytes.starts_with(prefix) || prefix.starts_with(bytes))
}

pub(crate) struct Stream {
    state: State,
    utf8_continuations: u8,
}
impl Default for Stream {
    fn default() -> Self {
        Self {
            state: State::Ground,
            utf8_continuations: 0,
        }
    }
}
#[derive(Default)]
enum State {
    #[default]
    Ground,
    Escape,
    Csi {
        bytes: Vec<u8>,
    },
    String {
        kind: u8,
        bytes: Vec<u8>,
        escape: bool,
        overflow: bool,
    },
}
#[cfg_attr(test, derive(Debug, PartialEq))]
pub(crate) enum Token<'a> {
    Text(Cow<'a, [u8]>),
    Graphic(u8, Vec<u8>),
    Osc1337(Vec<u8>),
    Osc66(Vec<u8>),
    Osc5113(Vec<u8>),
    CellSizeQuery,
    Cancel,
}

impl Stream {
    pub fn advance<'a>(&mut self, bytes: &'a [u8]) -> Vec<Token<'a>> {
        let mut tokens = Vec::new();
        let mut text = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            // Ordinary ASCII cannot introduce a graphics string or continue UTF-8.
            // Copy mixed runs in bulk and borrow entirely ASCII reads, avoiding
            // a parser state move per byte. ESC and non-ASCII remain incremental.
            if matches!(self.state, State::Ground) {
                let count = bytes[offset..]
                    .iter()
                    .position(|&byte| byte == 0x1b || !byte.is_ascii())
                    .unwrap_or(bytes.len() - offset);
                if count > 0 {
                    self.utf8_continuations = 0;
                    if offset == 0 && count == bytes.len() {
                        tokens.push(Token::Text(Cow::Borrowed(bytes)));
                        return tokens;
                    }
                    text.extend_from_slice(&bytes[offset..offset + count]);
                    offset += count;
                    continue;
                }
            }
            let byte = bytes[offset];
            offset += 1;
            // C1 controls overlap UTF-8 continuation bytes. Track well-formed UTF-8
            // so non-ASCII text cannot accidentally start or end a control string.
            let utf8_continuation = if self.utf8_continuations > 0 && (0x80..=0xbf).contains(&byte)
            {
                self.utf8_continuations -= 1;
                true
            } else {
                self.utf8_continuations = match byte {
                    0xc2..=0xdf => 1,
                    0xe0..=0xef => 2,
                    0xf0..=0xf4 => 3,
                    _ => 0,
                };
                false
            };
            self.state = match std::mem::take(&mut self.state) {
                State::Ground if byte == 0x1b => State::Escape,
                State::Ground
                    if !utf8_continuation && matches!(byte, 0x90 | 0x9d | 0x9e | 0x9f | 0x98) =>
                {
                    if !text.is_empty() {
                        tokens.push(Token::Text(std::mem::take(&mut text).into()));
                    }
                    State::String {
                        kind: match byte {
                            0x90 => b'P',
                            0x9d => b']',
                            0x9e => b'^',
                            0x9f => b'_',
                            0x98 => b'X',
                            _ => unreachable!(),
                        },
                        bytes: Vec::new(),
                        escape: false,
                        overflow: false,
                    }
                }
                State::Ground => {
                    text.push(byte);
                    State::Ground
                }
                State::Escape if byte == b'[' => State::Csi { bytes: Vec::new() },
                State::Escape if matches!(byte, b'P' | b'_' | b']' | b'^' | b'X') => {
                    if !text.is_empty() {
                        tokens.push(Token::Text(std::mem::take(&mut text).into()));
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
                State::Csi { bytes } if byte == 0x1b => {
                    text.extend_from_slice(b"\x1b[");
                    text.extend_from_slice(&bytes);
                    State::Escape
                }
                State::Csi { mut bytes } => {
                    bytes.push(byte);
                    if (0x40..=0x7e).contains(&byte) {
                        if bytes == b"16t" {
                            if !text.is_empty() {
                                tokens.push(Token::Text(std::mem::take(&mut text).into()));
                            }
                            tokens.push(Token::CellSizeQuery);
                        } else {
                            text.extend_from_slice(b"\x1b[");
                            text.extend_from_slice(&bytes);
                        }
                        State::Ground
                    } else if bytes.len() >= 64 {
                        // Stop inspecting unusually long CSI sequences and let the
                        // normal VT parser consume the remainder incrementally.
                        text.extend_from_slice(b"\x1b[");
                        text.extend_from_slice(&bytes);
                        State::Ground
                    } else {
                        State::Csi { bytes }
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
                    } else if (!utf8_continuation && byte == 0x9c)
                        || (escape && byte == b'\\')
                        || (kind == b']' && byte == 7)
                    {
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
                            } else if kind == b']' && bytes.starts_with(b"66;") {
                                tokens.push(Token::Osc66(bytes));
                            } else if kind == b']' && bytes.starts_with(b"5113;") {
                                tokens.push(Token::Osc5113(bytes));
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
            tokens.push(Token::Text(text.into()));
        }
        tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(chunks: &[&[u8]]) -> Vec<Token<'static>> {
        let mut stream = Stream::default();
        let mut result = Vec::new();
        for chunk in chunks {
            for token in stream.advance(chunk) {
                if let Token::Text(bytes) = token {
                    if let Some(Token::Text(previous)) = result.last_mut() {
                        previous.to_mut().extend_from_slice(&bytes);
                    } else {
                        result.push(Token::Text(Cow::Owned(bytes.into_owned())));
                    }
                } else {
                    result.push(match token {
                        Token::Graphic(kind, bytes) => Token::Graphic(kind, bytes),
                        Token::Osc1337(bytes) => Token::Osc1337(bytes),
                        Token::Osc66(bytes) => Token::Osc66(bytes),
                        Token::Osc5113(bytes) => Token::Osc5113(bytes),
                        Token::CellSizeQuery => Token::CellSizeQuery,
                        Token::Cancel => Token::Cancel,
                        Token::Text(_) => unreachable!(),
                    });
                }
            }
        }
        result
    }

    #[test]
    fn borrows_plain_ascii_and_resets_interrupted_utf8() {
        let mut stream = Stream::default();
        stream.advance(b"\xc2");
        assert!(matches!(
            stream.advance(b"plain\r\n\ttext").as_slice(),
            [Token::Text(Cow::Borrowed(b"plain\r\n\ttext"))]
        ));
        assert_eq!(
            stream.advance(b"\x9fGq=2\x9c"),
            vec![Token::Graphic(b'_', b"Gq=2".to_vec())]
        );
    }

    #[test]
    fn preserves_text_and_protocol_order_at_every_chunk_boundary() {
        let fixtures: &[&[u8]] = &[
            "ASCII\r\n\x1b[32mАĝĞğ界😀\x1b[0m tail".as_bytes(),
            "prefix\x1b]2;АĝĞğ title\x07suffix".as_bytes(),
            b"before\x1b_Gq=2\x1b\\after\x1b[16tend",
            b"before\x9fGq=2\x9cafter\x9d1337;FileEnd\x07tail",
            b"text\x1bPqdata\x18tail\x1b]66;s=2;Hi\x1b\\end",
            b"\xc2text\x9fGq=2\x9c\x1b]2;cancel\x1bXafter\x1b[16t",
        ];
        for &fixture in fixtures {
            let expected = collect(&[fixture]);
            for split in 0..=fixture.len() {
                assert_eq!(
                    collect(&[&fixture[..split], &fixture[split..]]),
                    expected,
                    "fixture {fixture:?}, split {split}"
                );
            }
            for size in 1..=fixture.len() {
                assert_eq!(collect(&fixture.chunks(size).collect::<Vec<_>>()), expected);
            }
        }
        assert_eq!(
            collect(&[fixtures[2]]),
            vec![
                Token::Text(Cow::Borrowed(b"before")),
                Token::Graphic(b'_', b"Gq=2".to_vec()),
                Token::Text(Cow::Borrowed(b"after")),
                Token::CellSizeQuery,
                Token::Text(Cow::Borrowed(b"end")),
            ]
        );
    }
}
