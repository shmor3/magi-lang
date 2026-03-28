//! LSP protocol types for the MAGI language server.
//!
//! Self-contained type definitions that mirror the subset of the Language Server
//! Protocol used by the MAGI LSP implementation.  These types are intentionally
//! simple structs/enums with public fields.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;


/// A URI (typically `file:///...`) used throughout the LSP protocol.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Url(String);

impl Url {
    /// Parse a string into a `Url`.
    pub fn parse(s: &str) -> Result<Self, String> {
        Ok(Url(s.to_string()))
    }

    /// Create a `Url` from a filesystem path.
    pub fn from_file_path(path: &std::path::Path) -> Result<Self, ()> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_err(|_| ())?.join(path)
        };
        let s = abs.to_string_lossy();
        Ok(Url(format!("file://{}", s)))
    }

    /// Convert back to a filesystem path.
    pub fn to_file_path(&self) -> Result<PathBuf, ()> {
        if let Some(rest) = self.0.strip_prefix("file://") {
            Ok(PathBuf::from(rest))
        } else {
            Err(())
        }
    }

    /// The URI scheme (e.g. `"file"`).
    pub fn scheme(&self) -> &str {
        self.0.split(':').next().unwrap_or("")
    }

    /// Return the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}


/// A zero-based position in a text document (line + character as UTF-16 offset).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Position {
    pub fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

/// A range in a text document expressed as start and end `Position`s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }
}

/// A location inside a resource (URI + range).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub uri: Url,
    pub range: Range,
}

/// A textual edit applicable to a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

/// A workspace-wide edit that may touch multiple documents.
#[derive(Clone, Debug, Default)]
pub struct WorkspaceEdit {
    pub changes: Option<HashMap<Url, Vec<TextEdit>>>,
    pub document_changes: Option<Vec<TextDocumentEdit>>,
}

/// An edit on a single versioned document.
#[derive(Clone, Debug)]
pub struct TextDocumentEdit {
    pub text_document: VersionedTextDocumentIdentifier,
    pub edits: Vec<TextEdit>,
}


/// A diagnostic (error, warning, etc.) in a document.
#[derive(Clone, Debug, Default)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Option<DiagnosticSeverity>,
    pub code: Option<NumberOrString>,
    pub code_description: Option<CodeDescription>,
    pub source: Option<String>,
    pub message: String,
    pub related_information: Option<Vec<DiagnosticRelatedInformation>>,
    pub tags: Option<Vec<DiagnosticTag>>,
    pub data: Option<crate::util::JsonValue>,
}

/// Diagnostic severity constants (matching the LSP numeric values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiagnosticSeverity(pub u32);

impl DiagnosticSeverity {
    pub const ERROR: DiagnosticSeverity = DiagnosticSeverity(1);
    pub const WARNING: DiagnosticSeverity = DiagnosticSeverity(2);
    pub const INFORMATION: DiagnosticSeverity = DiagnosticSeverity(3);
    pub const HINT: DiagnosticSeverity = DiagnosticSeverity(4);
}

/// Diagnostic tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiagnosticTag(pub u32);

impl DiagnosticTag {
    pub const UNNECESSARY: DiagnosticTag = DiagnosticTag(1);
    pub const DEPRECATED: DiagnosticTag = DiagnosticTag(2);
}

/// Either a number or a string (used for diagnostic codes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NumberOrString {
    Number(i64),
    String(String),
}

/// Code description (a href for the diagnostic code).
#[derive(Clone, Debug)]
pub struct CodeDescription {
    pub href: String,
}

/// Related information for a diagnostic.
#[derive(Clone, Debug)]
pub struct DiagnosticRelatedInformation {
    pub location: Location,
    pub message: String,
}


/// Identifies a text document by its URI.
#[derive(Clone, Debug)]
pub struct TextDocumentIdentifier {
    pub uri: Url,
}

/// Identifies a text document by URI and version number.
#[derive(Clone, Debug)]
pub struct VersionedTextDocumentIdentifier {
    pub uri: Url,
    pub version: i32,
}

/// An item describing a text document.
#[derive(Clone, Debug)]
pub struct TextDocumentItem {
    pub uri: Url,
    pub language_id: String,
    pub version: i32,
    pub text: String,
}

/// How the server synchronizes document content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextDocumentSyncKind(pub u32);

impl TextDocumentSyncKind {
    pub const NONE: TextDocumentSyncKind = TextDocumentSyncKind(0);
    pub const FULL: TextDocumentSyncKind = TextDocumentSyncKind(1);
    pub const INCREMENTAL: TextDocumentSyncKind = TextDocumentSyncKind(2);
}

/// Text document sync capability.
#[derive(Clone, Debug)]
pub enum TextDocumentSyncCapability {
    Kind(TextDocumentSyncKind),
    Options(TextDocumentSyncOptions),
}

/// Detailed text document sync options.
#[derive(Clone, Debug, Default)]
pub struct TextDocumentSyncOptions {
    pub open_close: Option<bool>,
    pub change: Option<TextDocumentSyncKind>,
}

