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
fn str_literal_lowers_to_strlit_temp() {
    let m = mir_of("fn f() { let s = \"hi\" }", "f");
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
    let lit = m
        .blocks
        .iter()
        .flat_map(|b| &b.stmts)
        .find_map(|s| match s {
            aura_mir::MirStmt::Assign(p, aura_mir::Rvalue::StrLit(t)) => Some((p, t)),
            _ => None,
        });
    let (p, text) = lit.expect("expected StrLit");
    assert_eq!(text, "hi");
    assert!(
        matches!(m.locals[p.local as usize].ty, aura_semantic::Type::Str),
        "strlit dest must be `str`: {:?}",
        dump(&m)
    );
}

#[test]
fn str_len_lowers_to_field_one() {
    let m = mir_of("fn f() -> usize { let s = \"hi\"\n s.len }", "f");
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
    let has_len_proj = m.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(
                s,
                aura_mir::MirStmt::Assign(_, aura_mir::Rvalue::Use(Operand::Place(p)))
                    if p.proj == [aura_mir::Proj::Field(1)]
            )
        })
    });
    assert!(has_len_proj, "expected `.1` projection in {:?}", dump(&m));
}

#[test]
fn str_match_pattern_uses_strlit() {
    let m = mir_of(
        "fn f(s: str) -> i64 { match s { \"a\" => 1, _ => 0 } }",
        "f",
    );
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
    let lit = m.blocks.iter().flat_map(|b| &b.stmts).any(
        |s| matches!(s, aura_mir::MirStmt::Assign(_, aura_mir::Rvalue::StrLit(t)) if t == "a"),
    );
    assert!(lit, "expected pattern strlit in {:?}", dump(&m));
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

#[test]
fn enum_constructs_and_match_lowers() {
    let m = mir_of(
        "enum S { A(i64), B, C(i64, i64) }\n\
         fn f(s: S) -> i64 {\n  match s {\n    A(x) => x,\n    B => 0,\n    C(a, b) => a + b,\n  }\n}",
        "f",
    );
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
    // Discriminant reads drive the arm tests.
    let disc_reads = m
        .blocks
        .iter()
        .flat_map(|b| &b.stmts)
        .filter(|s| {
            matches!(
                s,
                aura_mir::MirStmt::Assign(_, aura_mir::Rvalue::Discriminant(_))
            )
        })
        .count();
    assert_eq!(
        disc_reads,
        3,
        "expected one disc test per arm: {:?}",
        dump(&m)
    );
    // Payload bindings read through VariantField projections.
    let vf = m
        .blocks
        .iter()
        .flat_map(|b| &b.stmts)
        .filter(|s| {
            matches!(
                s,
                aura_mir::MirStmt::Assign(_, aura_mir::Rvalue::Use(Operand::Place(p)))
                    if p.proj.iter().any(|pr| matches!(pr, aura_mir::Proj::VariantField { .. }))
            )
        })
        .count();
    assert_eq!(vf, 3, "expected payload bindings: {:?}", dump(&m));
    // The residue after the last arm is unreachable (exhaustive match).
    let last = m.blocks.last().unwrap();
    assert!(matches!(last.term, MirTerm::Unreachable));
}

#[test]
fn enum_lit_constructs_tagged_value() {
    let m = mir_of(
        "enum E { X, Y(i64) }\nfn f() -> E { Y(7) }\nfn g() -> E { X }",
        "f",
    );
    assert!(m.diagnostics.is_empty());
    let has_enum_lit = m.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(
                s,
                aura_mir::MirStmt::Assign(_, aura_mir::Rvalue::EnumLit { variant: 1, .. })
            )
        })
    });
    assert!(has_enum_lit, "expected EnumLit: {:?}", dump(&m));
    let g = mir_of(
        "enum E { X, Y(i64) }\nfn f() -> E { Y(7) }\nfn g() -> E { X }",
        "g",
    );
    let unit_lit = g.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(
                s,
                aura_mir::MirStmt::Assign(
                    _,
                    aura_mir::Rvalue::EnumLit {
                        variant: 0,
                        fields,
                        ..
                    }
                ) if fields.is_empty()
            )
        })
    });
    assert!(unit_lit, "expected unit EnumLit: {:?}", dump(&g));
}

