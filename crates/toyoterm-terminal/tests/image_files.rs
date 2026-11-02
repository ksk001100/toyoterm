use std::io::{Cursor, Write};

use base64::{Engine, engine::general_purpose::STANDARD};
use tempfile::NamedTempFile;
use toyoterm_terminal::{
    AlacrittyTerminalBackend, ImageFileRequest, TerminalBackend, TerminalEvent,
};

fn terminal() -> AlacrittyTerminalBackend {
    let mut t = AlacrittyTerminalBackend::with_scrollback(20, 6, 10);
    t.set_cell_size(1, 1);
    t
}

fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 128]));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn file(bytes: &[u8]) -> NamedTempFile {
    let mut file = tempfile::Builder::new()
        .prefix("toyoterm-画像-")
        .tempfile()
        .unwrap();
    file.write_all(bytes).unwrap();
    file.flush().unwrap();
    file
}

fn command(header: &str, path: &std::path::Path) -> Vec<u8> {
    format!(
        "\x1b_Gt=f,{header};{}\x1b\\",
        STANDARD.encode(path.to_str().unwrap())
    )
    .into_bytes()
}

fn request(t: &mut AlacrittyTerminalBackend) -> ImageFileRequest {
    let events = t.drain_events();
    assert_eq!(events.len(), 1, "{events:?}");
    match events.into_iter().next().unwrap() {
        TerminalEvent::ImageFileRequested(request) => request,
        event => panic!("unexpected event: {event:?}"),
    }
}

#[test]
fn file_transmit_display_and_following_text_match_direct_at_every_pty_split() {
    let png = png();
    let file = file(&png);
    let suffix = b"after\x1b]2;loaded\x07\x1b]133;A\x07\r\nnext";
    let mut stream = command("a=T,f=100,i=9", file.path());
    stream.extend_from_slice(suffix);
    let mut direct = terminal();
    direct.advance(format!("\x1b_Ga=T,f=100,i=9;{}\x1b\\", STANDARD.encode(&png)).as_bytes());
    direct.advance(suffix);
    let expected = direct.snapshot();
    let expected_events = direct.drain_events();
    for split in 0..=stream.len() {
        let mut t = terminal();
        t.advance(&stream[..split]);
        t.advance(&stream[split..]);
        assert!(t.snapshot().images.is_empty());
        assert!(!t.visible_text().contains("after"));
        let job = request(&mut t);
        // Only the immutable job moves; terminal state stays on the owner.
        let loaded = std::thread::spawn(move || job.load()).join().unwrap();
        assert!(t.complete_image_file(loaded));
        assert_eq!(t.snapshot(), expected, "split={split}");
        assert_eq!(t.drain_events(), expected_events, "split={split}");
    }
}

#[test]
fn snacks_file_transmission_precedes_virtual_placement_and_unicode_cells() {
    let file = file(&png());
    let mut t = terminal();
    t.advance(&command("f=100,i=7,q=2", file.path()));
    t.advance(b"\x1b_Ga=p,U=1,i=7,p=3,C=1,c=2,r=1,q=2\x1b\\");
    t.advance(
        "\x1b[38;2;0;0;7;58;2;0;0;3m\u{10eeee}\u{0305}\u{0305}\u{10eeee}\u{0305}\u{030d}"
            .as_bytes(),
    );
    let job = request(&mut t);
    assert!(t.complete_image_file(job.load()));
    let snapshot = t.snapshot();
    assert_eq!(snapshot.images.len(), 1);
    assert_eq!(&*snapshot.images[0].rgba, &[255, 0, 0, 128, 255, 0, 0, 128]);
    assert!(t.drain_events().is_empty());
}

#[test]
fn offset_size_rgb_zlib_and_queries_use_the_normal_image_decoder() {
    for compressed in [false, true] {
        let mut bytes = vec![255, 0, 0];
        if compressed {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&bytes).unwrap();
            bytes = encoder.finish().unwrap();
        }
        let mut content = b"prefix".to_vec();
        content.extend_from_slice(&bytes);
        content.extend_from_slice(b"suffix");
        let file = file(&content);
        let mut t = terminal();
        let compression = if compressed { ",o=z" } else { "" };
        let params = format!("f=24,s=1,v=1,i=5,O=6,S={}{}", bytes.len(), compression);
        t.advance(&command(&format!("a=q,{params}"), file.path()));
        let job = request(&mut t);
        t.complete_image_file(job.load());
        assert!(t.snapshot().images.is_empty());
        assert_eq!(
            t.drain_events(),
            [TerminalEvent::PtyWrite("\x1b_Gi=5;OK\x1b\\".into())]
        );
        t.advance(&command(&format!("a=T,C=1,{params}"), file.path()));
        let job = request(&mut t);
        t.complete_image_file(job.load());
        assert_eq!(&*t.snapshot().images[0].rgba, &[255, 0, 0, 255]);
    }
}