/// A content change event for a document.
#[derive(Clone, Debug)]
pub struct TextDocumentContentChangeEvent {
    pub range: Option<Range>,
    pub text: String,
}

// Common param helper types

/// Work-done progress parameters (placeholder, always `Default`).
#[derive(Clone, Debug, Default)]
pub struct WorkDoneProgressParams {}

/// Partial result parameters (placeholder, always `Default`).
#[derive(Clone, Debug, Default)]
pub struct PartialResultParams {}

/// Work-done progress options (placeholder, always `Default`).
#[derive(Clone, Debug, Default)]
pub struct WorkDoneProgressOptions {}


/// Parameters for the `initialize` request.
#[derive(Clone, Debug, Default)]
pub struct InitializeParams {
    pub root_uri: Option<Url>,
    pub capabilities: ClientCapabilities,
    pub workspace_folders: Option<Vec<WorkspaceFolder>>,
}

/// Client capabilities (opaque for our purposes).
#[derive(Clone, Debug, Default)]
pub struct ClientCapabilities {}

/// A workspace folder.
#[derive(Clone, Debug)]
pub struct WorkspaceFolder {
    pub uri: Url,
    pub name: String,
}

/// Parameters for the `initialized` notification.
#[derive(Clone, Debug, Default)]
pub struct InitializedParams {}

/// Parameters for `textDocument/didOpen`.
#[derive(Clone, Debug)]
pub struct DidOpenTextDocumentParams {
    pub text_document: TextDocumentItem,
}

/// Parameters for `textDocument/didChange`.
#[derive(Clone, Debug)]
pub struct DidChangeTextDocumentParams {
    pub text_document: VersionedTextDocumentIdentifier,
    pub content_changes: Vec<TextDocumentContentChangeEvent>,
}

/// Parameters for `textDocument/didClose`.
#[derive(Clone, Debug)]
pub struct DidCloseTextDocumentParams {
    pub text_document: TextDocumentIdentifier,
}

/// A text document + position pair.
#[derive(Clone, Debug)]
pub struct TextDocumentPositionParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

/// Parameters for `textDocument/hover`.
#[derive(Clone, Debug)]
pub struct HoverParams {
    pub text_document_position_params: TextDocumentPositionParams,
    pub work_done_progress_params: WorkDoneProgressParams,
}

/// Parameters for `textDocument/definition`.
#[derive(Clone, Debug)]
pub struct GotoDefinitionParams {
    pub text_document_position_params: TextDocumentPositionParams,
    pub work_done_progress_params: WorkDoneProgressParams,
    pub partial_result_params: PartialResultParams,
}

/// Parameters for `textDocument/completion`.
#[derive(Clone, Debug)]
pub struct CompletionParams {
    pub text_document_position: TextDocumentPositionParams,
    pub context: Option<CompletionContext>,
    pub work_done_progress_params: WorkDoneProgressParams,
    pub partial_result_params: PartialResultParams,
}

/// Additional context for a completion request.
#[derive(Clone, Debug)]
pub struct CompletionContext {
    pub trigger_kind: u32,
    pub trigger_character: Option<String>,
}

/// Parameters for `textDocument/codeAction`.
#[derive(Clone, Debug)]
pub struct CodeActionParams {
    pub text_document: TextDocumentIdentifier,
    pub range: Range,
    pub context: CodeActionContext,
    pub work_done_progress_params: WorkDoneProgressParams,
    pub partial_result_params: PartialResultParams,
}

/// Additional context for a code-action request.
#[derive(Clone, Debug)]
pub struct CodeActionContext {
    pub diagnostics: Vec<Diagnostic>,
    pub only: Option<Vec<CodeActionKind>>,
    pub trigger_kind: Option<u32>,
}

/// Parameters for `textDocument/signatureHelp`.
#[derive(Clone, Debug)]
pub struct SignatureHelpParams {
    pub text_document_position_params: TextDocumentPositionParams,
    pub context: Option<SignatureHelpContext>,
    pub work_done_progress_params: WorkDoneProgressParams,
}

/// Additional context for a signature help request.
#[derive(Clone, Debug)]
pub struct SignatureHelpContext {
    pub trigger_kind: u32,
    pub trigger_character: Option<String>,
    pub is_retrigger: bool,
}

/// Parameters for `textDocument/documentSymbol`.
#[derive(Clone, Debug)]
pub struct DocumentSymbolParams {
    pub text_document: TextDocumentIdentifier,
}

/// Parameters for `textDocument/formatting`.
#[derive(Clone, Debug)]
pub struct DocumentFormattingParams {
    pub text_document: TextDocumentIdentifier,
    pub options: FormattingOptions,
}

/// Formatting options.
#[derive(Clone, Debug, Default)]
pub struct FormattingOptions {
    pub tab_size: u32,
    pub insert_spaces: bool,
}

