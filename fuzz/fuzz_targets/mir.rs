//! Fuzz target: MIR lowering of every fn — never panic.
#![no_main]

use aura_common::SourceCache;
use aura_salsa_db::{AuraDatabase, SourceFile, file_items};
use aura_mir::mir_fn;
use aura_semantic::check_file;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    let mut cache = SourceCache::new();
    let file_id = cache.add("fuzz.aura", src.to_string());
    let db = AuraDatabase::new();
    let file = SourceFile::new(&db, src.into_owned(), file_id);
    // Only lower when semantics are clean — MIR assumes typeck passed.
    if check_file(&db, file).iter().any(|d| d.is_error()) {
        return;
    }
    for (i, _) in file_items(&db, file).iter() {
        let _ = mir_fn(&db, file, i);
    }
});
