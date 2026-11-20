#![no_main]

use cargo_soro_mutants::filters::fuzz_parse_all_filters;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        fuzz_parse_all_filters(s);
    }
});