/// Parameters for `textDocument/codeLens`.
#[derive(Clone, Debug)]
pub struct CodeLensParams {
    pub text_document: TextDocumentIdentifier,
}

/// Parameters for `textDocument/foldingRange`.
#[derive(Clone, Debug)]
pub struct FoldingRangeParams {
    pub text_document: TextDocumentIdentifier,
}

/// Parameters for `textDocument/selectionRange`.
#[derive(Clone, Debug)]
pub struct SelectionRangeParams {
    pub text_document: TextDocumentIdentifier,
    pub positions: Vec<Position>,
}

/// Parameters for `textDocument/documentLink`.
#[derive(Clone, Debug)]
pub struct DocumentLinkParams {
    pub text_document: TextDocumentIdentifier,
}

/// Parameters for `textDocument/rename`.
#[derive(Clone, Debug)]
pub struct RenameParams {
    pub text_document_position: TextDocumentPositionParams,
    pub new_name: String,
    pub work_done_progress_params: WorkDoneProgressParams,
}

/// Parameters for `textDocument/references`.
#[derive(Clone, Debug)]
pub struct ReferenceParams {
    pub text_document_position: TextDocumentPositionParams,
    pub context: ReferenceContext,
    pub work_done_progress_params: WorkDoneProgressParams,
    pub partial_result_params: PartialResultParams,
}

/// Additional context for a references request.
#[derive(Clone, Debug)]
pub struct ReferenceContext {
    pub include_declaration: bool,
}

/// Parameters for `callHierarchy/prepare`.
#[derive(Clone, Debug)]
pub struct CallHierarchyPrepareParams {
    pub text_document_position_params: TextDocumentPositionParams,
    pub work_done_progress_params: WorkDoneProgressParams,
}

/// Parameters for `callHierarchy/incomingCalls`.
#[derive(Clone, Debug)]
pub struct CallHierarchyIncomingCallsParams {
    pub item: CallHierarchyItem,
    pub work_done_progress_params: WorkDoneProgressParams,
    pub partial_result_params: PartialResultParams,
}

/// Parameters for `callHierarchy/outgoingCalls`.
#[derive(Clone, Debug)]
pub struct CallHierarchyOutgoingCallsParams {
    pub item: CallHierarchyItem,
    pub work_done_progress_params: WorkDoneProgressParams,
    pub partial_result_params: PartialResultParams,
}

/// Parameters for `textDocument/semanticTokens/full`.
#[derive(Clone, Debug)]
pub struct SemanticTokensParams {
    pub text_document: TextDocumentIdentifier,
}

/// Parameters for `textDocument/inlayHint`.
#[derive(Clone, Debug)]
pub struct InlayHintParams {
    pub text_document: TextDocumentIdentifier,
    pub range: Range,
}

/// Parameters for `textDocument/linkedEditingRange`.
#[derive(Clone, Debug)]
pub struct LinkedEditingRangeParams {
    pub text_document_position_params: TextDocumentPositionParams,
    pub work_done_progress_params: WorkDoneProgressParams,
}

/// Parameters for `workspace/symbol`.
#[derive(Clone, Debug)]
pub struct WorkspaceSymbolParams {
    pub query: String,
}


/// Result for the `initialize` request.
#[derive(Clone, Debug)]
pub struct InitializeResult {
    pub capabilities: ServerCapabilities,
    pub server_info: Option<ServerInfo>,
}

/// Server metadata.
#[derive(Clone, Debug)]
pub struct ServerInfo {
    pub name: String,
    pub version: Option<String>,
}

/// The capabilities the server announces.
#[derive(Clone, Debug, Default)]
pub struct ServerCapabilities {
    pub text_document_sync: Option<TextDocumentSyncCapability>,
    pub hover_provider: Option<HoverProviderCapability>,
    pub completion_provider: Option<CompletionOptions>,
    pub signature_help_provider: Option<SignatureHelpOptions>,
    pub definition_provider: Option<OneOf<bool, ()>>,
    pub references_provider: Option<OneOf<bool, ()>>,
    pub document_symbol_provider: Option<OneOf<bool, ()>>,
    pub document_formatting_provider: Option<OneOf<bool, ()>>,
    pub workspace_symbol_provider: Option<OneOf<bool, ()>>,
    pub code_action_provider: Option<CodeActionProviderCapability>,
    pub code_lens_provider: Option<CodeLensOptions>,
    pub document_link_provider: Option<DocumentLinkOptions>,
    pub selection_range_provider: Option<SelectionRangeProviderCapability>,
    pub linked_editing_range_provider: Option<LinkedEditingRangeServerCapabilities>,
    pub call_hierarchy_provider: Option<CallHierarchyServerCapability>,
    pub rename_provider: Option<OneOf<bool, ()>>,
    pub inlay_hint_provider: Option<OneOf<bool, InlayHintServerCapabilities>>,
    pub semantic_tokens_provider: Option<SemanticTokensServerCapabilities>,
    pub folding_range_provider: Option<FoldingRangeProviderCapability>,
    pub document_highlight_provider: Option<OneOf<bool, ()>>,
    pub document_range_formatting_provider: Option<OneOf<bool, ()>>,
}

