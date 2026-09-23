//! Fuzz target: formatting must be panic-free, idempotent, and produce
//! parse-clean output.
#![no_main]

use aura_common::SourceCache;
use aura_fmt::format_source;
use aura_parser::parse_file;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    let Ok(formatted) = format_source(&src) else {
        return; // unparseable input — fmt correctly refused
    };
    // Output must re-parse with zero diagnostics.
    let mut cache = SourceCache::new();
    let file = cache.add("fuzz.aura", formatted.clone());
    let reparsed = parse_file(&formatted, file);
    assert!(
        reparsed.diagnostics.is_empty(),
        "fmt produced unparseable output:\n{formatted}"
    );
    // Idempotent: fmt(fmt(x)) == fmt(x).
    let twice = format_source(&formatted).expect("formatted output must re-format");
    assert_eq!(formatted, twice, "fmt not idempotent");
});
