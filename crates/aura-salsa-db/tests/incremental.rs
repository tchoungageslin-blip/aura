//! Salsa wiring + early-cutoff tests for the query layer.

use aura_common::FileId;
use aura_salsa_db::{AuraDatabase, ItemSig, SourceFile, TypeName, file_items, fn_body, parsed};
use salsa::Setter;

const SRC: &str = "\
struct Point { x: f64, y: f64 }

fn make(x: f64, y: f64) -> Point {
    Point { x: x, y: y }
}

fn main() -> i64 {
    0
}
";

fn db_with(src: &str) -> (AuraDatabase, SourceFile) {
    let db = AuraDatabase::new();
    let file = SourceFile::new(&db, src.to_owned(), FileId(0));
    (db, file)
}

#[test]
fn parses_through_salsa() {
    let (db, file) = db_with(SRC);
    let p = parsed(&db, file);
    assert_eq!(p.items.len(), 3);
    assert!(p.diagnostics.is_empty());
}

#[test]
fn file_items_extracts_signatures() {
    let (db, file) = db_with(SRC);
    let items = file_items(&db, file);
    assert_eq!(items.items.len(), 3);
    match &items.items[1] {
        ItemSig::Fn {
            name,
            params,
            ret,
            is_extern,
        } => {
            assert_eq!(name, "make");
            assert_eq!(params.len(), 2);
            assert_eq!(params[0].name, "x");
            assert!(!is_extern);
            assert_eq!(
                *ret,
                Some(TypeName::Named {
                    name: "Point".into(),
                    args: vec![]
                })
            );
        }
        other => panic!("expected fn, got {other:?}"),
    }
    match &items.items[0] {
        ItemSig::Struct { name, fields } => {
            assert_eq!(name, "Point");
            assert_eq!(fields.len(), 2);
        }
        other => panic!("expected struct, got {other:?}"),
    }
}

#[test]
fn fn_body_clones_subtree() {
    let (db, file) = db_with(SRC);
    // item 2 = fn main
    let body = fn_body(&db, file, 2).as_ref().expect("main has a body");
    let block = body.ast.block(body.root);
    assert!(block.tail.is_some());
    // item 0 = struct — no body
    assert!(fn_body(&db, file, 0).is_none());
}

#[test]
fn body_edit_preserves_file_items() {
    let (mut db, file) = db_with(SRC);
    let before = file_items(&db, file).clone();

    // Edit inside `main`'s body only — signatures untouched.
    let edited = SRC.replace("    0\n}", "    42\n}");
    file.set_text(&mut db).to(edited);

    let after = file_items(&db, file);
    // Same signature tree: dependents of file_items are backdated.
    assert_eq!(&before, after);
}

#[test]
fn later_body_edit_preserves_earlier_fn_body() {
    let (mut db, file) = db_with(SRC);
    let make_body_before = format!("{:?}", fn_body(&db, file, 1));

    // Edit `main` (item 2, comes AFTER `make`) — `make`'s spans don't move.
    let edited = SRC.replace("    0\n}", "    42\n}");
    file.set_text(&mut db).to(edited);

    let make_body_after = format!("{:?}", fn_body(&db, file, 1));
    assert_eq!(make_body_before, make_body_after);

    // And `main`'s own body DID change (new literal).
    let main_body = format!("{:?}", fn_body(&db, file, 2));
    assert!(main_body.contains("Int(42)"));
}

#[test]
fn signature_edit_invalidates_file_items() {
    let (mut db, file) = db_with(SRC);
    let _ = file_items(&db, file);
    db.clear_events();

    // Rename `main` → `entry` — a signature-level change.
    file.set_text(&mut db)
        .to(SRC.replace("fn main", "fn entry"));
    let items = file_items(&db, file);
    assert_eq!(items.items[2].name(), Some("entry"));

    // file_items must have re-executed (its input changed semantically).
    assert!(db.execution_count("file_items") >= 1);
}

#[test]
fn parse_diagnostics_flow_through_accumulator() {
    let (db, file) = db_with("fn broken( {\n");
    let _ = parsed(&db, file);
    let diags = parsed::accumulated::<aura_salsa_db::Diagnostics>(&db, file);
    assert!(!diags.is_empty());
}
