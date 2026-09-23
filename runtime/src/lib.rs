//! aura-runtime — freestanding runtime linked into every Aura binary.
//!
//! `#![no_std]` + raw-dylib imports keep the runtime fully self-contained:
//! `lld-link` can produce a working `.exe` from `hello.obj +
//! aura_runtime.lib` with `/nodefaultlib` — no Windows SDK or CRT import
//! libraries required.
//!
//! Exports:
//! - `mainCRTStartup` — PE entry point (console subsystem): calls the
//!   user program's `main` and forwards its return code to `ExitProcess`.
//! - `aura_rt_alloc`/`aura_rt_free` — process-heap allocation for future
//!   ARC-managed objects.
//! - `aura_rt_retain`/`aura_rt_release` — ARC refcount ops (header layout:
//!   `[refcount: u64][payload..]`, `retain`/`release` adjust the count,
//!   `release` frees on zero).
//! - `aura_rt_print`/`aura_rt_println`/`aura_rt_eprint`/`aura_rt_eprintln` —
//!   stdout/stderr writes behind the `print`-family prelude builtins.
//! - `aura_rt_exit` — `ExitProcess` behind the `exit` builtin.
//! - `aura_str_eq` — byte equality behind `str ==`/`!=`.
//!
//! `unsafe_code` is allowed here — the FFI boundary is this crate's whole
//! reason to exist. Workspace lint denial still applies everywhere else.

#![no_std]
#![allow(unsafe_code)]

use core::ffi::c_void;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU64, Ordering};

// `main` as defined by the compiled Aura program (`fn main() -> i64`).
// Any `fn main` returning an integer ABI-conforms: the low 32 bits of
// the return value become the process exit code.
#[cfg(target_os = "windows")]
unsafe extern "C" {
    fn main() -> i64;
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "C" {
    fn ExitProcess(code: u32) -> !;
    fn GetProcessHeap() -> *mut c_void;
    fn HeapAlloc(heap: *mut c_void, flags: u32, bytes: usize) -> *mut c_void;
    fn HeapReAlloc(heap: *mut c_void, flags: u32, ptr: *mut c_void, bytes: usize)
        -> *mut c_void;
    fn HeapFree(heap: *mut c_void, flags: u32, ptr: *mut c_void) -> i32;
    fn GetStdHandle(which: i32) -> *mut c_void;
    fn WriteFile(
        handle: *mut c_void,
        buf: *const c_void,
        len: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
}

/// PE console entry point. The loader starts here; we delegate to the
/// Aura `main` and terminate with its exit code.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    unsafe { ExitProcess(u32::try_from(main()).unwrap_or(u32::MAX)) }
}

/// `panic = "abort"` strategy needs no personality routine — a panic
/// straight to `ExitProcess` with the conventional abort code.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    #[cfg(target_os = "windows")]
    unsafe {
        ExitProcess(101)
    }
    #[cfg(not(target_os = "windows"))]
    loop {}
}

// Some objects emit an `INCLUDE _fltused` directive when floats are used;
// with `/nodefaultlib` no CRT provides it, so we do.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub static _fltused: u32 = 0x9875;

// ----- libc surface ---------------------------------------------------------------
//
// `core`'s precompiled objects reference the C memory intrinsics and the
// SEH personality (`__CxxFrameHandler3` in `.xdata`). On MSVC targets
// compiler-builtins deliberately omits them — the CRT normally supplies
// them. `/nodefaultlib` links find them here instead.

/// C `memcpy` — forward byte copy.
///
/// # Safety
/// Regions must not overlap and `n` bytes must be valid at both ends.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(
    dst: *mut c_void,
    src: *const c_void,
    n: usize,
) -> *mut c_void {
    unsafe {
        let (d, s) = (dst.cast::<u8>(), src.cast::<u8>());
        let mut i = 0;
        while i < n {
            *d.add(i) = *s.add(i);
            i += 1;
        }
    }
    dst
}

/// C `memmove` — overlap-safe copy via direction selection.
///
/// # Safety
/// `n` bytes must be valid at both `dst` and `src`.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(
    dst: *mut c_void,
    src: *const c_void,
    n: usize,
) -> *mut c_void {
    unsafe {
        let (d, s) = (dst.cast::<u8>(), src.cast::<u8>());
        if (d as usize) <= (s as usize) {
            let mut i = 0;
            while i < n {
                *d.add(i) = *s.add(i);
                i += 1;
            }
        } else {
            let mut i = n;
            while i > 0 {
                i -= 1;
                *d.add(i) = *s.add(i);
            }
        }
    }
    dst
}

/// C `memset` — fill `n` bytes with `v`.
///
/// # Safety
/// `dst` must be valid for `n` bytes.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dst: *mut c_void, v: i32, n: usize) -> *mut c_void {
    unsafe {
        let d = dst.cast::<u8>();
        let b = v as u8;
        let mut i = 0;
        while i < n {
            *d.add(i) = b;
            i += 1;
        }
    }
    dst
}

