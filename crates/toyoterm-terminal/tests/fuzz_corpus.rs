// Replay the exact fuzz harness without requiring nightly or libFuzzer.
#[path = "../../../fuzz/src/lib.rs"]
mod harness;

#[test]
fn committed_fuzz_corpus_survives_public_state_inspection() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus");
    for (name, target) in [
        ("raw_terminal", harness::Target::RawTerminal),
        ("osc", harness::Target::Osc),
        ("kitty_graphics", harness::Target::KittyGraphics),
        ("sixel", harness::Target::Sixel),
        ("iterm_image", harness::Target::ItermImage),
        ("kitty_file_transfer", harness::Target::KittyFileTransfer),
    ] {
        let mut paths: Vec<_> = std::fs::read_dir(corpus.join(name))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        assert!(!paths.is_empty(), "missing seeds for {name}");
        for path in paths {
            eprintln!("replay {}", path.display());
            harness::run(target, &std::fs::read(path).unwrap());
        }
    }
}
