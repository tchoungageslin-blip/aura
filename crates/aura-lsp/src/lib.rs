//! `aura-lsp` — Language Server Protocol server for Aura over stdio.
//!
//! The server holds one [`AuraDatabase`] and one salsa [`SourceFile`] input
//! per open document; `didOpen`/`didChange` update the input and immediately
//! republish `textDocument/publishDiagnostics` from [`check_file`], so the
//! incremental pipeline (parse → items → resolve → typeck) pays early-cutoff
//! dividends on every keystroke.
//!
//! Implemented surface:
//! - `textDocument/didOpen`, `didChange` (full sync), `didSave`, `didClose`
//! - `textDocument/hover` — inferred type of the innermost expression
//! - `textDocument/definition` — file-scope items (fn/struct/enum/variant)
//! - `textDocument/documentSymbol` — top-level outline
//! - `textDocument/formatting` — via [`aura_fmt`]
//!
//! Positions are UTF-16 code units per the LSP base protocol.

use std::collections::HashMap;
use std::ops::ControlFlow;

use async_lsp::ClientSocket;
use async_lsp::client_monitor::ClientProcessMonitorLayer;
use async_lsp::concurrency::ConcurrencyLayer;
use async_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    DiagnosticSeverity, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeResult, Location, MarkedString, OneOf, Position,
    PublishDiagnosticsParams, Range, ReferenceParams, SemanticToken, SemanticTokenType,
    SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions,
    SemanticTokensParams, SemanticTokensResult, SemanticTokensServerCapabilities,
    ServerCapabilities, ServerInfo, SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextEdit, Url, notification, request,
};
use async_lsp::panic::CatchUnwindLayer;
use async_lsp::router::Router;
use async_lsp::server::LifecycleLayer;
use aura_ast::{Expr, ExprId, Item, TypeExpr, TypeExprId};
use aura_common::{SourceCache, Span};
use aura_lexer::{TokenKind, lex};
use aura_salsa_db::{AuraDatabase, ItemSig, SourceFile, TypeName, file_items, fn_body, parsed};
use aura_semantic::{Def, check_file, resolved_file, typeck_fn};
use salsa::Setter;
use tower::ServiceBuilder;

/// Entry point for `aura lsp`: run the server over stdin/stdout until the
/// client disconnects or sends `exit`. Returns a process exit code.
pub fn serve() -> i32 {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("aura-lsp: failed to start runtime: {e}");
            return 1;
        }
    };
    match rt.block_on(run()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("aura-lsp: {e}");
            1
        }
    }
}

fn capabilities() -> InitializeResult {
    InitializeResult {
        capabilities: ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            definition_provider: Some(OneOf::Left(true)),
            document_symbol_provider: Some(OneOf::Left(true)),
            document_formatting_provider: Some(OneOf::Left(true)),
            completion_provider: Some(CompletionOptions::default()),
            references_provider: Some(OneOf::Left(true)),
            semantic_tokens_provider: Some(
                SemanticTokensServerCapabilities::SemanticTokensOptions(SemanticTokensOptions {
                    legend: SemanticTokensLegend {
                        token_types: token_legend(),
                        token_modifiers: Vec::new(),
                    },
                    full: Some(SemanticTokensFullOptions::Bool(true)),
                    range: None,
                    work_done_progress_options:
                        async_lsp::lsp_types::WorkDoneProgressOptions::default(),
                }),
            ),
            ..ServerCapabilities::default()
        },
        server_info: Some(ServerInfo {
            name: "aura-lsp".into(),
            version: Some(env!("CARGO_PKG_VERSION").into()),
        }),
    }
}

