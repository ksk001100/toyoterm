use base64::{Engine, engine::general_purpose::STANDARD};
use std::io::Cursor;
use toyoterm_terminal::{AlacrittyTerminalBackend, TerminalBackend, TerminalEvent};

fn terminal() -> AlacrittyTerminalBackend {
    let mut terminal = AlacrittyTerminalBackend::with_scrollback(20, 6, 10);
    terminal.set_cell_size(1, 1);
    terminal
}
fn kitty(header: &str, bytes: &[u8]) -> Vec<u8> {
    format!("\x1b_G{header};{}\x1b\\", STANDARD.encode(bytes)).into_bytes()
}
fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 128]));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn gif() -> Vec<u8> {
    let image = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Gif).unwrap();
    bytes.into_inner()
}

fn encoded_image(format: image::ImageFormat) -> Vec<u8> {
    let image = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, format).unwrap();
    bytes.into_inner()
}

#[test]
fn every_split_preserves_graphics_text_and_cursor_order() {
    let mut bytes = b"before\r\n".to_vec();
    bytes.extend(kitty(
        "a=T,f=32,s=2,v=1,i=42",
        &[255, 0, 0, 255, 0, 255, 0, 128],
    ));
    bytes.extend(b"after");
    let mut complete = terminal();
    complete.advance(&bytes);
    let expected = complete.snapshot();
    assert_eq!(expected.images.len(), 1);
    assert_eq!((expected.images[0].column, expected.images[0].row), (0, 1));
    assert_eq!(expected.lines[1], "  after");
    assert_eq!(
        complete.drain_events(),
        [TerminalEvent::PtyWrite("\x1b_Gi=42;OK\x1b\\".into())]
    );
    for split in 0..=bytes.len() {
        let mut split_terminal = terminal();
        split_terminal.advance(&bytes[..split]);
        split_terminal.advance(&bytes[split..]);
        assert_eq!(split_terminal.snapshot(), expected, "split={split}");
    }
    let mut single = terminal();
    for byte in bytes {
        single.advance(&[byte]);
    }
    assert_eq!(single.snapshot(), expected);
}

#[test]
fn kitty_chunked_png_transmit_place_delete_and_query() {
    let mut t = terminal();
    let encoded = STANDARD.encode(png());
    t.advance(format!("\x1b_Ga=t,f=100,i=9,m=1;{}\x1b\\", &encoded[..20]).as_bytes());
    assert!(t.snapshot().images.is_empty());
    t.advance(format!("\x1b_Gm=0;{}\x1b\\", &encoded[20..]).as_bytes());
    assert!(t.snapshot().images.is_empty());
    t.advance(b"\x1b_Ga=p,i=9,p=3,c=4,r=2,C=1;\x1b\\");
    let image = t.snapshot().images.remove(0);
    assert_eq!(
        (image.width, image.height, image.columns, image.rows),
        (2, 1, 4, 2)
    );
    assert_eq!(&*image.rgba, &[255, 0, 0, 128, 255, 0, 0, 128]);
    assert_eq!(t.cursor().column, 0);
    t.advance(b"\x1b_Ga=d,d=i,i=9,p=3;\x1b\\");
    assert!(t.snapshot().images.is_empty());
    t.advance(b"\x1b_Ga=p,i=9,C=1;\x1b\\");
    assert_eq!(t.snapshot().images.len(), 1);
    t.advance(b"\x1b_Ga=d,d=I,i=9;\x1b\\\x1b_Ga=p,i=9;\x1b\\");
    assert!(t.snapshot().images.is_empty());
    assert!(
        t.drain_events()
            .iter()
            .any(|e| matches!(e, TerminalEvent::PtyWrite(s) if s.contains("ENOENT")))
    );
    t.advance(&kitty("a=q,f=24,s=1,v=1,i=2", &[0, 0, 255]));
    assert!(t.snapshot().images.is_empty());
    assert_eq!(
        t.drain_events(),
        [TerminalEvent::PtyWrite("\x1b_Gi=2;OK\x1b\\".into())]
    );
}