/// Either `Left(L)` or `Right(R)` (used for simple bool capabilities).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OneOf<L, R> {
    Left(L),
    Right(R),
}


/// Hover provider capability.
#[derive(Clone, Debug)]
pub enum HoverProviderCapability {
    Simple(bool),
}

/// The result of a hover request.
#[derive(Clone, Debug)]
pub struct Hover {
    pub contents: HoverContents,
    pub range: Option<Range>,
}

/// Hover content variants.
#[derive(Clone, Debug)]
pub enum HoverContents {
    Markup(MarkupContent),
    Array(Vec<MarkedString>),
    Scalar(MarkedString),
}

/// A marked string (used in older hover responses).
#[derive(Clone, Debug)]
pub enum MarkedString {
    String(String),
    LanguageString { language: String, value: String },
}

/// Rich markup content.
#[derive(Clone, Debug)]
pub struct MarkupContent {
    pub kind: MarkupKind,
    pub value: String,
}

/// The kind of markup (plaintext or Markdown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkupKind {
    PlainText,
    Markdown,
}


/// Completion options for the server capabilities.
#[derive(Clone, Debug, Default)]
pub struct CompletionOptions {
    pub trigger_characters: Option<Vec<String>>,
    pub resolve_provider: Option<bool>,
    pub work_done_progress_options: WorkDoneProgressOptions,
}

/// Completion response.
#[derive(Clone, Debug)]
pub enum CompletionResponse {
    Array(Vec<CompletionItem>),
    List(CompletionList),
}

/// A completion list.
#[derive(Clone, Debug)]
pub struct CompletionList {
    pub is_incomplete: bool,
    pub items: Vec<CompletionItem>,
}

/// A single completion item.
#[derive(Clone, Debug, Default)]
pub struct CompletionItem {
    pub label: String,
    pub kind: Option<CompletionItemKind>,
    pub detail: Option<String>,
    pub documentation: Option<Documentation>,
    pub deprecated: Option<bool>,
    pub preselect: Option<bool>,
    pub sort_text: Option<String>,
    pub filter_text: Option<String>,
    pub insert_text: Option<String>,
    pub insert_text_format: Option<InsertTextFormat>,
    pub text_edit: Option<CompletionTextEdit>,
    pub additional_text_edits: Option<Vec<TextEdit>>,
    pub command: Option<Command>,
    pub data: Option<crate::util::JsonValue>,
}

/// Completion item kind constants (matching LSP spec numeric values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CompletionItemKind(pub u32);

impl CompletionItemKind {
    pub const TEXT: CompletionItemKind = CompletionItemKind(1);
    pub const METHOD: CompletionItemKind = CompletionItemKind(2);
    pub const FUNCTION: CompletionItemKind = CompletionItemKind(3);
    pub const CONSTRUCTOR: CompletionItemKind = CompletionItemKind(4);
    pub const FIELD: CompletionItemKind = CompletionItemKind(5);
    pub const VARIABLE: CompletionItemKind = CompletionItemKind(6);
    pub const CLASS: CompletionItemKind = CompletionItemKind(7);
    pub const INTERFACE: CompletionItemKind = CompletionItemKind(8);
    pub const MODULE: CompletionItemKind = CompletionItemKind(9);
    pub const PROPERTY: CompletionItemKind = CompletionItemKind(10);
    pub const UNIT: CompletionItemKind = CompletionItemKind(11);
    pub const VALUE: CompletionItemKind = CompletionItemKind(12);
    pub const ENUM: CompletionItemKind = CompletionItemKind(13);
    pub const KEYWORD: CompletionItemKind = CompletionItemKind(14);
    pub const SNIPPET: CompletionItemKind = CompletionItemKind(15);
    pub const COLOR: CompletionItemKind = CompletionItemKind(16);
    pub const FILE: CompletionItemKind = CompletionItemKind(17);
    pub const REFERENCE: CompletionItemKind = CompletionItemKind(18);
    pub const FOLDER: CompletionItemKind = CompletionItemKind(19);
    pub const ENUM_MEMBER: CompletionItemKind = CompletionItemKind(20);
    pub const CONSTANT: CompletionItemKind = CompletionItemKind(21);
    pub const STRUCT: CompletionItemKind = CompletionItemKind(22);
    pub const EVENT: CompletionItemKind = CompletionItemKind(23);
    pub const OPERATOR: CompletionItemKind = CompletionItemKind(24);
    pub const TYPE_PARAMETER: CompletionItemKind = CompletionItemKind(25);
}

/// Insert text format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InsertTextFormat(pub u32);

impl InsertTextFormat {
    pub const PLAIN_TEXT: InsertTextFormat = InsertTextFormat(1);
    pub const SNIPPET: InsertTextFormat = InsertTextFormat(2);
}

/// Documentation content (string or markup).
#[derive(Clone, Debug)]
pub enum Documentation {
    String(String),
    MarkupContent(MarkupContent),
}