fn router(client: ClientSocket) -> Router<ServerState> {
    let mut router = Router::new(ServerState {
        client,
        db: AuraDatabase::new(),
        cache: SourceCache::new(),
        docs: HashMap::new(),
    });
    router
        .request::<request::Initialize, _>(|_, _| async { Ok(capabilities()) })
        .request::<request::HoverRequest, _>(|st, params| {
            let res = st.hover(&params);
            async move { Ok(res) }
        })
        .request::<request::GotoDefinition, _>(|st, params| {
            let res = st.goto_definition(&params);
            async move { Ok(res) }
        })
        .request::<request::DocumentSymbolRequest, _>(|st, params| {
            let res = st.document_symbols(&params);
            async move { Ok(res) }
        })
        .request::<request::Completion, _>(|st, params| {
            let res = st.completion(&params);
            async move { Ok(res) }
        })
        .request::<request::References, _>(|st, params| {
            let res = st.references(&params);
            async move { Ok(res) }
        })
        .request::<request::SemanticTokensFullRequest, _>(|st, params| {
            let res = st.semantic_tokens(&params);
            async move { Ok(res) }
        })
        .request::<request::Shutdown, _>(|_, ()| async { Ok(()) })
        .request::<request::Formatting, _>(|st, params| {
            let res = st.formatting(&params);
            async move { Ok(res) }
        })
        .notification::<notification::Initialized>(|_, _| ControlFlow::Continue(()))
        .notification::<notification::DidOpenTextDocument>(|st, params| {
            st.open(&params.text_document.uri, params.text_document.text);
            ControlFlow::Continue(())
        })
        .notification::<notification::DidChangeTextDocument>(|st, params| {
            if let Some(change) = params.content_changes.into_iter().next_back() {
                st.change(&params.text_document.uri, change.text);
            }
            ControlFlow::Continue(())
        })
        .notification::<notification::DidSaveTextDocument>(|_, _| ControlFlow::Continue(()))
        .notification::<notification::Exit>(|_, ()| ControlFlow::Continue(()))
        .notification::<notification::DidCloseTextDocument>(|st, params| {
            st.close(params.text_document.uri);
            ControlFlow::Continue(())
        });
    router
}

async fn run() -> async_lsp::Result<()> {
    let (server, _) = async_lsp::MainLoop::new_server(|client| {
        let monitor = ClientProcessMonitorLayer::new(client.clone());
        ServiceBuilder::new()
            .layer(LifecycleLayer::default())
            .layer(CatchUnwindLayer::default())
            .layer(ConcurrencyLayer::default())
            .layer(monitor)
            .service(router(client))
    });

    // Blocking stdio behind the compat adapter works on every platform;
    // async-lsp's signal-driven PipeStdin would need its `tokio` feature.
    let (stdin, stdout) = (
        tokio_util::compat::TokioAsyncReadCompatExt::compat(tokio::io::stdin()),
        tokio_util::compat::TokioAsyncWriteCompatExt::compat_write(tokio::io::stdout()),
    );

    server.run_buffered(stdin, stdout).await
}

// ----- state ------------------------------------------------------------------

struct ServerState {
    client: ClientSocket,
    db: AuraDatabase,
    cache: SourceCache,
    docs: HashMap<Url, Document>,
}

/// One open document: the salsa input plus a cached line index for position
/// math.
struct Document {
    input: SourceFile,
    /// Byte offset of each line's first byte (UTF-16 columns derive from it).
    line_starts: Vec<u32>,
}

impl ServerState {
    fn open(&mut self, uri: &Url, text: String) {
        let file_id = self.cache.add(uri.to_string(), text.clone());
        let lines = line_starts(&text);
        let input = SourceFile::new(&self.db, text, file_id);
        self.docs.insert(
            uri.clone(),
            Document {
                input,
                line_starts: lines,
            },
        );
        self.publish(uri);
    }

    fn change(&mut self, uri: &Url, text: String) {
        let Some(doc) = self.docs.get_mut(uri) else {
            // A change for an unopened doc (racing close) is treated as open.
            self.open(uri, text);
            return;
        };
        doc.line_starts = line_starts(&text);
        doc.input.set_text(&mut self.db).to(text);
        self.publish(uri);
    }

    fn close(&mut self, uri: Url) {
        self.docs.remove(&uri);
        let _ = self
            .client
            .notify::<notification::PublishDiagnostics>(PublishDiagnosticsParams {
                uri,
                diagnostics: Vec::new(),
                version: None,
            });
    }

