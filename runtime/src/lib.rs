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
    fn GetCommandLineW() -> *const u16;
    fn LocalFree(ptr: *mut c_void) -> *mut c_void;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn WideCharToMultiByte(
        cp: u32,
        flags: u32,
        wide: *const u16,
        wide_len: i32,
        multi: *mut u8,
        multi_len: i32,
        default_char: *const u8,
        used_default: *mut i32,
    ) -> i32;
    fn MultiByteToWideChar(
        cp: u32,
        flags: u32,
        multi: *const u8,
        multi_len: i32,
        wide: *mut u16,
        wide_len: i32,
    ) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        creation: u32,
        flags: u32,
        template: *mut c_void,
    ) -> *mut c_void;
    fn GetFileSizeEx(handle: *mut c_void, size: *mut i64) -> i32;
    fn ReadFile(
        handle: *mut c_void,
        buf: *mut c_void,
        len: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn CreateProcessW(
        app: *const u16,
        cmdline: *mut u16,
        proc_attr: *mut c_void,
        thread_attr: *mut c_void,
        inherit: i32,
        flags: u32,
        env: *mut c_void,
        cwd: *const u16,
        si: *mut StartupInfoW,
        pi: *mut ProcessInfo,
    ) -> i32;
    fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
    fn GetExitCodeProcess(handle: *mut c_void, code: *mut u32) -> i32;
}

/// `STARTUPINFOW` — 104 bytes on x64; `cb` is the byte size, the std
/// handles sit at the tail.
#[cfg(target_os = "windows")]
#[repr(C)]
pub struct StartupInfoW {
    cb: u32,
    _pad0: u32,
    reserved: *const u16,
    desktop: *const u16,
    title: *const u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count: u32,
    y_count: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved2: u16,
    _pad1: u32,
    reserved3: *mut c_void,
    std_in: *mut c_void,
    std_out: *mut c_void,
    std_err: *mut c_void,
}

/// `PROCESS_INFORMATION` — `{hProcess, hThread, pid, tid}`.
#[cfg(target_os = "windows")]
#[repr(C)]
pub struct ProcessInfo {
    process: *mut c_void,
    thread: *mut c_void,
    pid: u32,
    tid: u32,
}

#[cfg(target_os = "windows")]
#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "C" {
    fn CommandLineToArgvW(cmd: *const u16, argc: *mut i32) -> *mut *mut u16;
}

/// UTF-8 code page for the `*ToMultiByte`/`MultiByteTo*` conversions.
#[cfg(target_os = "windows")]
const CP_UTF8: u32 = 65001;

/// Aura's `str` as the runtime sees it — `{ptr, len}`.
#[cfg(target_os = "windows")]
#[repr(C)]
pub struct RawStr {
    ptr: *mut u8,
    len: usize,
}

/// Aura's `vec<str>` as the runtime sees it — `{ptr, len, cap}` where
/// `ptr` addresses a flat `RawStr` array.
#[cfg(target_os = "windows")]
#[repr(C)]
pub struct RawVec {
    ptr: *mut u8,
    len: usize,
    cap: usize,
}