#[test]
fn kitty_unicode_placeholders_render_ratatui_image_virtual_placements() {
    let mut t = terminal();
    let image_id = 0x0200_002a;
    t.advance(&kitty(
        &format!("q=2,i={image_id},a=T,U=1,f=32,t=d,s=2,v=2"),
        &[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    ));
    assert!(t.snapshot().images.is_empty());

    // ratatui-image emits all three diacritics on the first cell of each row
    // and inherits them across the remaining columns.
    t.advance(
        "\x1b[2;3H\x1b[38;2;0;0;42m\u{10eeee}\u{0305}\u{0305}\u{030e}\u{10eeee}\
         \x1b[3;3H\u{10eeee}\u{030d}\u{0305}\u{030e}\u{10eeee}\x1b[39m"
            .as_bytes(),
    );
    let snapshot = t.snapshot();
    assert_eq!(snapshot.images.len(), 1);
    let image = &snapshot.images[0];
    assert_eq!(
        (image.column, image.row, image.columns, image.rows),
        (2, 1, 2, 2)
    );
    assert_eq!(snapshot.lines[1], "");
    assert_eq!(snapshot.lines[2], "");

    // Placeholder-backed images follow the grid: overwriting the placeholder
    // cells removes the displayed image without a graphics deletion command.
    t.advance(b"\x1b[2;3H  \x1b[3;3H  ");
    assert!(t.snapshot().images.is_empty());
}

#[test]
fn sixel_palette_repeat_and_carriage_return_preserve_previous_pixels() {
    let mut t = terminal();
    for byte in b"\x1bP0;1q\"1;1;3;6#1;2;100;0;0!3~$#2;2;0;100;0A\x1b\\" {
        t.advance(&[*byte]);
    }
    let image = t.snapshot().images.remove(0);
    assert_eq!((image.width, image.height), (3, 6));
    assert_eq!(
        &image.rgba[..12],
        &[255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255]
    );
    assert_eq!(&image.rgba[12..16], &[0, 255, 0, 255]);
    assert_eq!(&image.rgba[16..20], &[255, 0, 0, 255]);
}

#[test]
fn osc1337_png_bel_and_st_do_not_leak_payload_into_text() {
    for end in ["\x07", "\x1b\\"] {
        let mut t = terminal();
        let bytes = png();
        let sequence = format!(
            "\x1b]1337;File=inline=1;size={};width=4px:{}{end}ok",
            bytes.len(),
            STANDARD.encode(bytes)
        );
        for byte in sequence.bytes() {
            t.advance(&[byte]);
        }
        let snap = t.snapshot();
        assert_eq!(snap.images.len(), 1);
        assert_eq!(
            (snap.images[0].display_width, snap.images[0].display_height),
            (4, 2)
        );
        assert_eq!(snap.lines[1], "    ok");
    }
}

#[test]
fn osc1337_gif_displays_its_first_frame() {
    let mut t = terminal();
    let bytes = gif();
    t.advance(
        format!(
            "\x1b]1337;File=inline=1;size={}:{}\x1b\\",
            bytes.len(),
            STANDARD.encode(bytes)
        )
        .as_bytes(),
    );

    let image = t.snapshot().images.remove(0);
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(&*image.rgba, &[255, 0, 0, 255, 0, 255, 0, 255]);
}

#[test]
fn osc1337_displays_bmp_and_webp_images() {
    for format in [image::ImageFormat::Bmp, image::ImageFormat::WebP] {
        let mut t = terminal();
        let bytes = encoded_image(format);
        t.advance(
            format!(
                "\x1b]1337;File=inline=1;size={}:{}\x1b\\",
                bytes.len(),
                STANDARD.encode(bytes)
            )
            .as_bytes(),
        );

        let image = t.snapshot().images.remove(0);
        assert_eq!((image.width, image.height), (2, 1), "format={format:?}");
        assert_eq!(
            &*image.rgba,
            &[255, 0, 0, 255, 0, 255, 0, 255],
            "format={format:?}"
        );
    }
}

#[test]
fn osc1337_multipart_png_preserves_fragmentation_size_and_cursor_order() {
    let bytes = png();
    let encoded = STANDARD.encode(&bytes);
    let split = encoded.len() / 2;
    let sequence = format!(
        "before\r\n\x1b]1337;MultipartFile=inline=1;size={};width=4px\x07\
         \x1b]1337;FilePart={}\x1b\\\
         \x1b]1337;FilePart={}\x07\
         \x1b]1337;FileEnd\x1b\\after",
        bytes.len(),
        &encoded[..split],
        &encoded[split..]
    );
    let sequence = sequence.as_bytes();

    let mut complete = terminal();
    complete.advance(sequence);
    let expected = complete.snapshot();
    assert_eq!(expected.images.len(), 1);
    assert_eq!(
        (
            expected.images[0].column,
            expected.images[0].row,
            expected.images[0].display_width,
            expected.images[0].display_height,
        ),
        (0, 1, 4, 2)
    );
    assert_eq!(expected.lines[2], "    after");

    for split in 0..=sequence.len() {
        let mut fragmented = terminal();
        fragmented.advance(&sequence[..split]);
        fragmented.advance(&sequence[split..]);
        assert_eq!(fragmented.snapshot(), expected, "split={split}");
    }
}

#[test]
fn osc1337_multipart_rejects_downloads_malformed_data_and_oversized_parts() {
    let mut t = terminal();
    t.advance(b"\x1b]1337;MultipartFile=inline=0\x07");
    t.advance(b"\x1b]1337;FilePart=AAAA\x07\x1b]1337;FileEnd\x07");
    t.advance(b"\x1b]1337;MultipartFile=inline=1\x07");
    t.advance(b"\x1b]1337;FilePart=not-base64\x07\x1b]1337;FileEnd\x07");
    t.advance(b"\x1b]1337;MultipartFile=inline=1\x07\x1b]1337;FilePart=");
    t.advance(&vec![b'A'; 1024 * 1024]);
    t.advance(b"\x07\x1b]1337;FileEnd\x07restored");
    assert!(t.snapshot().images.is_empty());
    assert_eq!(t.snapshot().lines[0], "restored");
}

#[test]
fn osc1337_multipart_cancel_and_restart_discards_previous_chunks() {
    let mut t = terminal();
    t.advance(b"\x1b]1337;MultipartFile=inline=1\x07");
    t.advance(b"\x1b]1337;FilePart=AAAA\x18");
    let bytes = png();
    let encoded = STANDARD.encode(&bytes);
    t.advance(
        format!(
            "\x1b]1337;MultipartFile=inline=1;size={}\x07\
             \x1b]1337;FilePart={encoded}\x07\
             \x1b]1337;FileEnd\x07",
            bytes.len()
        )
        .as_bytes(),
    );
    assert_eq!(t.snapshot().images.len(), 1);
}

#[test]
fn graphics_follow_history_and_alternate_screen_and_clear() {
    let mut t = terminal();
    t.advance(&kitty("a=T,f=24,s=1,v=1,i=1,C=1", &[255, 0, 0]));
    t.advance(b"\r\n\r\n\r\n\r\n\r\n\r\n");
    assert!(t.snapshot().images.is_empty());
    t.scroll_display(1);
    assert_eq!(t.snapshot().images[0].row, 0);
    t.scroll_display(-1);
    t.advance(b"\x1b[?1049h");
    t.advance(&kitty("a=T,f=24,s=1,v=1,i=2,C=1", &[0, 255, 0]));
    assert_eq!(t.snapshot().images.len(), 1);
    t.advance(b"\x1b[?1049l");
    t.scroll_display(1);
    assert_eq!(&*t.snapshot().images[0].rgba, &[255, 0, 0, 255]);
    t.advance(b"\x1b[3J");
    assert!(t.snapshot().images.is_empty());
    t.advance(&kitty("a=T,f=24,s=1,v=1,i=3,C=1", &[0, 0, 255]));
    t.advance(b"\x1b[2J");
    assert!(t.snapshot().images.is_empty());
}

#[test]
fn region_scroll_preserves_images_outside_the_region() {
    let mut t = terminal();
    t.advance(b"\x1b[6;1H");
    t.advance(&kitty("a=T,f=24,s=1,v=1,C=1", &[255, 0, 0]));
    t.advance(b"\x1b[2;4r\x1b[4;1H\n");
    assert_eq!(t.snapshot().images[0].row, 5);
}

#[test]
fn malformed_unsupported_cancelled_and_oversized_data_recovers() {
    let mut t = terminal();
    for bytes in [
        kitty("a=T,f=32,s=999999,v=999999,i=1", &[0]),
        kitty("a=T,f=32,s=1,v=1,i=1", &[0]),
        kitty("a=T,t=f,i=1", b"/etc/passwd"),
        b"\x1bPq!99999999999999999999999~\x1b\\".to_vec(),
        b"\x1bPq#999999~\x1b\\".to_vec(),
        b"\x1b_Ga=T;unfinished\x18".to_vec(),
    ] {
        t.advance(&bytes);
        assert!(t.snapshot().images.is_empty());
    }
    t.advance(b"\x1b_Ga=T;");
    for _ in 0..513 {
        t.advance(&vec![b'A'; 65536]);
    }
    t.advance(b"\x1b\\restored");
    assert_eq!(t.snapshot().lines[0], "restored");
    assert!(t.snapshot().images.is_empty());
}

#[test]
fn regular_osc_utf8_and_graphics_can_share_a_read() {
    let mut t = terminal();
    let mut bytes = "日本語\x1b]0;title\x07".as_bytes().to_vec();
    bytes.extend(kitty("a=T,f=24,s=1,v=1,C=1", &[0, 0, 0]));
    t.advance(&bytes);
    assert_eq!(t.snapshot().images[0].column, 6);
    assert_eq!(t.snapshot().lines[0], "日本語");
    assert!(
        t.drain_events()
            .contains(&TerminalEvent::TitleChanged("title".into()))
    );
}

#[test]
fn capability_size_and_query_replies_keep_wire_order() {
    let mut t = terminal();
    t.set_cell_size(9, 18);
    t.advance(b"\x1b[c\x1b[14t\x1b[18t");
    t.advance(&kitty("a=q,i=5,f=24,s=1,v=1", &[1, 2, 3]));
    t.advance(b"\x1b[6n");
    assert_eq!(
        t.drain_events(),
        [
            "\x1b[?62;4c",
            "\x1b[4;108;180t",
            "\x1b[8;6;20t",
            "\x1b_Gi=5;OK\x1b\\",
            "\x1b[1;1R"
        ]
        .map(|s| TerminalEvent::PtyWrite(s.into()))
    );
}

#[test]
fn iterm_cell_size_query_reports_logical_height_width_and_scale() {
    let mut t = terminal();
    t.set_cell_size(9, 18);
    t.set_cell_scale_factor(1.5);
    t.advance(b"before\x1b]1337;ReportCellSize\x07after");
    assert_eq!(t.snapshot().lines[0], "beforeafter");
    assert_eq!(
        t.drain_events(),
        [TerminalEvent::PtyWrite(
            "\x1b]1337;ReportCellSize=12.00;6.00;1.50\x1b\\".into()
        )]
    );
}

#[test]
fn cancelled_chunks_do_not_contaminate_a_new_image() {
    let mut t = terminal();
    t.advance(b"\x1b_Ga=T,f=24,s=2,v=1,m=1;AAAA\x1b\\\x1b_Gm=0;broken\x18");
    t.advance(&kitty("a=T,f=24,s=1,v=1,C=1", &[255, 0, 0]));
    assert_eq!(&*t.snapshot().images[0].rgba, &[255, 0, 0, 255]);
}

#[test]
fn compressed_pixels_resize_and_reset() {
    use std::io::Write;
    let mut t = terminal();
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&[255, 0, 0, 128]).unwrap();
    t.advance(&kitty(
        "a=T,f=32,s=1,v=1,i=8,o=z,C=1",
        &encoder.finish().unwrap(),
    ));
    assert_eq!(&*t.snapshot().images[0].rgba, &[255, 0, 0, 128]);
    t.resize(30, 10);
    assert!(t.snapshot().images.is_empty());
    t.advance(b"\x1b_Ga=p,i=8,C=1;\x1b\\");
    assert_eq!(t.snapshot().images.len(), 1);
    t.advance(b"\x1bc\x1b_Ga=p,i=8;\x1b\\");
    assert!(t.snapshot().images.is_empty());
}

