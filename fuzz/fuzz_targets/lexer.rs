//! Fuzz target: the lexer must never panic and must always terminate.
#![no_main]

use aura_common::SourceCache;
use aura_lexer::lex;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    let mut cache = SourceCache::new();
    let file = cache.add("fuzz.aura", src.to_string());
    let out = lex(&src, file);
    // Invariant: tokens are ordered, within bounds, and ends with Eof.
    let mut prev = 0u32;
    for t in &out.tokens {
        assert!(t.span.start <= t.span.end, "inverted span {t:?}");
        assert!(t.span.end as usize <= src.len(), "span past EOF {t:?}");
        assert!(t.span.start >= prev, "tokens out of order {t:?}");
        prev = t.span.start;
    }
});