/// NUL-terminated UTF-16 length (no `lstrlenW` import needed).
#[cfg(target_os = "windows")]
unsafe fn wstrlen(s: *const u16) -> usize {
    let mut n = 0;
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

/// UTF-16 → freshly allocated UTF-8 buffer; `(ptr, len)` — `ptr` is
/// `aura_rt_alloc`-owned. Empty/NULL input yields `{null, 0}`.
#[cfg(target_os = "windows")]
unsafe fn wstr_to_utf8(w: *const u16, wlen: usize) -> RawStr {
    if w.is_null() || wlen == 0 {
        return RawStr {
            ptr: core::ptr::null_mut(),
            len: 0,
        };
    }
    let need = unsafe {
        WideCharToMultiByte(
            CP_UTF8,
            0,
            w,
            i32::try_from(wlen).unwrap_or(i32::MAX),
            core::ptr::null_mut(),
            0,
            core::ptr::null(),
            core::ptr::null_mut(),
        )
    };
    if need <= 0 {
        return RawStr {
            ptr: core::ptr::null_mut(),
            len: 0,
        };
    }
    let buf = aura_rt_alloc(usize::try_from(need).unwrap_or(usize::MAX));
    if buf.is_null() {
        return RawStr {
            ptr: core::ptr::null_mut(),
            len: 0,
        };
    }
    let wrote = unsafe {
        WideCharToMultiByte(
            CP_UTF8,
            0,
            w,
            i32::try_from(wlen).unwrap_or(i32::MAX),
            buf,
            need,
            core::ptr::null(),
            core::ptr::null_mut(),
        )
    };
    RawStr {
        ptr: buf,
        len: usize::try_from(wrote).unwrap_or(0),
    }
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

/// Bounds-checked byte load: `str_get(s, i)` — exit 101 when
/// `idx >= len` (same trap as `aura_vec_get`).
///
/// # Safety
/// `ptr` must be valid for `len` bytes (or null when `len` is zero —
/// the bounds check fires before any dereference).
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_str_get(ptr: *const u8, len: usize, idx: usize) -> u8 {
    if idx >= len {
        unsafe { ExitProcess(101) };
    }
    unsafe { *ptr.add(idx) }
}

/// Bounds-checked substring view: `str_slice(s, lo, hi)` writes
/// `{ptr + lo, hi - lo}` at `out` — no copy, the slice borrows the
/// source buffer (Aura's heap never frees). Exits 101 when
/// `lo > hi` or `hi > len`.
///
/// # Safety
/// `ptr` must be valid for `len` bytes; `out` must be writable for a
/// `RawStr`.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_str_slice(
    ptr: *const u8,
    len: usize,
    lo: usize,
    hi: usize,
    out: *mut RawStr,
) {
    if lo > hi || hi > len {
        unsafe { ExitProcess(101) };
    }
    if out.is_null() {
        return;
    }
    unsafe {
        *out = RawStr {
            ptr: ptr.add(lo).cast_mut(),
            len: hi - lo,
        }
    };
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

// ----- process / env -----------------------------------------------------------

/// `args() -> vec<str>` — writes a `{ptr, len, cap}` triple at `out`
/// whose `ptr` addresses `argc` `RawStr` elements (program name first).
/// All buffers are `aura_rt_alloc`-owned; empty argv yields a null vec.
///
/// # Safety
/// `out` must be writable for a `RawVec`.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_rt_args(out: *mut RawVec) {
    if out.is_null() {
        return;
    }
    let mut argc: i32 = 0;
    let argv = unsafe { CommandLineToArgvW(GetCommandLineW(), &mut argc) };
    let count = usize::try_from(argc).unwrap_or(0);
    if argv.is_null() || count == 0 {
        unsafe {
            *out = RawVec {
                ptr: core::ptr::null_mut(),
                len: 0,
                cap: 0,
            }
        };
        return;
    }
    let elems = aura_rt_alloc(count.saturating_mul(core::mem::size_of::<RawStr>())) as *mut RawStr;
    if elems.is_null() {
        unsafe { ExitProcess(14) };
    }
    for i in 0..count {
        let w = unsafe { *argv.add(i) };
        let s = unsafe { wstr_to_utf8(w, wstrlen(w)) };
        unsafe { *elems.add(i) = s };
    }
    unsafe { LocalFree(argv.cast()) };
    unsafe {
        *out = RawVec {
            ptr: elems.cast(),
            len: count,
            cap: count,
        }
    };
}

/// `env(name: str) -> str` — writes `{ptr, len}` at `out`; a missing
/// variable yields `{null, 0}` (reads as `""`).
///
/// # Safety
/// `name` must be valid for `name_len` bytes; `out` must be writable
/// for a `RawStr`.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_rt_env(name: *const u8, name_len: usize, out: *mut RawStr) {
    let empty = RawStr {
        ptr: core::ptr::null_mut(),
        len: 0,
    };
    if out.is_null() {
        return;
    }
    if name.is_null() || name_len == 0 {
        unsafe { *out = empty };
        return;
    }
    // UTF-8 name → NUL-terminated UTF-16.
    let wlen = unsafe {
        MultiByteToWideChar(
            CP_UTF8,
            0,
            name,
            i32::try_from(name_len).unwrap_or(i32::MAX),
            core::ptr::null_mut(),
            0,
        )
    };
    if wlen <= 0 {
        unsafe { *out = empty };
        return;
    }
    let wname = aura_rt_alloc((usize::try_from(wlen).unwrap_or(usize::MAX) + 1) * 2) as *mut u16;
    if wname.is_null() {
        unsafe { ExitProcess(14) };
    }
    unsafe {
        MultiByteToWideChar(
            CP_UTF8,
            0,
            name,
            i32::try_from(name_len).unwrap_or(i32::MAX),
            wname,
            wlen,
        );
        *wname.add(usize::try_from(wlen).unwrap_or(usize::MAX)) = 0;
    }
    // Value length in UTF-16 units, then UTF-16 → UTF-8.
    let need = unsafe { GetEnvironmentVariableW(wname, core::ptr::null_mut(), 0) };
    if need == 0 {
        unsafe { aura_rt_free(wname.cast()) };
        unsafe { *out = empty };
        return;
    }
    let wbuf = aura_rt_alloc(usize::try_from(need).unwrap_or(usize::MAX) * 2) as *mut u16;
    if wbuf.is_null() {
        unsafe { ExitProcess(14) };
    }
    let got = unsafe { GetEnvironmentVariableW(wname, wbuf, need) };
    unsafe { aura_rt_free(wname.cast()) };
    if got == 0 {
        unsafe { aura_rt_free(wbuf.cast()) };
        unsafe { *out = empty };
        return;
    }
    let s = unsafe { wstr_to_utf8(wbuf, usize::try_from(got).unwrap_or(0)) };
    unsafe { aura_rt_free(wbuf.cast()) };
    unsafe { *out = s };
}