#[test]
fn sixel_preserves_the_final_partial_band() {
    let mut t = terminal();
    t.advance(b"\x1bPq\"1;1;1;2#1;2;100;0;0B\x1b\\");
    assert_eq!(t.snapshot().images[0].height, 2);
}

#[test]
fn sixel_accepts_aspect_only_raster_attributes_and_full_implicit_bands() {
    let mut t = terminal();
    t.advance(b"\x1bPq\"1;1#1;2;100;0;0@\x1b\\");
    let image = t.snapshot().images.remove(0);
    assert_eq!((image.width, image.height), (1, 6));
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
}

#[test]
fn sixel_opaque_background_uses_the_terminal_background() {
    let mut t = terminal();
    t.set_default_colors(
        [220, 225, 232],
        [12, 34, 56],
        [245, 247, 250],
        [55, 88, 145],
        [[0, 0, 0]; 16],
    );
    t.advance(b"\x1bPq\"1;1;1;2#1;2;100;0;0@\x1b\\");
    let image = t.snapshot().images.remove(0);
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(&image.rgba[4..8], &[12, 34, 56, 255]);
}

#[test]
fn eight_bit_dcs_and_st_display_sixel() {
    let mut t = terminal();
    t.advance(b"\x90q\"1;1;1;1#1;2;100;0;0@\x9c");
    assert_eq!(t.snapshot().images.len(), 1);
    assert_eq!(&*t.snapshot().images[0].rgba, &[255, 0, 0, 255]);
}