    /// Run the semantic pipeline on `uri`'s current text and publish the
    /// diagnostics. Salsa memoization keeps repeat calls cheap.
    fn publish(&mut self, uri: &Url) {
        let Some(doc) = self.docs.get(uri) else {
            return;
        };
        let diags = check_file(&self.db, doc.input).clone();
        let text = doc.input.text(&self.db);
        let lsp_diags = diags
            .iter()
            .filter_map(|d| to_lsp_diagnostic(text, &doc.line_starts, uri, d))
            .collect();
        let _ = self
            .client
            .notify::<notification::PublishDiagnostics>(PublishDiagnosticsParams {
                uri: uri.clone(),
                diagnostics: lsp_diags,
                version: None,
            });
    }

    fn hover(&self, params: &HoverParams) -> Option<Hover> {
        let uri = &params.text_document_position_params.text_document.uri;
        let doc = self.docs.get(uri)?;
        let offset = pos_to_offset(
            doc.input.text(&self.db),
            &doc.line_starts,
            params.text_document_position_params.position,
        )?;
        hover_at(&self.db, doc, offset)
    }

    fn goto_definition(&self, params: &GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let uri = &params.text_document_position_params.text_document.uri;
        let doc = self.docs.get(uri)?;
        let offset = pos_to_offset(
            doc.input.text(&self.db),
            &doc.line_starts,
            params.text_document_position_params.position,
        )?;
        definition_at(&self.db, doc, uri, offset)
    }

    fn document_symbols(&self, params: &DocumentSymbolParams) -> Option<DocumentSymbolResponse> {
        let doc = self.docs.get(&params.text_document.uri)?;
        Some(symbols(&self.db, doc))
    }

    fn formatting(
        &self,
        params: &async_lsp::lsp_types::DocumentFormattingParams,
    ) -> Option<Vec<TextEdit>> {
        let doc = self.docs.get(&params.text_document.uri)?;
        let text = doc.input.text(&self.db);
        // Unparseable file: no edits, no error pop-up.
        let formatted = aura_fmt::format_source(text).ok()?;
        if formatted == *text {
            return Some(Vec::new());
        }
        let end = offset_to_position(text, &doc.line_starts, sat(text.len()));
        Some(vec![TextEdit {
            range: Range::new(Position::new(0, 0), end),
            new_text: formatted,
        }])
    }

    fn completion(&self, params: &CompletionParams) -> Option<CompletionResponse> {
        let doc = self
            .docs
            .get(&params.text_document_position.text_document.uri)?;
        Some(completions(&self.db, doc))
    }

    fn references(&self, params: &ReferenceParams) -> Option<Vec<Location>> {
        let uri = &params.text_document_position.text_document.uri;
        let doc = self.docs.get(uri)?;
        let offset = pos_to_offset(
            doc.input.text(&self.db),
            &doc.line_starts,
            params.text_document_position.position,
        )?;
        references_at(
            &self.db,
            doc,
            uri,
            offset,
            params.context.include_declaration,
        )
    }

    fn semantic_tokens(&self, params: &SemanticTokensParams) -> Option<SemanticTokensResult> {
        let doc = self.docs.get(&params.text_document.uri)?;
        Some(semantic_tokens(&self.db, doc))
    }
}

// ----- feature implementations --------------------------------------------------

/// `textDocument/hover`: the inferred type of the smallest expression whose
/// span contains `offset`. Builtins answer with their signature + doc.
fn hover_at(db: &AuraDatabase, doc: &Document, offset: u32) -> Option<Hover> {
    // Builtin under the cursor → signature + doc (Def::Builtin carries it).
    if let Some((_, name)) = name_at(db, doc, offset)
        && let Some(aura_semantic::Def::Builtin(b)) = resolved_file(db, doc.input).lookup(&name)
    {
        return Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(format!(
                "```aura\n{}\n```",
                b.doc()
            ))),
            range: None,
        });
    }
    let item = enclosing_fn(db, doc, offset)?;
    let body = fn_body(db, doc.input, item).as_ref()?;
    let types = typeck_fn(db, doc.input, item).as_ref()?;
    let items = file_items(db, doc.input);

    // Smallest containing expr wins — children outrank their parents.
    let mut best: Option<(usize, u32)> = None; // (expr idx, span width)
    for i in 0..body.ast.exprs.len() {
        let id = ExprId(sat(i));
        let span = body.ast.exprs.span(id);
        if span.start <= offset
            && offset <= span.end
            && best.is_none_or(|(_, w)| span.end - span.start < w)
        {
            best = Some((i, span.end - span.start));
        }
    }
    let (idx, _) = best?;
    let ty = types.exprs.get(idx)?;
    let name = ty.display(items);
    Some(Hover {
        contents: HoverContents::Scalar(MarkedString::String(format!("```aura\n{name}\n```"))),
        range: Some(span_range(
            doc.input.text(db),
            &doc.line_starts,
            body.ast.exprs.span(ExprId(sat(idx))),
        )),
    })
}

