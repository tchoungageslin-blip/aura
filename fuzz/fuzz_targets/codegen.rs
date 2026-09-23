//! Fuzz target: MIR → CLIF → COFF emission — never panic. Err on unsupported
//! constructs is fine; that's what E3004 is for.
#![no_main]

use aura_common::SourceCache;
use aura_salsa_db::{AuraDatabase, SourceFile};
use aura_codegen::compile_file;
use aura_semantic::check_file;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    let mut cache = SourceCache::new();
    let file_id = cache.add("fuzz.aura", src.to_string());
    let db = AuraDatabase::new();
    let file = SourceFile::new(&db, src.into_owned(), file_id);
    if check_file(&db, file).iter().any(|d| d.is_error()) {
        return;
    }
    let out = compile_file(&db, file);
    // Invariant: a successful compile yields a non-empty COFF object.
    if let Some(obj) = &out.object {
        assert!(!obj.is_empty());
    }
});
