#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| cfa4_fuzz::dng(data));
