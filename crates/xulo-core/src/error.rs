use std::fmt;
use std::ops::Range;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Lex,
    Parse,
    Semantic,
    Io,
    Codegen,
    Runtime,
    Warning,
}

impl ErrorKind {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            ErrorKind::Lex => "lexical",
            ErrorKind::Parse => "syntax",
            ErrorKind::Semantic => "semantic",
            ErrorKind::Io => "io",
            ErrorKind::Codegen => "code generation",
            ErrorKind::Runtime => "runtime",
            ErrorKind::Warning => "warning",
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        match self {
            ErrorKind::Lex => "E0001",
            ErrorKind::Parse => "E0002",
            ErrorKind::Semantic => "E0003",
            ErrorKind::Io => "E0004",
            ErrorKind::Codegen => "E0005",
            ErrorKind::Runtime => "E0006",
            ErrorKind::Warning => "W0001",
        }
    }
}

#[derive(Debug, Clone)]
pub struct XuloError {
    pub kind: ErrorKind,
    pub message: String,
    pub span: Option<Range<usize>>,
    pub file: Option<PathBuf>,
    /// Fine-grained diagnostic code from the specification's error table
    /// (`spec/type-system/errors.md`), e.g. `E0230`. When absent the coarse
    /// [`ErrorKind`] code is reported instead.
    pub code: Option<&'static str>,
    /// An attached note (spec error rows carry `= note: …` guidance, e.g.
    /// `E0111`'s registered-target list). Rendered after the diagnostic.
    pub note: Option<String>,
}

impl XuloError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            span: None,
            file: None,
            code: None,
            note: None,
        }
    }

    pub fn at(mut self, span: impl Into<Range<usize>>) -> Self {
        self.span = Some(span.into());
        self
    }

    /// Attach a fine-grained diagnostic code, overriding the kind's code.
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    /// Attach a `= note: …` line to the rendered diagnostic.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// The fine-grained code when one was attached, else the kind's code.
    pub fn code(&self) -> &'static str {
        self.code.unwrap_or_else(|| self.kind.code())
    }

    pub fn with_file(mut self, file: PathBuf) -> Self {
        self.file = Some(file);
        self
    }

    pub fn with_message_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.message = format!("{}{}", prefix.into(), self.message);
        self
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for XuloError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.label(), self.message)
    }
}

impl std::error::Error for XuloError {}

impl From<std::io::Error> for XuloError {
    fn from(e: std::io::Error) -> Self {
        XuloError::new(ErrorKind::Io, e.to_string())
    }
}
