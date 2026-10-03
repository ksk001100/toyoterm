#![no_main]
use libfuzzer_sys::fuzz_target;
use toyoterm_fuzz::{Target, run};

fuzz_target!(|data: &[u8]| run(Target::Osc, data));