/// The `(spur, name)` of the smallest `Ident`/`StructLit` expr containing
/// `offset`, resolved in its own body's name map. Spurs are interned per
/// parse, so the returned spur compares equal across every fn body.
fn name_at(db: &AuraDatabase, doc: &Document, offset: u32) -> Option<(lasso::Spur, String)> {
    let item = enclosing_fn(db, doc, offset)?;
    let body = fn_body(db, doc.input, item).as_ref()?;

    let mut found: Option<(lasso::Spur, u32)> = None;
    for i in 0..body.ast.exprs.len() {
        let id = ExprId(sat(i));
        let span = body.ast.exprs.span(id);
        if !(span.start <= offset && offset <= span.end) {
            continue;
        }
        let (Expr::Ident(spur) | Expr::StructLit { name: spur, .. }) = body.ast.expr(id) else {
            continue;
        };
        let width = span.end - span.start;
        if found.is_none_or(|(_, w)| width < w) {
            found = Some((*spur, width));
        }
    }
    let spur = found?.0;
    Some((spur, body.name(spur).to_owned()))
}

/// `textDocument/definition`: resolve the identifier under the cursor to its
/// file-scope definition's span.
fn definition_at(
    db: &AuraDatabase,
    doc: &Document,
    uri: &Url,
    offset: u32,
) -> Option<GotoDefinitionResponse> {
    let (_, name) = name_at(db, doc, offset)?;

    let res = resolved_file(db, doc.input);
    let item_idx = match res.lookup(&name)? {
        Def::Fn(i) | Def::Struct(i) | Def::Enum(i) => i,
        Def::Variant(enum_i, _) => enum_i,
        Def::ExternFn(block_i, _) => block_i,
        Def::ResultOk | Def::ResultErr | Def::Builtin(_) => return None, // builtins have no source
    };
    let span = parsed(db, doc.input)
        .items
        .get(item_idx as usize)
        .map(Item::span)?;
    Some(GotoDefinitionResponse::Scalar(Location {
        uri: uri.clone(),
        range: span_range(doc.input.text(db), &doc.line_starts, span),
    }))
}