/// C `memcmp` — lexicographic byte compare.
///
/// # Safety
/// Both pointers must be valid for `n` bytes.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    unsafe {
        let (x, y) = (a.cast::<u8>(), b.cast::<u8>());
        let mut i = 0;
        while i < n {
            let (u, v) = (*x.add(i), *y.add(i));
            if u != v {
                return i32::from(u) - i32::from(v);
            }
            i += 1;
        }
    }
    0
}

/// SEH personality referenced by `core`'s unwind tables. Unreachable —
/// the runtime is compiled `panic = "abort"`, so no Rust frame ever
/// unwinds. If a foreign unwind ever walks here, abort the process.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub extern "C" fn __CxxFrameHandler3() -> ! {
    unsafe { ExitProcess(0xC000_0409) } // STACK_BUFFER_OVERRUN / fast-fail
}

// ----- C-math surface -------------------------------------------------------------
//
// `/nodefaultlib` means no msvcrt — extern "C" math functions Aura code
// declares resolve here instead. Implemented on hardware instructions;
// soft-fp fallbacks land with the other targets.

/// `sqrt` for `extern "C" { fn sqrt(f64) -> f64 }`.
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[unsafe(no_mangle)]
pub extern "C" fn sqrt(x: f64) -> f64 {
    use core::arch::x86_64::{_mm_cvtsd_f64, _mm_set_sd, _mm_sqrt_sd};
    // SAFETY: SSE2 is baseline on x86_64; the intrinsics are pure.
    unsafe { _mm_cvtsd_f64(_mm_sqrt_sd(_mm_set_sd(x), _mm_set_sd(x))) }
}

// ----- heap -------------------------------------------------------------------

/// Allocate `bytes` from the process heap. Returns null on failure
/// (callers are generated code; the compiler inserts OOM handling).
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub extern "C" fn aura_rt_alloc(bytes: usize) -> *mut u8 {
    unsafe { HeapAlloc(GetProcessHeap(), 0, bytes).cast() }
}

/// Free a pointer from `aura_rt_alloc`. Null is a no-op.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_rt_free(ptr: *mut u8) {
    if !ptr.is_null() {
        unsafe {
            HeapFree(GetProcessHeap(), 0, ptr.cast());
        }
    }
}

// ----- ARC ----------------------------------------------------------------------
//
// Managed object header: `[refcount: AtomicU64][payload …]`.
// `aura_rt_retain` increments, `aura_rt_release` decrements and frees at
// zero. Week-1 scope: strong refs only — the cycle detector lands later.

/// Bump the refcount of an ARC-managed pointer (payload address).
///
/// # Safety
/// `ptr` must be a live pointer returned by the ARC allocator, or null.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_rt_retain(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let rc = ptr.cast::<AtomicU64>().sub(1);
        (*rc).fetch_add(1, Ordering::Relaxed);
    }
}

/// Drop one ref; frees the allocation (header included) at zero.
///
/// # Safety
/// `ptr` must be a live pointer returned by the ARC allocator, or null.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_rt_release(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let rc = ptr.cast::<AtomicU64>().sub(1);
        if (*rc).fetch_sub(1, Ordering::Release) == 1 {
            core::sync::atomic::fence(Ordering::Acquire);
            aura_rt_free(rc.cast());
        }
    }
}

// ----- io -----------------------------------------------------------------------
//
// `aura_rt_print`-family — the codegen target of the `print`/`eprint`
// prelude builtins. `str` arguments arrive already flattened to
// `{ptr, len}` by the caller, so these stay scalar-only C ABI.

#[cfg(target_os = "windows")]
const STD_OUTPUT_HANDLE: i32 = -11;
#[cfg(target_os = "windows")]
const STD_ERROR_HANDLE: i32 = -12;

/// Write `len` bytes at `ptr` to `which` std handle; failures are
/// swallowed (console closed, redirected to a dead pipe — nothing a
/// freestanding runtime can report to anyway).
///
/// # Safety
/// `ptr` must be valid for `len` bytes (or null when `len == 0`).
#[cfg(target_os = "windows")]
unsafe fn write_all(which: i32, ptr: *const u8, len: usize) {
    if len == 0 || ptr.is_null() {
        return;
    }
    let handle = unsafe { GetStdHandle(which) };
    if handle.is_null() || handle as usize == usize::MAX {
        return; // no console — detached process (INVALID_HANDLE_VALUE)
    }
    // WriteFile caps `len` at u32 — chunk oversized writes.
    let mut off = 0usize;
    while off < len {
        let chunk = u32::try_from(len - off).unwrap_or(u32::MAX);
        let mut written = 0u32;
        unsafe {
            WriteFile(
                handle,
                ptr.add(off).cast(),
                chunk,
                &mut written,
                core::ptr::null_mut(),
            );
        }
        if written == 0 {
            break;
        }
        off += written as usize;
    }
}

/// `aura_rt_print(ptr, len)` — backend of `print(s: str)`.
///
/// # Safety
/// `ptr` must be valid for `len` bytes.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_rt_print(ptr: *const u8, len: usize) {
    unsafe { write_all(STD_OUTPUT_HANDLE, ptr, len) }
}