#[test]
fn eight_bit_apc_and_st_display_kitty_images_without_corrupting_utf8() {
    let mut t = terminal();
    t.advance("日本語".as_bytes());
    let encoded = STANDARD.encode([255, 0, 0]);
    let mut sequence = vec![0x9f];
    sequence.extend_from_slice(format!("Ga=T,f=24,s=1,v=1,C=1;{encoded}").as_bytes());
    sequence.push(0x9c);
    t.advance(&sequence);
    assert_eq!(t.snapshot().lines[0], "日本語");
    assert_eq!(&*t.snapshot().images[0].rgba, &[255, 0, 0, 255]);
}

#[test]
fn kitty_zero_placement_ids_do_not_replace_each_other() {
    let mut t = terminal();
    t.advance(&kitty("a=t,f=24,s=1,v=1,i=7", &[255, 0, 0]));
    t.advance(b"\x1b_Ga=p,i=7,C=1;\x1b\\\x1b[2;1H\x1b_Ga=p,i=7,C=1;\x1b\\");
    let images = t.snapshot().images;
    assert_eq!(images.len(), 2);
    assert_eq!((images[0].row, images[1].row), (0, 1));
}

#[test]
fn kitty_retransmission_replaces_data_and_all_old_placements() {
    let mut t = terminal();
    t.advance(&kitty("a=T,f=24,s=1,v=1,i=7,p=1,C=1", &[255, 0, 0]));
    t.advance(b"\x1b[2;1H\x1b_Ga=p,i=7,p=2,C=1;\x1b\\");
    t.advance(&kitty("a=t,f=24,s=1,v=1,i=7", &[0, 255, 0]));
    assert!(t.snapshot().images.is_empty());
    t.advance(b"\x1b_Ga=p,i=7,C=1;\x1b\\");
    assert_eq!(&*t.snapshot().images[0].rgba, &[0, 255, 0, 255]);
}

