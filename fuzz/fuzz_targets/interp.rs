//! Fuzz target: reference interpreter — never panic, never hang (fuel).
#![no_main]

use aura_interp::run_source;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    // Any result is fine — Diagnostics or InterpError are expected on garbage;
    // a panic or an unbounded loop is not.
    let _ = run_source(&src);
});