// ----- conversions -------------------------------------------------------------

/// `str_from_int(v)` — decimal render of `v` into a fresh heap str.
/// Writes `{ptr, len}` to `out`; `out` must be writable.
///
/// # Safety
/// `out` must point to a `RawStr`.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_str_from_int(v: i64, out: *mut RawStr) {
    if out.is_null() {
        return;
    }
    let mut tmp = [0u8; 20];
    let neg = v < 0;
    let mut u = v.unsigned_abs();
    let mut n = 0usize;
    loop {
        tmp[n] = b'0' + u8::try_from(u % 10).unwrap_or(b'0');
        u /= 10;
        n += 1;
        if u == 0 {
            break;
        }
    }
    let total = n + usize::from(neg);
    let p = aura_rt_alloc(total);
    if p.is_null() {
        unsafe { ExitProcess(14) };
    }
    unsafe {
        let mut w = p;
        if neg {
            *w = b'-';
            w = w.add(1);
        }
        let mut i = 0;
        while i < n {
            *w.add(i) = tmp[n - 1 - i];
            i += 1;
        }
        *out = RawStr { ptr: p, len: total };
    }
}

/// `str_from_bool(v)` — `"true"`/`"false"` into a fresh heap str.
///
/// # Safety
/// `out` must point to a `RawStr`.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_str_from_bool(v: u8, out: *mut RawStr) {
    if out.is_null() {
        return;
    }
    let (s, n): (*const u8, usize) = if v == 0 {
        (b"false".as_ptr(), 5)
    } else {
        (b"true".as_ptr(), 4)
    };
    let p = aura_rt_alloc(n);
    if p.is_null() {
        unsafe { ExitProcess(14) };
    }
    unsafe {
        memcpy(p.cast(), s.cast(), n);
        *out = RawStr { ptr: p, len: n };
    }
}

/// `str_from_byte(v)` — a one-byte string holding `v`'s low 8 bits.
///
/// # Safety
/// `out` must point to a `RawStr`.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_str_from_byte(v: i64, out: *mut RawStr) {
    if out.is_null() {
        return;
    }
    let p = aura_rt_alloc(1);
    if p.is_null() {
        unsafe { ExitProcess(14) };
    }
    unsafe {
        *p = v as u8;
        *out = RawStr { ptr: p, len: 1 };
    }
}

// ----- file / process io ---------------------------------------------------------

/// `Result<str, str>` as the runtime writes it — `i32` tag at 0,
/// payload `RawStr` at 8 (matches `result_layout`: tag 4B aligned to
/// the 8B payload).
#[cfg(target_os = "windows")]
#[repr(C)]
pub struct RawResultStr {
    tag: i32,
    _pad: i32,
    payload: RawStr,
}