#[test]
fn kitty_delete_during_chunking_aborts_transfer_and_preserves_unrelated_storage() {
    let mut t = terminal();
    t.advance(&kitty("a=t,f=24,s=1,v=1,i=8", &[0, 255, 0]));
    t.advance(b"\x1b_Ga=T,f=24,s=1,v=1,i=7,m=1;/wAA\x1b\\");
    t.advance(b"\x1b_Ga=d,d=A;\x1b\\");
    t.advance(b"\x1b_Gm=0;/w==\x1b\\");
    assert!(t.snapshot().images.is_empty());
    t.advance(b"\x1b_Ga=p,i=8,C=1;\x1b\\");
    assert_eq!(&*t.snapshot().images[0].rgba, &[0, 255, 0, 255]);
}

#[test]
fn kitty_image_id_deletion_applies_across_screen_buffers() {
    let mut t = terminal();
    t.advance(&kitty("a=T,f=24,s=1,v=1,i=7,C=1", &[255, 0, 0]));
    t.advance(b"\x1b[?1049h\x1b_Ga=d,d=i,i=7;\x1b\\\x1b[?1049l");
    assert!(t.snapshot().images.is_empty());
}

#[test]
fn images_observe_synchronized_text_cursor() {
    let mut t = terminal();
    t.advance(b"\x1b[?2026habc");
    t.advance(&kitty("a=T,f=24,s=1,v=1,C=1", &[0, 0, 0]));
    t.advance(b"\x1b[?2026l");
    assert_eq!(t.snapshot().images[0].column, 3);
}

#[test]
fn shrinking_a_wide_image_never_rounds_its_height_to_zero() {
    let mut t = terminal();
    t.advance(&kitty("a=T,f=24,s=2,v=1,c=1,C=1", &[255, 0, 0, 255, 0, 0]));
    let image = t.snapshot().images.remove(0);
    assert_eq!(
        (image.display_width, image.display_height, image.rows),
        (1, 1, 1)
    );
    t.advance(
        format!(
            "\x1b]1337;File=inline=1;width=1px:{}\x07",
            STANDARD.encode(png())
        )
        .as_bytes(),
    );
    let image = t.snapshot().images.pop().unwrap();
    assert_eq!(
        (image.display_width, image.display_height, image.rows),
        (1, 1, 1)
    );
}