/// A completion text edit (inline or snippet).
#[derive(Clone, Debug)]
pub enum CompletionTextEdit {
    Edit(TextEdit),
    InsertAndReplace(InsertReplaceEdit),
}

/// An insert/replace edit.
#[derive(Clone, Debug)]
pub struct InsertReplaceEdit {
    pub new_text: String,
    pub insert: Range,
    pub replace: Range,
}


/// Go-to-definition response.
#[derive(Clone, Debug)]
pub enum GotoDefinitionResponse {
    Scalar(Location),
    Array(Vec<Location>),
    Link(Vec<LocationLink>),
}

/// A location link (used in definition responses).
#[derive(Clone, Debug)]
pub struct LocationLink {
    pub origin_selection_range: Option<Range>,
    pub target_uri: Url,
    pub target_range: Range,
    pub target_selection_range: Range,
}


/// Document symbol response.
#[derive(Clone, Debug)]
pub enum DocumentSymbolResponse {
    Flat(Vec<SymbolInformation>),
    Nested(Vec<DocumentSymbol>),
}

/// Symbol information (flat style).
#[derive(Clone, Debug)]
pub struct SymbolInformation {
    pub name: String,
    pub kind: SymbolKind,
    pub tags: Option<Vec<SymbolTag>>,
    pub deprecated: Option<bool>,
    pub location: Location,
    pub container_name: Option<String>,
}

/// A symbol tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolTag(pub u32);

impl SymbolTag {
    pub const DEPRECATED: SymbolTag = SymbolTag(1);
}

/// A hierarchical document symbol.
#[derive(Clone, Debug)]
pub struct DocumentSymbol {
    pub name: String,
    pub detail: Option<String>,
    pub kind: SymbolKind,
    pub tags: Option<Vec<SymbolTag>>,
    pub deprecated: Option<bool>,
    pub range: Range,
    pub selection_range: Range,
    pub children: Option<Vec<DocumentSymbol>>,
}

/// Symbol kind constants (matching LSP spec numeric values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SymbolKind(pub u32);

impl SymbolKind {
    pub const FILE: SymbolKind = SymbolKind(1);
    pub const MODULE: SymbolKind = SymbolKind(2);
    pub const NAMESPACE: SymbolKind = SymbolKind(3);
    pub const PACKAGE: SymbolKind = SymbolKind(4);
    pub const CLASS: SymbolKind = SymbolKind(5);
    pub const METHOD: SymbolKind = SymbolKind(6);
    pub const PROPERTY: SymbolKind = SymbolKind(7);
    pub const FIELD: SymbolKind = SymbolKind(8);
    pub const CONSTRUCTOR: SymbolKind = SymbolKind(9);
    pub const ENUM: SymbolKind = SymbolKind(10);
    pub const INTERFACE: SymbolKind = SymbolKind(11);
    pub const FUNCTION: SymbolKind = SymbolKind(12);
    pub const VARIABLE: SymbolKind = SymbolKind(13);
    pub const CONSTANT: SymbolKind = SymbolKind(14);
    pub const STRING: SymbolKind = SymbolKind(15);
    pub const NUMBER: SymbolKind = SymbolKind(16);
    pub const BOOLEAN: SymbolKind = SymbolKind(17);
    pub const ARRAY: SymbolKind = SymbolKind(18);
    pub const OBJECT: SymbolKind = SymbolKind(19);
    pub const KEY: SymbolKind = SymbolKind(20);
    pub const NULL: SymbolKind = SymbolKind(21);
    pub const ENUM_MEMBER: SymbolKind = SymbolKind(22);
    pub const STRUCT: SymbolKind = SymbolKind(23);
    pub const EVENT: SymbolKind = SymbolKind(24);
    pub const OPERATOR: SymbolKind = SymbolKind(25);
    pub const TYPE_PARAMETER: SymbolKind = SymbolKind(26);
}


/// Code action response.
pub type CodeActionResponse = Vec<CodeActionOrCommand>;

/// Either a code action or a command.
#[derive(Clone, Debug)]
pub enum CodeActionOrCommand {
    CodeAction(CodeAction),
    Command(Command),
}

/// A code action.
#[derive(Clone, Debug, Default)]
pub struct CodeAction {
    pub title: String,
    pub kind: Option<CodeActionKind>,
    pub diagnostics: Option<Vec<Diagnostic>>,
    pub is_preferred: Option<bool>,
    pub disabled: Option<CodeActionDisabled>,
    pub edit: Option<WorkspaceEdit>,
    pub command: Option<Command>,
    pub data: Option<crate::util::JsonValue>,
}

/// Why a code action is disabled.
#[derive(Clone, Debug)]
pub struct CodeActionDisabled {
    pub reason: String,
}