/// UTF-8 → freshly allocated NUL-terminated UTF-16 — paths and command
/// lines for the `*W` APIs. `aura_rt_alloc`-owned; null on failure.
#[cfg(target_os = "windows")]
unsafe fn utf8_to_wstr(ptr: *const u8, len: usize) -> *mut u16 {
    if len == 0 {
        let buf = aura_rt_alloc(2).cast::<u16>();
        if !buf.is_null() {
            unsafe { *buf = 0 };
        }
        return buf;
    }
    let need = unsafe {
        MultiByteToWideChar(
            CP_UTF8,
            0,
            ptr,
            i32::try_from(len).unwrap_or(i32::MAX),
            core::ptr::null_mut(),
            0,
        )
    };
    if need <= 0 {
        return core::ptr::null_mut();
    }
    let n = usize::try_from(need).unwrap_or(0);
    let buf = aura_rt_alloc((n + 1) * 2).cast::<u16>();
    if buf.is_null() {
        return buf;
    }
    unsafe {
        MultiByteToWideChar(
            CP_UTF8,
            0,
            ptr,
            i32::try_from(len).unwrap_or(i32::MAX),
            buf,
            need,
        );
        *buf.add(n) = 0;
    }
    buf
}

/// `read_file(path) -> Result<str, str>` — whole file as bytes. The
/// `Err` payload is a static message (no allocation) so callers can
/// match on it; the OS error detail isn't surfaced yet.
///
/// # Safety
/// `path` must be valid for `path_len` bytes; `out` writable for a
/// `RawResultStr`.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_read_file(
    path: *const u8,
    path_len: usize,
    out: *mut RawResultStr,
) {
    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const OPEN_EXISTING: u32 = 3;
    unsafe fn err(out: *mut RawResultStr, msg: &'static [u8]) {
        unsafe {
            *out = RawResultStr {
                tag: 1,
                _pad: 0,
                payload: RawStr {
                    ptr: msg.as_ptr().cast_mut(),
                    len: msg.len(),
                },
            }
        };
    }
    if out.is_null() {
        return;
    }
    let w = unsafe { utf8_to_wstr(path, path_len) };
    if w.is_null() {
        unsafe { err(out, b"bad path") };
        return;
    }
    let h = unsafe {
        CreateFileW(
            w,
            GENERIC_READ,
            FILE_SHARE_READ,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    unsafe { aura_rt_free(w.cast()) };
    if h.is_null() || h == (-1isize as *mut c_void) {
        unsafe { err(out, b"cannot open file") };
        return;
    }
    let mut size: i64 = 0;
    if unsafe { GetFileSizeEx(h, &mut size) } == 0 || size < 0 {
        unsafe { CloseHandle(h) };
        unsafe { err(out, b"cannot stat file") };
        return;
    }
    let total_size = usize::try_from(size).unwrap_or(usize::MAX);
    let buf = aura_rt_alloc(total_size.saturating_add(1));
    if buf.is_null() && total_size != 0 {
        unsafe { CloseHandle(h) };
        unsafe { err(out, b"out of memory") };
        return;
    }
    let mut total = 0usize;
    while total < total_size {
        let chunk = u32::try_from(total_size - total).unwrap_or(u32::MAX);
        let mut n: u32 = 0;
        let ok = unsafe {
            ReadFile(
                h,
                buf.add(total).cast(),
                chunk,
                &mut n,
                core::ptr::null_mut(),
            )
        };
        if ok == 0 || n == 0 {
            unsafe { CloseHandle(h) };
            unsafe { err(out, b"read failed") };
            return;
        }
        total += usize::try_from(n).unwrap_or(0);
    }
    unsafe { CloseHandle(h) };
    unsafe {
        *out = RawResultStr {
            tag: 0,
            _pad: 0,
            payload: RawStr {
                ptr: buf,
                len: total,
            },
        }
    };
}

/// `write_file(path, data) -> bool` — create/truncate then write all
/// bytes; `0` on any OS failure.
///
/// # Safety
/// `path`/`data` must be valid for their lengths.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_write_file(
    path: *const u8,
    path_len: usize,
    data: *const u8,
    data_len: usize,
) -> i32 {
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const CREATE_ALWAYS: u32 = 2;
    let w = unsafe { utf8_to_wstr(path, path_len) };
    if w.is_null() {
        return 0;
    }
    let h = unsafe {
        CreateFileW(
            w,
            GENERIC_WRITE,
            0,
            core::ptr::null_mut(),
            CREATE_ALWAYS,
            0,
            core::ptr::null_mut(),
        )
    };
    unsafe { aura_rt_free(w.cast()) };
    if h.is_null() || h == (-1isize as *mut c_void) {
        return 0;
    }
    let mut total = 0usize;
    while total < data_len {
        let chunk = u32::try_from(data_len - total).unwrap_or(u32::MAX);
        let mut n: u32 = 0;
        let ok = unsafe {
            WriteFile(
                h,
                data.add(total).cast(),
                chunk,
                &mut n,
                core::ptr::null_mut(),
            )
        };
        if ok == 0 {
            unsafe { CloseHandle(h) };
            return 0;
        }
        total += usize::try_from(n).unwrap_or(0);
        if n == 0 {
            break;
        }
    }
    unsafe { CloseHandle(h) };
    i32::from(total == data_len)
}

/// `read_stdin() -> str` — drain stdin to EOF into a growing heap
/// buffer. No stdin / closed stdin yields `{null, 0}`.
///
/// # Safety
/// `out` must be writable for a `RawStr`.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_read_stdin(out: *mut RawStr) {
    const STD_INPUT_HANDLE: i32 = -10;
    if out.is_null() {
        return;
    }
    let empty = RawStr {
        ptr: core::ptr::null_mut(),
        len: 0,
    };
    let h = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if h.is_null() || h == (-1isize as *mut c_void) {
        unsafe { *out = empty };
        return;
    }
    let mut cap = 4096usize;
    let mut buf = aura_rt_alloc(cap);
    if buf.is_null() {
        unsafe { *out = empty };
        return;
    }
    let mut total = 0usize;
    loop {
        if total == cap {
            let ncap = cap.saturating_mul(2);
            let nb = unsafe {
                HeapReAlloc(GetProcessHeap(), 0, buf.cast(), ncap).cast::<u8>()
            };
            if nb.is_null() {
                break;
            }
            buf = nb;
            cap = ncap;
        }
        let mut n: u32 = 0;
        let ok = unsafe {
            ReadFile(
                h,
                buf.add(total).cast(),
                u32::try_from(cap - total).unwrap_or(u32::MAX),
                &mut n,
                core::ptr::null_mut(),
            )
        };
        if ok == 0 || n == 0 {
            break;
        }
        total += usize::try_from(n).unwrap_or(0);
    }
    unsafe { *out = RawStr { ptr: buf, len: total } };
}

