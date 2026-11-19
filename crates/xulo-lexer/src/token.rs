use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Ident,
    Fn,
    Let,
    Const,
    Return,
    If,
    Else,
    For,
    In,
    Type,
    Enum,
    Null,
    Number,
    String,
    /// A literal text run inside a backtick template literal (`` `...` ``).
    /// Templates are emitted as a run of `TChunk` tokens separated by
    /// `InterpOpen`…`InterpClose` bracketing each `${expr}` section.
    TChunk,
    /// The `${` that opens an interpolated section of a template literal.
    InterpOpen,
    /// The `}` that closes an interpolated section of a template literal.
    InterpClose,
    Boolean,
    Colon,
    DoubleColon,
    Semicolon,
    Comma,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    Arrow,
    Pipe,
    Amp,
    Question,
    Bang,
    Dot,
    QuestionDot,
    Nullish,
    RangeOp,
    Ellipsis,
    ColonAssign,
    While,
    Match,
    And,
    Or,
    Break,
    Continue,
    Modulo,
    Power,
    Tilde,
    Xor,
    Async,
    Await,
    Pub,
    Use,
    As,
    Trait,
    Where,
    Impl,
    Mut,
    Struct,
    /// A keyword from the specification's closed set (spec
    /// `lexical-structure.md`) that has no dedicated kind above: the primitive
    /// type names, `defer`, `move`, `self`, `super`, `spawn`, and so on.
    Keyword,
    /// A reserved-for-future-use word (see `RESERVED_WORDS`): it lexes as a
    /// token instead of an identifier so it can never be used as a name.
    Reserved,
    /// `#`, the attribute opener (`#[test]`, `#[derive(Debug)]`).
    Hash,
    Dollar,
    EOF,
}

/// Words reserved for future language features (spec `lexical-structure.md`
/// §Reserved Words). They are not keywords, but can never be used as
/// identifiers (checked in `from_keyword`), keeping the door open for later
/// syntax without breaking reserved-word code. Every keyword is excluded:
/// `defer`, `move`, `mod`, `self`, `spawn`, and `super` are keywords.
pub const RESERVED_WORDS: &[&str] = &[
    "abstract",
    "actor",
    "arguments",
    "associatedtype",
    "bench",
    "case",
    "cfg",
    "channel",
    "class",
    "debugger",
    "default",
    "deinit",
    "delete",
    "derive",
    "doc",
    "do",
    "eval",
    "export",
    "extension",
    "fallthrough",
    "final",
    "function",
    "generic",
    "generator",
    "global",
    "guard",
    "init",
    "instanceof",
    "interface",
    "isolated",
    "iterator",
    "lazy",
    "library",
    "local",
    "macro",
    "meta",
    "module",
    "new",
    "open",
    "override",
    "package",
    "priv",
    "protocol",
    "receiver",
    "ref",
    "rethrows",
    "select",
    "sender",
    "static",
    "switch",
    "task",
    "this",
    "typealias",
    "unowned",
    "unsafe",
    "virtual",
    "weak",
    "with",
    "var",
    "void",
    "yield",
];

impl Token {
    /// Map a word to its token: a keyword from the specification's closed set,
    /// a reserved word, or `None` for an ordinary identifier. `print` and
    /// `println` are prelude intrinsics, not keywords, so they return `None`.
    pub fn from_keyword(s: &str) -> Option<Token> {
        Some(match s {
            "fn" => Token::Fn,
            "let" => Token::Let,
            "const" => Token::Const,
            "return" => Token::Return,
            "if" => Token::If,
            "else" => Token::Else,
            "for" => Token::For,
            "in" => Token::In,
            "type" => Token::Type,
            "enum" => Token::Enum,
            "null" => Token::Null,
            "while" => Token::While,
            "match" => Token::Match,
            "and" => Token::And,
            "or" => Token::Or,
            "async" => Token::Async,
            "await" => Token::Await,
            "pub" => Token::Pub,
            "use" => Token::Use,
            "as" => Token::As,
            "trait" => Token::Trait,
            "where" => Token::Where,
            "impl" => Token::Impl,
            "mut" => Token::Mut,
            "struct" => Token::Struct,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "true" | "false" => Token::Boolean,
            // The remaining keywords of the spec's 61-word set.
            "bool" | "copy" | "defer" | "extern" | "f32" | "f64" | "float" | "i8" | "i16"
            | "i32" | "i64" | "int" | "is" | "isize" | "lock" | "mod" | "move" | "not"
            | "number" | "panic" | "self" | "shared" | "spawn" | "string" | "super" | "u8"
            | "u16" | "u32" | "u64" | "unit" | "unknown" | "usize" => Token::Keyword,
            _ => {
                // A reserved word is not a keyword, but it is already
                // forbidden as an identifier (spec `lexical-structure.md`).
                if RESERVED_WORDS.contains(&s) {
                    return Some(Token::Reserved);
                }
                return None;
            }
        })
    }