/// Code action kind (string-based per the LSP spec).
///
/// Wraps a `&'static str` so constants can be defined without heap allocation.
/// Use [`CodeActionKind::as_str`] to get the string value.
#[derive(Clone, Debug)]
pub struct CodeActionKind(&'static str);

impl CodeActionKind {
    pub const EMPTY: CodeActionKind = CodeActionKind("");
    pub const QUICKFIX: CodeActionKind = CodeActionKind("quickfix");
    pub const REFACTOR: CodeActionKind = CodeActionKind("refactor");
    pub const REFACTOR_EXTRACT: CodeActionKind = CodeActionKind("refactor.extract");
    pub const REFACTOR_INLINE: CodeActionKind = CodeActionKind("refactor.inline");
    pub const REFACTOR_REWRITE: CodeActionKind = CodeActionKind("refactor.rewrite");
    pub const SOURCE: CodeActionKind = CodeActionKind("source");
    pub const SOURCE_ORGANIZE_IMPORTS: CodeActionKind = CodeActionKind("source.organizeImports");
    pub const SOURCE_FIX_ALL: CodeActionKind = CodeActionKind("source.fixAll");

    /// The string value of this code action kind.
    pub fn as_str(&self) -> &str {
        self.0
    }
}

impl PartialEq for CodeActionKind {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for CodeActionKind {}

impl std::hash::Hash for CodeActionKind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Display for CodeActionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// A command that can be executed by the editor.
#[derive(Clone, Debug)]
pub struct Command {
    pub title: String,
    pub command: String,
    pub arguments: Option<Vec<crate::util::JsonValue>>,
}

/// Code action provider capability.
#[derive(Clone, Debug)]
pub enum CodeActionProviderCapability {
    Simple(bool),
    Options(CodeActionOptions),
}

/// Code action options.
#[derive(Clone, Debug)]
pub struct CodeActionOptions {
    pub code_action_kinds: Option<Vec<CodeActionKind>>,
}


/// A code lens.
#[derive(Clone, Debug)]
pub struct CodeLens {
    pub range: Range,
    pub command: Option<Command>,
    pub data: Option<crate::util::JsonValue>,
}

/// Code lens options.
#[derive(Clone, Debug)]
pub struct CodeLensOptions {
    pub resolve_provider: Option<bool>,
}


/// A folding range.
#[derive(Clone, Debug)]
pub struct FoldingRange {
    pub start_line: u32,
    pub start_character: Option<u32>,
    pub end_line: u32,
    pub end_character: Option<u32>,
    pub kind: Option<FoldingRangeKind>,
    pub collapsed_text: Option<String>,
}

/// Folding range kind (string-based per the LSP spec).
#[derive(Clone, Debug)]
pub struct FoldingRangeKind(&'static str);

impl FoldingRangeKind {
    pub const COMMENT: FoldingRangeKind = FoldingRangeKind("comment");
    pub const IMPORTS: FoldingRangeKind = FoldingRangeKind("imports");
    pub const REGION: FoldingRangeKind = FoldingRangeKind("region");

    pub fn as_str(&self) -> &str {
        self.0
    }
}

impl PartialEq for FoldingRangeKind {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for FoldingRangeKind {}

impl fmt::Display for FoldingRangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Folding range provider capability.
#[derive(Clone, Debug)]
pub enum FoldingRangeProviderCapability {
    Simple(bool),
}


/// A selection range with optional parent.
#[derive(Clone, Debug)]
pub struct SelectionRange {
    pub range: Range,
    pub parent: Option<Box<SelectionRange>>,
}

/// Selection range provider capability.
#[derive(Clone, Debug)]
pub enum SelectionRangeProviderCapability {
    Simple(bool),
}


/// An inlay hint.
#[derive(Clone, Debug)]
pub struct InlayHint {
    pub position: Position,
    pub label: InlayHintLabel,
    pub kind: Option<InlayHintKind>,
    pub text_edits: Option<Vec<TextEdit>>,
    pub tooltip: Option<InlayHintTooltip>,
    pub padding_left: Option<bool>,
    pub padding_right: Option<bool>,
    pub data: Option<crate::util::JsonValue>,
}

/// Inlay hint label (a plain string or label parts).
#[derive(Clone, Debug)]
pub enum InlayHintLabel {
    String(String),
    LabelParts(Vec<InlayHintLabelPart>),
}

/// A part of an inlay hint label.
#[derive(Clone, Debug)]
pub struct InlayHintLabelPart {
    pub value: String,
    pub tooltip: Option<InlayHintTooltip>,
    pub location: Option<Location>,
    pub command: Option<Command>,
}

/// Tooltip content for an inlay hint.
#[derive(Clone, Debug)]
pub enum InlayHintTooltip {
    String(String),
    MarkupContent(MarkupContent),
}

/// Inlay hint kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlayHintKind(pub u32);

impl InlayHintKind {
    pub const TYPE: InlayHintKind = InlayHintKind(1);
    pub const PARAMETER: InlayHintKind = InlayHintKind(2);
}

/// Inlay hint options.
#[derive(Clone, Debug, Default)]
pub struct InlayHintOptions {
    pub resolve_provider: Option<bool>,
    pub work_done_progress_options: WorkDoneProgressOptions,
}

/// Inlay hint server capabilities.
#[derive(Clone, Debug)]
pub enum InlayHintServerCapabilities {
    Options(InlayHintOptions),
}


/// Signature help response.
#[derive(Clone, Debug)]
pub struct SignatureHelp {
    pub signatures: Vec<SignatureInformation>,
    pub active_signature: Option<u32>,
    pub active_parameter: Option<u32>,
}

/// Information about a function signature.
#[derive(Clone, Debug)]
pub struct SignatureInformation {
    pub label: String,
    pub documentation: Option<Documentation>,
    pub parameters: Option<Vec<ParameterInformation>>,
    pub active_parameter: Option<u32>,
}

/// Information about a parameter.
#[derive(Clone, Debug)]
pub struct ParameterInformation {
    pub label: ParameterLabel,
    pub documentation: Option<Documentation>,
}

/// A parameter label (simple string or offset pair).
#[derive(Clone, Debug)]
pub enum ParameterLabel {
    Simple(String),
    Offsets([u32; 2]),
}

/// Signature help options for server capabilities.
#[derive(Clone, Debug, Default)]
pub struct SignatureHelpOptions {
    pub trigger_characters: Option<Vec<String>>,
    pub retrigger_characters: Option<Vec<String>>,
    pub work_done_progress_options: WorkDoneProgressOptions,
}


/// A document link.
#[derive(Clone, Debug)]
pub struct DocumentLink {
    pub range: Range,
    pub target: Option<Url>,
    pub tooltip: Option<String>,
    pub data: Option<crate::util::JsonValue>,
}

/// Document link options.
#[derive(Clone, Debug, Default)]
pub struct DocumentLinkOptions {
    pub resolve_provider: Option<bool>,
    pub work_done_progress_options: WorkDoneProgressOptions,
}


/// Semantic tokens result.
#[derive(Clone, Debug)]
pub enum SemanticTokensResult {
    Tokens(SemanticTokens),
    Partial(SemanticTokensPartialResult),
}

/// Semantic tokens partial result.
#[derive(Clone, Debug)]
pub struct SemanticTokensPartialResult {
    pub data: Vec<SemanticToken>,
}

/// Full semantic tokens data.
#[derive(Clone, Debug)]
pub struct SemanticTokens {
    pub result_id: Option<String>,
    pub data: Vec<SemanticToken>,
}

/// A single semantic token (delta-encoded).
#[derive(Clone, Copy, Debug)]
pub struct SemanticToken {
    pub delta_line: u32,
    pub delta_start: u32,
    pub length: u32,
    pub token_type: u32,
    pub token_modifiers_bitset: u32,
}

/// Semantic token type (string-based per the LSP spec).
///
/// Used in [`SemanticTokensLegend`] to declare supported token types.
#[derive(Clone, Debug)]
pub struct SemanticTokenType(&'static str);

impl SemanticTokenType {
    pub const NAMESPACE: SemanticTokenType = SemanticTokenType("namespace");
    pub const TYPE: SemanticTokenType = SemanticTokenType("type");
    pub const CLASS: SemanticTokenType = SemanticTokenType("class");
    pub const ENUM: SemanticTokenType = SemanticTokenType("enum");
    pub const INTERFACE: SemanticTokenType = SemanticTokenType("interface");
    pub const STRUCT: SemanticTokenType = SemanticTokenType("struct");
    pub const TYPE_PARAMETER: SemanticTokenType = SemanticTokenType("typeParameter");
    pub const PARAMETER: SemanticTokenType = SemanticTokenType("parameter");
    pub const VARIABLE: SemanticTokenType = SemanticTokenType("variable");
    pub const PROPERTY: SemanticTokenType = SemanticTokenType("property");
    pub const ENUM_MEMBER: SemanticTokenType = SemanticTokenType("enumMember");
    pub const EVENT: SemanticTokenType = SemanticTokenType("event");
    pub const FUNCTION: SemanticTokenType = SemanticTokenType("function");
    pub const METHOD: SemanticTokenType = SemanticTokenType("method");
    pub const MACRO: SemanticTokenType = SemanticTokenType("macro");
    pub const KEYWORD: SemanticTokenType = SemanticTokenType("keyword");
    pub const MODIFIER: SemanticTokenType = SemanticTokenType("modifier");
    pub const COMMENT: SemanticTokenType = SemanticTokenType("comment");
    pub const STRING: SemanticTokenType = SemanticTokenType("string");
    pub const NUMBER: SemanticTokenType = SemanticTokenType("number");
    pub const REGEXP: SemanticTokenType = SemanticTokenType("regexp");
    pub const OPERATOR: SemanticTokenType = SemanticTokenType("operator");
    pub const DECORATOR: SemanticTokenType = SemanticTokenType("decorator");

    pub fn as_str(&self) -> &str {
        self.0
    }
}

impl PartialEq for SemanticTokenType {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for SemanticTokenType {}

impl std::hash::Hash for SemanticTokenType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Display for SemanticTokenType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Semantic token modifier (string-based per the LSP spec).
#[derive(Clone, Debug)]
pub struct SemanticTokenModifier(&'static str);

impl SemanticTokenModifier {
    pub const DECLARATION: SemanticTokenModifier = SemanticTokenModifier("declaration");
    pub const DEFINITION: SemanticTokenModifier = SemanticTokenModifier("definition");
    pub const READONLY: SemanticTokenModifier = SemanticTokenModifier("readonly");
    pub const STATIC: SemanticTokenModifier = SemanticTokenModifier("static");
    pub const DEPRECATED: SemanticTokenModifier = SemanticTokenModifier("deprecated");
    pub const ABSTRACT: SemanticTokenModifier = SemanticTokenModifier("abstract");
    pub const ASYNC: SemanticTokenModifier = SemanticTokenModifier("async");
    pub const MODIFICATION: SemanticTokenModifier = SemanticTokenModifier("modification");
    pub const DOCUMENTATION: SemanticTokenModifier = SemanticTokenModifier("documentation");
    pub const DEFAULT_LIBRARY: SemanticTokenModifier = SemanticTokenModifier("defaultLibrary");

    pub fn as_str(&self) -> &str {
        self.0
    }
}

impl PartialEq for SemanticTokenModifier {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for SemanticTokenModifier {}

impl std::hash::Hash for SemanticTokenModifier {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Display for SemanticTokenModifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Semantic tokens options.
#[derive(Clone, Debug)]
pub struct SemanticTokensOptions {
    pub legend: SemanticTokensLegend,
    pub full: Option<SemanticTokensFullOptions>,
    pub range: Option<bool>,
    pub work_done_progress_options: WorkDoneProgressOptions,
}

/// Semantic tokens legend (declares supported token types and modifiers).
#[derive(Clone, Debug)]
pub struct SemanticTokensLegend {
    pub token_types: Vec<SemanticTokenType>,
    pub token_modifiers: Vec<SemanticTokenModifier>,
}

/// Full semantic tokens options.
#[derive(Clone, Debug)]
pub enum SemanticTokensFullOptions {
    Bool(bool),
    Delta { delta: Option<bool> },
}

/// Semantic tokens server capabilities.
#[derive(Clone, Debug)]
pub enum SemanticTokensServerCapabilities {
    SemanticTokensOptions(SemanticTokensOptions),
    SemanticTokensRegistrationOptions(SemanticTokensOptions),
}


/// A call hierarchy item.
#[derive(Clone, Debug)]
pub struct CallHierarchyItem {
    pub name: String,
    pub kind: SymbolKind,
    pub tags: Option<Vec<SymbolTag>>,
    pub detail: Option<String>,
    pub uri: Url,
    pub range: Range,
    pub selection_range: Range,
    pub data: Option<crate::util::JsonValue>,
}

/// An incoming call.
#[derive(Clone, Debug)]
pub struct CallHierarchyIncomingCall {
    pub from: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

/// An outgoing call.
#[derive(Clone, Debug)]
pub struct CallHierarchyOutgoingCall {
    pub to: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

/// Call hierarchy server capability.
#[derive(Clone, Debug)]
pub enum CallHierarchyServerCapability {
    Simple(bool),
}


/// Linked editing ranges response.
#[derive(Clone, Debug)]
pub struct LinkedEditingRanges {
    pub ranges: Vec<Range>,
    pub word_pattern: Option<String>,
}

/// Linked editing range server capabilities.
#[derive(Clone, Debug)]
pub enum LinkedEditingRangeServerCapabilities {
    Simple(bool),
}


/// A document highlight — marks an occurrence of a symbol for highlighting.
#[derive(Clone, Debug)]
pub struct DocumentHighlight {
    pub range: Range,
    pub kind: Option<DocumentHighlightKind>,
}

/// Document highlight kind constants (matching the LSP numeric values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentHighlightKind(pub u32);

impl DocumentHighlightKind {
    /// A textual occurrence.
    pub const TEXT: DocumentHighlightKind = DocumentHighlightKind(1);
    /// Read-access of a symbol (e.g. reading a variable).
    pub const READ: DocumentHighlightKind = DocumentHighlightKind(2);
    /// Write-access of a symbol (e.g. writing to a variable).
    pub const WRITE: DocumentHighlightKind = DocumentHighlightKind(3);
}


/// Parameters for `textDocument/rangeFormatting`.
#[derive(Clone, Debug)]
pub struct DocumentRangeFormattingParams {
    pub text_document: TextDocumentIdentifier,
    pub range: Range,
    pub options: FormattingOptions,
}


/// Log/show message type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageType(pub u32);

impl MessageType {
    pub const ERROR: MessageType = MessageType(1);
    pub const WARNING: MessageType = MessageType(2);
    pub const INFO: MessageType = MessageType(3);
    pub const LOG: MessageType = MessageType(4);
}
