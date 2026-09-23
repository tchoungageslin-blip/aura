//! Shared types for the Aura compiler: source locations, diagnostics, error codes.
//!
//! Every crate in the workspace depends on this one. It must stay small and
//! dependency-light.

mod diagnostic;
mod render;
mod span;

pub use diagnostic::{Diagnostic, DiagnosticSink, Label, Severity};
pub use render::{SourceCache, render_diagnostics};
pub use span::{FileId, Span};

/// Namespaced error codes. Each compiler phase owns a range:
/// `E0xxx` lexer, `E1xxx` parser, `E2xxx` semantic, `E3xxx` codegen.
pub mod codes {
    // Lexer (E0xxx)
    pub const LEX_INVALID_CHAR: &str = "E0001";
    pub const LEX_UNTERMINATED_STRING: &str = "E0002";
    pub const LEX_UNTERMINATED_BLOCK_COMMENT: &str = "E0003";
    pub const LEX_INVALID_NUMBER: &str = "E0004";
    pub const LEX_INVALID_ESCAPE: &str = "E0005";
    pub const LEX_INVALID_UTF8: &str = "E0006";

    // Parser (E1xxx)
    pub const PARSE_UNEXPECTED_TOKEN: &str = "E1001";
    pub const PARSE_EXPECTED_EXPR: &str = "E1002";
    pub const PARSE_EXPECTED_IDENT: &str = "E1003";
    pub const PARSE_EXPECTED_TYPE: &str = "E1004";
    pub const PARSE_UNCLOSED_DELIMITER: &str = "E1005";
    pub const PARSE_EXPECTED_ITEM: &str = "E1006";
    pub const PARSE_MAX_DEPTH: &str = "E1007";
    pub const PARSE_BLOCK_IN_EXPR_POS: &str = "E1008";

    // Semantic (E2xxx)
    pub const SEM_UNDECLARED_VAR: &str = "E2001";
    pub const SEM_USE_BEFORE_INIT: &str = "E2002";
    pub const SEM_REDEFINITION: &str = "E2003";
    pub const SEM_MUTATE_IMMUTABLE: &str = "E2004";
    pub const SEM_CIRCULAR_DEP: &str = "E2010";
    pub const SEM_TYPE_MISMATCH: &str = "E2100";
    pub const SEM_IF_MISSING_ELSE: &str = "E2101";
    pub const SEM_ARG_COUNT: &str = "E2102";
    pub const SEM_NOT_CALLABLE: &str = "E2103";
    pub const SEM_NO_FIELD: &str = "E2104";
    pub const SEM_NOT_BOOL_CONDITION: &str = "E2105";
    pub const SEM_NON_EXHAUSTIVE_MATCH: &str = "E2106";
    pub const SEM_RETURN_TYPE: &str = "E2107";

    // Codegen (E3xxx)
    pub const CG_INTERNAL: &str = "E3001";
    pub const CG_LINK_FAILED: &str = "E3002";
    pub const CG_LINKER_NOT_FOUND: &str = "E3003";
}