/// `textDocument/completion`: keywords, primitive types, and every file-scope
/// definition (fns, structs, enums, variants, extern fns).
fn completions(db: &AuraDatabase, doc: &Document) -> CompletionResponse {
    const KEYWORDS: &[&str] = &[
        "fn", "let", "mut", "if", "else", "while", "loop", "return", "struct", "enum", "match",
        "use", "unsafe", "extern", "break", "continue", "true", "false",
    ];
    const PRIMITIVES: &[&str] = &[
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
        "f32", "f64", "bool", "str",
    ];

    let mut items: Vec<CompletionItem> = KEYWORDS
        .iter()
        .map(|k| CompletionItem {
            label: (*k).into(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..CompletionItem::default()
        })
        .chain(PRIMITIVES.iter().map(|p| CompletionItem {
            label: (*p).into(),
            kind: Some(CompletionItemKind::STRUCT),
            ..CompletionItem::default()
        }))
        .collect();

    let res = resolved_file(db, doc.input);
    for (name, def) in &res.defs {
        let kind = match def {
            Def::Fn(_) | Def::ExternFn(..) | Def::Builtin(_) => CompletionItemKind::FUNCTION,
            Def::Struct(_) => CompletionItemKind::STRUCT,
            Def::Enum(_) => CompletionItemKind::ENUM,
            Def::Variant(..) | Def::ResultOk | Def::ResultErr => CompletionItemKind::ENUM_MEMBER,
        };
        items.push(CompletionItem {
            label: name.clone(),
            kind: Some(kind),
            ..CompletionItem::default()
        });
    }
    CompletionResponse::Array(items)
}

/// `textDocument/references`: every `Ident`/`StructLit`/`TypeName` use of the
/// name under the cursor across all fn bodies, plus the declaration site when
/// `include_decl`. Spurs are interned per parse, so cross-body comparison is
/// exact — though two same-named locals in different fns alias (no scope
/// tracking in v1).
fn references_at(
    db: &AuraDatabase,
    doc: &Document,
    uri: &Url,
    offset: u32,
    include_decl: bool,
) -> Option<Vec<Location>> {
    let (target, name) = name_at(db, doc, offset)?;
    let text = doc.input.text(db);
    let mut out = Vec::new();

    if include_decl && let Some(def) = resolved_file(db, doc.input).lookup(&name) {
        let item_idx = match def {
            Def::Fn(i) | Def::Struct(i) | Def::Enum(i) => i,
            Def::Variant(enum_i, _) => enum_i,
            Def::ExternFn(block_i, _) => block_i,
            Def::ResultOk | Def::ResultErr | Def::Builtin(_) => u32::MAX,
        };
        if let Some(item) = parsed(db, doc.input).items.get(item_idx as usize) {
            out.push(Location {
                uri: uri.clone(),
                range: name_selection(text, &doc.line_starts, item.span()),
            });
        }
    }

    for (i, item) in parsed(db, doc.input).items.iter().enumerate() {
        if !matches!(item, Item::Function(f) if f.body.is_some()) {
            continue;
        }
        let Some(body) = fn_body(db, doc.input, sat(i)).as_ref() else {
            continue;
        };
        for j in 0..body.ast.exprs.len() {
            let id = ExprId(sat(j));
            let spur = match body.ast.expr(id) {
                Expr::Ident(spur) | Expr::StructLit { name: spur, .. } => *spur,
                _ => continue,
            };
            if spur == target {
                // StructLit spans cover `{ .. }` too — the name is the first
                // token, so clamp to it.
                let mut span = body.ast.exprs.span(id);
                if matches!(body.ast.expr(id), Expr::StructLit { .. }) {
                    span.end = span.start + sat(name.len());
                }
                out.push(Location {
                    uri: uri.clone(),
                    range: span_range(text, &doc.line_starts, span),
                });
            }
        }
        for j in 0..body.ast.types.len() {
            let id = TypeExprId(sat(j));
            if let TypeExpr::Named { name: spur, .. } = body.ast.types.get(id)
                && *spur == target
            {
                out.push(Location {
                    uri: uri.clone(),
                    range: span_range(text, &doc.line_starts, body.ast.types.span(id)),
                });
            }
        }
    }
    Some(out)
}

/// Semantic token legend — indices into this table are what `data` carries.
fn token_legend() -> Vec<SemanticTokenType> {
    vec![
        SemanticTokenType::TYPE,     // 0
        SemanticTokenType::FUNCTION, // 1
        SemanticTokenType::VARIABLE, // 2
        SemanticTokenType::KEYWORD,  // 3
        SemanticTokenType::NUMBER,   // 4
        SemanticTokenType::STRING,   // 5
        SemanticTokenType::OPERATOR, // 6
    ]
}

const TT_TYPE: u32 = 0;
const TT_FUNCTION: u32 = 1;
const TT_VARIABLE: u32 = 2;
const TT_KEYWORD: u32 = 3;
const TT_NUMBER: u32 = 4;
const TT_STRING: u32 = 5;
const TT_OPERATOR: u32 = 6;

/// `textDocument/semanticTokens/full`: re-lex the buffer and classify each
/// token. `Ident` disambiguation is syntactic: leading-uppercase → `type`,
/// followed by `(` → `function`, else `variable`. Comments are lexed away, so
/// they stay the `TextMate` grammar's job.
fn semantic_tokens(db: &AuraDatabase, doc: &Document) -> SemanticTokensResult {
    let text = doc.input.text(db);
    let lexed = lex(text, doc.input.file_id(db));
    let mut data: Vec<SemanticToken> = Vec::with_capacity(lexed.tokens.len());
    let (mut prev_line, mut prev_col) = (0u32, 0u32);

    for (i, tok) in lexed.tokens.iter().enumerate() {
        let ty = match tok.kind {
            TokenKind::Fn
            | TokenKind::Let
            | TokenKind::Mut
            | TokenKind::If
            | TokenKind::Else
            | TokenKind::While
            | TokenKind::Loop
            | TokenKind::Return
            | TokenKind::Struct
            | TokenKind::Enum
            | TokenKind::Match
            | TokenKind::Use
            | TokenKind::Unsafe
            | TokenKind::Extern
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::True
            | TokenKind::False => TT_KEYWORD,
            TokenKind::IntLit | TokenKind::FloatLit => TT_NUMBER,
            TokenKind::StringLit => TT_STRING,
            TokenKind::Ident => {
                let Some(sym) = tok.sym else { continue };
                let name = lexed.rodeo.resolve(&sym);
                if name.starts_with(|c: char| c.is_ascii_uppercase()) {
                    TT_TYPE
                } else {
                    // Lowercase ident followed by `(` (skipping newlines) is a
                    // call callee — color it as a function.
                    let next = lexed.tokens[i + 1..]
                        .iter()
                        .find(|t| t.kind != TokenKind::Newline);
                    if next.is_some_and(|t| t.kind == TokenKind::LParen) {
                        TT_FUNCTION
                    } else {
                        TT_VARIABLE
                    }
                }
            }
            TokenKind::Newline | TokenKind::Eof | TokenKind::Error => continue,
            _ => TT_OPERATOR, // operators + punctuation
        };
        let start = offset_to_position(text, &doc.line_starts, tok.span.start);
        let len = text[tok.span.start as usize..tok.span.end as usize]
            .chars()
            .map(char::len_utf16)
            .sum::<usize>();
        data.push(SemanticToken {
            delta_line: start.line - prev_line,
            delta_start: if start.line == prev_line {
                start.character - prev_col
            } else {
                start.character
            },
            length: sat(len),
            token_type: ty,
            token_modifiers_bitset: 0,
        });
        prev_line = start.line;
        prev_col = start.character;
    }
    SemanticTokensResult::Tokens(SemanticTokens {
        result_id: None,
        data,
    })
}

/// `textDocument/documentSymbol`: flat outline of the file's top-level items.
fn symbols(db: &AuraDatabase, doc: &Document) -> DocumentSymbolResponse {
    let file_parsed = parsed(db, doc.input);
    let items = file_items(db, doc.input);
    let text = doc.input.text(db);
    let mut out = Vec::new();
    for (i, item) in file_parsed.items.iter().enumerate() {
        let sig = items.items.get(i);
        let kind = match item {
            Item::Function(_) => SymbolKind::FUNCTION,
            Item::Struct(_) => SymbolKind::STRUCT,
            Item::Enum(_) => SymbolKind::ENUM,
            Item::Use { .. } | Item::ExternBlock { .. } => SymbolKind::NAMESPACE,
            Item::Error { .. } => continue,
        };
        let name = sig.and_then(ItemSig::name).map_or_else(
            || match item {
                Item::Use { .. } => "use".into(),
                Item::ExternBlock { .. } => format!("extern {}", abi_str(text, item.span())),
                _ => "<item>".into(),
            },
            str::to_owned,
        );
        let detail = sig.and_then(sig_detail);
        #[allow(deprecated)]
        out.push(DocumentSymbol {
            name,
            detail,
            kind,
            tags: None,
            deprecated: None,
            range: span_range(text, &doc.line_starts, item.span()),
            selection_range: name_selection(text, &doc.line_starts, item.span()),
            children: None,
        });
    }
    DocumentSymbolResponse::Nested(out)
}

/// Index of the fn item whose span contains `offset`, if any.
fn enclosing_fn(db: &AuraDatabase, doc: &Document, offset: u32) -> Option<u32> {
    for (i, item) in parsed(db, doc.input).items.iter().enumerate() {
        if let Item::Function(f) = item
            && f.body.is_some()
            && f.span.start <= offset
            && offset <= f.span.end
        {
            return u32::try_from(i).ok();
        }
    }
    None
}

// ----- text / position helpers --------------------------------------------------

/// Saturating `usize` → `u32` for byte offsets and positions.
fn sat(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Byte offsets of the first byte of every line.
fn line_starts(text: &str) -> Vec<u32> {
    let mut v = vec![0];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            v.push(sat(i + 1));
        }
    }
    v
}