/// `exec(cmd) -> i64` — `CreateProcessW` with the string as the command
/// line, wait, return the child's exit code; `-1` on spawn failure.
/// The child inherits no std handles (STARTF_USESTDHANDLES + null
/// handles) so its output can't interleave with the parent's — a
/// program's `exec` contract is the exit code plus file system effects.
///
/// # Safety
/// `cmd` must be valid for `cmd_len` bytes.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aura_exec(cmd: *const u8, cmd_len: usize) -> i64 {
    const STARTF_USESTDHANDLES: u32 = 0x100;
    const INFINITE: u32 = 0xFFFF_FFFF;
    let w = unsafe { utf8_to_wstr(cmd, cmd_len) };
    if w.is_null() {
        return -1;
    }
    let mut si = StartupInfoW {
        cb: u32::try_from(core::mem::size_of::<StartupInfoW>()).unwrap_or(104),
        _pad0: 0,
        reserved: core::ptr::null(),
        desktop: core::ptr::null(),
        title: core::ptr::null(),
        x: 0,
        y: 0,
        x_size: 0,
        y_size: 0,
        x_count: 0,
        y_count: 0,
        fill: 0,
        flags: STARTF_USESTDHANDLES,
        show: 0,
        reserved2: 0,
        _pad1: 0,
        reserved3: core::ptr::null_mut(),
        std_in: core::ptr::null_mut(),
        std_out: core::ptr::null_mut(),
        std_err: core::ptr::null_mut(),
    };
    let mut pi = ProcessInfo {
        process: core::ptr::null_mut(),
        thread: core::ptr::null_mut(),
        pid: 0,
        tid: 0,
    };
    let ok = unsafe {
        CreateProcessW(
            core::ptr::null(),
            w,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0,
            0,
            core::ptr::null_mut(),
            core::ptr::null(),
            &mut si,
            &mut pi,
        )
    };
    unsafe { aura_rt_free(w.cast()) };
    if ok == 0 {
        return -1;
    }
    unsafe { WaitForSingleObject(pi.process, INFINITE) };
    let mut code: u32 = 0;
    unsafe { GetExitCodeProcess(pi.process, &mut code) };
    unsafe {
        CloseHandle(pi.process);
        CloseHandle(pi.thread);
    }
    i64::from(code)
}
