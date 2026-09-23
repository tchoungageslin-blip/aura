//! MIR lowering tests — CFG shapes, temporaries, unsupported constructs.

use aura_common::FileId;
use aura_mir::{MirTerm, Operand, dump, mir_fn};
use aura_salsa_db::{AuraDatabase, SourceFile, file_items};

fn mir_of(src: &str, name: &str) -> aura_mir::MirBody {
    let db = AuraDatabase::with_event_log(false);
    let file = SourceFile::new(&db, src.to_owned(), FileId(0));
    let items = file_items(&db, file);
    let idx = items.find(name).expect("fn not found");
    mir_fn(&db, file, idx).as_ref().expect("no mir").clone()
}

#[test]
fn scalar_fn_lowers_to_single_block() {
    let m = mir_of("fn f(x: i64) -> i64 { let y = x + 1\n y * 2 }", "f");
    assert_eq!(m.name, "f");
    assert_eq!(m.param_count, 1);
    // _0 ret, _1 x, _2 y, temps for `x+1` and `y*2` → 5 locals.
    assert_eq!(m.locals.len(), 5);
    assert_eq!(m.blocks.len(), 1);
    assert_eq!(m.blocks[0].term, MirTerm::Return);
    // stmts: _t=x+1; _2=_t; _t=y*2; _0=_t → 4
    assert_eq!(m.blocks[0].stmts.len(), 4);
}

#[test]
fn if_else_produces_diamond_cfg() {
    let m = mir_of("fn f(c: bool) -> i64 { if c { 1 } else { 2 } }", "f");
    // entry + then + else + join
    assert_eq!(m.blocks.len(), 4);
    let MirTerm::Branch { then, else_, .. } = m.blocks[0].term else {
        panic!("entry must branch");
    };
    assert_eq!(then, 1);
    assert_eq!(else_, 2);
    assert!(matches!(m.blocks[1].term, MirTerm::Goto(3)));
    assert!(matches!(m.blocks[2].term, MirTerm::Goto(3)));
    assert!(matches!(m.blocks[3].term, MirTerm::Return));
}

#[test]
fn while_loop_has_back_edge() {
    let m = mir_of(
        "fn f(n: i64) -> i64 { let mut i = n\n while i > 0 { i = i - 1 } i }",
        "f",
    );
    // entry → header(1) → body(2)/exit(3)
    assert_eq!(m.blocks.len(), 4);
    assert!(matches!(m.blocks[0].term, MirTerm::Goto(1)));
    let MirTerm::Branch { then, else_, .. } = m.blocks[1].term else {
        panic!("header must branch");
    };
    assert_eq!((then, else_), (2, 3));
    assert!(matches!(m.blocks[2].term, MirTerm::Goto(1))); // back edge
}

#[test]
fn break_continue_use_loop_stack() {
    let m = mir_of(
        "fn f() { loop { if done() { break } continue } }\nfn done() -> bool { true }",
        "f",
    );
    // any block that branches on the `done()` call must goto exit on true
    let has_break_edge = m.blocks.iter().any(|b| {
        matches!(
            &b.term,
            MirTerm::Branch {
                cond: Operand::Place(_),
                ..
            }
        )
    });
    assert!(has_break_edge);
    // loop body back-edge exists
    let has_back = m
        .blocks
        .iter()
        .any(|b| matches!(&b.term, MirTerm::Goto(t) if *t == 1));
    assert!(has_back);
}

#[test]
fn short_circuit_and_shortcircuits() {
    let m = mir_of(
        "fn f(a: bool, b: bool) -> bool { a && b }\nfn g(a: bool, b: bool) -> bool { a || b }",
        "f",
    );
    // entry branches on _1; shortcut block assigns false; rhs block assigns b
    assert!(matches!(m.blocks[0].term, MirTerm::Branch { .. }));
    let joins = m
        .blocks
        .iter()
        .filter(|b| matches!(b.term, MirTerm::Goto(_)))
        .count();
    assert!(joins >= 2);
}

#[test]
fn struct_literal_and_field_access() {
    let m = mir_of(
        "struct P { x: i64, y: i64 }\nfn f(p: P) -> i64 { p.x + p.y }",
        "f",
    );
    assert!(m.diagnostics.is_empty());
    let has_proj = m.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(
                s,
                aura_mir::MirStmt::Assign(
                    _,
                    aura_mir::Rvalue::Binary(_, Operand::Place(l), _)
                        | aura_mir::Rvalue::Binary(_, _, Operand::Place(l))
                ) if !l.proj.is_empty()
            )
        })
    });
    assert!(has_proj, "expected field projection in {:?}", dump(&m));
}

#[test]
fn string_literal_is_unsupported_diag() {
    let m = mir_of("fn f() { let s = \"hi\" }", "f");
    assert!(
        m.diagnostics
            .iter()
            .any(|d| d.code == Some(aura_common::codes::CG_UNSUPPORTED))
    );
}

#[test]
fn calls_and_externs_resolve() {
    let m = mir_of(
        "extern \"C\" { fn sq(x: f64) -> f64 }\nfn h(x: f64) -> f64 { sq(x) }\nfn f() -> f64 { h(1.0) }",
        "f",
    );
    let calls_fn = m.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(
                s,
                aura_mir::MirStmt::Assign(_, aura_mir::Rvalue::Call(aura_mir::Callee::Fn(_), _))
            )
        })
    });
    assert!(calls_fn);
    let m2 = mir_of(
        "extern \"C\" { fn sq(x: f64) -> f64 }\nfn h(x: f64) -> f64 { sq(x) }\nfn f() -> f64 { h(1.0) }",
        "h",
    );
    let calls_ext = m2.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(
                s,
                aura_mir::MirStmt::Assign(
                    _,
                    aura_mir::Rvalue::Call(aura_mir::Callee::Extern(..), _)
                )
            )
        })
    });
    assert!(calls_ext);
}

#[test]
fn dump_is_stable() {
    let m = mir_of("fn f(x: i64) -> i64 { x + 1 }", "f");
    let text = dump(&m);
    assert!(text.contains("fn f("));
    assert!(text.contains("bb0:"));
    assert!(text.contains("return"));
}