#[test]
fn file_failures_are_indistinguishable_and_quiet_modes_are_respected() {
    let directory = tempfile::tempdir().unwrap();
    let invalid = file(b"not a PNG");
    let oversized = file(b"");
    oversized.as_file().set_len(32 * 1024 * 1024 + 1).unwrap();
    for path in [
        directory.path().join("missing"),
        directory.path().to_owned(),
        invalid.path().to_owned(),
        oversized.path().to_owned(),
    ] {
        for quiet in 0..=2 {
            let mut t = terminal();
            t.advance(&command(&format!("a=T,f=100,i=8,q={quiet}"), &path));
            t.advance(b"still responsive");
            let job = request(&mut t);
            t.complete_image_file(job.load());
            assert!(t.visible_text().contains("still responsive"));
            assert!(t.snapshot().images.is_empty());
            let expected = if quiet == 2 {
                vec![]
            } else {
                vec![TerminalEvent::PtyWrite(
                    "\x1b_Gi=8;EBADF:Failed to read image file\x1b\\".into(),
                )]
            };
            assert_eq!(t.drain_events(), expected);
        }
    }
}

#[test]
fn invalid_file_ranges_fail_without_disclosing_the_file_state() {
    let file = file(&png());
    for range in ["O=999999", "S=999999", "O=18446744073709551615,S=10"] {
        let mut t = terminal();
        t.advance(&command(&format!("a=q,f=100,i=8,{range}"), file.path()));
        let job = request(&mut t);
        t.complete_image_file(job.load());
        assert_eq!(
            t.drain_events(),
            [TerminalEvent::PtyWrite(
                "\x1b_Gi=8;EBADF:Failed to read image file\x1b\\".into()
            )]
        );
    }
}

#[cfg(windows)]
#[test]
fn non_local_windows_paths_fail_before_any_network_or_device_open() {
    for path in [
        r"\\server\share\image.png",
        r"\\.\pipe\image",
        r"\\?\GLOBALROOT\Device\image",
    ] {
        let mut t = terminal();
        t.advance(&command("a=q,f=100,i=8", std::path::Path::new(path)));
        let job = request(&mut t);
        t.complete_image_file(job.load());
        assert_eq!(
            t.drain_events(),
            [TerminalEvent::PtyWrite(
                "\x1b_Gi=8;EBADF:Failed to read image file\x1b\\".into()
            )]
        );
    }
}

#[test]
fn consecutive_files_and_shell_markers_resume_in_order() {
    let file = file(&png());
    let mut t = terminal();
    t.advance(&command("f=100,i=1,q=2", file.path()));
    t.advance(b"\x1b]133;A\x07");
    t.advance(&command("a=T,f=100,i=2,C=1,q=2", file.path()));
    let first = request(&mut t);
    t.complete_image_file(first.load());
    let events = t.drain_events();
    assert!(matches!(
        &events[..],
        [
            TerminalEvent::PromptStarted,
            TerminalEvent::ImageFileRequested(_)
        ]
    ));
    let TerminalEvent::ImageFileRequested(second) = events.into_iter().nth(1).unwrap() else {
        unreachable!()
    };
    t.complete_image_file(second.load());
    assert_eq!(t.snapshot().images.len(), 1);
    t.advance(b"\x1bc");
    assert!(t.snapshot().images.is_empty());
}

#[test]
fn resize_overflow_and_other_terminal_results_cannot_resurrect_images() {
    let file = file(&png());
    let mut t = terminal();
    t.advance(&command("a=T,f=100,i=8,q=2", file.path()));
    let old = request(&mut t);
    t.resize(30, 6);
    assert!(!t.complete_image_file(old.load()));
    t.advance(&command("a=T,f=100,i=8,q=2", file.path()));
    let job = request(&mut t);
    let mut other = terminal();
    other.advance(&command("a=T,f=100,i=8,q=2", file.path()));
    let wrong = request(&mut other);
    assert!(!t.complete_image_file(wrong.load()));
    t.advance(&vec![b' '; 1024 * 1024 + 1]);
    t.advance(b"end");
    assert!(!t.complete_image_file(job.load()));
    assert!(t.snapshot().images.is_empty());
    assert!(t.visible_text().contains("end"));
}

#[cfg(unix)]
#[test]
fn symlinks_work_but_devices_and_fifos_are_rejected() {
    use std::os::unix::fs::symlink;
    let file = file(&png());
    let directory = tempfile::tempdir().unwrap();
    let link = directory.path().join("link");
    symlink(file.path(), &link).unwrap();
    let mut t = terminal();
    t.advance(&command("a=T,f=100,i=8", &link));
    let job = request(&mut t);
    t.complete_image_file(job.load());
    assert_eq!(t.snapshot().images.len(), 1);
    t.drain_events();
    let fifo = directory.path().join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    for path in [std::path::Path::new("/dev/zero"), &fifo] {
        t.advance(&command("a=q,f=100,i=8", path));
        let job = request(&mut t);
        t.complete_image_file(job.load());
        assert_eq!(
            t.drain_events(),
            [TerminalEvent::PtyWrite(
                "\x1b_Gi=8;EBADF:Failed to read image file\x1b\\".into()
            )]
        );
    }
}
