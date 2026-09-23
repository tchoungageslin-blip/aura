//! Salsa database layer for Aura.
//!
//! Query graph (each `#[salsa::tracked]` fn is an invalidation boundary):
//!
//! ```text
//! SourceFile (input: text)
//!   └─ parsed        — full AST + interner + lex/parse diagnostics (no_eq:
//!                      every text change counts as changed)
//!        ├─ file_items — position-free signature tree: names, kinds, types.
//!        │              Editing *inside* a body produces an equal FileItems,
//!        │              so dependents are backdated (early cutoff).
//!        └─ fn_body    — per-function cloned arena keyed by (file, index).
//!                        Equal iff that fn's nodes are identical, spans
//!                        included — editing a *later* fn's body preserves it.
//! ```
//!
//! Diagnostics flow through the [`Diagnostics`] accumulator: any tracked fn
//! may `Diagnostics(d).accumulate(db)`; callers collect the whole subtree's
//! diagnostics with `check_file::accumulated::<Diagnostics>(db, file)`.
//!
//! The [`AuraDatabase`] installs a salsa event callback recording
//! `WillExecute`/`DidValidateMemoizedValue` — used by incrementality tests to
//! prove which queries actually re-executed.

mod body;
mod items;

pub use body::{Body, fn_body};
pub use items::{
    ExternFnSig, FileItems, ItemSig, ParamSig, ProjectItems, TypeName, file_items, project_items,
};

use std::sync::{Arc, Mutex};

use aura_common::{Diagnostic, FileId};
use aura_parser::ParsedFile;
use salsa::Accumulator;

/// Database trait every Aura query is written against.
#[salsa::db]
pub trait Db: salsa::Database {}

/// Concrete salsa database for batch (CLI) and interactive (LSP) use.
///
/// `event_log` records salsa event debug strings (`WillExecute { … }`,
/// `DidValidateMemoizedValue { … }`) so tests can observe which queries ran.
/// The log grows unboundedly — tests clear it via [`AuraDatabase::clear_events`].
#[salsa::db]
pub struct AuraDatabase {
    storage: salsa::Storage<Self>,
    event_log: Arc<Mutex<Vec<String>>>,
}

#[salsa::db]
impl Db for AuraDatabase {}

#[salsa::db]
impl salsa::Database for AuraDatabase {}

impl AuraDatabase {
    /// New database with the event log enabled.
    ///
    /// # Panics
    /// Never panics; the signature exists for API symmetry.
    #[must_use]
    pub fn new() -> Self {
        Self::with_event_log(true)
    }

    /// `log_events` controls whether the salsa event callback is installed.
    /// Production callers should pass `false` (avoids a mutex per event).
    ///
    /// # Panics
    /// Panics only if the internal mutex is poisoned by a prior panic.
    #[must_use]
    pub fn with_event_log(log_events: bool) -> Self {
        let event_log = Arc::new(Mutex::new(Vec::new()));
        let callback = log_events.then(|| {
            let log = Arc::clone(&event_log);
            Box::new(move |event: salsa::Event| {
                // Debug format embeds the ingredient name, e.g.
                // `WillExecute { database_key: fn_body(Id(3)) }`.
                log.lock().unwrap().push(format!("{:?}", event.kind));
            }) as Box<dyn Fn(salsa::Event) + Send + Sync + 'static>
        });
        Self {
            storage: salsa::Storage::new(callback),
            event_log,
        }
    }

    /// Snapshot of recorded salsa event debug strings.
    ///
    /// # Panics
    /// Panics only if the event-log mutex is poisoned by a prior panic.
    #[must_use]
    pub fn events(&self) -> Vec<String> {
        self.event_log.lock().unwrap().clone()
    }

    /// Count of `WillExecute` events whose debug name contains `needle`
    /// (e.g. `"fn_body"`, `"file_items"`, `"parsed"`).
    ///
    /// # Panics
    /// Panics only if the event-log mutex is poisoned by a prior panic.
    #[must_use]
    pub fn execution_count(&self, needle: &str) -> usize {
        self.event_log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.starts_with("WillExecute") && e.contains(needle))
            .count()
    }

    /// Clear the recorded event log.
    ///
    /// # Panics
    /// Panics only if the event-log mutex is poisoned by a prior panic.
    pub fn clear_events(&self) {
        self.event_log.lock().unwrap().clear();
    }
}

impl Default for AuraDatabase {
    fn default() -> Self {
        Self::new()
    }
}

// ----- inputs ----------------------------------------------------------------

/// One source file under incremental tracking.
#[salsa::input]
pub struct SourceFile {
    /// Full source text (LF or CRLF as-is).
    #[returns(ref)]
    pub text: String,
    /// Compiler-assigned id embedded in every [`Span`](aura_common::Span).
    #[returns(copy)]
    pub file_id: FileId,
}

/// A multi-file compilation unit: every file's items share one flat
/// namespace (alpha semantics — no module system yet). `files` is ordered
/// deps-first; the entry point (`src/main.aura`) is always last.
#[salsa::input]
pub struct Project {
    /// All compilation sources in project order.
    #[returns(ref)]
    pub files: Vec<SourceFile>,
}

// Salsa inputs don't derive `Debug`; `ProjectItems` wants it for `map`.
impl std::fmt::Debug for SourceFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SourceFile").field(&self.0).finish()
    }
}

// ----- queries ---------------------------------------------------------------

/// Accumulator channel for diagnostics produced by tracked queries.
/// Push with `Diagnostics(d).accumulate(db)`; read with
/// `query::accumulated::<Diagnostics>(db, key)`.
#[salsa::accumulator]
pub struct Diagnostics(pub Diagnostic);

/// Parse `file`'s current text. `no_eq`: the parse output contains body
/// content, so any text edit yields a different tree — early cutoff lives in
/// `file_items`/`fn_body`, not here.
#[salsa::tracked(no_eq, returns(ref))]
pub fn parsed(db: &dyn Db, file: SourceFile) -> ParsedFile {
    let out = aura_parser::parse_file(file.text(db), file.file_id(db));
    for diag in &out.diagnostics {
        Diagnostics(diag.clone()).accumulate(db);
    }
    out
}
