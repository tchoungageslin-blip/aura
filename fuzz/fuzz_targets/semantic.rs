//! Fuzz target: full semantic pipeline (parse→resolve→typeck) — never panic.
#![no_main]

use aura_common::SourceCache;
use aura_salsa_db::{AuraDatabase, SourceFile};
use aura_semantic::check_file;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    let mut cache = SourceCache::new();
    let file_id = cache.add("fuzz.aura", src.to_string());
    let db = AuraDatabase::new();
    let file = SourceFile::new(&db, src.into_owned(), file_id);
    // Diagnostics are expected on garbage input; a panic is not.
    let _ = check_file(&db, file);
});
