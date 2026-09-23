//! Fuzz target: the parser must never panic and must always terminate.
#![no_main]

use aura_common::SourceCache;
use aura_parser::parse_file;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    let mut cache = SourceCache::new();
    let file = cache.add("fuzz.aura", src.to_string());
    let parsed = parse_file(&src, file);
    // Invariant: every item span is within bounds.
    for item in &parsed.items {
        let s = item.span();
        assert!(s.start <= s.end && s.end as usize <= src.len());
    }
});