/// Line index containing byte `offset`.
fn line_of(lines: &[u32], offset: u32) -> usize {
    match lines.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i - 1,
    }
}

/// Byte offset → LSP `Position` (UTF-16 code units from the line start).
fn offset_to_position(text: &str, lines: &[u32], offset: u32) -> Position {
    let offset = offset.min(sat(text.len()));
    let line = line_of(lines, offset);
    let start = lines.get(line).copied().unwrap_or(0) as usize;
    let col: usize = text[start..offset as usize]
        .chars()
        .map(char::len_utf16)
        .sum();
    Position::new(sat(line), sat(col))
}

/// LSP `Position` → byte offset. `None` if the line is out of range; clamps
/// to the line end when the character is past it.
fn pos_to_offset(text: &str, lines: &[u32], pos: Position) -> Option<u32> {
    let line = usize::try_from(pos.line).ok()?;
    let start = *lines.get(line)? as usize;
    let end = lines
        .get(line + 1)
        .map_or(text.len(), |e| (*e as usize).saturating_sub(1));
    let mut units = 0u32;
    for (i, ch) in text[start..end].char_indices() {
        if units >= pos.character {
            return u32::try_from(start + i).ok();
        }
        units += sat(ch.len_utf16());
    }
    Some(u32::try_from(end).unwrap_or_else(|_| sat(text.len())))
}

