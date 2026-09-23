//! The parser must never panic — malformed input produces `Error` nodes and
//! diagnostics. This suite byte-mutates a representative program and feeds
//! truncated/duplicated/randomized variants through the full pipeline.

use aura_common::FileId;
use aura_parser::parse_file;

const SAMPLE: &str = r#"
struct Point { x: f64, y: f64 }
enum Shape { Circle(f64) Rect(f64, f64) }
use std.io
extern "C" { fn abs(x: i32) -> i32 }

fn main() {
    let mut p = Point { x: 1.0, y: 2.0 }
    p.x = p.x + 1
    if p.x > 0 { print_f64(p.x) } else { print_str("neg") }
    match s { Circle(r) => r, _ => 0 }
    while cond { work() }
    loop { break }
    let t = try_thing()?
    unsafe { poke(0) }
    return 42
}
"#;

#[test]
fn byte_mutations_never_panic() {
    let bytes = SAMPLE.as_bytes();
    for i in 0..bytes.len() {
        for replacement in [0u8, b'\n', b'{', b'}', b'(', b'"', b'\\', 0xFF, b';'] {
            let mut mutated = bytes.to_vec();
            mutated[i] = replacement;
            let s = String::from_utf8_lossy(&mutated);
            let _ = parse_file(&s, FileId(0)); // must not panic
        }
    }
}

#[test]
fn truncations_never_panic() {
    for i in 0..SAMPLE.len() {
        if SAMPLE.is_char_boundary(i) {
            let _ = parse_file(&SAMPLE[..i], FileId(0));
        }
    }
}

#[test]
fn duplications_never_panic() {
    for i in 0..SAMPLE.len() {
        if SAMPLE.is_char_boundary(i) {
            let s = format!("{}{}", &SAMPLE[..i], SAMPLE[i..].repeat(2));
            let _ = parse_file(&s, FileId(0));
        }
    }
}

#[test]
fn random_byte_streams_never_panic() {
    // deterministic xorshift — no rand dep needed
    let mut state = 0x243F_6A88_85A3_08D3_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..200 {
        let len = usize::try_from(next() % 512).unwrap();
        let bytes: Vec<u8> = (0..len)
            .map(|_| u8::try_from(next() % 256).unwrap())
            .collect();
        let s = String::from_utf8_lossy(&bytes);
        let _ = parse_file(&s, FileId(0));
    }
}

#[test]
fn token_soup_never_panics() {
    // every token kind in pathological sequences
    let soup = "fn let mut if else while loop return struct enum match use unsafe extern break continue true false \
                ident 42 3.14 \"str\" + - * / % = == ! != < <= > >= && || -> => ? \
                ( ) { } [ ] , : ; . .. \n\n\n";
    for _ in 0..50 {
        let mut chunks: Vec<&str> = soup.split_whitespace().collect();
        // deterministic shuffle
        let mut s = 0xDEAD_BEEF_u64;
        for i in (1..chunks.len()).rev() {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let j = usize::try_from(s % (i as u64 + 1)).unwrap();
            chunks.swap(i, j);
        }
        let _ = parse_file(&chunks.join(" "), FileId(0));
    }
}