#[test]
fn match_on_literal_uses_eq_tests() {
    let m = mir_of("fn f(n: i64) -> i64 { match n { 0 => 1, _ => 2 } }", "f");
    assert!(m.diagnostics.is_empty());
    let eq_tests = m
        .blocks
        .iter()
        .flat_map(|b| &b.stmts)
        .filter(|s| {
            matches!(
                s,
                aura_mir::MirStmt::Assign(_, aura_mir::Rvalue::Binary(aura_ast::BinOp::Eq, _, _))
            )
        })
        .count();
    assert_eq!(eq_tests, 1, "expected one literal test: {:?}", dump(&m));
}

// ----- Result / `?` ---------------------------------------------------------------

#[test]
fn result_ctors_lower_to_tagged_lits() {
    let m = mir_of(
        "fn f(ok: bool) -> Result<i64, i64> { if ok { Ok(1) } else { Err(2) } }",
        "f",
    );
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
    // Both ctors become EnumLit on the RESULT_ITEM sentinel (Ok=v0, Err=v1).
    for variant in [0u32, 1] {
        let found = m.blocks.iter().any(|b| {
            b.stmts.iter().any(|s| {
                matches!(
                    s,
                    aura_mir::MirStmt::Assign(
                        _,
                        aura_mir::Rvalue::EnumLit {
                            item,
                            variant: v,
                            ..
                        }
                    ) if *item == aura_mir::RESULT_ITEM && *v == variant
                )
            })
        });
        assert!(found, "missing Result v{variant} lit: {:?}", dump(&m));
    }
}

#[test]
fn try_desugars_to_disc_branch_and_err_return() {
    let m = mir_of(
        "fn inner() -> Result<i64, i64> { Ok(1) }\n\
         fn outer() -> Result<i64, i64> {\n  let v = inner()?\n  Ok(v)\n}",
        "outer",
    );
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
    // Entry: disc(scr) == 0 test, branch Ok/Err.
    let MirTerm::Branch { then, else_, .. } = m.blocks[0].term else {
        panic!("entry must branch on the discriminant: {:?}", dump(&m));
    };
    // Err block writes `_0 = Result::Err { .. }` and returns.
    let err = &m.blocks[else_ as usize];
    let writes_err = err.stmts.iter().any(|s| {
        matches!(
            s,
            aura_mir::MirStmt::Assign(
                p,
                aura_mir::Rvalue::EnumLit {
                    item,
                    variant: 1,
                    fields,
                }
            ) if *item == aura_mir::RESULT_ITEM
                && p.local == 0
                && matches!(fields.first(), Some((0, Operand::Place(src)))
                    if src.proj.iter().any(|pr| matches!(pr,
                        aura_mir::Proj::VariantField { variant: 1, field: 0 })))
        )
    });
    assert!(
        writes_err,
        "Err block must write _0 = Err(scr.<v1>.0): {:?}",
        dump(&m)
    );
    assert!(matches!(err.term, MirTerm::Return));
    // Ok block binds the payload place `scr.<v0>.0`.
    let ok = &m.blocks[then as usize];
    let reads_ok = ok.stmts.iter().any(|s| {
        matches!(
            s,
            aura_mir::MirStmt::Assign(_, aura_mir::Rvalue::Use(Operand::Place(p)))
                if p.proj.iter().any(|pr| matches!(pr,
                    aura_mir::Proj::VariantField { variant: 0, field: 0 }))
        )
    });
    assert!(reads_ok, "Ok block must read scr.<v0>.0: {:?}", dump(&m));
}

#[test]
fn match_on_result_binds_payloads() {
    let m = mir_of(
        "fn inner() -> Result<i64, i64> { Ok(1) }\n\
         fn f() -> i64 {\n  match inner() {\n    Ok(v) => v,\n    Err(e) => e,\n  }\n}",
        "f",
    );
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
    // Two discriminant tests (v0 Ok, v1 Err), payload binds via VariantField.
    let discs = m
        .blocks
        .iter()
        .flat_map(|b| &b.stmts)
        .filter(|s| {
            matches!(
                s,
                aura_mir::MirStmt::Assign(_, aura_mir::Rvalue::Discriminant(_))
            )
        })
        .count();
    assert_eq!(discs, 2, "expected Ok+Err tests: {:?}", dump(&m));
}