fn span_range(text: &str, lines: &[u32], span: Span) -> Range {
    Range::new(
        offset_to_position(text, lines, span.start),
        offset_to_position(text, lines, span.end),
    )
}

/// Convert one compiler diagnostic. `None` if it carries no span.
fn to_lsp_diagnostic(
    text: &str,
    lines: &[u32],
    uri: &Url,
    d: &aura_common::Diagnostic,
) -> Option<async_lsp::lsp_types::Diagnostic> {
    let span = d.span?;
    let mut message = d.message.clone();
    for note in &d.notes {
        message.push_str("\nnote: ");
        message.push_str(note);
    }
    let related: Vec<async_lsp::lsp_types::DiagnosticRelatedInformation> = d
        .labels
        .iter()
        .map(|l| async_lsp::lsp_types::DiagnosticRelatedInformation {
            location: Location {
                uri: uri.clone(),
                range: span_range(text, lines, l.span),
            },
            message: l.message.clone(),
        })
        .collect();
    Some(async_lsp::lsp_types::Diagnostic {
        range: span_range(text, lines, span),
        severity: Some(match d.severity {
            aura_common::Severity::Error => DiagnosticSeverity::ERROR,
            aura_common::Severity::Warning => DiagnosticSeverity::WARNING,
            aura_common::Severity::Note | aura_common::Severity::Help => DiagnosticSeverity::HINT,
        }),
        code: d
            .code
            .map(|c| async_lsp::lsp_types::NumberOrString::String(c.to_owned())),
        source: Some("aura".into()),
        message,
        related_information: (!related.is_empty()).then_some(related),
        tags: None,
        code_description: None,
        data: None,
    })
}

// ----- name helpers -------------------------------------------------------------

/// The abi string of an `extern "abi"` block, sliced from its span's source.
fn abi_str(text: &str, span: Span) -> String {
    let src = &text[span.start as usize..span.end as usize];
    let q = src.find('"');
    match q.and_then(|s| src[s + 1..].find('"').map(|e| &src[s + 1..s + 1 + e])) {
        Some(abi) => abi.to_owned(),
        None => "\"C\"".into(),
    }
}