    pub fn describe(&self) -> &'static str {
        match self {
            Token::Ident => "identifier",
            Token::Fn => "`fn`",
            Token::Let => "`let`",
            Token::Const => "`const`",
            Token::Return => "`return`",
            Token::If => "`if`",
            Token::Else => "`else`",
            Token::For => "`for`",
            Token::In => "`in`",
            Token::Type => "`type`",
            Token::Enum => "`enum`",
            Token::Null => "`null`",
            Token::While => "`while`",
            Token::Match => "`match`",
            Token::And => "`and`",
            Token::Or => "`or`",
            Token::Async => "`async`",
            Token::Await => "`await`",
            Token::Pub => "`pub`",
            Token::Use => "`use`",
            Token::As => "`as`",
            Token::Trait => "`trait`",
            Token::Where => "`where`",
            Token::Impl => "`impl`",
            Token::Mut => "`mut`",
            Token::Struct => "`struct`",
            Token::Keyword => "keyword",
            Token::Reserved => "`<reserved keyword>`",
            Token::Break => "`break`",
            Token::Continue => "`continue`",
            Token::Hash => "`#`",
            Token::Dollar => "`$`",
            Token::Number => "number",
            Token::String => "string",
            Token::TChunk => "template text",
            Token::InterpOpen => "`${`",
            Token::InterpClose => "`}`",
            Token::Boolean => "boolean",
            Token::Colon => "`:`",
            Token::DoubleColon => "`::`",
            Token::Semicolon => "`;`",
            Token::Comma => "`,`",
            Token::LParen => "`(`",
            Token::RParen => "`)`",
            Token::LBrace => "`{`",
            Token::RBrace => "`}`",
            Token::LBracket => "`[`",
            Token::RBracket => "`]`",
            Token::Assign => "`=`",
            Token::Plus => "`+`",
            Token::Minus => "`-`",
            Token::Star => "`*`",
            Token::Slash => "`/`",
            Token::Modulo => "`%`",
            Token::Power => "`**`",
            Token::Tilde => "`~`",
            Token::Xor => "`^`",
            Token::Eq => "`==`",
            Token::Neq => "`!=`",
            Token::Lt => "`<`",
            Token::Gt => "`>`",
            Token::Lte => "`<=`",
            Token::Gte => "`>=`",
            Token::Arrow => "`=>`",
            Token::Pipe => "`|`",
            Token::Amp => "`&`",
            Token::Question => "`?`",
            Token::Bang => "`!`",
            Token::Dot => "`.`",
            Token::QuestionDot => "`?.`",
            Token::Nullish => "`??`",
            Token::RangeOp => "`..<` / `..=`",
            Token::Ellipsis => "`...`",
            Token::ColonAssign => "`:=`",
            Token::EOF => "end of file",
        }
    }
}

/// A token annotated with its byte span in the source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexedToken {
    pub kind: Token,
    pub text: String,
    pub span: Range<usize>,
}

impl LexedToken {
    pub fn new(kind: Token, text: impl Into<String>, span: Range<usize>) -> Self {
        Self {
            kind,
            text: text.into(),
            span,
        }
    }
}