/// `aura_rt_println(ptr, len)` — backend of `println(s: str)`.
///
/// # Safety
/// `ptr` must be valid for `len` bytes.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_rt_println(ptr: *const u8, len: usize) {
    unsafe {
        write_all(STD_OUTPUT_HANDLE, ptr, len);
        write_all(STD_OUTPUT_HANDLE, b"\n".as_ptr(), 1);
    }
}

/// `aura_rt_eprint(ptr, len)` — backend of `eprint(s: str)`.
///
/// # Safety
/// `ptr` must be valid for `len` bytes.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_rt_eprint(ptr: *const u8, len: usize) {
    unsafe { write_all(STD_ERROR_HANDLE, ptr, len) }
}

/// `aura_rt_eprintln(ptr, len)` — backend of `eprintln(s: str)`.
///
/// # Safety
/// `ptr` must be valid for `len` bytes.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_rt_eprintln(ptr: *const u8, len: usize) {
    unsafe {
        write_all(STD_ERROR_HANDLE, ptr, len);
        write_all(STD_ERROR_HANDLE, b"\n".as_ptr(), 1);
    }
}

/// `aura_rt_exit(code)` — backend of `exit(code)`. Never returns.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub extern "C" fn aura_rt_exit(code: i64) -> ! {
    unsafe { ExitProcess(u32::try_from(code).unwrap_or(u32::MAX)) }
}

// ----- str ----------------------------------------------------------------------

/// Byte equality of two `str` payloads — the backend of `str ==`.
/// Lengths are compared first, then contents via `memcmp`.
///
/// # Safety
/// `l`/`r` must be valid for `l_len`/`r_len` bytes (or null when the
/// length is zero).
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_str_eq(
    l: *const u8,
    l_len: usize,
    r: *const u8,
    r_len: usize,
) -> i32 {
    if l_len != r_len {
        return 0;
    }
    if l_len == 0 {
        return 1;
    }
    i32::from(unsafe { memcmp(l.cast(), r.cast(), l_len) } == 0)
}

/// Concatenate two `str` payloads into a fresh process-heap buffer —
/// the backend of `str + str`. Returns the new buffer; the caller
/// computes the result length as `l_len + r_len`.
///
/// # Safety
/// `l`/`r` must be valid for `l_len`/`r_len` bytes (or null when the
/// length is zero). The returned pointer is `aura_rt_alloc`-owned.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_str_concat(
    l: *const u8,
    l_len: usize,
    r: *const u8,
    r_len: usize,
) -> *mut u8 {
    let total = l_len.wrapping_add(r_len);
    let buf = aura_rt_alloc(total);
    if buf.is_null() {
        return buf;
    }
    if l_len != 0 {
        unsafe { memcpy(buf.cast(), l.cast(), l_len) };
    }
    if r_len != 0 {
        unsafe { memcpy(buf.add(l_len).cast(), r.cast(), r_len) };
    }
    buf
}

// ----- vec ---------------------------------------------------------------------

/// Append `elem` (`esize` bytes at `elem`) to a `vec`'s buffer,
/// growing it when `len == cap` (`cap` doubles from a floor of 4).
/// Returns the — possibly reallocated — data pointer and writes the
/// new capacity through `cap_out`; the caller stores both back and
/// bumps `len`.
///
/// # Safety
/// `elem` must be valid for `esize` bytes; `data` must be null or a
/// process-heap buffer of `cap * esize` bytes; `cap_out` must be
/// writable for a `usize`.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_vec_push(
    data: *mut u8,
    len: usize,
    cap: usize,
    elem: *const u8,
    esize: usize,
    cap_out: *mut usize,
) -> *mut u8 {
    let heap = unsafe { GetProcessHeap() };
    let mut data = data;
    let mut cap = cap;
    if len >= cap {
        cap = if cap == 0 { 4 } else { cap.saturating_mul(2) };
        let bytes = cap.saturating_mul(esize);
        data = if data.is_null() {
            unsafe { HeapAlloc(heap, 0, bytes).cast() }
        } else {
            unsafe { HeapReAlloc(heap, 0, data.cast(), bytes).cast() }
        };
        if data.is_null() {
            // OOM — no diagnostic channel; fail the process.
            unsafe { ExitProcess(14) };
        }
    }
    if esize != 0 {
        unsafe { memcpy(data.add(len * esize).cast(), elem.cast(), esize) };
    }
    unsafe { *cap_out = cap };
    data
}

/// Bounds-checked element address: `data + idx * esize`, or exit 101
/// (Rust's panic code) when `idx >= len`. `vec_get`/`vec_set` share it.
///
/// # Safety
/// `data` must be a buffer of at least `len * esize` bytes.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_vec_get(
    data: *mut u8,
    len: usize,
    idx: usize,
    esize: usize,
) -> *mut u8 {
    if idx >= len {
        unsafe { ExitProcess(101) };
    }
    unsafe { data.add(idx * esize) }
}