/// A reasonable `selection_range`: the first identifier after the item's
/// leading keyword.
fn name_selection(text: &str, lines: &[u32], span: Span) -> Range {
    let src = &text[span.start as usize..span.end as usize];
    for kw in ["fn", "struct", "enum", "use", "extern"] {
        let Some(kwpos) = src.find(kw) else {
            continue;
        };
        let rest = &src[kwpos + kw.len()..];
        let bytes = rest.as_bytes();
        let mut i = 0;
        while i < bytes.len() && !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        if i > start {
            let off = span.start.saturating_add(sat(kwpos + kw.len() + start));
            return Range::new(
                offset_to_position(text, lines, off),
                offset_to_position(text, lines, off + sat(i - start)),
            );
        }
    }
    span_range(text, lines, span)
}

/// Signature detail string for a fn symbol, e.g. `(a: i64) -> i64`.
fn sig_detail(sig: &ItemSig) -> Option<String> {
    if let ItemSig::Fn { params, ret, .. } = sig {
        let ps: Vec<String> = params
            .iter()
            .map(|p| format!("{}: {}", p.name, ty_name_str(&p.ty)))
            .collect();
        let r = ret.as_ref().map_or_else(|| "()".into(), ty_name_str);
        Some(format!("({}) -> {r}", ps.join(", ")))
    } else {
        None
    }
}

fn ty_name_str(t: &TypeName) -> String {
    match t {
        TypeName::Error => "<?>".into(),
        TypeName::Unit => "()".into(),
        TypeName::Never => "!".into(),
        TypeName::Named { name, args } if args.is_empty() => name.clone(),
        TypeName::Named { name, args } => {
            let inner: Vec<String> = args.iter().map(ty_name_str).collect();
            format!("{name}<{}>", inner.join(", "))
        }
        TypeName::Pointer { mutable, pointee } => format!(
            "*{} {}",
            if *mutable { "mut" } else { "const" },
            ty_name_str(pointee)
        ),
        TypeName::Tuple(ts) => {
            let inner: Vec<String> = ts.iter().map(ty_name_str).collect();
            format!("({})", inner.join(", "))
        }
    }
}

// ----- tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "fn main() -> i64 {\n    let é: i64 = 1\n    é\n}\n";

    #[test]
    fn line_index() {
        let lines = line_starts(SRC);
        assert_eq!(lines[0], 0);
        assert_eq!(lines[1], 19); // after "fn main() -> i64 {\n"
        assert_eq!(lines[2], 39); // after "    let é: i64 = 1\n" (é = 2 bytes)
    }

    #[test]
    fn offset_to_pos_counts_utf16() {
        let lines = line_starts(SRC);
        // 'é' is U+00E9 — 1 UTF-16 unit, 2 UTF-8 bytes. Col of ':' after
        // "    let é" = 4+3+1+1 = 9 utf16 units.
        let colon = sat(SRC.find(':').unwrap());
        let pos = offset_to_position(SRC, &lines, colon);
        assert_eq!(pos, Position::new(1, 9));
    }

    #[test]
    fn pos_to_offset_decodes_utf16() {
        let lines = line_starts(SRC);
        // Position (1,9) → the ':' byte offset.
        let off = pos_to_offset(SRC, &lines, Position::new(1, 9)).unwrap();
        assert_eq!(SRC.as_bytes()[off as usize], b':');
        // Past-end character clamps to the line's last byte.
        let clamped = pos_to_offset(SRC, &lines, Position::new(1, 999)).unwrap();
        assert!(clamped as usize <= SRC.len());
        // Out-of-range line → None.
        assert!(pos_to_offset(SRC, &lines, Position::new(99, 0)).is_none());
    }

    #[test]
    fn pos_roundtrip() {
        let lines = line_starts(SRC);
        for off in [0u32, 5, 21, 27, 45] {
            let pos = offset_to_position(SRC, &lines, off);
            let back = pos_to_offset(SRC, &lines, pos).unwrap();
            assert_eq!(back, off, "roundtrip failed for {off}");
        }
    }

    #[test]
    fn emoji_is_two_utf16_units() {
        // U+1F600 = surrogate pair = 2 UTF-16 units, 4 UTF-8 bytes.
        let src = "fn f() {\n    \u{1F600}\n}\n";
        let lines = line_starts(src);
        let pos = offset_to_position(src, &lines, sat(src.find('\n').unwrap()) + 5 + 4);
        // char after emoji: utf16 col = 4 spaces + 2 units = 6
        assert_eq!(pos.character, 6);
    }
}
