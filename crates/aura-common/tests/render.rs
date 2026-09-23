use aura_common::codes;
use aura_common::{Diagnostic, DiagnosticSink, FileId, SourceCache, Span, render_diagnostics};

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip CSI sequence: ESC [ ... final-byte
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn render_to_string(diags: &[Diagnostic], cache: &SourceCache) -> String {
    let mut buf = Vec::new();
    render_diagnostics(diags, cache, &mut buf).unwrap();
    strip_ansi(&String::from_utf8(buf).unwrap())
}

#[test]
fn renders_spanned_error_with_snippet() {
    let mut cache = SourceCache::new();
    let file = cache.add("hello.aura", "fn main() {\n    let x = unknown\n}\n");

    let diag = Diagnostic::error(
        codes::SEM_UNDECLARED_VAR,
        "cannot find value `unknown` in this scope",
        Span::new(file, 23, 30),
    )
    .with_label(Span::new(file, 8, 12), "in this function")
    .with_note("variables must be declared with `let` before use");

    let rendered = render_to_string(&[diag], &cache);
    assert!(rendered.contains("E2001"));
    assert!(rendered.contains("unknown"));
    insta::assert_snapshot!(rendered);
}

#[test]
fn renders_detached_diagnostic_as_plain_line() {
    let cache = SourceCache::new();
    let diag = Diagnostic::detached(
        aura_common::Severity::Error,
        "linker `mold` not found on PATH",
    );
    let rendered = render_to_string(&[diag], &cache);
    assert_eq!(rendered.trim(), "error: linker `mold` not found on PATH");
}

#[test]
fn diagnostic_sink_collects_and_reports() {
    let mut sink = DiagnosticSink::new();
    assert!(!sink.has_errors());

    sink.error(
        codes::LEX_INVALID_CHAR,
        "unexpected character `@`",
        Span::point(FileId(0), 42),
    );
    sink.push(Diagnostic::detached(
        aura_common::Severity::Warning,
        "be careful",
    ));

    assert!(sink.has_errors());
    assert_eq!(sink.len(), 2);
    assert_eq!(sink.as_slice()[0].code, Some(codes::LEX_INVALID_CHAR));
}
