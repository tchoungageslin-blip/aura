//! aura-runtime — freestanding runtime linked into every Aura binary.
//!
//! Phase 3 will add `#[no_mangle] extern "C"` exports:
//! `aura_alloc`, `aura_dealloc`, `aura_incref`, `aura_decref`,
//! `aura_print_i64`, `aura_print_f64`, `aura_print_bool`, `aura_print_str`.
//! All exports use fixed signatures — no C varargs (Cranelift cannot emit them).
#![allow(unsafe_code)]
