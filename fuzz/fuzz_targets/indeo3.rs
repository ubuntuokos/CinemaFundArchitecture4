#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| cfa4_fuzz::indeo3(data));
