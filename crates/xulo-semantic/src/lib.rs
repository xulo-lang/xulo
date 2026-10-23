pub mod symbol_table;

use std::collections::HashMap;
use std::ops::Range;

use symbol_table::{Symbol, SymbolKind, SymbolTable};
use xulo_core::ast::{
    AssignStmt, AssignTarget, Attribute, BinaryOperator, Block, Call, CallArg, EnumPayloadParam,
    EnumVariant, ExportItem, Expression, FnBound, FnDef, IfExpr, ImplDecl, LetBinding, Literal,
    ObjectField, PathHead, Program, Statement, TraitDecl, Type, TypeAlias, UiElement, UseDecl,
    UseTarget, impl_fn_name,
};
use xulo_core::error::{ErrorKind, XuloError};

/// Result of a single semantic error, without a source span (spans are
/// attached by the lexer/parser; semantic checks target names).
type SResult<T> = Result<T, XuloError>;

/// The virtual module id the `regex` module resolves to. It carries a
/// control-byte prefix so it can never collide with a real module id (spec
/// `regular-expressions.mdx`: the module's types are reached directly, with
/// no loader graph behind the head).
/// Id of the built-in `std` module root: the six std modules
/// (`std::math` / `std::time` / `std::fs` / `std::io` / `std::os` /
/// `std::regex`) are not
/// package sources, so they resolve against this synthetic id instead of the
/// world (spec `builtins/README.md`).
const BUILTIN_STD: &str = "\u{1}std";

/// The synthetic id of one std module (`\u{1}std::math`).
fn std_module_id(module: &str) -> String {
    format!("{BUILTIN_STD}::{module}")
}

/// Whether a module id is the synthetic id of a built-in `std` module (the
/// control-byte prefix [`BUILTIN_STD`] carries). The runtime resolves
/// `use std::…` itself, so an import table that saw these ids must not stand
/// in with a placeholder binding of its own.
pub fn is_std_module_id(id: &str) -> bool {
    id.starts_with(BUILTIN_STD)
}

/// The module name of a synthetic std id, or `None` for any other id.
fn std_module_name(id: &str) -> Option<&str> {
    id.strip_prefix(&format!("{BUILTIN_STD}::"))
}

/// The std module behind a resolved `use` path head, given the id the head
/// resolved to and the segments that followed it.
fn std_path_module<'a>(start: &'a str, segments: &'a [String]) -> Option<&'a str> {
    if start == BUILTIN_STD {
        segments.first().map(|s| s.as_str())
    } else {
        std_module_name(start)
    }
}

/// The std module behind a `Head::member` callee (`std::math` in
/// `std::math::floor(…)`), keyed the same way [`builtin_member`] is.
fn std_ns_key(enum_name: &str) -> Option<&str> {
    matches!(
        enum_name,
        "std::math" | "std::time" | "std::fs" | "std::io" | "std::os"
    )
    .then_some(enum_name)
}

/// The prelude's own value names (spec `builtins/prelude.md`): the interpreter
/// keeps them in its global scope, where a `use` never rebinds them. A std
/// import under one of these names — `std::io` exports `print` and `println` —
/// would type-check as the std member while the runtime kept calling the
/// intrinsic, so the import is refused (E0601) and the qualified path stands.
const PRELUDE_INTRINSIC_VALUES: &[&str] = &["print", "println", "str", "assert"];

/// One walk's derive targets pending field checks: attribute span -> declared
/// target types as (type, label span).
type DeriveList = Vec<(Range<usize>, Vec<(Type, Range<usize>)>)>;

/// A named type known to the program: a user `type` alias, an `enum`, a
/// `trait`, or a `struct`.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeEntryKind {
    Alias(Type),
    Enum(Vec<EnumVariant>),
    Trait(TraitDef),
    Struct(Vec<(String, Type, Option<xulo_core::ast::Expression>)>),
}

impl TypeEntryKind {
    pub fn variants(&self) -> Option<&Vec<EnumVariant>> {
        match self {
            TypeEntryKind::Enum(variants) => Some(variants),
            TypeEntryKind::Alias(_) | TypeEntryKind::Trait(_) | TypeEntryKind::Struct(_) => None,
        }
    }
}

/// The fixed arity of a prelude type (spec `builtins/prelude.md`): the number
/// of type arguments an application of it must write. Names whose declaration
/// the program provides are answered from the type table instead; these exist
/// so an application like `Map<string>` reports the arity rule (E0230) rather
/// than an unknown head.
fn builtin_type_arity(name: &str) -> Option<usize> {
    match name {
        "List" | "Set" | "Task" => Some(1),
        "Map" | "Result" => Some(2),
        // The primitive value types are keywords (spec
        // `lexical-structure.md`), so an annotation reaches the checker
        // through `PrimitiveType` and not as a named head — except the ones
        // the parser still carries spelled as a `Named`: `unit`, `unknown`,
        // and the fixed-bit layer of the two-layer numeric system. The
        // base types (`bool`, `string`, `number`, `int`, `float`, `null`)
        // lower to their own `Type` variants and never get here.
        "unit" | "unknown" => Some(0),
        // The fixed-bit layer of the two-layer numeric system.
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64" => Some(0),
        // `usize`/`isize` (spec `types/primitive-types.md`) and the opaque
        // foreign-handle type `Ptr` (spec ffi.md) are non-generic too; they
        // arrive as `Named` heads because they are not parser keywords.
        "usize" | "isize" | "Ptr" => Some(0),
        // `View` and `Range` are primitive but UpperCamelCase, and `Error` is
        // a prelude type (spec `types/primitive-types.md`).
        "View" | "Error" | "Range" => Some(0),
        _ => None,
    }
}

/// The primitive a match pattern's type name spells (spec
/// `types/primitive-types.md`): a pattern carries the designator's text
/// rather than a parsed type, so the keyword spellings resolve to the same
/// primitives an annotation gets. `None` leaves the name as a `Named` type
/// for the ordinary rules (user enums, structs, prelude shapes, and the
/// fixed-bit / singleton keywords the parser spells as a `Named`).
fn pattern_primitive_type(name: &str) -> Option<Type> {
    match name {
        "string" => Some(Type::String),
        "number" | "int" | "float" => Some(Type::Number),
        "bool" => Some(Type::Boolean),
        "null" => Some(Type::Null),
        _ => None,
    }
}

/// One of the fixed-bit numeric types (spec `types/primitive-types.md`):
/// an integer width, or one of the two IEEE float widths.
enum FixedBit {
    Int { bits: i32, signed: bool },
    Float32,
    Float64,
}

/// The fixed-bit type `name` spells, if any: `i8`…`u64`, `f32`, `f64`, and
/// the pointer-width pair `usize`/`isize`. The implementation documents
/// `W = 64` for the pointer width (spec `memory-and-runtime.md`, which
/// requires the target's `W` to be documented).
fn fixed_bit(name: &str) -> Option<FixedBit> {
    match name {
        "i8" => Some(FixedBit::Int {
            bits: 8,
            signed: true,
        }),
        "i16" => Some(FixedBit::Int {
            bits: 16,
            signed: true,
        }),
        "i32" => Some(FixedBit::Int {
            bits: 32,
            signed: true,
        }),
        "i64" | "isize" => Some(FixedBit::Int {
            bits: 64,
            signed: true,
        }),
        "u8" => Some(FixedBit::Int {
            bits: 8,
            signed: false,
        }),
        "u16" => Some(FixedBit::Int {
            bits: 16,
            signed: false,
        }),
        "u32" => Some(FixedBit::Int {
            bits: 32,
            signed: false,
        }),
        "u64" | "usize" => Some(FixedBit::Int {
            bits: 64,
            signed: false,
        }),
        "f32" => Some(FixedBit::Float32),
        "f64" => Some(FixedBit::Float64),
        _ => None,
    }
}

/// The fixed-bit type `ty` *is* (after alias resolution): the plain name when
/// `ty` is a fixed-bit `Named` with no arguments, `None` otherwise.
fn fixed_bit_name(ty: &Type) -> Option<&str> {
    match ty {
        Type::Named(name, args) if args.is_empty() && fixed_bit(name).is_some() => Some(name),
        _ => None,
    }
}

/// Whether an integral literal value fits the fixed-bit integer type
/// `bits`/`signed`. The AST carries every number as `f64`, so a non-integral
/// value answers `false` (it is a float literal, which never coerces to an
/// integer type — spec `type-system/coercion.md`) and the upper bound is
/// checked exclusively so `2^63`/`2^64` (exactly representable) is rejected
/// while every in-range integral value below it is accepted.
fn int_fixed_fits(value: f64, bits: i32, signed: bool) -> bool {
    if !value.is_finite() || value.fract() != 0.0 {
        return false;
    }
    let upper = 2f64.powi(if signed { bits - 1 } else { bits });
    let lower = if signed { -upper } else { 0.0 };
    value >= lower && value < upper
}

/// How a numeric literal relates to a fixed-bit integer target: fits, does
/// not fit (E0209), or is not an integer literal at all (the caller's E0201).
enum LiteralFit {
    Fits,
    OutOfRange,
    NotAnInteger,
}

fn int_literal_fit(value: f64, bits: i32, signed: bool) -> LiteralFit {
    if !value.is_finite() || value.fract() != 0.0 {
        LiteralFit::NotAnInteger
    } else if int_fixed_fits(value, bits, signed) {
        LiteralFit::Fits
    } else {
        LiteralFit::OutOfRange
    }
}

/// A literal's text as written, for diagnostics: integers print without a
/// fractional part (`300`, not `300.0`).
fn num_literal_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Whether `ty` names the opaque foreign-handle type `Ptr` (spec ffi.md),
/// unwrapping an optional wrapper (`Ptr?`).
fn is_ptr_type(ty: &Type) -> bool {
    match ty {
        Type::Named(name, _) => name == "Ptr",
        Type::Optional(inner) => is_ptr_type(inner),
        _ => false,
    }
}

/// The numeric value of a literal as written: a direct `Literal::Number`,
/// or the unary `-` applied to one (spec `expressions/literals.md` gives
/// the sign its own precedence, but `abs(-1)` in spec ffi.md must still
/// coerce — a negated literal is recognized as a signed literal for
/// coercion). `None` for anything else.
fn numeric_literal_of(expr: &Expression) -> Option<f64> {
    match expr {
        Expression::Literal {
            value: Literal::Number(v),
            ..
        } => Some(*v),
        Expression::Unary(un) if matches!(un.operator, xulo_core::ast::UnaryOperator::Neg) => {
            match &un.operand {
                Expression::Literal {
                    value: Literal::Number(v),
                    ..
                } => Some(-*v),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The closed ABI registry (spec ffi.md, "ABI tags"): every tag an `extern`
/// declaration may name. Adding a tag later is a minor change.
const REGISTERED_ABIS: [&str; 2] = ["C", "system"];

/// The built-in `Result`/`Error` types (spec `error-handling.md`):
/// `Result<T, E>` is the two-variant result data every fallible path carries,
/// and `Error` is the base error type carrying one `message` field. They live
/// in their own map — consulted only when the program declares nothing of the
/// name — so a module's own `enum Result` shadows them without tripping the
/// "type is already defined" rule (spec: `Result` is both a prelude type and
/// a name a program may redefine).
fn builtin_type_entries() -> HashMap<String, TypeEntry> {
    let mut builtins = HashMap::new();
    builtins.insert(
        "Result".to_string(),
        TypeEntry {
            type_params: vec!["T".to_string(), "E".to_string()],
            type_bounds: Vec::new(),
            kind: TypeEntryKind::Enum(vec![
                EnumVariant {
                    name: "Ok".to_string(),
                    name_span: 0..0,
                    payload: Some(vec![EnumPayloadParam {
                        name: None,
                        type_: Type::Named("T".to_string(), Vec::new()),
                    }]),
                },
                EnumVariant {
                    name: "Err".to_string(),
                    name_span: 0..0,
                    payload: Some(vec![EnumPayloadParam {
                        name: None,
                        type_: Type::Named("E".to_string(), Vec::new()),
                    }]),
                },
            ]),
        },
    );
    builtins.insert(
        "Error".to_string(),
        TypeEntry {
            type_params: Vec::new(),
            type_bounds: Vec::new(),
            kind: TypeEntryKind::Struct(vec![("message".to_string(), Type::String, None)]),
        },
    );
    // `ToString` is the built-in protocol trait (spec `types/traits.md`):
    // `str(v)` and `${v}` accept a base type or a type with this impl, and
    // `#Derive(ToString)` expands to the same impl an author would write.
    builtins.insert(
        "ToString".to_string(),
        TypeEntry {
            type_params: Vec::new(),
            type_bounds: Vec::new(),
            kind: TypeEntryKind::Trait(TraitDef {
                methods: vec![TraitMethodSig {
                    name: "toString".to_string(),
                    params: Vec::new(),
                    return_type: Type::String,
                    has_self: true,
                    is_async: false,
                }],
            }),
        },
    );
    // `RegexMatch` needs no import: an annotation of the name answers from
    // here, while `use regex::RegexMatch` binds the same shape locally.
    builtins.insert("RegexMatch".to_string(), regex_match_type_entry());
    builtins
}

/// The result type of `RegExp::compile`: the built-in `Result` carrying the
/// compiled `RegExp` or the `RegexError` describing what failed to compile.
fn regex_compile_result_type() -> Type {
    Type::Named(
        "Result".to_string(),
        vec![
            Type::Named("RegExp".to_string(), Vec::new()),
            Type::Named("RegexError".to_string(), Vec::new()),
        ],
    )
}

/// The `RegExp` type entry (spec `regular-expressions.mdx`): a compiled
/// pattern's three operations, typed here as method signatures so a member
/// call through the imported name answers with its declared result. The
/// runtime object carries matching natives of the same names.
fn regex_regexp_type_entry() -> TypeEntry {
    let match_ty = Type::Named("RegexMatch".to_string(), Vec::new());
    TypeEntry {
        type_params: Vec::new(),
        type_bounds: Vec::new(),
        kind: TypeEntryKind::Struct(vec![
            (
                "test".to_string(),
                Type::FnSig {
                    params: vec![Type::String],
                    ret: Some(Box::new(Type::Boolean)),
                },
                None,
            ),
            (
                "find".to_string(),
                Type::FnSig {
                    params: vec![Type::String],
                    ret: Some(Box::new(Type::Optional(Box::new(match_ty.clone())))),
                },
                None,
            ),
            (
                "find_all".to_string(),
                Type::FnSig {
                    params: vec![Type::String],
                    ret: Some(Box::new(Type::List(Box::new(match_ty)))),
                },
                None,
            ),
        ]),
    }
}

/// The `RegexError` type entry: `RegExp::compile`'s failure payload carries
/// one `message` field, shaped like the base `Error` type.
fn regex_error_type_entry() -> TypeEntry {
    TypeEntry {
        type_params: Vec::new(),
        type_bounds: Vec::new(),
        kind: TypeEntryKind::Struct(vec![("message".to_string(), Type::String, None)]),
    }
}

/// The `RegexMatch` type entry: one match's text (`value`) and capture
/// groups by 1-based index (`group`, `null` for an absent group). It lives in
/// the always-consulted `builtins` map as well, so an annotation of the name
/// needs no import (spec `regular-expressions.mdx`).
fn regex_match_type_entry() -> TypeEntry {
    TypeEntry {
        type_params: Vec::new(),
        type_bounds: Vec::new(),
        kind: TypeEntryKind::Struct(vec![
            ("value".to_string(), Type::String, None),
            (
                "group".to_string(),
                Type::FnSig {
                    params: vec![Type::Number],
                    ret: Some(Box::new(Type::Optional(Box::new(Type::String)))),
                },
                None,
            ),
        ]),
    }
}

/// A comparison of a binding against `null`: the binding's name and whether
/// the operator was `==` (`false` for `!=`), in either operand order. This is
/// the shape `if x != null` must have for flow-sensitive narrowing (spec
/// `types/composite-types.md`).
fn match_null_test(cond: &Expression) -> Option<(String, bool)> {
    let Expression::BinaryOp(b) = cond else {
        return None;
    };
    let eq = match b.operator {
        BinaryOperator::Eq => true,
        BinaryOperator::Neq => false,
        _ => return None,
    };
    match (&b.left, &b.right) {
        (
            Expression::Identifier { name, .. },
            Expression::Literal {
                value: Literal::Null,
                ..
            },
        ) => Some((name.clone(), eq)),
        (
            Expression::Literal {
                value: Literal::Null,
                ..
            },
            Expression::Identifier { name, .. },
        ) => Some((name.clone(), eq)),
        _ => None,
    }
}

/// A parameter of an intrinsic namespace function (spec
/// `builtins/intrinsic-functions.md`): the name its signature writes, for
/// diagnostics, and the type it accepts. `float`/`int`/`u64` are written
/// the way the spec writes them, while every numeric parameter accepts the
/// implementation's one `number` type. `string` (`Fs`, `io`) is
/// implementation-first and has no spec entry yet.
#[derive(Clone, Copy)]
enum BuiltinTy {
    Number,
    Float,
    Int,
    U64,
    String,
}

impl BuiltinTy {
    fn display(self) -> &'static str {
        match self {
            BuiltinTy::Number => "number",
            BuiltinTy::Float => "float",
            BuiltinTy::Int => "int",
            BuiltinTy::U64 => "u64",
            BuiltinTy::String => "string",
        }
    }

    fn ty(self) -> Type {
        match self {
            BuiltinTy::String => Type::String,
            _ => Type::Number,
        }
    }
}

/// The declared return kind of an intrinsic namespace function.
#[derive(Clone, Copy)]
enum BuiltinRet {
    Number,
    Float,
    Int,
    String,
    Bool,
    TaskUnit,
    /// `Result<string, Error>` — a fallible operation whose success payload is
    /// a string (`Fs::read`).
    ResultStrErr,
    /// `Result<unit, Error>` — a fallible operation with no success payload
    /// (`Fs::write`, `Fs::remove`, `Fs::mkdir`).
    ResultUnitErr,
    /// `Result<string?, Error>` — a fallible read whose success payload is an
    /// optional string: `null` is the end of the stream (`std::io::readLine`).
    ResultOptStrErr,
}

impl BuiltinRet {
    fn ty(self) -> Type {
        match self {
            BuiltinRet::Number | BuiltinRet::Float | BuiltinRet::Int => Type::Number,
            BuiltinRet::String => Type::String,
            BuiltinRet::Bool => Type::Boolean,
            BuiltinRet::TaskUnit => Type::Named(
                "Task".to_string(),
                vec![Type::Named("unit".to_string(), Vec::new())],
            ),
            BuiltinRet::ResultStrErr => Type::Named(
                "Result".to_string(),
                vec![Type::String, Type::Named("Error".to_string(), Vec::new())],
            ),
            BuiltinRet::ResultUnitErr => Type::Named(
                "Result".to_string(),
                vec![
                    Type::Named("unit".to_string(), Vec::new()),
                    Type::Named("Error".to_string(), Vec::new()),
                ],
            ),
            BuiltinRet::ResultOptStrErr => Type::Named(
                "Result".to_string(),
                vec![
                    Type::Optional(Box::new(Type::String)),
                    Type::Named("Error".to_string(), Vec::new()),
                ],
            ),
        }
    }
}

/// One intrinsic function's registered signature.
struct BuiltinSig {
    params: &'static [BuiltinTy],
    ret: BuiltinRet,
}

/// One member of a built-in namespace: a constant (`Math::PI`) or a function.
enum BuiltinMember {
    Constant,
    Fn(BuiltinSig),
}

/// The intrinsic table of `Math` and `Time` (spec
/// `builtins/intrinsic-functions.md`), plus `Fs` and `io`, which are
/// implementation-first (the language itself has no I/O — the host runtime
/// supplies it, spec `memory-and-runtime.md`). `Task` calls route through
/// [`Analyzer::check_task_utility`], which infers the task element types a
/// static table cannot.
fn builtin_member(ns: &str, member: &str) -> Option<BuiltinMember> {
    use BuiltinMember as M;
    use BuiltinRet as R;
    use BuiltinTy as T;
    let f = |params: &'static [T], ret: R| Some(M::Fn(BuiltinSig { params, ret }));
    match ns {
        "std::math" => match member {
            "PI" | "E" | "TAU" | "INFINITY" | "NAN" => Some(M::Constant),
            "abs" => f(&[T::Number], R::Number),
            "min" | "max" => f(&[T::Number, T::Number], R::Number),
            "sqrt" | "cbrt" => f(&[T::Float], R::Float),
            "floor" | "ceil" | "round" | "trunc" => f(&[T::Float], R::Int),
            "pow" => f(&[T::Float, T::Float], R::Float),
            "toFixed" => f(&[T::Float, T::Int], R::String),
            "sin" | "cos" | "tan" | "asin" | "acos" | "atan" => f(&[T::Float], R::Float),
            "atan2" => f(&[T::Float, T::Float], R::Float),
            "log" | "log2" | "log10" | "exp" => f(&[T::Float], R::Float),
            _ => None,
        },
        "std::time" => match member {
            "now" | "now_nanos" | "monotonic" | "monotonic_nanos" => f(&[], R::Number),
            "sleep" => f(&[T::U64], R::TaskUnit),
            _ => None,
        },
        "std::fs" => match member {
            "read" => f(&[T::String], R::ResultStrErr),
            "write" => f(&[T::String, T::String], R::ResultUnitErr),
            "exists" => f(&[T::String], R::Bool),
            "remove" | "mkdir" => f(&[T::String], R::ResultUnitErr),
            _ => None,
        },
        // The host streams: writes take the text to emit, `flush` pushes what
        // is buffered, and the two reads take no operand (`readLine` yields
        // `string?`, so `null` ends a line loop).
        "std::io" => match member {
            "print" | "println" | "eprint" | "eprintln" => f(&[T::String], R::ResultUnitErr),
            "flush" => f(&[], R::ResultUnitErr),
            "readLine" => f(&[], R::ResultOptStrErr),
            "read" => f(&[], R::ResultStrErr),
            _ => None,
        },
        // OS surface: the working directory, the environment, and process /
        // host identity. `getenv` yields `string?` (null when unset), and
        // `pid`/`platform`/`arch` are total.
        "std::os" => match member {
            "getenv" => f(&[T::String], R::ResultOptStrErr),
            "setenv" => f(&[T::String, T::String], R::ResultUnitErr),
            "cwd" => f(&[], R::ResultStrErr),
            "chdir" => f(&[T::String], R::ResultUnitErr),
            "pid" => f(&[], R::Number),
            "platform" | "arch" => f(&[], R::String),
            _ => None,
        },
        _ => None,
    }
}

/// The public member list of one built-in std module, as a glob or group
/// `use` enumerates it — kept parallel to [`builtin_member`] (constants
/// first, then functions; `regex` exports only its three types).
fn std_face_members(module: &str) -> Option<Vec<(String, FaceKind)>> {
    let values = |names: &[&str]| {
        names
            .iter()
            .map(|n| (n.to_string(), FaceKind::Value))
            .collect()
    };
    let fns = |names: &[&str]| {
        names
            .iter()
            .map(|n| (n.to_string(), FaceKind::Fn))
            .collect()
    };
    Some(match module {
        "math" => [
            values(&["PI", "E", "TAU", "INFINITY", "NAN"]),
            fns(&[
                "abs", "min", "max", "sqrt", "cbrt", "pow", "floor", "ceil", "round", "trunc",
                "toFixed", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "log", "log2",
                "log10", "exp",
            ]),
        ]
        .concat(),
        "time" => fns(&["now", "now_nanos", "monotonic", "monotonic_nanos", "sleep"]),
        "fs" => fns(&["read", "write", "exists", "remove", "mkdir"]),
        // `print`/`println` are exported by the module but absent from its
        // glob face: their names belong to the prelude intrinsics, which a
        // glob cannot rebind (`PRELUDE_INTRINSIC_VALUES`).
        "io" => fns(&["eprint", "eprintln", "flush", "readLine", "read"]),
        "os" => fns(&[
            "getenv", "setenv", "cwd", "chdir", "pid", "platform", "arch",
        ]),
        "regex" => ["RegExp", "RegexError", "RegexMatch"]
            .iter()
            .map(|n| (n.to_string(), FaceKind::Type))
            .collect(),
        _ => return None,
    })
}

/// A loose signature for a bare `Task::all`-style reference (the combinator
/// used as a value): element types are inferred at the call, so the bare
/// reference only needs the shape.
fn task_bare_member_sig(member: &str) -> Option<Type> {
    let task_any = || Type::Named("Task".to_string(), vec![Type::Any]);
    let sig = |params: Vec<Type>, ret: Type| Type::FnSig {
        params,
        ret: Some(Box::new(ret)),
    };
    match member {
        "all" | "race" => Some(sig(vec![Type::List(Box::new(task_any()))], task_any())),
        "resolve" => Some(sig(vec![Type::Any], task_any())),
        "timeout" => Some(sig(vec![task_any(), Type::Number], task_any())),
        _ => None,
    }
}

/// The method contract of a `trait`: method name, parameter types (the `self`
/// receiver excluded), and return type.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef {
    pub methods: Vec<TraitMethodSig>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethodSig {
    pub name: String,
    pub params: Vec<Type>,
    pub return_type: Type,
    /// Whether the trait method declares a `self` receiver.
    pub has_self: bool,
    /// Whether the trait method is declared `async`.
    pub is_async: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeEntry {
    pub type_params: Vec<String>,
    /// Inline bounds on `type_params`, parallel to it (spec `types/generics.md`).
    pub type_bounds: Vec<FnBound>,
    pub kind: TypeEntryKind,
}

/// A method a type declares in an `impl` block, as seen by the statement
/// pre-scan that backs the `spawn.process` service rule (spec `concurrency.md`,
/// E0409).
struct ImplMethodSig {
    name: String,
    has_self: bool,
    param_count: usize,
    /// Written parameter types (or `any` when unannotated), in order —
    /// `on_message`'s second entry is the service's message type `M`.
    param_types: Vec<Type>,
}

/// One method of a pre-scanned [`ImplRecord`], as seen by member-call
/// resolution: its declared return type (`any` when unannotated) and
/// async-ness, so a resolved call can be typed without re-entering the
/// body checker.
#[derive(Debug, Clone)]
pub(crate) struct ImplMethodType {
    name: String,
    /// Declared return type; async methods wrap it in `Type::Async` here.
    ret: Type,
}

/// A pre-scanned `impl` block: enough to decide applicability (spec
/// `types/traits.md`, "When an impl applies") and to resolve member calls
/// (`value.method(…)`) without re-entering the body checker. Populated
/// before checking so resolution is order-independent, like
/// [`ImplMethodSig`] backs the E0409 service rule.
#[derive(Debug, Clone)]
pub(crate) struct ImplRecord {
    /// `None` for an inherent `impl Type { … }`.
    trait_name: Option<String>,
    type_name: String,
    /// Declared type parameters (`impl<T> …`); empty for a non-generic
    /// `impl`, which applies to its receiver name alone.
    type_params: Vec<String>,
    /// The receiver's written type arguments — the pattern applicability
    /// is decided against (`Pair<T, T>` in `impl<T> Pair<T, T> { … }`).
    type_args: Vec<Type>,
    /// Inline bounds on `type_params`; validated (E0806) when the `impl`
    /// itself is checked, consulted as-is here.
    bounds: Vec<FnBound>,
    /// Methods the block declares, in declaration order.
    methods: Vec<ImplMethodType>,
}

pub struct Analyzer {
    table: SymbolTable,
    current_return: Option<Type>,
    /// Whether the current function declared a return type (docs §21.2: only
    /// then does a trailing expression count as its implicit return).
    declared_return: bool,
    type_table: HashMap<String, TypeEntry>,
    /// The built-in `Result`/`Error` entries, consulted only when
    /// `type_table` (and the imports) do not declare the name, so a program's
    /// own declaration of either name shadows the builtin (see
    /// [`builtin_type_entries`]).
    builtins: HashMap<String, TypeEntry>,
    /// Type-parameter names currently in scope (these are valid as `Named`)
    /// and are erased to `Any` for kind/arithmetic checks.
    generics: Vec<String>,
    /// Stack of bound maps for in-scope generic type parameters: parameter name
    /// -> the trait's structural `ObjectType` shape. Enables `member_field_type`
    /// refinement and call-site `assignable` checks for `T: Trait` bounds.
    bound_shapes: Vec<HashMap<String, Type>>,
    /// Stack parallel to `bound_shapes` recording the *trait names* each
    /// in-scope type parameter requires: parameter name -> its bounds. A type
    /// parameter satisfies a bound when its own declaration grants it, which
    /// is how bound satisfaction is answered inside a generic body (spec
    /// `types/traits.md`).
    bound_traits: Vec<HashMap<String, Vec<String>>>,
    /// Registered `impl Trait for Type` blocks: `(trait, type)` -> method names
    /// provided. The receiver type's concrete name must be known statically.
    impls: std::collections::HashSet<(String, String, String)>,
    /// Every type with a string form — a `ToString` impl (hand-written or
    /// `#Derive(ToString)`) — built before checking so `str(v)` / `${v}`
    /// (E0212) and derive field checks (E0801) are order-independent (spec
    /// `types/traits.md`, `builtins/intrinsic-functions.md`).
    string_forms: std::collections::HashSet<String>,
    /// Mangled dispatch function names already claimed by `impl` methods, plus
    /// user functions present in the symbol table. Mangles join `trait`, `type`
    /// and `method` with `_`, so distinct `impl`s can collapse onto the same
    /// function name (e.g. `a_b`+`c` vs `a`+`b_c`), which would silently
    /// overwrite in both the JS and native runtimes; duplicates are an error.
    impl_mangled: std::collections::HashSet<String>,
    /// Every `impl` block declared in this module, pre-scanned before
    /// checking so member-call resolution and bound satisfaction are
    /// order-independent (spec `types/traits.md`, "When an impl applies").
    declared_impls: Vec<ImplRecord>,
    /// Trait-dispatch call sites found during checking: `(span, method name)`.
    /// These are applied back onto the AST (`Call.trait_impl`) by
    /// Bare calls resolved to a built-in std member (`use std::math::*` then
    /// `floor(…)`): `(callee name span, qualified callee)` applied with
    /// [`apply_std_calls`] after analysis, keeping the checker call-site
    /// knowledge (the runtime binds the bare name itself; the AOT lowering
    /// only understands qualified intrinsic callees).
    std_calls: Vec<(Range<usize>, String)>,

    /// [`apply_trait_dispatch`] after analysis, keeping the checker call-site
    /// signatures shareable.
    trait_dispatch: Vec<(Range<usize>, String)>,
    /// `+` expressions whose operands both checked out as `List` types; applied
    /// back onto the AST (`BinaryOp.list_concat`) by [`apply_list_concat`] so
    /// the JS codegen emits an array concatenation instead of a stringifying
    /// bare `+`.
    list_concat: Vec<Range<usize>>,
    /// Depth of enclosing async functions; `await` is only valid when > 0.
    async_depth: usize,
    /// Depth of enclosing `View`-returning functions; `@State`/`@Store`/
    /// `@Effect`/`@Environment` are only valid when > 0.
    component_depth: usize,
    /// Borrows whose call returned a `Task`: they end where control passes an
    /// `await` of that task in the same block, or at the block's end (spec
    /// `memory-and-runtime.md`, Span end).
    pending_borrows: Vec<PendingBorrow>,
    /// Id of the block currently being checked; a borrow is released only by
    /// an `await` in the block that holds its task binding.
    block_id: usize,
    /// Next block id to hand out.
    next_block_id: usize,
    /// Nesting depth of a place expression being checked as the object of a
    /// member/index access: only the outermost place is a "use" for the
    /// borrow-span rules (E0405/E0411).
    place_depth: usize,
    /// Depth of closure bodies (`fn(…)` expressions and `=>` arrows) currently
    /// being checked: a capture of a `mut` parameter is read-only (E0401).
    closure_depth: usize,
    /// Scope index at which each enclosing `spawn` block starts: references
    /// to bindings declared outside that index must be captured (spec
    /// `concurrency.md`, E0408).
    spawn_scopes: Vec<usize>,
    /// True while checking a `let x = move y` / `let x = copy y` initializer
    /// inside a `spawn` block: that operand *is* the capture.
    capture_operand: bool,
    /// Bindings already moved out of (`move`), keyed by name with the site of
    /// the transfer: any later use of one is E0403 (spec
    /// `memory-and-runtime.md`). Cleared per function body.
    moved: std::collections::HashMap<String, std::ops::Range<usize>>,
    /// Set by the three move positions — a `let` initializer, an argument,
    /// and a `return` — and consumed by the expression checked there.
    in_move_position: bool,
    /// The move-position state of the expression currently in
    /// `check_expression_inner` (consumed at entry).
    cur_move_pos: bool,
    /// Methods each type declares in an `impl` block, pre-scanned so the
    /// `spawn.process` service rule (E0409) is order-independent.
    impl_methods: HashMap<String, Vec<ImplMethodSig>>,
    /// Nonzero while checking an `on_message` method body: `reply` is only
    /// in scope there (spec `concurrency.md`).
    in_on_message: bool,
    /// Operand types of the `reply(...)` calls in the `on_message` body being
    /// checked; their common type is the service's reply type `R`.
    reply_operands: Vec<Type>,
    /// Per service type, the computed reply type `R` (spec `concurrency.md`):
    /// the common type of the `on_message` reply operands, or `Type::Null`
    /// (unit)
    /// when the method never calls `reply`.
    service_reply_types: HashMap<String, Type>,
    /// Bindings whose `lock` section encloses the code being checked: their
    /// contents are reachable (and mutable) only under one (spec
    /// `concurrency.md`, E0406).
    locked: Vec<String>,
    /// Depth of `lock` bodies currently being checked: `await` is never a
    /// legal suspension point inside one (spec `concurrency.md`, E0410).
    lock_depth: usize,
    /// Depth of nested blocks below the current function body's top level;
    /// decorators are only allowed when this is 0 (docs §12).
    block_depth: usize,
    /// Depth of `if`/`match` expressions checked in value position; their
    /// arms compile to a plain (non-`async`) function, so `await` is
    /// rejected there regardless of the enclosing `async_depth`.
    no_await_depth: usize,
    /// Names of `#Test fn` declarations in this module (spec testing.md): the
    /// identifier is declared but binds no value, so any reference to it is a
    /// compile-time error (E0107). Pre-scanned before checking so the ban is
    /// order-independent.
    test_names: std::collections::HashSet<String>,
    /// Names imported from an unseeded external package (no module graph): the
    /// loader provides no signature, so calls to these are checked opaquely.
    opaque: std::collections::HashSet<String>,
    /// Imported type/alias names available to this module.
    imported_types: HashMap<String, TypeEntry>,
    /// Local names bound from the built-in `regex` module (`use regex::RegExp`):
    /// the heads whose static members are the module's (`RegExp::compile`).
    /// A program-declared type of the same name shadows the import, so this
    /// records only what the import bound — never what the program declared.
    regex_names: std::collections::HashSet<String>,
    /// Locals a std `use` bound to an intrinsic function
    /// (`use std::math::*` → `floor`), keyed by local name: their value maps
    /// to the builtin table so bare calls keep `std::` diagnostics.
    std_fns: HashMap<String, (String, String)>,
    /// Value symbols seeded by the caller (the editor workspace): consulted
    /// only by the lenient no-world binds, never by strict analysis.
    imports: HashMap<String, Symbol>,
    /// Names exported from this module for codegen / module resolution.
    exported: Vec<String>,
    /// Exported symbols captured for cross-module type checking.
    exported_symbols: Vec<(String, Symbol)>,
    /// Local names bound by extern import items, mapped to the item's name
    /// span: referencing one outside direct-call position is `E0903` (spec
    /// ffi.md, "Calling"). Keyed by declaration span so a shadowing local
    /// binding of the same spelling — whose own declaration span differs —
    /// is unaffected.
    extern_imports: HashMap<String, Range<usize>>,
    /// Non-fatal diagnostics raised during analysis (e.g. ignored return
    /// values); surfaced by the CLI after a successful compile.
    warnings: Vec<XuloError>,
    /// Per-component stack of names declared by the component's *render* code
    /// (plain `let`s and nested function declarations in the function body).
    /// These live inside the `__component(...)` render closure at runtime, so
    /// `@Effect`/`@State`/`@Store` setup code (hoisted above it) may not
    /// reference them.
    render_locals: Vec<std::collections::HashSet<String>>,
    /// Set while checking an `@Effect` closure (and its deps): identifiers that
    /// resolve to render-scoped locals are rejected (they are out of scope when
    /// the effect runs).
    in_effect: bool,
    /// Depth of enclosing `for`/`while` loops; `break`/`continue` are only
    /// valid when > 0.
    in_loop: usize,
    /// Depth of enclosing `defer` bodies: `?` may not appear in a `defer`
    /// operand, since control is already leaving the block when it runs
    /// (spec `error-handling.md`, E0229). Closures formed inside reset it —
    /// their body does not run as part of the defer.
    in_defer: usize,
    /// Span of the expression most recently entered by `check_expression`;
    /// semantic errors are attached to it (they cannot name their own span).
    current_span: Range<usize>,
    /// Editor-tooling mode: keep checking after an error instead of failing
    /// fast, so partial resolutions survive broken files.
    lenient: bool,
    /// The first error raised while checking (only meaningful when `lenient`).
    first_error: Option<XuloError>,
    /// Every checked expression's inferred type, keyed by its span
    /// (out-of-band for editor tooling).
    expr_types: Vec<(Range<usize>, Type)>,
    /// Name resolutions captured while checking (see [`UseRecord`]).
    resolutions: Vec<UseRecord>,
    /// Resolved `use` bindings for the runtime (see [`UseBinding`]).
    use_bindings: Vec<UseBinding>,
    /// Local-form `pub use { a, b }` names whose own declarations appear
    /// later in the file (the header precedes every declaration, E0605):
    /// re-export validation runs once the pass has seen them all.
    pending_reexports: Vec<(String, Range<usize>)>,
    /// Package tree for path resolution (`None` for single-file analysis).
    world: Option<ModuleWorld>,
    /// Id of the module being checked ("" for single-file analysis).
    module_id: String,
    /// Local namespace bindings: `mod` names and `use … as` aliases, each
    /// mapped to the module id it names.
    namespaces: HashMap<String, String>,
    /// `use`-bound value/type names -> (origin module id, kind), for the
    /// initialization-order check (E0611).
    use_origins: HashMap<String, (String, FaceKind)>,
    /// `use`-bound names -> binding span, for the unused check (W0102).
    use_bound: std::cell::RefCell<HashMap<String, Range<usize>>>,
    /// Glob `use` spans and the names each still contributes unused (W0102).
    glob_unused: std::cell::RefCell<Vec<(Range<usize>, std::collections::HashSet<String>)>>,
    /// Type-only `use` names -> binding span (E0606).
    type_only: HashMap<String, Range<usize>>,
    /// Names this module's interface contains: `pub` declarations, `pub mod`
    /// names, and `pub use` exports (E0604).
    own_interface: std::collections::HashSet<String>,
    /// Type entries contributed by `pub use` (re-exports of imported types).
    exported_types_for_iface: Vec<(String, TypeEntry)>,
}

/// The kind of a value symbol a name-use resolved to (a light projection of
/// `SymbolKind` for tooling; parameter bindings and plain `let`s are both
/// `Variable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseKind {
    Variable,
    State,
    Store,
    Function,
    /// A UI component invocation: either a user-declared component factory
    /// function (then [`UseRecord::def`] is set) or a builtin component such
    /// as `View`/`Text`/`Row` (no definition).
    Component,
}

impl UseKind {
    fn of(symbol: &symbol_table::Symbol) -> Self {
        match &symbol.kind {
            SymbolKind::Variable => UseKind::Variable,
            SymbolKind::State => UseKind::State,
            SymbolKind::Store => UseKind::Store,
            SymbolKind::Function(_, _, _, _) => UseKind::Function,
        }
    }
}

/// A name-use the checker resolved while analyzing: its source span, the
/// resolved symbol, and (when statically known) the source span of the
/// declaration it binds to. Collected out-of-band for editor tooling
/// (go-to-definition / find-references / hover) the way trait dispatch and
/// list-concat annotations are for codegen — the checker never annotates the
/// AST itself.
#[derive(Debug, Clone, PartialEq)]
pub struct UseRecord {
    pub span: Range<usize>,
    pub name: String,
    pub kind: UseKind,
    pub type_: Option<Type>,
    /// The declaration site of the resolved symbol (`None` when unknown, e.g.
    /// for imported, builtin, or synthesized names).
    pub def: Option<Range<usize>>,
}

/// One `use` binding resolved against the package world, recorded for the
/// runtime: the local name it introduces and the module (by id) it reads.
/// `member: None` binds the target's namespace object; `Some` copies one
/// export (a glob arrives as one binding per contributed member). Only
/// value-relevant bindings are recorded — `use type …` is erased everywhere.
#[derive(Debug, Clone)]
pub struct UseBinding {
    /// The file-scope name this binding introduces (after `as`).
    pub local: String,
    /// Id of the module the binding reads from.
    pub module_id: String,
    /// The export to copy; `None` binds the whole namespace.
    pub member: Option<String>,
    /// Whether `pub use` also puts this name into the module's interface
    /// (the runtime mirrors it into the module's export map).
    pub is_pub: bool,
}

/// The result of analyzing a module: the names/symbols/types it exports, so a
/// loader can seed dependent modules.
#[derive(Debug, Clone, Default)]
pub struct AnalysisResult {
    pub exported_symbols: Vec<(String, Symbol)>,
    pub exported_types: Vec<(String, TypeEntry)>,
    /// Non-fatal diagnostics from this module.
    pub warnings: Vec<XuloError>,
    /// Trait-dispatch call sites: `(call span, mangled impl function name)`.
    /// The loader applies these to the module's AST before codegen/runtime.
    pub trait_dispatch: Vec<(Range<usize>, String)>,
    /// List-concat `+` sites (spans of `BinaryOp`s whose operands are both
    /// `List`); applied to the AST by [`apply_list_concat`] before codegen.
    pub list_concat: Vec<Range<usize>>,
    /// `impl Trait for Type { method }` registrations declared directly in this
    /// module. Exported so dependent modules that import `Type` (unaliased) can
    /// also dispatch `Trait::method(recv)`: the receiver's concrete type carries
    /// its impls, and the mangled impl function is registered module-wide.
    pub impls: Vec<(String, String, String)>,
    /// Every expression's inferred type, keyed by its source span. Out-of-band
    /// data for editor tooling (hover); the checker never annotates the AST.
    pub expr_types: Vec<(Range<usize>, Type)>,
    /// The checker's name resolutions (see [`UseRecord`]).
    pub resolutions: Vec<UseRecord>,
    /// Resolved `use` bindings (see [`UseBinding`]), for the runtime to wire
    /// each module's environment.
    pub use_bindings: Vec<UseBinding>,
    /// Bare calls resolved to a built-in std member: `(callee name span,
    /// qualified callee)`; see [`apply_std_calls`].
    pub std_calls: Vec<(Range<usize>, String)>,
}

/// Every named type reachable from `ty` (E0604's signature walk).
fn collect_named_types(ty: &Type, out: &mut Vec<String>) {
    match ty {
        Type::Named(n, args) => {
            out.push(n.clone());
            for a in args {
                collect_named_types(a, out);
            }
        }
        Type::List(inner) | Type::Optional(inner) | Type::Async(inner) => {
            collect_named_types(inner, out)
        }
        Type::Tuple(parts) => {
            for p in parts {
                collect_named_types(p, out);
            }
        }
        Type::Union(parts) | Type::Intersection(parts) => {
            for p in parts {
                collect_named_types(p, out);
            }
        }
        Type::FnSig { params, ret } => {
            for p in params {
                collect_named_types(p, out);
            }
            if let Some(ret) = ret {
                collect_named_types(ret, out);
            }
        }
        Type::ObjectType(fields) => {
            for (_, t) in fields {
                collect_named_types(t, out);
            }
        }
        _ => {}
    }
}

/// The last `::` segment of a module id, for diagnostics.
fn module_display(id: &str) -> String {
    id.rsplit("::").next().unwrap_or(id).to_string()
}

/// A path head's written name, for diagnostics.
fn head_display(head: &xulo_core::ast::PathHead) -> String {
    match head {
        xulo_core::ast::PathHead::Name(n) | xulo_core::ast::PathHead::Package(n) => n.clone(),
        xulo_core::ast::PathHead::SelfModule => "self".to_string(),
        xulo_core::ast::PathHead::Super => "super".to_string(),
    }
}

/// What a file-scope name is, for visibility and the initialization-order
/// check (spec `modules/visibility.md`, `modules/import-export.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceKind {
    /// A function or other callable value.
    Fn,
    /// A file-scope `let` binding (the E0611-sensitive kind).
    Let,
    /// Any other value binding (const, enum, struct-as-value, use alias).
    Value,
    /// A nominal type, alias, or trait.
    Type,
    /// A child module namespace.
    Module,
}

/// One file-scope name of a module, as the rest of the package sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaceName {
    pub is_pub: bool,
    pub kind: FaceKind,
}

/// One module's file-scope interface, as assembled by the loader for the
/// rest of the package (spec `modules/README.md`).
#[derive(Debug, Clone, Default)]
pub struct ModuleFace {
    /// Every file-scope name the module declares, with its visibility —
    /// present for private names too, so an inaccessible name is `E0602`
    /// rather than "does not export".
    pub names: HashMap<String, FaceName>,
    /// `pub` value symbols: what a `use` binding receives.
    pub exports: Vec<(String, Symbol)>,
    /// `pub` type entries.
    pub types: Vec<(String, TypeEntry)>,
    /// `impl Trait for Type { method }` registrations (dispatch seeding).
    pub impls: Vec<(String, String, String)>,
    /// Child module name -> module id (all children; privacy in `names`).
    pub children: HashMap<String, String>,
    /// A manifest dependency whose exports this compiler cannot see (version
    /// dependency, no local path): every member resolves opaquely instead of
    /// `E0601`/`E0602` (spec `modules/import-export.md`, "Package names and
    /// the manifest" — the mapping is given to the compiler, not the
    /// package's contents).
    pub external: bool,
}

/// Phase 1: a module's file-scope interface, extracted syntactically from its
/// declarations (spec `modules/README.md`). Runs without the world, so `use`
/// paths are not resolved here: the loader's fixpoint completes `pub use`
/// re-exports (target symbol/type/kind), and its face refresh upgrades
/// `exports`/`types` with inferred signatures after phase-2 analysis.
/// `children` maps each `mod` declaration to its child module id
/// (`{module_id}::{name}`, or the bare `name` under the package root, whose
/// id is the empty string).
pub fn module_face(program: &Program, module_id: &str) -> ModuleFace {
    let mut face = ModuleFace::default();
    for statement in &program.statements {
        match statement {
            Statement::Fn(f) => set_face_name(&mut face, &f.name, false, FaceKind::Fn),
            // `#Test` functions are entry points, not bindings.
            Statement::Test(_) => {}
            Statement::Let(b) => set_face_name(
                &mut face,
                &b.name,
                false,
                if b.is_const {
                    FaceKind::Value
                } else {
                    FaceKind::Let
                },
            ),
            Statement::TypeAlias(a) => set_face_name(&mut face, &a.name, false, FaceKind::Type),
            Statement::Enum(e) => {
                set_face_name(&mut face, &e.name, false, FaceKind::Type);
                face_derive_tostring(&mut face, &e.attributes, &e.name, e.type_params.is_empty());
            }
            Statement::Struct(s) => {
                set_face_name(&mut face, &s.name, false, FaceKind::Type);
                face_derive_tostring(&mut face, &s.attributes, &s.name, s.type_params.is_empty());
            }
            Statement::Trait(t) => set_face_name(&mut face, &t.name, false, FaceKind::Type),
            // Inherent methods live in the receiver's object at runtime, not
            // in the trait-dispatch table.
            Statement::Impl(imp) if !imp.is_inherent => {
                for m in &imp.methods {
                    face.impls.push((
                        imp.trait_name.clone(),
                        imp.type_name.clone(),
                        m.name.clone(),
                    ));
                }
            }
            Statement::Mod(m) => {
                set_face_name(&mut face, &m.name, m.is_pub, FaceKind::Module);
                face.children
                    .insert(m.name.clone(), child_module_id(module_id, &m.name));
            }
            Statement::Use(u) => face_use_names(&mut face, u),
            Statement::Export(ex) => face_export(&mut face, &ex.item),
            _ => {}
        }
    }
    face
}

/// Child module id: `form` under the package root, `outer::form` below it.
pub fn child_module_id(module_id: &str, name: &str) -> String {
    if module_id.is_empty() {
        name.to_string()
    } else {
        format!("{module_id}::{name}")
    }
}

/// Declare (or refine) one file-scope name. A `pub use { a }` that ran first
/// keeps its `pub` even when the declaration itself is private.
fn set_face_name(face: &mut ModuleFace, name: &str, is_pub: bool, kind: FaceKind) {
    match face.names.get_mut(name) {
        Some(existing) => {
            existing.is_pub |= is_pub;
            existing.kind = kind;
        }
        None => {
            face.names
                .insert(name.to_string(), FaceName { is_pub, kind });
        }
    }
}

/// `#Derive(ToString)` on a non-generic struct/enum registers the derived
/// method for dispatch, mirroring `validate_derive_decls`.
fn face_derive_tostring(face: &mut ModuleFace, attrs: &[Attribute], name: &str, non_generic: bool) {
    if !non_generic {
        return;
    }
    if attrs
        .iter()
        .any(|a| a.name == "Derive" && a.args.iter().any(|(arg, _)| arg == "ToString"))
    {
        face.impls.push((
            "ToString".to_string(),
            name.to_string(),
            "toString".to_string(),
        ));
    }
}

/// The written name a head-only path binds: `use form;` binds `form`.
fn head_local(head: &PathHead) -> String {
    match head {
        PathHead::Name(n) | PathHead::Package(n) => n.clone(),
        PathHead::SelfModule => "self".to_string(),
        PathHead::Super => "super".to_string(),
    }
}

/// `pub use { a }` marks an already-bound name public; the name may not be
/// declared yet (`pub use` precedes declarations, E0605), so claim it as a
/// value placeholder for the fixpoint to upgrade.
fn mark_face_pub(face: &mut ModuleFace, name: &str) {
    match face.names.get_mut(name) {
        Some(existing) => existing.is_pub = true,
        None => {
            face.names.insert(
                name.to_string(),
                FaceName {
                    is_pub: true,
                    kind: FaceKind::Value,
                },
            );
        }
    }
}

/// File-scope names a `use` binding claims: its privacy is syntactic; a
/// `pub use` re-export's kind and value come from the target module in the
/// loader's fixpoint.
fn face_use_names(face: &mut ModuleFace, u: &UseDecl) {
    match &u.target {
        UseTarget::Path {
            head,
            segments,
            alias,
        } => {
            let local = alias
                .clone()
                .unwrap_or_else(|| segments.last().cloned().unwrap_or_else(|| head_local(head)));
            let kind = if segments.is_empty() {
                FaceKind::Module
            } else {
                FaceKind::Value
            };
            set_face_name(face, &local, u.is_pub, kind);
        }
        UseTarget::Group { head, entries } => {
            for entry in entries {
                let local = entry.alias.clone().unwrap_or_else(|| {
                    entry
                        .segments
                        .last()
                        .cloned()
                        .unwrap_or_else(|| head_local(head))
                });
                let kind = if entry.segments.is_empty() {
                    FaceKind::Module
                } else {
                    FaceKind::Value
                };
                set_face_name(face, &local, u.is_pub, kind);
            }
        }
        UseTarget::Glob { .. } => {}
        UseTarget::Local(names) => {
            for name in names {
                mark_face_pub(face, name);
            }
        }
    }
}

/// The value symbol callers bind for a `pub fn`: the `async T` wrap mirrors
/// `check_fn`.
fn face_fn_symbol(f: &FnDef) -> Symbol {
    let return_type = f.return_type.clone().unwrap_or(Type::Any);
    let symbol_return = if f.is_async {
        Type::Async(Box::new(return_type))
    } else {
        return_type
    };
    Symbol {
        name: f.name.clone(),
        type_: symbol_return.clone(),
        kind: SymbolKind::Function(
            f.type_params.clone(),
            f.params.clone(),
            symbol_return,
            f.bounds.clone(),
        ),
        is_shared: false,
        is_param: false,
        is_const: true,
        is_mutable: false,
    }
}

/// The value symbol for a nominal type exported as a runtime value.
fn face_type_value(name: &str) -> Symbol {
    Symbol {
        name: name.to_string(),
        type_: Type::Named(name.to_string(), Vec::new()),
        kind: SymbolKind::Variable,
        is_shared: false,
        is_param: false,
        is_const: true,
        is_mutable: false,
    }
}

/// A `pub` declaration: the public twin of the private cases in
/// [`module_face`], plus the value symbol/type entry a `use` binding
/// receives (shapes mirror `check_export_item`, without body checking).
fn face_export(face: &mut ModuleFace, item: &ExportItem) {
    match item {
        ExportItem::Fn(f) => {
            set_face_name(face, &f.name, true, FaceKind::Fn);
            face.exports.push((f.name.clone(), face_fn_symbol(f)));
        }
        ExportItem::Let(b) => {
            set_face_name(
                face,
                &b.name,
                true,
                if b.is_const {
                    FaceKind::Value
                } else {
                    FaceKind::Let
                },
            );
            face.exports.push((
                b.name.clone(),
                Symbol {
                    name: b.name.clone(),
                    type_: b.type_annotation.clone().unwrap_or(Type::Any),
                    kind: SymbolKind::Variable,
                    is_shared: b.is_shared,
                    is_param: false,
                    is_const: b.is_const,
                    is_mutable: b.is_mutable,
                },
            ));
        }
        ExportItem::Type(a) => {
            set_face_name(face, &a.name, true, FaceKind::Type);
            face.types.push((
                a.name.clone(),
                TypeEntry {
                    type_params: a.type_params.clone(),
                    type_bounds: a.type_bounds.clone(),
                    kind: TypeEntryKind::Alias(a.type_.clone()),
                },
            ));
        }
        ExportItem::Enum(e) => {
            set_face_name(face, &e.name, true, FaceKind::Type);
            face.exports
                .push((e.name.clone(), face_type_value(&e.name)));
            face.types.push((
                e.name.clone(),
                TypeEntry {
                    type_params: e.type_params.clone(),
                    type_bounds: e.type_bounds.clone(),
                    kind: TypeEntryKind::Enum(e.variants.clone()),
                },
            ));
            face_derive_tostring(face, &e.attributes, &e.name, e.type_params.is_empty());
        }
        ExportItem::Struct(s) => {
            set_face_name(face, &s.name, true, FaceKind::Type);
            face.exports
                .push((s.name.clone(), face_type_value(&s.name)));
            face.types.push((
                s.name.clone(),
                TypeEntry {
                    type_params: s.type_params.clone(),
                    type_bounds: s.type_bounds.clone(),
                    kind: TypeEntryKind::Struct(
                        s.fields
                            .iter()
                            .map(|f| (f.name.clone(), f.type_.clone(), f.default.clone()))
                            .collect(),
                    ),
                },
            ));
            face_derive_tostring(face, &s.attributes, &s.name, s.type_params.is_empty());
        }
        ExportItem::Trait(t) => {
            set_face_name(face, &t.name, true, FaceKind::Type);
            face.types.push((
                t.name.clone(),
                TypeEntry {
                    type_params: t.type_params.clone(),
                    type_bounds: Vec::new(),
                    kind: TypeEntryKind::Trait(TraitDef {
                        methods: t
                            .methods
                            .iter()
                            .map(|m| TraitMethodSig {
                                name: m.name.clone(),
                                params: m
                                    .params
                                    .iter()
                                    .map(|p| p.type_annotation.clone().unwrap_or(Type::Any))
                                    .collect(),
                                return_type: m.return_type.clone().unwrap_or(Type::Any),
                                has_self: m.has_self,
                                is_async: m.is_async,
                            })
                            .collect(),
                    }),
                },
            ));
        }
        ExportItem::Names(names) => {
            for name in names {
                mark_face_pub(face, name);
            }
        }
    }
}

/// The package tree bare paths resolve against: every module keyed by id
/// (the root module's id is [`ModuleWorld::root`]), the manifest's
/// dependency path names, and the structural initialization order
/// (spec `modules/import-export.md`, `modules/visibility.md`).
#[derive(Debug, Clone, Default)]
pub struct ModuleWorld {
    pub modules: HashMap<String, ModuleFace>,
    /// Id of the package's root module.
    pub root: String,
    /// Id of the module being analyzed.
    pub current: String,
    /// Module id -> initialization index (post-order: children first).
    pub init_order: HashMap<String, usize>,
    /// Manifest path name (after aliasing) -> package root module id.
    pub packages: HashMap<String, String>,
    /// Whether to enforce the entry-point rules on the root module
    /// (`E0608`/`E0609`; the run command, not the checker on libraries).
    pub check_entry: bool,
}

impl ModuleWorld {
    /// The package root a module id belongs to: `@pkg` for a dependency's
    /// own tree (`@pkg` / `@pkg::child`), the empty id for this package's
    /// tree. Bare path heads resolve against this root's children.
    pub fn package_root_of(id: &str) -> &str {
        match id.split_once("::") {
            Some((head, _)) if head.starts_with('@') => head,
            Some(_) => "",
            None if id.starts_with('@') => id,
            None => "",
        }
    }
}

/// Semantic checks against a package world: the module's own body, plus
/// every `use`/`pub use` resolved through the interfaces the loader
/// extracted (spec `modules/`). Used by the multi-module loader; single-file
/// analysis keeps [`analyze_with`].
pub fn analyze_module_with(
    program: &Program,
    world: &ModuleWorld,
) -> Result<AnalysisResult, XuloError> {
    // Trait dispatch is package-global (one mangled function per
    // `impl Trait for Type`), so this module sees the impls every other
    // module has already registered. Init order runs dependencies first;
    // the current module's own impls are registered as its body checks.
    let imported_impls: Vec<(String, String, String)> = world
        .modules
        .iter()
        .filter(|(id, _)| *id != &world.current)
        .flat_map(|(_, face)| face.impls.iter().cloned())
        .collect();
    let (result, error) = run_analysis(program, &[], &[], &imported_impls, Some(world), false);
    match error {
        Some(err) => Err(err),
        None => Ok(result),
    }
}

/// Run all semantic checks over a parsed program.
pub fn analyze(program: &Program) -> Result<(), XuloError> {
    analyze_with(program, &[], &[], &[]).map(|_: AnalysisResult| ())
}

/// Semantic checks with imported module symbols/types registered before the
/// module's own declarations. Returns the names/symbols the module exports.
/// `imported_impls` are `impl Trait for Type` registrations gathered from the
/// modules that defined the imported receiver types, so dispatch calls on those
/// types resolve here just as they do in the declaring module.
///
/// Fail-fast: returns `Err` at the first error, matching the compiler's
/// contract. Editor tooling that wants the partial resolutions even when a
/// file has errors should use [`analyze_partial`] instead.
pub fn analyze_with(
    program: &Program,
    imports: &[Symbol],
    imported_types: &[(String, TypeEntry)],
    imported_impls: &[(String, String, String)],
) -> Result<AnalysisResult, XuloError> {
    let (result, error) = run_analysis(
        program,
        imports,
        imported_types,
        imported_impls,
        None,
        false,
    );
    match error {
        Some(err) => Err(err),
        None => Ok(result),
    }
}

/// Like [`analyze_with`], but never fails: it keeps checking after an error
/// (recording only the *first* one, to avoid cascade noise) and returns
/// whatever resolutions/types were gathered before, at, and after the failure
/// point. Editor tooling (hover, go-to-definition, references) uses this so a
/// single broken statement does not blank out the whole document.
pub fn analyze_partial(
    program: &Program,
    imports: &[Symbol],
    imported_types: &[(String, TypeEntry)],
    imported_impls: &[(String, String, String)],
) -> (AnalysisResult, Option<XuloError>) {
    run_analysis(program, imports, imported_types, imported_impls, None, true)
}

fn run_analysis(
    program: &Program,
    imports: &[Symbol],
    imported_types: &[(String, TypeEntry)],
    imported_impls: &[(String, String, String)],
    world: Option<&ModuleWorld>,
    lenient: bool,
) -> (AnalysisResult, Option<XuloError>) {
    let mut analyzer = Analyzer {
        lenient,
        first_error: None,
        table: SymbolTable::new(),
        current_return: None,
        declared_return: false,
        regex_names: std::collections::HashSet::new(),
        std_fns: HashMap::new(),
        type_table: HashMap::new(),
        builtins: builtin_type_entries(),
        generics: Vec::new(),
        bound_shapes: Vec::new(),
        bound_traits: Vec::new(),
        impls: imported_impls.iter().cloned().collect(),
        string_forms: std::collections::HashSet::new(),
        impl_mangled: std::collections::HashSet::new(),
        declared_impls: Vec::new(),
        trait_dispatch: Vec::new(),
        std_calls: Vec::new(),
        list_concat: Vec::new(),
        async_depth: 0,
        component_depth: 0,
        pending_borrows: Vec::new(),
        block_id: 0,
        next_block_id: 1,
        place_depth: 0,
        closure_depth: 0,
        spawn_scopes: Vec::new(),
        capture_operand: false,
        moved: std::collections::HashMap::new(),
        in_move_position: false,
        cur_move_pos: false,
        impl_methods: HashMap::new(),
        in_on_message: false,
        reply_operands: Vec::new(),
        service_reply_types: HashMap::new(),
        locked: Vec::new(),
        lock_depth: 0,
        block_depth: 0,
        no_await_depth: 0,
        test_names: std::collections::HashSet::new(),
        opaque: std::collections::HashSet::new(),
        imports: imports
            .iter()
            .map(|s| (s.name.clone(), s.clone()))
            .collect(),
        imported_types: imported_types
            .iter()
            .map(|(n, t)| (n.clone(), t.clone()))
            .collect(),
        exported: Vec::new(),
        exported_symbols: Vec::new(),
        extern_imports: HashMap::new(),
        warnings: Vec::new(),
        render_locals: Vec::new(),
        in_effect: false,
        in_loop: 0,
        in_defer: 0,
        current_span: 0..0,
        expr_types: Vec::new(),
        resolutions: Vec::new(),
        use_bindings: Vec::new(),
        pending_reexports: Vec::new(),
        world: world.cloned(),
        module_id: world.map(|w| w.current.clone()).unwrap_or_default(),
        namespaces: HashMap::new(),
        use_origins: HashMap::new(),
        use_bound: std::cell::RefCell::new(HashMap::new()),
        glob_unused: std::cell::RefCell::new(Vec::new()),
        type_only: HashMap::new(),
        own_interface: std::collections::HashSet::new(),
        exported_types_for_iface: Vec::new(),
    };
    // Pre-scan `#Test` names so any reference to one — before or after its
    // declaration — fails with E0107 instead of "undefined variable".
    for statement in &program.statements {
        if let Statement::Test(f) = statement {
            analyzer.test_names.insert(f.name.clone());
        }
    }
    // Pre-scan `impl` methods so `spawn.process Tally { … }` can ask whether
    // the service type declares `on_message` wherever the `impl` sits in the
    // file (E0409).
    for statement in &program.statements {
        if let Statement::Impl(imp) = statement {
            analyzer.declared_impls.push(ImplRecord {
                trait_name: if imp.is_inherent {
                    None
                } else {
                    Some(imp.trait_name.clone())
                },
                type_name: imp.type_name.clone(),
                type_params: imp.type_params.clone(),
                type_args: imp.type_args.clone(),
                bounds: imp.type_bounds.clone(),
                methods: imp
                    .methods
                    .iter()
                    .map(|m| ImplMethodType {
                        name: m.name.clone(),
                        ret: if m.is_async {
                            Type::Async(Box::new(m.return_type.clone().unwrap_or(Type::Any)))
                        } else {
                            m.return_type.clone().unwrap_or(Type::Any)
                        },
                    })
                    .collect(),
            });
            for m in &imp.methods {
                analyzer
                    .impl_methods
                    .entry(imp.type_name.clone())
                    .or_default()
                    .push(ImplMethodSig {
                        name: m.name.clone(),
                        has_self: m.params.first().is_some_and(|p| p.name == "self"),
                        param_count: m.params.len(),
                        param_types: m
                            .params
                            .iter()
                            .map(|p| p.type_annotation.clone().unwrap_or(Type::Any))
                            .collect(),
                    });
            }
        }
    }
    // Hand-written `impl ToString for T` contributes to the string-form index
    // wherever it sits, so `str(v)` and `${v}` (E0212) and derive field
    // checks (E0801) see it regardless of position (spec
    // `builtins/intrinsic-functions.md`, `types/traits.md`).
    for statement in &program.statements {
        if let Statement::Impl(imp) = statement
            && imp.trait_name == "ToString"
            && !imp.is_inherent
        {
            analyzer.string_forms.insert(imp.type_name.clone());
        }
    }
    // 导入模块的 `impl ToString for T` / `#Derive(ToString)` 同样赋予 T
    // 字符串形式：跨模块 `str(v)`（E0212）与派生字段检查（E0801）要能看到。
    for (trait_name, type_name, method) in imported_impls {
        if trait_name == "ToString" && method == "toString" {
            analyzer.string_forms.insert(type_name.clone());
        }
    }
    // Header forms (`mod`, `use`, `pub use`) may appear only before any
    // other top-level declaration (spec modules/source-files.md, E0605).
    if analyzer.first_error.is_none() || lenient {
        let mut seen_body = false;
        let mut header_err = None;
        for statement in &program.statements {
            let is_header = matches!(statement, Statement::Mod(_) | Statement::Use(_));
            if is_header && seen_body {
                let span = match statement {
                    Statement::Mod(m) => m.span.clone(),
                    Statement::Use(u) => u.span.clone(),
                    _ => unreachable!(),
                };
                header_err = Some(
                    XuloError::new(
                        ErrorKind::Semantic,
                        "module and use declarations must appear at the top of the file",
                    )
                    .at(span)
                    .with_code("E0605"),
                );
                break;
            }
            if !is_header {
                seen_body = true;
            }
        }
        if let Some(err) = header_err {
            if lenient {
                if analyzer.first_error.is_none() {
                    analyzer.first_error = Some(err);
                }
            } else {
                analyzer.first_error = Some(err);
            }
        }
    }
    // The entry-point rules apply to an executable package's root module
    // (spec modules/source-files.md, E0608/E0609).
    if let Some(world) = &analyzer.world
        && world.check_entry
        && world.current == world.root
        && analyzer.first_error.is_none()
    {
        let main = program.statements.iter().find_map(|st| match st {
            Statement::Fn(f) if f.name == "main" => Some(f),
            Statement::Export(xulo_core::ast::ExportStmt {
                item: ExportItem::Fn(f),
                ..
            }) if f.name == "main" => Some(f),
            _ => None,
        });
        match main {
            None => {
                analyzer.first_error = Some(
                    XuloError::new(ErrorKind::Semantic, "entry module does not declare `main`")
                        .at(0..0)
                        .with_code("E0608"),
                );
            }
            Some(f) => {
                if !f.params.is_empty() {
                    analyzer.first_error = Some(
                        XuloError::new(ErrorKind::Semantic, "`main` must take no parameters")
                            .at(f.name_span.clone())
                            .with_code("E0609"),
                    );
                } else if let Some(rt) = &f.return_type
                    && !matches!(rt, Type::Named(n, _) if n == "View" || n == "unit")
                {
                    analyzer.first_error = Some(
                        XuloError::new(ErrorKind::Semantic, "`main` must return unit or View")
                            .at(f.name_span.clone())
                            .with_code("E0609"),
                    );
                }
            }
        }
    }
    // The module's interface is the set of every `pub` declaration in the
    // file, collected up front so a `pub` signature may name a `pub` type
    // declared later in the file (E0604), and so a local `pub use { x }` —
    // checked after the pass, once every declaration is known — can tell an
    // interface name from a private one (E0602).
    for statement in &program.statements {
        if let Statement::Export(export) = statement {
            let name = match &export.item {
                xulo_core::ast::ExportItem::Fn(f) => Some(f.name.clone()),
                xulo_core::ast::ExportItem::Let(b) => Some(b.name.clone()),
                xulo_core::ast::ExportItem::Type(a) => Some(a.name.clone()),
                xulo_core::ast::ExportItem::Trait(t) => Some(t.name.clone()),
                xulo_core::ast::ExportItem::Enum(e) => Some(e.name.clone()),
                xulo_core::ast::ExportItem::Struct(st) => Some(st.name.clone()),
                xulo_core::ast::ExportItem::Names(ns) => {
                    analyzer.own_interface.extend(ns.iter().cloned());
                    None
                }
            };
            if let Some(name) = name {
                analyzer.own_interface.insert(name);
            }
        }
        if let Statement::Mod(m) = statement
            && m.is_pub
        {
            analyzer.own_interface.insert(m.name.clone());
        }
        // A `pub` import block exports every one of its items (spec ffi.md),
        // so each local name is part of the interface from the first pass.
        if let Statement::ExternBlock(eb) = statement
            && eb.is_pub
        {
            for item in &eb.items {
                analyzer.own_interface.insert(item.local_name.clone());
            }
        }
    }
    // The attribute registry runs before the check loop: `#Derive(ToString)`
    // registers its impl up front so a `str(s)` or `s.to_string()` use of the
    // derived type resolves no matter where it sits in the file (spec
    // `types/traits.md`, "Attributes").
    if let Err(err) = analyzer.validate_attributes(program) {
        analyzer.first_error = Some(err);
    }
    if analyzer.first_error.is_none() || lenient {
        if lenient {
            for statement in &program.statements {
                analyzer.check_statement_lenient(statement);
            }
        } else {
            for statement in &program.statements {
                if let Err(err) = analyzer.check_statement(statement) {
                    analyzer.first_error = Some(err);
                    break;
                }
            }
        }
    }
    // Local-form `pub use { … }` sits in the header, before every
    // declaration, so its names are validated only now that the
    // declarations have been seen: each name must be part of the module's
    // interface (a `pub` declaration, a `pub mod`, or a name a `use` brought
    // in) — a private local declaration is `E0602`, an unbound name `E0101`
    // (spec `modules/import-export.md`, "Well-formedness table").
    let pending = std::mem::take(&mut analyzer.pending_reexports);
    if !pending.is_empty() && (analyzer.first_error.is_none() || lenient) {
        let mut reexport_err = None;
        for (name, span) in pending {
            let in_interface = analyzer.own_interface.contains(&name)
                || analyzer.use_bindings.iter().any(|b| b.local == name)
                || analyzer.type_only.contains_key(&name)
                || analyzer.imported_types.contains_key(&name);
            if in_interface {
                let is_enum = matches!(
                    analyzer.type_table.get(&name).map(|e| &e.kind),
                    Some(TypeEntryKind::Enum(_))
                );
                let already = analyzer.exported_symbols.iter().any(|(n, _)| n == &name);
                if !already {
                    if is_enum {
                        analyzer.exported_symbols.push((
                            name.clone(),
                            Symbol {
                                name: name.clone(),
                                type_: Type::Named(name.clone(), Vec::new()),
                                kind: symbol_table::SymbolKind::Variable,
                                is_shared: false,
                                is_param: false,
                                is_const: true,
                                is_mutable: false,
                            },
                        ));
                    } else if let Some(sym) = analyzer.table.lookup(&name).cloned() {
                        analyzer.exported_symbols.push((name.clone(), sym));
                    }
                }
                // Feed the exported-types fill below so a re-exported enum's
                // `Enum` entry reaches `face.types` for importers.
                if !analyzer.exported.iter().any(|n| n == &name) {
                    analyzer.exported.push(name.clone());
                }
                analyzer.own_interface.insert(name);
                continue;
            }
            let bound = analyzer.table.lookup(&name).is_some()
                || analyzer.type_table.contains_key(&name)
                || analyzer.namespaces.contains_key(&name);
            reexport_err = Some(if !bound {
                XuloError::new(
                    ErrorKind::Semantic,
                    format!("cannot re-export `{name}`: it is not bound in this module"),
                )
                .at(span)
                .with_code("E0101")
            } else if matches!(
                analyzer.type_table.get(&name).map(|e| &e.kind),
                Some(TypeEntryKind::Alias(_))
            ) {
                XuloError::new(
                    ErrorKind::Semantic,
                    format!(
                        "cannot re-export `{name}`: it is a type with no runtime value; use `pub type` or `pub enum`"
                    ),
                )
                .at(span)
                .with_code("E0602")
            } else {
                XuloError::new(
                    ErrorKind::Semantic,
                    format!(
                        "cannot re-export `{name}`: it is a private declaration of this module"
                    ),
                )
                .at(span)
                .with_code("E0602")
            });
            break;
        }
        if let Some(err) = reexport_err {
            if lenient {
                if analyzer.first_error.is_none() {
                    analyzer.first_error = Some(err);
                }
            } else {
                analyzer.first_error = Some(err);
            }
        }
    }
    let mut result = AnalysisResult {
        exported_symbols: analyzer.exported_symbols,
        exported_types: Vec::new(),
        warnings: analyzer.warnings.clone(),
        trait_dispatch: analyzer.trait_dispatch.clone(),
        std_calls: analyzer.std_calls.clone(),
        list_concat: analyzer.list_concat.clone(),
        // `impls` 只含本模块自己声明的 impl（派生也在此列）。导入的 impl
        // 已经出现在声明模块的 face 里，每个模块都能看到，重复导出会让
        // 后来者把「已导入的 impl」误当成重复声明（`#Derive` 的 E0805）。
        impls: module_face(program, &analyzer.module_id).impls,
        expr_types: analyzer.expr_types.clone(),
        resolutions: analyzer.resolutions.clone(),
        use_bindings: analyzer.use_bindings,
    };
    for name in &analyzer.exported {
        if let Some(entry) = analyzer.type_table.get(name).cloned() {
            result.exported_types.push((name.clone(), entry));
        }
    }
    result
        .exported_types
        .extend(analyzer.exported_types_for_iface);
    // W0102: a `use` binding nothing this module reads warns once; a glob
    // whose contributed names are all unused warns once at the glob span.
    let unused = analyzer.use_bound.into_inner();
    for (name, span) in &unused {
        analyzer.warnings.push(
            XuloError::new(ErrorKind::Warning, format!("unused `use` of `{name}`"))
                .at(span.clone())
                .with_code("W0102"),
        );
    }
    for (span, names) in analyzer.glob_unused.into_inner() {
        let contributed_used = names.iter().any(|n| !unused.contains_key(n));
        if names.is_empty() || !contributed_used {
            analyzer.warnings.push(
                XuloError::new(
                    ErrorKind::Warning,
                    "unused `use` glob: no name it contributes is referenced",
                )
                .at(span)
                .with_code("W0102"),
            );
        }
    }
    result.warnings = analyzer.warnings.clone();
    (result, analyzer.first_error)
}

/// Apply the trait-dispatch annotations found during analysis back onto the
/// AST, setting `Call.trait_impl` for each `Trait::method(recv, ...)` call.
/// The checker never holds `&mut Expression`, so annotations travel out-of-band
/// and are applied here in a single pass after checking.
/// Rewrite the bare callees that resolved to a built-in std member
/// (`floor(…)` → `std::math::floor(…)`) so AOT lowering sees only the
/// qualified intrinsic form. The interpreter is unaffected: it binds the
/// bare name itself from the `use` import.
pub fn apply_std_calls(program: &mut Program, calls: &[(Range<usize>, String)]) {
    walk_program(program, &mut |expr| {
        if let Expression::Call(call) = expr
            && call.object.is_none()
            && let Some((_, qualified)) = calls.iter().find(|(span, _)| *span == call.span)
        {
            call.callee = qualified.clone();
        }
    });
}

pub fn apply_trait_dispatch(program: &mut Program, dispatch: &[(Range<usize>, String)]) {
    walk_program(program, &mut |expr| {
        if let Expression::Call(call) = expr
            && let Some((_, mangled)) = dispatch.iter().find(|(span, _)| *span == call.span)
        {
            call.trait_impl = Some(mangled.clone());
        }
    });
}

/// Apply the list-concat annotations found during analysis: every `a + b`
/// whose operands checked out as `List` types gets `BinaryOp.list_concat` set,
/// so the JS codegen emits an array concatenation instead of the bare `+`.
pub fn apply_list_concat(program: &mut Program, spans: &[Range<usize>]) {
    walk_program(program, &mut |expr| {
        if let Expression::BinaryOp(bin) = expr
            && spans.contains(&bin.span)
        {
            bin.list_concat = true;
        }
    });
}

/// Visit every expression in a program (statement bodies, exports, UI
/// elements, and nested expressions) with `f`.
fn walk_program(program: &mut Program, f: &mut dyn FnMut(&mut Expression)) {
    for statement in &mut program.statements {
        walk_stmt(statement, f);
    }
}

fn walk_stmt(stmt: &mut Statement, f: &mut dyn FnMut(&mut Expression)) {
    match stmt {
        Statement::Fn(fd) | Statement::Test(fd) => walk_block(&mut fd.body, f),
        Statement::Let(l) => {
            if let Some(value) = &mut l.value {
                walk_expr(value, f);
            }
        }
        Statement::Return(r) => {
            if let Some(value) = &mut r.value {
                walk_expr(value, f);
            }
        }
        Statement::For(fr) => {
            walk_expr(&mut fr.iterable, f);
            walk_block(&mut fr.body, f);
        }
        Statement::While(w) => {
            walk_expr(&mut w.condition, f);
            walk_block(&mut w.body, f);
        }
        Statement::Assign(a) => {
            match &mut a.target {
                AssignTarget::Member(object, _) => walk_expr(object, f),
                AssignTarget::Index(object, index) => {
                    walk_expr(object, f);
                    walk_expr(index, f);
                }
                AssignTarget::Name(_) => {}
            }
            walk_expr(&mut a.value, f);
        }
        Statement::Expr(e) => walk_expr(&mut e.expr, f),
        Statement::Block(b) => walk_block(b, f),
        Statement::Defer(d) => match &mut d.body {
            xulo_core::ast::DeferBody::Block(b) => walk_block(b, f),
            xulo_core::ast::DeferBody::Expr(e) => walk_expr(e, f),
        },
        Statement::Try(t) => {
            walk_block(&mut t.try_block, f);
            walk_block(&mut t.catch_block, f);
        }
        Statement::Throw(e) => walk_expr(e, f),
        Statement::Export(e) => walk_export(&mut e.item, f),
        Statement::State(s) => {
            if let Some(value) = &mut s.binding.value {
                walk_expr(value, f);
            }
        }
        Statement::Store(s) => walk_expr(&mut s.value, f),
        Statement::Effect(e) => {
            walk_block(&mut e.closure.body, f);
            if let Some(deps) = &mut e.deps {
                for dep in deps {
                    walk_expr(dep, f);
                }
            }
        }
        Statement::Environment(_) => {}
        // An extern block carries types only: no expressions to walk.
        Statement::ExternBlock(_) => {}
        Statement::Component(c) => {
            for arg in &mut c.args {
                walk_expr(&mut arg.value, f);
            }
            walk_ui_elements(&mut c.children, f);
        }
        Statement::Impl(imp) => {
            for method in &mut imp.methods {
                walk_block(&mut method.body, f);
            }
        }
        Statement::TypeAlias(_)
        | Statement::Enum(_)
        | Statement::Struct(_)
        | Statement::Mod(_)
        | Statement::Use(_)
        | Statement::Trait(_)
        | Statement::Break
        | Statement::Continue => {}
    }
}

fn walk_export(item: &mut ExportItem, f: &mut dyn FnMut(&mut Expression)) {
    match item {
        ExportItem::Fn(fd) => walk_block(&mut fd.body, f),
        ExportItem::Let(l) => {
            if let Some(value) = &mut l.value {
                walk_expr(value, f);
            }
        }
        ExportItem::Names(_)
        | ExportItem::Type(_)
        | ExportItem::Enum(_)
        | ExportItem::Struct(_)
        | ExportItem::Trait(_) => {}
    }
}

fn walk_ui_elements(elements: &mut [UiElement], f: &mut dyn FnMut(&mut Expression)) {
    for element in elements {
        match element {
            UiElement::Component(c) => {
                for arg in &mut c.args {
                    walk_expr(&mut arg.value, f);
                }
                walk_ui_elements(&mut c.children, f);
            }
            UiElement::Expr(e) => walk_expr(e, f),
            UiElement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                walk_expr(condition, f);
                walk_ui_elements(then_branch, f);
                if let Some(els) = else_branch {
                    walk_ui_elements(els, f);
                }
            }
            UiElement::For { iterable, body, .. } => {
                walk_expr(iterable, f);
                walk_ui_elements(body, f);
            }
            UiElement::Group(g) => walk_ui_elements(g, f),
            UiElement::Text(_) => {}
        }
    }
}

fn walk_block(block: &mut Block, f: &mut dyn FnMut(&mut Expression)) {
    for statement in &mut block.statements {
        walk_stmt(statement, f);
    }
}

fn walk_expr(expr: &mut Expression, f: &mut dyn FnMut(&mut Expression)) {
    f(expr);
    match expr {
        Expression::Literal { value, .. } => match value {
            Literal::List(items) => {
                for item in items {
                    walk_expr(item, f);
                }
            }
            Literal::Object(fields) => {
                for field in fields {
                    match field {
                        ObjectField::Field { value, .. } => walk_expr(value, f),
                        ObjectField::Spread { value } => walk_expr(value, f),
                    }
                }
            }
            _ => {}
        },
        Expression::Call(call) => {
            if let Some(object) = &mut call.object {
                walk_expr(object, f);
            }
            for arg in &mut call.arguments {
                walk_expr(&mut arg.value, f);
            }
        }
        Expression::BinaryOp(b) => {
            walk_expr(&mut b.left, f);
            walk_expr(&mut b.right, f);
        }
        Expression::Unary(u) => walk_expr(&mut u.operand, f),
        Expression::If(e) => {
            walk_expr(&mut e.condition, f);
            walk_block(&mut e.then_branch, f);
            if let Some(els) = &mut e.else_branch {
                walk_block(els, f);
            }
        }
        Expression::Ternary(t) => {
            walk_expr(&mut t.condition, f);
            walk_expr(&mut t.then_value, f);
            walk_expr(&mut t.else_value, f);
        }
        Expression::Match(m) => {
            walk_expr(&mut m.value, f);
            for arm in &mut m.arms {
                if let Some(guard) = &mut arm.guard {
                    walk_expr(guard, f);
                }
                match &mut arm.value {
                    xulo_core::ast::MatchBody::Expr(e) => walk_expr(e, f),
                    xulo_core::ast::MatchBody::Block(b) => walk_block(b, f),
                }
            }
        }
        Expression::Member(m) => walk_expr(&mut m.object, f),
        Expression::Index(i) => {
            walk_expr(&mut i.object, f);
            walk_expr(&mut i.index, f);
        }
        Expression::Nullish(n) => {
            walk_expr(&mut n.left, f);
            walk_expr(&mut n.right, f);
        }
        Expression::Range(r) => {
            walk_expr(&mut r.start, f);
            walk_expr(&mut r.end, f);
        }
        Expression::Await { expr: inner, .. } => walk_expr(inner, f),
        Expression::FnExpr(fd) => walk_block(&mut fd.body, f),
        Expression::Spread { expr: inner, .. } => walk_expr(inner, f),
        Expression::CallValue(c) => {
            walk_expr(&mut c.callee, f);
            for arg in &mut c.arguments {
                walk_expr(&mut arg.value, f);
            }
        }
        Expression::Component(c) => {
            for arg in &mut c.args {
                walk_expr(&mut arg.value, f);
            }
            walk_ui_elements(&mut c.children, f);
        }
        Expression::Tuple { items, .. } => {
            for item in items {
                walk_expr(item, f);
            }
        }
        Expression::TypedMap(tm) => {
            for entry in &mut tm.entries {
                walk_expr(&mut entry.key, f);
                walk_expr(&mut entry.value, f);
            }
        }
        Expression::Is(is) => walk_expr(&mut is.expr, f),
        Expression::Panic { expr: inner, .. } => walk_expr(inner, f),
        Expression::Propagate { expr: inner, .. } => walk_expr(inner, f),
        Expression::Arrow(arrow) => match &mut arrow.body {
            xulo_core::ast::ArrowBody::Expr(e) => walk_expr(e, f),
            xulo_core::ast::ArrowBody::Block(b) => walk_block(b, f),
        },
        Expression::Spawn(spawn) => match &mut spawn.body {
            xulo_core::ast::SpawnBody::Async(b) => walk_block(b, f),
            xulo_core::ast::SpawnBody::Service { fields, .. } => {
                for field in fields {
                    walk_expr(&mut field.value, f);
                }
            }
        },
        Expression::Lock(lock) => walk_block(&mut lock.body, f),
        Expression::Identifier { .. } | Expression::EnumRef(_) | Expression::Binding { .. } => {}
    }
}

/// One step of a place path: a field (or tuple position) name, or a list
/// subscript. `Sub(None)` is a subscript whose index is not an integer
/// literal, which is never assumed distinct from any other subscript.
#[derive(Debug, Clone, PartialEq)]
enum PlaceSeg {
    Field(String),
    Sub(Option<i64>),
}

/// A place rooted at a binding: `xs[0].y` is root `xs` with
/// [`PlaceSeg::Sub(Some(0))`, `PlaceSeg::Field("y")`].
#[derive(Debug, Clone, PartialEq)]
struct Place {
    root: String,
    segs: Vec<PlaceSeg>,
    display: String,
    /// Span of the root binding's name, used to point the diagnostic at the
    /// place rather than wherever the checker happened to be.
    span: std::ops::Range<usize>,
}

/// The place a place-expression denotes, when it is a chain of field accesses
/// and subscripts rooted at a binding.
fn place_of(expr: &Expression) -> Option<Place> {
    let mut segs = Vec::new();
    let mut cur = expr;
    let (root, span) = loop {
        match cur {
            Expression::Identifier { name, span } => break (name.clone(), span.clone()),
            Expression::Member(m) => {
                segs.push(PlaceSeg::Field(m.property.clone()));
                cur = &m.object;
            }
            Expression::Index(i) => {
                let index = match &*i.index {
                    Expression::Literal {
                        value: xulo_core::ast::Literal::Number(n),
                        ..
                    } if n.fract() == 0.0 && n.is_finite() => Some(*n as i64),
                    _ => None,
                };
                segs.push(PlaceSeg::Sub(index));
                cur = &i.object;
            }
            _ => return None,
        }
    };
    segs.reverse();
    let mut display = root.clone();
    for seg in &segs {
        match seg {
            PlaceSeg::Field(f) => display.push_str(&format!(".{f}")),
            PlaceSeg::Sub(Some(n)) => display.push_str(&format!("[{n}]")),
            PlaceSeg::Sub(None) => display.push_str("[...]"),
        }
    }
    Some(Place {
        root,
        segs,
        display,
        span,
    })
}

/// A borrow that outlives its call because the call returned a `Task`: the
/// borrowed place, whether the parameter was `mut`, the task binding that
/// carries the call's result, and the block that holds the binding (spec
/// `memory-and-runtime.md`, Span end).
#[derive(Debug, Clone)]
struct PendingBorrow {
    place: Place,
    is_mut: bool,
    task: String,
    block: usize,
}

/// Two places overlap unless, walking their paths from the base, the first
/// position where they differ proves them disjoint: different field names
/// (tuple positions included), or the same list subscripted by two distinct
/// integer literals (spec `memory-and-runtime.md`, Overlap).
fn places_overlap(a: &Place, b: &Place) -> bool {
    if a.root != b.root {
        return false;
    }
    for (x, y) in a.segs.iter().zip(b.segs.iter()) {
        match (x, y) {
            (PlaceSeg::Field(f), PlaceSeg::Field(g)) if f != g => return false,
            (PlaceSeg::Sub(Some(i)), PlaceSeg::Sub(Some(j))) if i != j => return false,
            _ => {}
        }
    }
    // No divergence proving them disjoint: one is a prefix of the other, or
    // every shared step matched (a non-literal index is never assumed
    // distinct from anything).
    true
}

/// The place an assignment target writes: `xs[0].y` roots at `xs` with the
/// same path an expression would have.
fn assign_target_place(target: &AssignTarget) -> Option<Place> {
    match target {
        AssignTarget::Name(name) => Some(Place {
            root: name.clone(),
            segs: Vec::new(),
            display: name.clone(),
            span: 0..0,
        }),
        AssignTarget::Member(object, property) => {
            let mut place = place_of(object)?;
            place.segs.push(PlaceSeg::Field(property.clone()));
            place.display.push_str(&format!(".{property}"));
            Some(place)
        }
        AssignTarget::Index(object, index) => {
            let mut place = place_of(object)?;
            let idx = match &**index {
                Expression::Literal {
                    value: xulo_core::ast::Literal::Number(n),
                    ..
                } if n.fract() == 0.0 && n.is_finite() => Some(*n as i64),
                _ => None,
            };
            match idx {
                Some(n) => {
                    place.segs.push(PlaceSeg::Sub(Some(n)));
                    place.display.push_str(&format!("[{n}]"));
                }
                None => {
                    place.segs.push(PlaceSeg::Sub(None));
                    place.display.push_str("[...]");
                }
            }
            Some(place)
        }
    }
}

/// The task bindings an `await` operand waits for: `await t`,
/// `await Task::all([t, u])`, and `await Task::timeout(t, ms)` each settle the
/// tasks named in the operand (spec, Span end). Anything else is left alone —
/// the borrow simply lasts on.
fn collect_task_names(expr: &Expression, out: &mut Vec<String>) {
    match expr {
        Expression::Identifier { name, .. } => out.push(name.clone()),
        Expression::Call(call) => {
            for arg in &call.arguments {
                collect_task_names(&arg.value, out);
            }
        }
        Expression::CallValue(cv) => {
            for arg in &cv.arguments {
                collect_task_names(&arg.value, out);
            }
        }
        Expression::Literal {
            value: xulo_core::ast::Literal::List(items),
            ..
        } => {
            for item in items {
                collect_task_names(item, out);
            }
        }
        Expression::Tuple { items, .. } => {
            for item in items {
                collect_task_names(item, out);
            }
        }
        _ => {}
    }
}

/// How a place expression renders in a diagnostic: the root binding's name
/// with its path (`a.b`, `a[...]`), not the whole expression's source text.
fn place_root_display(expr: &Expression) -> String {
    match expr {
        Expression::Identifier { name, .. } => name.clone(),
        Expression::Member(m) => format!("{}.{}", place_root_display(&m.object), m.property),
        Expression::Index(i) => format!("{}[...]", place_root_display(&i.object)),
        _ => "<value>".to_string(),
    }
}

fn assign_target_name(target: &AssignTarget) -> String {
    match target {
        AssignTarget::Name(name) => name.clone(),
        AssignTarget::Member(object, property) => {
            format!("{}.{property}", target_base(object))
        }
        AssignTarget::Index(object, _) => format!("{}[...]", target_base(object)),
    }
}

fn target_base(expr: &Expression) -> String {
    match expr {
        Expression::Identifier { name, .. } => name.clone(),
        Expression::Member(m) => format!("{}.{}", target_base(&m.object), m.property),
        Expression::Index(i) => format!("{}[...]", target_base(&i.object)),
        Expression::Call(c) => c.callee.clone(),
        _ => "expr".into(),
    }
}

/// Every payload slot of an enum's variants as (type, label span) pairs for
/// derive field checking (E0801); positional and named payloads alike, keyed
/// by the variant's name span as the diagnostic location.
fn enum_field_forms(variants: &[EnumVariant]) -> Vec<(Type, Range<usize>)> {
    let mut out = Vec::new();
    for v in variants {
        if let Some(slots) = &v.payload {
            for s in slots {
                out.push((s.type_.clone(), v.name_span.clone()));
            }
        }
    }
    out
}

/// A registry entry for the closed attribute registry (spec
/// `types/traits.md`, "Attributes"): `#Test` and `#Derive`, nothing else.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AttrKind {
    Test,
    Derive,
}

/// The closed registry: every written attribute name must resolve here or
/// it is `E0108`. There are no inert attributes.
fn attribute_registry(name: &str) -> Option<AttrKind> {
    match name {
        "Test" => Some(AttrKind::Test),
        "Derive" => Some(AttrKind::Derive),
        _ => None,
    }
}

/// The noun phrase a declaration kind answers with in `E0109`
/// (`attribute `Derive` is not valid on a function`).
const DECL_PHRASE_FN: &str = "a function";
const DECL_PHRASE_STRUCT: &str = "a struct";
const DECL_PHRASE_ENUM: &str = "an enum";
const DECL_PHRASE_TRAIT: &str = "a trait";
const DECL_PHRASE_IMPL: &str = "an impl";

/// Record `err` unless an earlier one already stands: registry validation
/// keeps walking after a failure so later declarations still register their
/// derives, but only the first error is reported (fail-fast contract).
macro_rules! record_first {
    ($slot:expr, $err:expr) => {
        if $slot.is_none() {
            $slot = Some($err);
        }
    };
}

impl Analyzer {
    fn warn(&mut self, message: String) {
        self.warnings
            .push(XuloError::new(ErrorKind::Warning, message).at(self.current_span.clone()));
    }

    /// Raise a semantic error attached to the most recently checked
    /// expression's span (see [`Analyzer::current_span`]).
    fn err(&self, message: impl Into<String>) -> XuloError {
        XuloError::new(ErrorKind::Semantic, message).at(self.current_span.clone())
    }

    /// Like [`Analyzer::err`], but tags the diagnostic with a fine-grained
    /// code from `spec/type-system/errors.md` (e.g. `E0230`).
    fn err_code(&self, code: &'static str, message: impl Into<String>) -> XuloError {
        self.err(message).with_code(code)
    }

    /// Record that the name at `span` resolved to `symbol` (out-of-band, for
    /// editor tooling's go-to-definition / find-references / hover). Also
    /// drops an entry for bindings the cursor can sit on directly.
    fn record_use(&mut self, span: Range<usize>, name: &str, symbol: &Symbol) {
        let def = self.table.lookup_def(name).flatten().cloned();
        self.resolutions.push(UseRecord {
            span,
            name: name.to_string(),
            kind: UseKind::of(symbol),
            type_: Some(symbol.type_.clone()),
            def,
        });
    }

    /// Record a declaration site itself, so the cursor over a declared name
    /// (e.g. `let m ...`) resolves to that declaration.
    fn record_decl(&mut self, span: Range<usize>, name: &str, kind: UseKind, ty: Type) {
        self.resolutions.push(UseRecord {
            span: span.clone(),
            name: name.to_string(),
            kind,
            type_: Some(ty),
            def: Some(span),
        });
    }

    /// Record a callee-name resolution for a plain call, when the call's bare
    /// callee name has a precise source span (see `Call.callee_span`).
    fn record_call_callee(&mut self, call: &xulo_core::ast::Call, symbol: &Symbol) {
        // A call through the name reads it: mark the `use` binding used
        // (W0102) even though no identifier expression evaluates it.
        self.mark_name_used(&call.callee);
        if let Some(span) = &call.callee_span {
            let def = self.table.lookup_def(&call.callee).flatten().cloned();
            self.resolutions.push(UseRecord {
                span: span.clone(),
                name: call.callee.clone(),
                kind: UseKind::of(symbol),
                type_: Some(symbol.type_.clone()),
                def,
            });
        }
    }
}

impl Analyzer {
    /// Resolve a name to its symbol, rejecting references from inside an
    /// `@Effect` closure to bindings that live only in the component's render
    /// code: those are hoisted above the render closure and are not in scope
    /// when the effect runs. A `#Test fn`'s identifier is declared but binds no
    /// value, so mentioning it is the E0107 reference ban (spec testing.md).
    fn lookup_symbol(&self, name: &str) -> SResult<Symbol> {
        let render_scoped = self
            .render_locals
            .last()
            .is_some_and(|locals| locals.contains(name));
        if self.in_effect && render_scoped {
            return Err(self.err(format!(
                "`@Effect` closures cannot reference `{name}`: it is declared inside the component body and is out of scope when the effect runs"
            )));
        }
        if let Some(sym) = self.table.lookup(name) {
            self.mark_name_used(name);
            return Ok(sym.clone());
        }
        if let Some(span) = self.type_only.get(name) {
            return Err(self
                .err_code(
                    "E0606",
                    format!("`{name}` is a type-only binding and is not a value"),
                )
                .at(span.clone()));
        }
        // `Theme.Dark`: `.` is field access only, and an enum type has no
        // value binding of its own — name the fix instead of the miss
        // (spec `types/enums.md`, `expressions/path-and-access.md`).
        if let Some(entry) = self.type_entry(name)
            && matches!(entry.kind, TypeEntryKind::Enum(_))
        {
            return Err(self.err(format!(
                "`{name}` is an enum type with no value binding; a variant is `{name}::Variant`, not `{name}.…`"
            )));
        }
        if self.test_names.contains(name) {
            return Err(self.err_code(
                "E0107",
                format!("`{name}` is a test function and cannot be referenced"),
            ));
        }
        Err(self.err(format!("undefined variable `{name}`")))
    }
}

impl Analyzer {
    /// Validate every written attribute against the closed registry before
    /// the check loop (spec `types/traits.md`, "Attributes"): unknown names
    /// are `E0108`, a registered attribute on a declaration the registry
    /// does not allow is `E0109`, and arguments that do not fit the
    /// registered shape are `E0110` (`E0111` for an unregistered derive
    /// target, `E0112` for a derive on a generic type, `E0805` for a
    /// repeated or hand-conflicting `#Derive(ToString)`). A valid derive
    /// registers its impl up front — and its field types without a string
    /// form are `E0801` — so `str(s)` on the derived type resolves no
    /// matter where the use sits.
    fn validate_attributes(&mut self, program: &Program) -> SResult<()> {
        let mut first: Option<XuloError> = None;
        // Derive targets to field-check after the walk: (attribute span,
        // fields as (type, label span)).
        let mut derives: DeriveList = Vec::new();

        for statement in &program.statements {
            // A `TestDecl`'s `#Test` is part of the production: no registry
            // lookup runs for it (spec testing.md).
            if matches!(statement, Statement::Test(_)) {
                continue;
            }
            match statement {
                Statement::Fn(f) => self.validate_fn_attrs(f, false, &mut first),
                Statement::Struct(s) => self.validate_derive_decls(
                    &s.attributes,
                    DECL_PHRASE_STRUCT,
                    &s.name,
                    !s.type_params.is_empty() || !s.type_bounds.is_empty(),
                    || {
                        s.fields
                            .iter()
                            .map(|f| (f.type_.clone(), f.name_span.clone()))
                            .collect()
                    },
                    &mut first,
                    &mut derives,
                ),
                Statement::Enum(e) => self.validate_derive_decls(
                    &e.attributes,
                    DECL_PHRASE_ENUM,
                    &e.name,
                    !e.type_params.is_empty() || !e.type_bounds.is_empty(),
                    || enum_field_forms(&e.variants),
                    &mut first,
                    &mut derives,
                ),
                Statement::Trait(t) => {
                    for attr in &t.attributes {
                        self.attr_gate(attr, DECL_PHRASE_TRAIT, &mut first);
                    }
                }
                Statement::Impl(imp) => {
                    for attr in &imp.attributes {
                        self.attr_gate(attr, DECL_PHRASE_IMPL, &mut first);
                    }
                    for m in &imp.methods {
                        self.validate_fn_attrs(m, false, &mut first);
                    }
                }
                Statement::Export(ex) => match &ex.item {
                    // `pub` is part of the test form's fixed shape: a `#Test`
                    // fn that is exported is `E0110`, not a `TestDecl`.
                    ExportItem::Fn(f) => self.validate_fn_attrs(f, true, &mut first),
                    ExportItem::Struct(s) => self.validate_derive_decls(
                        &s.attributes,
                        DECL_PHRASE_STRUCT,
                        &s.name,
                        !s.type_params.is_empty() || !s.type_bounds.is_empty(),
                        || {
                            s.fields
                                .iter()
                                .map(|f| (f.type_.clone(), f.name_span.clone()))
                                .collect()
                        },
                        &mut first,
                        &mut derives,
                    ),
                    ExportItem::Enum(e) => self.validate_derive_decls(
                        &e.attributes,
                        DECL_PHRASE_ENUM,
                        &e.name,
                        !e.type_params.is_empty() || !e.type_bounds.is_empty(),
                        || enum_field_forms(&e.variants),
                        &mut first,
                        &mut derives,
                    ),
                    ExportItem::Trait(t) => {
                        for attr in &t.attributes {
                            self.attr_gate(attr, DECL_PHRASE_TRAIT, &mut first);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        // Every target is registered now: a field or payload type with no
        // string form fails the derive (spec `types/traits.md`, "Derive").
        for (_span, fields) in &derives {
            for (ty, label_span) in fields {
                if !self.type_has_string_form(ty) {
                    record_first!(
                        first,
                        self.err_code(
                            "E0801",
                            format!("`{}` does not implement `ToString`", ty.display())
                        )
                        .at(label_span.clone())
                    );
                }
            }
        }

        match first {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    /// Gate one attribute through `E0108` (unknown) and `E0109` (registry
    /// does not allow it here); returns its kind when it may proceed.
    fn attr_gate(
        &self,
        attr: &Attribute,
        target: &'static str,
        first: &mut Option<XuloError>,
    ) -> Option<AttrKind> {
        let Some(kind) = attribute_registry(&attr.name) else {
            record_first!(
                *first,
                self.err_code("E0108", format!("attribute `{}` is not defined", attr.name))
                    .at(attr.name_span.clone())
            );
            return None;
        };
        let allowed = match kind {
            AttrKind::Test => target == DECL_PHRASE_FN,
            AttrKind::Derive => target == DECL_PHRASE_STRUCT || target == DECL_PHRASE_ENUM,
        };
        if !allowed {
            record_first!(
                *first,
                self.err_code(
                    "E0109",
                    format!("attribute `{}` is not valid on {}", attr.name, target)
                )
                .at(attr.name_span.clone())
            );
            return None;
        }
        Some(kind)
    }

    /// Validate the attribute list of one function declaration: `#Test` on a
    /// `fn` must be the test form — no title group beyond the optional
    /// string, no parameters, no return type, no generics, no `where`
    /// clause, no `pub` — or it is `E0110` (a correct sequence at file scope
    /// became a `TestDecl` instead; a group holding an identifier is the same
    /// error; anywhere else the form's file-scope requirement still fails
    /// here).
    fn validate_fn_attrs(&mut self, f: &FnDef, is_pub: bool, first: &mut Option<XuloError>) {
        for attr in &f.attributes {
            let Some(kind) = self.attr_gate(attr, DECL_PHRASE_FN, first) else {
                continue;
            };
            if kind == AttrKind::Test {
                let shape_ok = attr.args.is_empty()
                    && f.params.is_empty()
                    && f.return_type.is_none()
                    && f.type_params.is_empty()
                    && f.bounds.is_empty()
                    && !is_pub;
                if !shape_ok {
                    record_first!(
                        *first,
                        self.err_code(
                            "E0110",
                            "attribute `Test` takes the form `#Test (\"…\")? [async] fn name() { … }`"
                        )
                        .at(attr.name_span.clone())
                    );
                }
            }
        }
    }

    /// Validate the attributes of a `struct`/`enum` declaration and register
    /// every valid `#Derive(ToString)` for field checking. `fields` is
    /// deferred as a closure so the caller's borrows end first.
    #[allow(clippy::too_many_arguments)]
    fn validate_derive_decls(
        &mut self,
        attrs: &[Attribute],
        target: &'static str,
        type_name: &str,
        is_generic: bool,
        fields: impl FnOnce() -> Vec<(Type, Range<usize>)>,
        first: &mut Option<XuloError>,
        derives: &mut DeriveList,
    ) {
        let Some(attr) = attrs
            .iter()
            .find(|a| self.attr_gate(a, target, first) == Some(AttrKind::Derive))
        else {
            // Still walk the rest for E0108/E0109 on other names.
            for a in attrs {
                if a.name == "Derive" {
                    continue;
                }
                self.attr_gate(a, target, first);
            }
            return;
        };
        self.validate_derive_attr(attr, type_name, is_generic, fields(), first, derives);
        for a in attrs {
            if std::ptr::eq(a, attr) {
                continue;
            }
            if self.attr_gate(a, target, first) == Some(AttrKind::Derive) {
                // A second `#Derive(ToString)` on the same declaration is the
                // conflicting-impl error (spec: "a second `#Derive(ToString)`").
                record_first!(
                    *first,
                    self.err_code(
                        "E0805",
                        format!(
                            "`ToString` is already implemented for `{}` (method `toString`)",
                            type_name
                        )
                    )
                    .at(a.span.clone())
                );
            }
        }
    }

    /// Shape-check and register one `#Derive(...)` attribute (spec
    /// `types/traits.md`, "Derive"): a parenthesized list of registered
    /// targets (`E0110`/`E0111`), no type parameters (`E0112`), no
    /// conflicting impl (`E0805`), then the impl registrations that make
    /// `str(s)` and `s.to_string()` resolve the derived type.
    fn validate_derive_attr(
        &mut self,
        attr: &Attribute,
        type_name: &str,
        is_generic: bool,
        fields: Vec<(Type, Range<usize>)>,
        first: &mut Option<XuloError>,
        derives: &mut DeriveList,
    ) {
        if !attr.has_parens || attr.args.is_empty() {
            record_first!(
                *first,
                self.err_code(
                    "E0110",
                    "attribute `Derive` takes a parenthesized list of trait names"
                )
                .at(attr.name_span.clone())
            );
            return;
        }
        let mut targets_ok = true;
        for (arg, arg_span) in &attr.args {
            if attribute_registry(arg) != Some(AttrKind::Derive)
                && !matches!(arg.as_str(), "ToString")
            {
                record_first!(
                    *first,
                    self.err_code(
                        "E0111",
                        format!("`{arg}` is not a registered derive target")
                    )
                    .at(arg_span.clone())
                    .with_note("registered targets: `ToString`")
                );
                targets_ok = false;
            }
        }
        if !targets_ok {
            return;
        }
        if is_generic {
            record_first!(
                *first,
                self.err_code(
                    "E0112",
                    format!("cannot derive `{}` for a generic type", attr.args[0].0)
                )
                .at(attr.name_span.clone())
            );
            return;
        }
        if !self.impls.insert((
            "ToString".to_string(),
            type_name.to_string(),
            "toString".to_string(),
        )) {
            record_first!(
                *first,
                self.err_code(
                    "E0805",
                    format!(
                        "`ToString` is already implemented for `{}` (method `toString`)",
                        type_name
                    )
                )
                .at(attr.span.clone())
            );
            return;
        }
        let mangled = impl_fn_name("ToString", type_name, "toString");
        if !self.impl_mangled.insert(mangled.clone()) {
            record_first!(
                *first,
                self.err(format!(
                    "trait dispatch name `{mangled}` is claimed by more than one `impl` (trait/type/method names that mangle to the same string collide, e.g. `a_b`/`c` vs `a`/`b_c`)"
                ))
                .at(attr.span.clone())
            );
            return;
        }
        if self.table.lookup(&mangled).is_some() {
            record_first!(
                *first,
                self.err(format!(
                    "trait dispatch name `{mangled}` collides with a function of the same name"
                ))
                .at(attr.span.clone())
            );
            return;
        }
        self.impl_methods
            .entry(type_name.to_string())
            .or_default()
            .push(ImplMethodSig {
                name: "toString".to_string(),
                has_self: true,
                param_count: 1,
                param_types: vec![Type::Any],
            });
        self.declared_impls.push(ImplRecord {
            trait_name: Some("ToString".to_string()),
            type_name: type_name.to_string(),
            type_params: Vec::new(),
            type_args: Vec::new(),
            bounds: Vec::new(),
            methods: vec![ImplMethodType {
                name: "toString".to_string(),
                ret: Type::String,
            }],
        });
        self.string_forms.insert(type_name.to_string());
        derives.push((attr.span.clone(), fields));
    }

    /// Does `t` have a string form — a base type, or a named type with a
    /// `ToString` impl (spec `builtins/intrinsic-functions.md`)? Shared by
    /// E0212 (`str`/`${}`) and E0801 (`#Derive` fields). Spec-silent forms
    /// (structural objects, functions, tuples, unions, async) stay
    /// permissive so dynamic paths keep working.
    fn type_has_string_form(&self, t: &Type) -> bool {
        match t {
            Type::String | Type::Number | Type::Boolean | Type::Any | Type::Object => true,
            Type::Null | Type::List(_) => false,
            // An in-scope generic parameter's form is unknown until
            // instantiation: `str(v)` inside `fn show<T>(v: T)` must not be
            // rejected by the absence of a bound (spec has no "is base or
            // ToString" bound to write).
            Type::Named(name, _) => {
                self.string_forms.contains(name) || self.generics.iter().any(|g| g == name)
            }
            Type::Optional(inner) => self.type_has_string_form(inner),
            _ => true,
        }
    }

    /// Check a statement, swallowing its error in lenient mode: the first
    /// error is remembered, later ones are dropped (cascade noise), and
    /// checking continues so sibling statements still resolve.
    fn check_statement_lenient(&mut self, statement: &Statement) {
        if let Err(err) = self.check_statement(statement)
            && self.first_error.is_none()
        {
            self.first_error = Some(err);
        }
    }

    fn check_statement(&mut self, statement: &Statement) -> SResult<()> {
        match statement {
            Statement::Fn(f) => {
                self.generics.extend(f.type_params.iter().cloned());
                let result = self.check_fn(f);
                self.generics
                    .truncate(self.generics.len().saturating_sub(f.type_params.len()));
                result
            }
            Statement::Test(f) => self.check_test(f),
            Statement::Let(binding) => {
                if binding.memo {
                    self.require_fn_body_top_level("@Memo")?;
                    if binding.value.is_none() {
                        return Err(self.err("@Memo requires an initializer: `@Memo let x = expr`"));
                    }
                    if let Some(deps) = &binding.memo_deps {
                        for dep in deps {
                            self.check_expression(dep)?;
                        }
                    }
                }
                if let Some(annotation) = &binding.type_annotation {
                    self.check_type(annotation)?;
                }
                // Inside a `spawn` block a `let x = move y` / `copy y`
                // initializer *is* the capture of `y`: the operand is the one
                // place an enclosing binding may be named.
                // `move` is legal as a whole `let` initializer (E0403).
                self.in_move_position = true;
                let saved_capture = self.capture_operand;
                self.capture_operand = !self.spawn_scopes.is_empty()
                    && matches!(
                        &binding.value,
                        Some(Expression::Unary(u))
                            if matches!(
                                u.operator,
                                xulo_core::ast::UnaryOperator::Move
                                    | xulo_core::ast::UnaryOperator::Copy
                            )
                    );
                let value_type = match &binding.value {
                    Some(value) => self.check_expression(value),
                    None => Ok(binding.type_annotation.clone().unwrap_or(Type::Any)),
                };
                self.capture_operand = saved_capture;
                let value_type = value_type?;

                // A file-scope initializer that directly reads a `use`-bound
                // name of a later-initializing module is `E0611` (spec
                // modules/import-export.md); the direct subset only — no
                // descent into closures.
                if self.world.is_some()
                    && self.block_depth == 0
                    && let Some(value) = &binding.value
                {
                    self.check_init_order(value)?;
                }

                // A call that returns a `Task` keeps its arguments borrowed
                // until that task settles: the borrow outlives the call and
                // is released by an `await` of this binding (spec
                // `memory-and-runtime.md`, Span end).
                if let Some(Expression::Call(call)) = &binding.value
                    && (matches!(value_type, Type::Async(_))
                        || matches!(&value_type, Type::Named(n, _) if n == "Task"))
                {
                    self.record_task_borrows(call, &binding.name);
                }

                // Tuple destructuring: `let (a, b, c) = expr`
                if let Some(tuple_names) = &binding.tuple_names {
                    let inner_type = match &value_type {
                        Type::List(inner) => *inner.clone(),
                        Type::Any => Type::Any,
                        other => {
                            return Err(self.err(format!(
                                "cannot destructure `{}` as a list (expected a list, found `{}`)",
                                binding.name,
                                other.name()
                            )));
                        }
                    };
                    let mut seen = std::collections::HashSet::new();
                    for name in tuple_names {
                        if !seen.insert(name.clone()) {
                            return Err(self.err(format!(
                                "duplicate binding name `{}` in tuple destructuring",
                                name
                            )));
                        }
                        let declared = self.table.declare_with_def(
                            Symbol {
                                name: name.clone(),
                                type_: inner_type.clone(),
                                kind: SymbolKind::Variable,
                                is_shared: binding.is_shared,
                                is_param: false,
                                is_const: binding.is_const,
                                is_mutable: false,
                            },
                            Some(binding.name_span.clone()),
                        );
                        if !declared {
                            return Err(self.err(format!(
                                "binding `{}` is already declared in this scope",
                                name
                            )));
                        }
                        self.record_decl(
                            binding.name_span.clone(),
                            name,
                            UseKind::Variable,
                            inner_type.clone(),
                        );
                    }
                    return Ok(());
                }

                // Object destructuring: `let { name, age: user_age } = expr`
                if let Some(fields) = &binding.object_destructuring {
                    let inner_type = match &value_type {
                        Type::Object | Type::Named(_, _) => value_type.clone(),
                        Type::Any => Type::Any,
                        other => {
                            return Err(self.err(format!(
                                "cannot destructure `{}` as an object (expected an object, found `{}`)",
                                binding.name,
                                other.name()
                            )));
                        }
                    };
                    let mut seen_names = std::collections::HashSet::new();
                    let mut seen_aliases = std::collections::HashSet::new();
                    for (field_name, alias) in fields {
                        if !seen_names.insert(field_name.to_string()) {
                            return Err(self.err(format!(
                                "duplicate field name `{}` in object destructuring",
                                field_name
                            )));
                        }
                        let bind_name = alias.as_deref().unwrap_or(field_name);
                        if !seen_aliases.insert(bind_name.to_string()) {
                            return Err(self.err(format!(
                                "duplicate binding name `{}` in object destructuring",
                                bind_name
                            )));
                        }
                        // If we have a Named type with struct fields, look up the field type
                        let field_type = if let Type::Named(type_name, _) = &inner_type {
                            if let Some(entry) = self.type_table.get(type_name) {
                                if let TypeEntryKind::Struct(fields) = &entry.kind {
                                    fields
                                        .iter()
                                        .find(|(n, _, _)| n == field_name)
                                        .map(|(_, t, _)| t.clone())
                                        .unwrap_or(Type::Any)
                                } else {
                                    Type::Any
                                }
                            } else {
                                Type::Any
                            }
                        } else {
                            Type::Any
                        };
                        let declared = self.table.declare_with_def(
                            Symbol {
                                name: bind_name.to_string(),
                                type_: field_type.clone(),
                                kind: SymbolKind::Variable,
                                is_shared: binding.is_shared,
                                is_param: false,
                                is_const: binding.is_const,
                                is_mutable: binding.is_mutable,
                            },
                            Some(binding.name_span.clone()),
                        );
                        if !declared {
                            return Err(self.err(format!(
                                "binding `{}` is already declared in this scope",
                                bind_name
                            )));
                        }
                        self.record_decl(
                            binding.name_span.clone(),
                            bind_name,
                            UseKind::Variable,
                            field_type,
                        );
                    }
                    return Ok(());
                }

                let mut ok = true;
                if let Some(annotation) = &binding.type_annotation
                    && !self.assignable(&value_type, annotation)
                {
                    // Value-aware fallback for string-literal types
                    // (`"active" | "inactive"`): a direct literal may match.
                    let literal_ok = match &binding.value {
                        Some(value) => self.literal_matches(value, annotation),
                        None => false,
                    };
                    if !literal_ok {
                        // An out-of-range numeric literal is E0209, ahead of
                        // the E0201 the mismatch would otherwise raise.
                        if let Some(value) = &binding.value
                            && let Some(e) =
                                self.fixed_literal_error(value, annotation, value.span().clone())
                        {
                            return Err(e);
                        }
                        ok = false;
                    }
                }
                if !ok {
                    // An annotated binding that does not accept its value is
                    // the registered `E0201` case (spec `type-system/errors.md`:
                    // "an annotation or annotated initializer").
                    return Err(self.err_code(
                        "E0201",
                        format!(
                            "cannot bind a value of type `{}` to `let {}: {}`",
                            value_type.name(),
                            binding.name,
                            binding.type_annotation.as_ref().unwrap().name()
                        ),
                    ));
                }
                let declared_type = binding.type_annotation.clone().unwrap_or(value_type);
                let declared = self.table.declare_with_def(
                    Symbol {
                        name: binding.name.clone(),
                        type_: declared_type.clone(),
                        kind: SymbolKind::Variable,
                        is_shared: binding.is_shared,
                        is_param: false,
                        is_const: binding.is_const,
                        is_mutable: binding.is_mutable,
                    },
                    Some(binding.name_span.clone()),
                );
                if !declared {
                    return Err(self.err(format!(
                        "binding `{}` is already declared in this scope",
                        binding.name
                    )));
                }
                self.record_decl(
                    binding.name_span.clone(),
                    &binding.name,
                    UseKind::Variable,
                    declared_type,
                );
                Ok(())
            }
            Statement::Assign(assign) => self.check_assign(assign),
            Statement::TypeAlias(alias) => self.check_type_alias(alias),
            Statement::Enum(e) => self.check_enum(e),
            Statement::Struct(s) => self.check_struct(s),
            Statement::Trait(t) => self.check_trait(t),
            Statement::Impl(imp) => self.check_impl(imp),
            Statement::Return(stmt) => {
                let Some(value) = &stmt.value else {
                    // Bare `return;` is valid in any function (docs EBNF §7),
                    // but only inside a function body.
                    if self.current_return.is_none() {
                        return Err(self
                            .err("`return` may only be used at the top level of a function body"));
                    }
                    return Ok(());
                };
                // `move` is legal as a whole `return` value (E0403).
                self.in_move_position = true;
                let value_type = self.check_expression(value)?;
                if let Some(expected) = &self.current_return {
                    let target = match expected {
                        Type::Async(inner) => inner.as_ref(),
                        other => other,
                    };
                    if !self.assignable(&value_type, target) && !self.literal_matches(value, target)
                    {
                        if let Some(e) =
                            self.fixed_literal_error(value, target, value.span().clone())
                        {
                            return Err(e);
                        }
                        return Err(self.err(format!(
                            "return type mismatch: expected `{}`, found `{}`",
                            expected.name(),
                            value_type.name()
                        )));
                    }
                } else {
                    return Err(self.err(
                        format!(
                            "`return` may only be used at the top level of a function body (found a value of type `{}`)",
                            value_type.name()
                        ),
                    ));
                }
                Ok(())
            }
            Statement::For(stmt) => {
                let iterable_type = self.check_expression(&stmt.iterable)?;
                match iterable_type {
                    Type::List(_) | Type::Any => {}
                    other => {
                        return Err(self.err(format!(
                            "for loop must iterate over a `List`, found `{}`",
                            other.name()
                        )));
                    }
                }
                self.table.push_scope();
                let iter_type = match &iterable_type {
                    Type::List(elem) => (**elem).clone(),
                    _ => Type::Any,
                };
                self.table.declare_with_def(
                    Symbol {
                        name: stmt.iter_var.clone(),
                        type_: iter_type,
                        kind: SymbolKind::Variable,
                        is_shared: false,
                        is_param: false,
                        is_const: false,
                        is_mutable: true,
                    },
                    Some(stmt.iter_var_span.clone()),
                );
                self.in_loop += 1;
                let result = self.check_block(&stmt.body);
                self.in_loop -= 1;
                self.table.pop_scope();
                result?;
                Ok(())
            }
            Statement::While(stmt) => {
                let condition = self.check_expression(&stmt.condition)?;
                if !self.assignable(&condition, &Type::Boolean) {
                    return Err(self.err(format!(
                        "while condition must be a `bool`, found `{}`",
                        condition.name()
                    )));
                }
                self.in_loop += 1;
                let result = self.check_block(&stmt.body);
                self.in_loop -= 1;
                result?;
                Ok(())
            }
            Statement::Expr(stmt) => {
                self.check_expr_stmt(stmt)?;
                Ok(())
            }
            Statement::Block(block) => {
                self.check_block(block)?;
                Ok(())
            }
            Statement::Try(try_stmt) => {
                self.check_block(&try_stmt.try_block)?;
                self.table.push_scope();
                self.table.declare_with_def(
                    Symbol {
                        name: try_stmt.catch_var.clone(),
                        type_: Type::Any,
                        kind: SymbolKind::Variable,
                        is_shared: false,
                        is_param: false,
                        is_const: true,
                        is_mutable: false,
                    },
                    Some(try_stmt.catch_var_span.clone()),
                );
                let result = self.check_block(&try_stmt.catch_block);
                self.table.pop_scope();
                result
            }
            Statement::Throw(expr) => {
                self.check_expression(expr)?;
                Ok(())
            }
            Statement::Mod(m) => self.check_mod(m),
            Statement::Use(u) => self.check_use(u),
            Statement::Export(export) => self.check_export(export),
            Statement::State(state) => self.check_state(&state.binding),
            Statement::Store(store) => self.check_store(store),
            Statement::Effect(effect) => self.check_effect(effect),
            Statement::Environment(env) => self.check_environment(env),
            Statement::Component(component) => self.check_component(component),
            Statement::Break => {
                if self.in_loop == 0 {
                    return Err(self.err("`break` may only be used inside a `for` or `while` loop"));
                }
                Ok(())
            }
            Statement::Continue => {
                if self.in_loop == 0 {
                    return Err(
                        self.err("`continue` may only be used inside a `for` or `while` loop")
                    );
                }
                Ok(())
            }
            Statement::Defer(d) => self.check_defer(d),
            Statement::ExternBlock(b) => self.check_extern_block(b),
        }
    }

    /// `[pub] extern "abi" { … }` — ABI registry (E0902), ABI-safe
    /// signatures (E0901), and one callable binding per item (spec ffi.md).
    /// Items register in source order like any other module-level
    /// declaration: a call before the block is an unknown function, exactly
    /// as for a `fn` declared later. `pub` exports every item.
    fn check_extern_block(&mut self, b: &xulo_core::ast::ExternBlockDecl) -> SResult<()> {
        self.check_abi_tag(&b.abi, b.span.clone())?;
        for item in &b.items {
            for p in &item.params {
                if let Some(ty) = &p.type_annotation {
                    self.check_boundary_type(ty, &b.abi, true, p.span.clone())?;
                }
            }
            if let Some(ret) = &item.return_type {
                self.check_boundary_type(ret, &b.abi, false, item.span.clone())?;
            }
            let ret = item.return_type.clone().unwrap_or(Type::Any);
            let symbol = Symbol {
                name: item.local_name.clone(),
                type_: ret.clone(),
                kind: SymbolKind::Function(Vec::new(), item.params.clone(), ret, Vec::new()),
                is_shared: false,
                is_param: false,
                is_const: true,
                is_mutable: false,
            };
            if !self
                .table
                .declare_with_def(symbol, Some(item.local_name_span.clone()))
            {
                return Err(self.err(format!("function `{}` is already defined", item.local_name)));
            }
            // Keyed by the local binding, valued by the span `declare_with_def`
            // recorded — E0903 matches a use against it to tell this import
            // from a shadowing re-declaration of the same spelling (the two
            // spans differ when an `as` alias is written).
            self.extern_imports
                .insert(item.local_name.clone(), item.local_name_span.clone());
            if b.is_pub {
                self.exported.push(item.local_name.clone());
                if let Some(sym) = self.table.lookup(&item.local_name).cloned() {
                    self.exported_symbols.push((item.local_name.clone(), sym));
                }
            }
        }
        Ok(())
    }

    /// The closed ABI registry (spec ffi.md, "ABI tags"): a tag outside it is
    /// `E0902`, whose note lists the registered set.
    fn check_abi_tag(&self, abi: &str, span: Range<usize>) -> SResult<()> {
        if REGISTERED_ABIS.contains(&abi) {
            return Ok(());
        }
        let registered = REGISTERED_ABIS
            .iter()
            .map(|t| format!("`\"{t}\"`"))
            .collect::<Vec<_>>()
            .join(", ");
        Err(self
            .err_code("E0902", format!("ABI `{abi}` is not registered"))
            .with_note(format!("registered ABIs: {registered}"))
            .at(span))
    }

    /// E0901: only the closed ABI-safe table may cross an extern boundary
    /// (spec ffi.md, "Types at the boundary") — `unit` is return-only and
    /// `Ptr?` is the one optional spelling. The value is resolved first, so
    /// an alias naming a table type is accepted while any other type is not.
    fn check_boundary_type(
        &self,
        ty: &Type,
        abi: &str,
        is_param: bool,
        span: Range<usize>,
    ) -> SResult<()> {
        let resolved = self.resolve_alias(ty, 0);
        let ok = match &resolved {
            Type::Boolean => true,
            Type::Named(name, _) => {
                if name == "unit" {
                    !is_param
                } else {
                    fixed_bit(name).is_some() || name == "Ptr"
                }
            }
            // Exactly `Ptr?` may be optional: `i32?`, `string?`, and `Ptr??`
            // are E0901.
            Type::Optional(inner) => matches!(
                inner.as_ref(),
                Type::Named(name, args) if name == "Ptr" && args.is_empty()
            ),
            _ => false,
        };
        if !ok {
            return Err(self
                .err_code(
                    "E0901",
                    format!(
                        "`{}` is not allowed across an `extern \"{abi}\"` boundary",
                        resolved.name()
                    ),
                )
                .with_note(
                    "ABI-safe types: `bool`, `i8`…`u64`, `f32`, `f64`, `usize`, `isize`, `unit`, `Ptr`, `Ptr?`",
                )
                .at(span));
        }
        // Well-formedness of an admitted head: `Ptr<u8>` / `i32<x>` are
        // E0230, not E0901 (spec ffi.md).
        self.check_type(&resolved)
    }

    fn check_defer(&mut self, d: &xulo_core::ast::DeferStmt) -> SResult<()> {
        self.in_defer += 1;
        let result = match &d.body {
            xulo_core::ast::DeferBody::Block(b) => self.check_block(b),
            xulo_core::ast::DeferBody::Expr(e) => self.check_expression(e).map(|_| ()),
        };
        self.in_defer -= 1;
        result
    }

    fn check_fn(&mut self, f: &xulo_core::ast::FnDef) -> SResult<()> {
        // `main` is invoked with no arguments by both runtimes (`main()` /
        // `call_fn(main, &[])`); a required parameter would silently bind
        // `undefined`/`null` and diverge, so reject it up front.
        if f.name == "main" && f.params.iter().any(|p| p.default.is_none()) {
            return Err(self.err(format!(
                "`main` must not require arguments (found `{}`): both runtimes call it with none",
                f.params
                    .iter()
                    .filter(|p| p.default.is_none())
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let mut param_names = std::collections::HashSet::new();
        for p in &f.params {
            if !param_names.insert(p.name.clone()) {
                return Err(self.err(format!(
                    "parameter `{}` shadows an earlier parameter of `{}`",
                    p.name, f.name
                )));
            }
            // Annotations and defaults are validated once the parameters are in
            // scope (`declare_params_with_defaults`), so a default can
            // reference an earlier parameter (`fn f(a, b = a)`).
        }
        // `pub extern "Abi" fn …` (spec ffi.md): the tag must be registered
        // (E0902) and the signature obeys the boundary type rules (E0901) —
        // the same rules an import item follows. The body is an ordinary
        // Xulo function body, and the function is an ordinary module-level
        // `fn` for every other purpose (no direct-call restriction: E0903
        // applies only to imported symbols).
        if let Some(abi) = &f.abi {
            self.check_abi_tag(abi, f.name_span.clone())?;
            for p in &f.params {
                if let Some(ty) = &p.type_annotation {
                    self.check_boundary_type(ty, abi, true, p.span.clone())?;
                }
            }
            if let Some(rt) = &f.return_type {
                self.check_boundary_type(rt, abi, false, f.name_span.clone())?;
            }
        }
        let return_type = f.return_type.clone().unwrap_or(Type::Any);
        self.check_type(&return_type)?;
        // Callers see an async function's result as `async T` (`await`
        // unwraps it); inside the body `return`/trailing expressions stay `T`.
        let symbol_return = if f.is_async {
            Type::Async(Box::new(return_type.clone()))
        } else {
            return_type.clone()
        };
        let symbol = Symbol {
            name: f.name.clone(),
            type_: symbol_return.clone(),
            kind: SymbolKind::Function(
                f.type_params.clone(),
                f.params.clone(),
                symbol_return,
                f.bounds.clone(),
            ),
            is_shared: false,
            is_param: false,
            is_const: true,
            is_mutable: false,
        };
        if !self
            .table
            .declare_with_def(symbol, Some(f.name_span.clone()))
        {
            return Err(self.err(format!("function `{}` is already defined", f.name)));
        }
        // A user function must not collide with a trait-dispatch name
        // (`impl_{Trait}_{Type}_{method}`). `impl` registration checks this
        // direction too, so the collision is order-independent: without this
        // check, an `impl` declared *before* a same-named `fn` silently
        // overwrote the mangled impl in the native runtime (dispatch then
        // called the wrong function), while the JS target kept both apart.
        if self.impl_mangled.contains(&f.name) {
            return Err(self.err(format!(
                "function `{}` collides with a trait dispatch impl of the same name (from `impl Trait for Type`)",
                f.name
            )));
        }

        // `T: Trait` bounds: validate the trait exists and make the bound shape
        // visible to `member_field_type` / `assignable` while checking the body.
        let mut bound_map = HashMap::new();
        let mut trait_map: HashMap<String, Vec<String>> = HashMap::new();
        for bound in &f.bounds {
            let shape = self.bound_shape(bound)?;
            bound_map.insert(bound.param.clone(), shape);
            trait_map
                .entry(bound.param.clone())
                .or_default()
                .extend(bound.traits.iter().cloned());
        }
        self.bound_shapes.push(bound_map);
        self.bound_traits.push(trait_map);

        self.table.push_scope();
        self.declare_params_with_defaults(&f.params)?;
        let saved = self.current_return.replace(return_type.clone());
        let saved_declared = self.declared_return;
        self.declared_return = f.return_type.is_some();
        let saved_async = self.async_depth;
        self.async_depth = if f.is_async { saved_async + 1 } else { 0 };
        // This function is its own scope: decorators are only legal at the top
        // level of a `View` function (itself), never inside a nested
        // function, and `await` needs this function to be `async` (docs §12).
        // `no_await_depth` is a property of the *current* function's if/match
        // arms, so a nested function body must not inherit it.
        let saved_no_await = self.no_await_depth;
        self.no_await_depth = 0;
        // A nested `fn` declared inside a lock body runs later, outside the
        // critical section, so it does not inherit the enclosing lock.
        let saved_lock = self.lock_depth;
        self.lock_depth = 0;
        // A named `fn` body is its own context: a nested declaration does not
        // inherit the enclosing closure's capture rules.
        let saved_closure = self.closure_depth;
        self.closure_depth = 0;
        let is_component = self.is_component_type(&return_type);
        let saved_component = self.component_depth;
        let saved_block = self.block_depth;
        self.component_depth = if is_component { 1 } else { 0 };
        self.block_depth = 0;
        if is_component {
            // Top-level `let`s and nested `fn` declarations of a component body
            // become part of the render closure at codegen time, so they are
            // off-limits to hoisted setup code (`@Effect`/`@State`/`@Store`).
            let mut render_locals = std::collections::HashSet::new();
            for stmt in &f.body.statements {
                match stmt {
                    Statement::Let(b) => {
                        render_locals.insert(b.name.clone());
                    }
                    Statement::Fn(nested) => {
                        render_locals.insert(nested.name.clone());
                    }
                    _ => {}
                }
            }
            self.render_locals.push(render_locals);
        }
        // Moves do not cross function boundaries: a binding of the same name
        // in another body is a different binding.
        let saved_moved = std::mem::take(&mut self.moved);
        let result = self.check_block_implicit(&f.body);
        self.moved = saved_moved;
        if is_component {
            self.render_locals.pop();
        }
        self.block_depth = saved_block;
        self.component_depth = saved_component;
        self.async_depth = saved_async;
        self.no_await_depth = saved_no_await;
        self.lock_depth = saved_lock;
        self.closure_depth = saved_closure;
        self.current_return = saved;
        self.declared_return = saved_declared;
        result?;
        self.table.pop_scope();
        self.bound_shapes.pop();
        self.bound_traits.pop();
        Ok(())
    }

    /// Check a `test [async] fn` body (spec testing.md): exactly as an
    /// implicit `fn(): unit` body — no parameters, no return type, so a
    /// trailing expression is a statement, not an implicit return, and
    /// `return e` checks `e` against the unit result. The identifier binds no
    /// value: it is never declared in the symbol table (any reference is the
    /// E0107 ban, enforced by [`Analyzer::lookup_symbol`] and
    /// [`Analyzer::check_call`] via the pre-scanned `test_names`), and two
    /// tests may share a name. The optional `async` marker opens an `async`
    /// context exactly as on `fn(): unit` (`await` legal; E0501 without it).
    fn check_test(&mut self, f: &xulo_core::ast::FnDef) -> SResult<()> {
        self.table.push_scope();
        // No declared return type: `current_return` is the unit stand-in
        // (`Any`, matching how every other unit-returning form is checked),
        // and `declared_return` stays false so a trailing expression is not an
        // implicit return.
        let saved = self.current_return.replace(Type::Any);
        let saved_declared = self.declared_return;
        self.declared_return = false;
        let saved_async = self.async_depth;
        self.async_depth = if f.is_async { saved_async + 1 } else { 0 };
        let saved_no_await = self.no_await_depth;
        self.no_await_depth = 0;
        let saved_lock = self.lock_depth;
        self.lock_depth = 0;
        let saved_closure = self.closure_depth;
        self.closure_depth = 0;
        let saved_block = self.block_depth;
        self.block_depth = 0;
        // Moves do not cross function boundaries.
        let saved_moved = std::mem::take(&mut self.moved);
        let result = self.check_block_implicit(&f.body);
        self.moved = saved_moved;
        self.block_depth = saved_block;
        self.no_await_depth = saved_no_await;
        self.lock_depth = saved_lock;
        self.closure_depth = saved_closure;
        self.async_depth = saved_async;
        self.current_return = saved;
        self.declared_return = saved_declared;
        result?;
        self.table.pop_scope();
        Ok(())
    }

    /// Validate a parameter list and declare each parameter in scope, one at a
    /// time. A parameter's default value is checked *before* the parameter
    /// itself enters scope but *after* every earlier one, so `fn f(a, b = a)`
    /// is legal while `fn f(x = x)` is not — mirroring JS default-parameter
    /// temporal-dead-zone semantics, and matching the JS the codegen emits
    /// (`function f(a, b = a)`). Returns each parameter's type.
    fn declare_params_with_defaults(
        &mut self,
        params: &[xulo_core::ast::Param],
    ) -> SResult<Vec<Type>> {
        let mut types = Vec::with_capacity(params.len());
        for p in params {
            if let Some(ty) = &p.type_annotation {
                self.check_type(ty)?;
            }
            if let Some(default) = &p.default {
                let default_type = self.check_expression(default)?;
                let param_type = p.type_annotation.clone().unwrap_or(Type::Any);
                if !self.assignable(&default_type, &param_type) {
                    return Err(self.err(format!(
                        "default value for parameter `{}` must be `{}`, found `{}`",
                        p.name,
                        param_type.name(),
                        default_type.name()
                    )));
                }
            }
            let ty = p.type_annotation.clone().unwrap_or(Type::Any);
            self.table.declare_with_def(
                Symbol {
                    name: p.name.clone(),
                    type_: ty.clone(),
                    kind: SymbolKind::Variable,
                    is_shared: p.mode == Some(xulo_core::ast::ParamMode::Shared),
                    is_param: true,
                    is_const: true,
                    is_mutable: p.mode == Some(xulo_core::ast::ParamMode::Mut),
                },
                Some(p.span.clone()),
            );
            types.push(ty);
        }
        Ok(types)
    }

    /// An anonymous function literal. Captured names are resolved through the
    /// enclosing scope (closures); the literal's type is a `fn(...) -> ...`
    /// signature.
    fn check_fn_expr(&mut self, f: &xulo_core::ast::FnExpr) -> SResult<Type> {
        let return_type = f.return_type.clone().unwrap_or(Type::Any);
        self.check_type(&return_type)?;

        self.table.push_scope();
        let param_types = self.declare_params_with_defaults(&f.params)?;
        let saved = self.current_return.replace(return_type.clone());
        let saved_declared = self.declared_return;
        self.declared_return = f.return_type.is_some();
        let saved_async = self.async_depth;
        self.async_depth = if f.is_async { saved_async + 1 } else { 0 };
        // A closure is its own (ordinary) function: `@State`/`@Store`/
        // `@Effect`/`@Environment` are not allowed inside it, even when the
        // closure appears at the top level of a `View` function (docs §12).
        let saved_component = self.component_depth;
        let saved_block = self.block_depth;
        let saved_no_await = self.no_await_depth;
        // A closure body runs when it is called, not at the point of the
        // enclosing critical section, so it does not inherit the lock.
        let saved_lock = self.lock_depth;
        // Inside a closure, every captured binding is part of the closure's
        // own value: only a `let mut` binding may be written through one.
        self.closure_depth += 1;
        self.component_depth = 0;
        self.block_depth = 0;
        self.no_await_depth = 0;
        self.lock_depth = 0;
        // Moves do not cross function boundaries.
        let saved_moved = std::mem::take(&mut self.moved);
        let result = self.check_block_implicit(&f.body);
        self.moved = saved_moved;
        self.closure_depth -= 1;
        self.no_await_depth = saved_no_await;
        self.lock_depth = saved_lock;
        self.component_depth = saved_component;
        self.block_depth = saved_block;
        self.async_depth = saved_async;
        self.current_return = saved;
        self.declared_return = saved_declared;
        result?;
        self.table.pop_scope();

        let ret = if f.is_async {
            Type::Async(Box::new(return_type))
        } else {
            return_type
        };
        Ok(Type::FnSig {
            params: param_types,
            ret: Some(Box::new(ret)),
        })
    }

    /// `mod name;` / `mod name { … }` — declare the module's namespace
    /// binding in this file (the loader claims files and analyzes child
    /// bodies as their own modules; spec `modules/source-files.md`).
    fn check_mod(&mut self, m: &xulo_core::ast::ModDecl) -> SResult<()> {
        let child_id = if self.module_id.is_empty() {
            m.name.clone()
        } else {
            format!("{}::{}", self.module_id, m.name)
        };
        // A root-level `mod` named like a manifest path head would shadow the
        // package at every path in this package: decide it here rather than
        // at each use (spec `modules/import-export.md`, "Name collisions").
        if let Some(world) = &self.world
            && self.module_id.is_empty()
            && world.packages.contains_key(&m.name)
        {
            return Err(self
                .err_code("E0104", format!("`{}` is a manifest path name", m.name))
                .at(m.span.clone())
                .with_note("rename the `mod`, or give the dependency an `alias` in the manifest"));
        }
        if self.namespaces.contains_key(&m.name)
            || self.table.lookup(&m.name).is_some()
            || self.imported_types.contains_key(&m.name)
        {
            return Err(self
                .err_code("E0104", format!("`{}` is already declared", m.name))
                .at(m.span.clone()));
        }
        self.namespaces.insert(m.name.clone(), child_id);
        if m.is_pub {
            self.own_interface.insert(m.name.clone());
        }
        Ok(())
    }

    /// `use …;` / `pub use …;` — resolve the target against the package
    /// world and bind its names (spec `modules/import-export.md`).
    fn check_use(&mut self, u: &xulo_core::ast::UseDecl) -> SResult<()> {
        match &u.target {
            xulo_core::ast::UseTarget::Path {
                head,
                segments,
                alias,
            } => {
                if segments.is_empty() {
                    // `use form;` / `use form as f;` — bind the namespace.
                    let id = self.resolve_path_head(head, &u.span)?;
                    if id == BUILTIN_STD {
                        return Err(self
                            .err_code(
                                "E0607",
                                "`std` is a namespace root: pick a module, e.g. `use std::math::*`",
                            )
                            .at(u.span.clone()));
                    }
                    if std_module_name(&id) == Some("regex") {
                        return Err(self
                            .err_code(
                                "E0607",
                                "the built-in `std::regex` module has no namespace: import its names with `use std::regex::{RegExp, RegexError}`",
                            )
                            .at(u.span.clone()));
                    }
                    let local = alias.clone().unwrap_or_else(|| match head {
                        xulo_core::ast::PathHead::Name(n)
                        | xulo_core::ast::PathHead::Package(n) => n.clone(),
                        xulo_core::ast::PathHead::SelfModule => "self".to_string(),
                        xulo_core::ast::PathHead::Super => "super".to_string(),
                    });
                    return self.bind_namespace(&local, &id, u);
                }
                let local = alias
                    .clone()
                    .unwrap_or_else(|| segments.last().cloned().expect("segments non-empty"));
                self.bind_path_member(head, segments, &local, u)
            }
            xulo_core::ast::UseTarget::Group { head, entries } => {
                for entry in entries {
                    let local = entry.alias.clone().unwrap_or_else(|| {
                        entry.segments.last().cloned().expect("entry non-empty")
                    });
                    self.bind_path_member(head, &entry.segments, &local, u)?;
                }
                Ok(())
            }
            xulo_core::ast::UseTarget::Glob { head, segments } => {
                let id = self.resolve_path_head(head, &u.span)?;
                // The built-in `std` tree globs without a world: `use
                // std::math::*`, or `use m::*` through a std alias.
                if id == BUILTIN_STD {
                    let module = match segments.as_slice() {
                        [module] => module.as_str(),
                        _ => {
                            return Err(self
                                .err_code(
                                    "E0607",
                                    if segments.is_empty() {
                                        "`std` has no direct members: glob a module, e.g. `use std::math::*`".to_string()
                                    } else {
                                        format!("cannot resolve `std::{}…`", segments.join("::"))
                                    },
                                )
                                .at(u.span.clone()));
                        }
                    };
                    return self.bind_std_glob(module, u);
                }
                if let Some(module) = std_module_name(&id) {
                    if !segments.is_empty() {
                        return Err(self
                            .err_code(
                                "E0607",
                                format!(
                                    "cannot resolve `{}::{}`",
                                    segments[..segments.len() - 1].join("::"),
                                    segments.last().unwrap()
                                ),
                            )
                            .at(u.span.clone()));
                    }
                    return self.bind_std_glob(module, u);
                }
                let Some(face) = self.world.as_ref().and_then(|w| w.modules.get(&id)) else {
                    if self.lenient {
                        // Editor partial mode has no face to enumerate:
                        // bind nothing rather than failing the document (the
                        // workspace seeds the target's exports itself).
                        return Ok(());
                    }
                    return Err(self
                        .err_code("E0607", format!("cannot resolve `{}`", head_display(head)))
                        .at(u.span.clone()));
                };
                let members: Vec<(String, FaceKind, bool)> = face
                    .names
                    .iter()
                    .filter(|(_, info)| info.is_pub)
                    .map(|(n, info)| (n.clone(), info.kind, true))
                    .collect();
                let mut contributed = std::collections::HashSet::new();
                for (name, kind, _) in members {
                    self.bind_glob_member(&id, &name, kind, u, &mut contributed)?;
                }
                if u.is_pub {
                    // A glob re-export is the whole module's interface.
                    if let Some(face) = self.world.as_ref().and_then(|w| w.modules.get(&id)) {
                        for (n, sym) in &face.exports {
                            self.exported_symbols.push((n.clone(), sym.clone()));
                        }
                        for (n, e) in &face.types {
                            self.exported_types_for_iface.push((n.clone(), e.clone()));
                        }
                    }
                }
                self.glob_unused
                    .borrow_mut()
                    .push((u.span.clone(), contributed));
                Ok(())
            }
            xulo_core::ast::UseTarget::Local(names) => {
                // `pub use { a, b }` — re-export names already bound here.
                for name in names {
                    let is_enum = matches!(
                        self.type_table.get(name).map(|e| &e.kind),
                        Some(TypeEntryKind::Enum(_))
                    );
                    if self.table.lookup(name).is_none() && !is_enum {
                        if self.type_table.contains_key(name) {
                            return Err(self.err(format!(
                                "cannot re-export `{name}`: it is a type with no runtime value; use `pub type` or `pub enum`"
                            )));
                        }
                        // The header precedes every declaration (E0605), so a
                        // `pub use { x }` naming a declaration of this file
                        // runs before `x` exists: defer the re-export
                        // validation to the end of the pass.
                        self.pending_reexports.push((name.clone(), u.span.clone()));
                        continue;
                    }
                    self.own_interface.insert(name.clone());
                    if is_enum {
                        self.exported_symbols.push((
                            name.clone(),
                            Symbol {
                                name: name.clone(),
                                type_: Type::Named(name.clone(), Vec::new()),
                                kind: symbol_table::SymbolKind::Variable,
                                is_shared: false,
                                is_param: false,
                                is_const: true,
                                is_mutable: false,
                            },
                        ));
                    }
                }
                for name in names {
                    if let Some(sym) = self.table.lookup(name).cloned() {
                        self.exported_symbols.push((name.clone(), sym));
                    }
                }
                self.exported.extend(names.iter().cloned());
                Ok(())
            }
        }
    }

    /// Resolve a path head to a module id: an ordinary name first (mod/use
    /// namespace bindings), then a manifest package, then a root module
    /// (spec `names.md`, "Name resolution").
    fn resolve_path_head(
        &self,
        head: &xulo_core::ast::PathHead,
        span: &Range<usize>,
    ) -> SResult<String> {
        match head {
            xulo_core::ast::PathHead::SelfModule => Ok(self.module_id.clone()),
            xulo_core::ast::PathHead::Super => {
                let parent = self
                    .module_id
                    .rsplit_once("::")
                    .map(|(p, _)| p.to_string())
                    .unwrap_or_default();
                // `super` stops at each package's own root: it may not climb
                // out of a dependency into this package (spec `modules/
                // import-export.md`, "Paths" — above the root, `super`
                // resolves to nothing).
                let above_root = parent.is_empty()
                    && (self.module_id.is_empty()
                        || self
                            .world
                            .as_ref()
                            .is_some_and(|w| w.packages.values().any(|p| *p == self.module_id)));
                if above_root {
                    return Err(self
                        .err_code("E0607", "cannot resolve `super` at the package root")
                        .at(span.clone()));
                }
                Ok(parent)
            }
            xulo_core::ast::PathHead::Name(n) | xulo_core::ast::PathHead::Package(n) => {
                if let Some(id) = self.namespaces.get(n) {
                    self.use_bound.borrow_mut().remove(n);
                    return Ok(id.clone());
                }
                if let Some(world) = &self.world {
                    // Manifest dependencies belong to this package's manifest
                    // only: a dependency's files have no manifest of their own.
                    let pkg_root = ModuleWorld::package_root_of(&self.module_id);
                    if pkg_root.is_empty()
                        && let Some(id) = world.packages.get(n)
                    {
                        return Ok(id.clone());
                    }
                    // Bare path heads are package-root absolute: the root
                    // module children of *this* package (spec `modules/
                    // import-export.md`, "Paths").
                    if let Some(child) = world.modules.get(pkg_root).and_then(|f| f.children.get(n))
                    {
                        return Ok(child.clone());
                    }
                }
                // The built-in `std` root resolves after any package or
                // root module of the same name — a program's own module
                // wins — and before the world-less fallthrough: `std` is the
                // one head that exists without a loader graph.
                if n == "std" {
                    return Ok(BUILTIN_STD.to_string());
                }
                // Heads that name a std module in its old top-level home:
                // name the new path instead of a bare "cannot resolve".
                if matches!(n.as_str(), "regex" | "math" | "time" | "fs" | "io" | "os") {
                    return Err(self
                        .err_code(
                            "E0607",
                            format!("`{n}` lives under `std`: import it with `use std::{n}::*` or reach it as `std::{n}::<member>`"),
                        )
                        .at(span.clone()));
                }
                if self.lenient {
                    // Editor partial mode has no loader graph to verify the
                    // head against: resolve through under a virtual id and
                    // let the member bind degrade opaquely.
                    return Ok(n.to_string());
                }
                Err(self
                    .err_code("E0607", format!("cannot resolve `{n}`"))
                    .at(span.clone()))
            }
        }
    }

    /// Bind `head::segments`'s last name under `local` (value, type, or
    /// child-module namespace), enforcing publication (`E0601`/`E0602`).
    fn bind_path_member(
        &mut self,
        head: &xulo_core::ast::PathHead,
        segments: &[String],
        local: &str,
        u: &xulo_core::ast::UseDecl,
    ) -> SResult<()> {
        let start = self.resolve_path_head(head, &u.span)?;
        // The built-in `std` tree has no module graph behind it: its
        // namespaces and members bind straight from the builtin tables
        // (`use std::math as m`, `use std::math::{floor}`).
        if start == BUILTIN_STD || std_module_name(&start).is_some() {
            return self.bind_std_path(&start, segments, local, u);
        }
        // An external dependency's module tree is invisible: every path into
        // it binds opaquely under `local` (spec `modules/import-export.md`).
        if self
            .world
            .as_ref()
            .and_then(|w| w.modules.get(&start))
            .is_some_and(|f| f.external)
        {
            return self.bind_external_name(local, &start, u);
        }
        if self.lenient && self.world.is_none() {
            // Editor partial mode: no faces to walk. Bind the last member
            // under `local` — typed from the caller's seed when it has one,
            // opaquely otherwise — instead of failing the whole document.
            let member = segments.last().map(|s| s.as_str()).unwrap_or(local);
            return self.bind_degraded_member(local, member, &start, u);
        }
        let mut current = start.clone();
        for (i, seg) in segments.iter().enumerate() {
            let last = i + 1 == segments.len();
            let entry = self
                .world
                .as_ref()
                .and_then(|w| w.modules.get(&current))
                .map(|face| {
                    let child = face.children.get(seg).cloned();
                    let info = face.names.get(seg).copied();
                    (child, info)
                });
            let Some((child, info)) = entry else {
                return Err(self
                    .err_code(
                        "E0607",
                        format!("cannot resolve `{}`", segments[..=i].join("::")),
                    )
                    .at(u.span.clone()));
            };
            if let Some(child) = child {
                if current != self.module_id && info.map(|n| !n.is_pub).unwrap_or(true) {
                    return Err(self
                        .err_code(
                            "E0602",
                            format!("`{seg}` is private to `{}`", module_display(&current)),
                        )
                        .at(u.span.clone()));
                }
                if last {
                    return self.bind_namespace(local, &child, u);
                }
                current = child;
                continue;
            }
            let Some(info) = info else {
                return Err(self
                    .err_code(
                        "E0601",
                        format!(
                            "module `{}` does not export `{seg}`",
                            module_display(&current)
                        ),
                    )
                    .at(u.span.clone()));
            };
            if current != self.module_id && !info.is_pub {
                return Err(self
                    .err_code(
                        "E0602",
                        format!("`{seg}` is private to `{}`", module_display(&current)),
                    )
                    .at(u.span.clone()));
            }
            if !last {
                return Err(self
                    .err_code(
                        "E0607",
                        format!("cannot resolve `{}`", segments[i + 1..].join("::")),
                    )
                    .at(u.span.clone()));
            }
            return self.bind_member(&current, seg, info, local, u);
        }
        Ok(())
    }

    /// Bind one exported name of the built-in `regex` module under `local`:
    /// its three types — `RegExp` (compiled pattern), `RegexError` (the
    /// failure message `RegExp::compile` reports), and `RegexMatch` (one
    /// match's text, span, and capture groups). Like an enum import, a type
    /// binds with no constructor value of its own: `RegExp::compile` is
    /// reached through the type name (spec `regular-expressions.mdx`).
    fn bind_regex_member(
        &mut self,
        segments: &[String],
        local: &str,
        u: &xulo_core::ast::UseDecl,
    ) -> SResult<()> {
        if segments.len() != 1 {
            return Err(self
                .err_code(
                    "E0607",
                    format!(
                        "cannot resolve `regex`{}",
                        if segments.is_empty() {
                            String::new()
                        } else {
                            format!("::{}", segments[1..].join("::"))
                        }
                    ),
                )
                .at(u.span.clone()));
        }
        let entry = match segments[0].as_str() {
            "RegExp" => regex_regexp_type_entry(),
            "RegexError" => regex_error_type_entry(),
            "RegexMatch" => regex_match_type_entry(),
            other => {
                return Err(self
                    .err_code(
                        "E0601",
                        format!("module `std::regex` does not export `{other}`"),
                    )
                    .at(u.span.clone()));
            }
        };
        self.imported_types.insert(local.to_string(), entry);
        self.regex_names.insert(local.to_string());
        if u.is_pub {
            self.own_interface.insert(local.to_string());
            if let Some(e) = self.imported_types.get(local).cloned() {
                self.exported_types_for_iface.push((local.to_string(), e));
            }
            self.exported.push(local.to_string());
        }
        self.use_bound
            .borrow_mut()
            .insert(local.to_string(), u.span.clone());
        Ok(())
    }

    /// Bind a path that resolved inside the built-in `std` tree: the module
    /// namespace itself (`use std::math as m`), one of its values
    /// (`use std::math::{floor}`), or `std::regex`'s three types.
    fn bind_std_path(
        &mut self,
        start: &str,
        segments: &[String],
        local: &str,
        u: &xulo_core::ast::UseDecl,
    ) -> SResult<()> {
        let Some(module) = std_path_module(start, segments).map(str::to_string) else {
            return Err(self
                .err_code(
                    "E0607",
                    format!("cannot resolve `std::{}`", segments.join("::")),
                )
                .at(u.span.clone()));
        };
        // From the `std` root the module is the first segment and the member
        // follows it; from an already-resolved std id every segment is a member.
        let rest: &[String] = if start == BUILTIN_STD {
            &segments[1..]
        } else {
            segments
        };
        let Some(member) = rest.first() else {
            if module == "regex" {
                return Err(self
                    .err_code(
                        "E0607",
                        "the built-in `std::regex` module has no namespace: import its names with `use std::regex::{RegExp, RegexError}`",
                    )
                    .at(u.span.clone()));
            }
            if PRELUDE_INTRINSIC_VALUES.contains(&local) {
                return Err(self
                    .err_code(
                        "E0601",
                        format!(
                            "`{local}` is a prelude intrinsic name and cannot be rebound by a `use`: alias `std::{module}` under another name"
                        ),
                    )
                    .at(u.span.clone()));
            }
            return self.bind_namespace(local, &std_module_id(&module), u);
        };
        if rest.len() > 1 {
            return Err(self
                .err_code("E0607", format!("cannot resolve `{}`", rest.join("::")))
                .at(u.span.clone()));
        }
        if module == "regex" {
            return self.bind_regex_member(std::slice::from_ref(member), local, u);
        }
        let ns = format!("std::{module}");
        self.bind_std_value_member(&ns, member, local, u)
    }

    /// Bind every public member of one built-in std module
    /// (`use std::math::*`, `use m::*` on a std alias).
    fn bind_std_glob(&mut self, module: &str, u: &xulo_core::ast::UseDecl) -> SResult<()> {
        let Some(members) = std_face_members(module) else {
            return Err(self
                .err_code("E0607", format!("cannot resolve `std::{module}`"))
                .at(u.span.clone()));
        };
        let mut contributed = std::collections::HashSet::new();
        for (name, kind) in members {
            if kind == FaceKind::Type {
                self.bind_regex_member(std::slice::from_ref(&name), &name, u)?;
            } else {
                let ns = format!("std::{module}");
                self.bind_std_value_member(&ns, &name, &name, u)?;
            }
            contributed.insert(name);
        }
        self.glob_unused
            .borrow_mut()
            .push((u.span.clone(), contributed));
        Ok(())
    }

    /// Bind one intrinsic of a std module under `local`: constants as
    /// `number` symbols, functions carrying their builtin table entry so a
    /// bare call keeps the qualified form's diagnostics.
    fn bind_std_value_member(
        &mut self,
        ns: &str,
        member: &str,
        local: &str,
        u: &xulo_core::ast::UseDecl,
    ) -> SResult<()> {
        let Some(registered) = builtin_member(ns, member) else {
            return Err(self
                .err_code("E0601", format!("module `{ns}` does not export `{member}`"))
                .at(u.span.clone()));
        };
        // `std::io` exports `print`/`println` under the prelude intrinsic's
        // own names; the interpreter keeps those in its global scope, so a
        // binding here would type-check as the member and run as the
        // intrinsic. The qualified path reaches the same function.
        if PRELUDE_INTRINSIC_VALUES.contains(&local) {
            return Err(self
                .err_code(
                    "E0601",
                        format!(
                            "`{local}` is a prelude intrinsic name and cannot be rebound by a `use`: reach `{ns}::{member}` as `{ns}::{member}(…)` or through a namespace alias"
                        ),
                )
                .at(u.span.clone()));
        }
        if u.type_only {
            // `use type std::math::floor` — the name exists for types only;
            // a later value-position use is `E0606`.
            self.type_only.insert(local.to_string(), u.span.clone());
            return Ok(());
        }
        let type_ = match &registered {
            BuiltinMember::Constant => Type::Number,
            BuiltinMember::Fn(sig) => Type::FnSig {
                params: sig.params.iter().map(|p| p.ty()).collect(),
                ret: Some(Box::new(sig.ret.ty())),
            },
        };
        let symbol = Symbol {
            name: local.to_string(),
            type_,
            kind: SymbolKind::Variable,
            is_const: true,
            is_mutable: false,
            is_param: false,
            is_shared: false,
        };
        if !self.table.declare(symbol) {
            return Err(self
                .err_code("E0104", format!("`{local}` is already declared"))
                .at(u.span.clone()));
        }
        if matches!(registered, BuiltinMember::Fn(_)) {
            self.std_fns
                .insert(local.to_string(), (ns.to_string(), member.to_string()));
        }
        self.use_origins
            .insert(local.to_string(), (ns.to_string(), FaceKind::Fn));
        self.use_bound
            .borrow_mut()
            .insert(local.to_string(), u.span.clone());
        Ok(())
    }

    /// Bind one resolved member of module `module_id` under `local`.
    fn bind_member(
        &mut self,
        module_id: &str,
        member: &str,
        info: FaceName,
        local: &str,
        u: &xulo_core::ast::UseDecl,
    ) -> SResult<()> {
        let face = self
            .world
            .as_ref()
            .and_then(|w| w.modules.get(module_id))
            .cloned();
        let Some(face) = face else {
            return Err(self
                .err_code("E0607", format!("cannot resolve `{member}`"))
                .at(u.span.clone()));
        };
        if face.external {
            return self.bind_external_name(local, module_id, u);
        }
        match info.kind {
            FaceKind::Type => {
                let entry = face
                    .types
                    .iter()
                    .find(|(n, _)| n == member)
                    .map(|(_, e)| e.clone())
                    .unwrap_or(TypeEntry {
                        type_params: Vec::new(),
                        type_bounds: Vec::new(),
                        kind: TypeEntryKind::Alias(Type::Any),
                    });
                self.imported_types.insert(local.to_string(), entry);
                if u.is_pub {
                    self.own_interface.insert(local.to_string());
                    if let Some(e) = self.imported_types.get(local).cloned() {
                        self.exported_types_for_iface.push((local.to_string(), e));
                    }
                    self.exported.push(local.to_string());
                }
                self.use_bound
                    .borrow_mut()
                    .insert(local.to_string(), u.span.clone());
                if !u.type_only {
                    // A struct's runtime value — its constructor name — is a
                    // `pub` face export that travels with the import, so a
                    // call through the imported name resolves like the
                    // defining module's own binding (`check_struct` declares
                    // the same symbol locally; `use`-imported struct types
                    // previously fell through to `unknown function`). Enums
                    // stay type-only, matching `check_enum`: the enum name
                    // has no value binding of its own (`Theme.Dark` is
                    // rejected via `lookup_symbol`), and construction
                    // (`Theme::Variant`) resolves through the type entry.
                    let is_struct = self
                        .imported_types
                        .get(local)
                        .is_some_and(|e| matches!(e.kind, TypeEntryKind::Struct(_)));
                    if is_struct
                        && let Some((_, sym)) = face.exports.iter().find(|(n, _)| n == member)
                    {
                        let mut sym = sym.clone();
                        sym.name = local.to_string();
                        if !self.table.declare(sym) {
                            return Err(self
                                .err_code("E0104", format!("`{local}` is already declared"))
                                .at(u.span.clone()));
                        }
                    }
                    self.use_bindings.push(UseBinding {
                        local: local.to_string(),
                        module_id: module_id.to_string(),
                        member: Some(member.to_string()),
                        is_pub: u.is_pub,
                    });
                }
            }
            FaceKind::Module => {
                if let Some(child) = face.children.get(member) {
                    return self.bind_namespace(local, child, u);
                }
                return Err(self
                    .err_code("E0607", format!("cannot resolve `{member}`"))
                    .at(u.span.clone()));
            }
            FaceKind::Fn | FaceKind::Let | FaceKind::Value => {
                if u.type_only {
                    // `use type form::helper` — a value name bound for types
                    // only; a later value-position use is `E0606`.
                    self.type_only.insert(local.to_string(), u.span.clone());
                    return Ok(());
                }
                let found = face
                    .exports
                    .iter()
                    .find(|(n, _)| n == member)
                    .map(|(_, s)| s.clone());
                let missing = found.is_none();
                let symbol = found.unwrap_or(Symbol {
                    name: local.to_string(),
                    type_: Type::Any,
                    kind: SymbolKind::Function(Vec::new(), Vec::new(), Type::Any, Vec::new()),
                    is_shared: false,
                    is_param: false,
                    is_const: true,
                    is_mutable: false,
                });
                let symbol = Symbol {
                    name: local.to_string(),
                    ..symbol
                };
                if !self.table.declare(symbol.clone()) {
                    return Err(self
                        .err_code("E0104", format!("`{local}` is already declared"))
                        .at(u.span.clone()));
                }
                if missing {
                    // The source face does not (yet) carry the symbol — an
                    // external re-export or a face this module read before
                    // completion. Bind opaquely so later calls/annotations
                    // degrade to `any` instead of failing outright.
                    self.imported_types.insert(
                        local.to_string(),
                        TypeEntry {
                            type_params: Vec::new(),
                            type_bounds: Vec::new(),
                            kind: TypeEntryKind::Alias(Type::Any),
                        },
                    );
                    self.opaque.insert(local.to_string());
                } else {
                    self.opaque.remove(local);
                }
                self.use_origins
                    .insert(local.to_string(), (module_id.to_string(), info.kind));
                self.use_bound
                    .borrow_mut()
                    .insert(local.to_string(), u.span.clone());
                self.use_bindings.push(UseBinding {
                    local: local.to_string(),
                    module_id: module_id.to_string(),
                    member: Some(member.to_string()),
                    is_pub: u.is_pub,
                });
                if u.is_pub {
                    self.own_interface.insert(local.to_string());
                    self.exported_symbols.push((local.to_string(), symbol));
                    self.exported.push(local.to_string());
                }
            }
        }
        Ok(())
    }

    /// Bind a glob-contributed name from module `module_id`.
    fn bind_glob_member(
        &mut self,
        module_id: &str,
        member: &str,
        kind: FaceKind,
        u: &xulo_core::ast::UseDecl,
        contributed: &mut std::collections::HashSet<String>,
    ) -> SResult<()> {
        let info = FaceName { is_pub: true, kind };
        // A glob binds without aliasing; the local name is the member name.
        let local = member.to_string();
        self.bind_member(module_id, member, info, &local, u)?;
        contributed.insert(local);
        Ok(())
    }

    /// Bind a namespace alias `local` -> module `id`.
    fn bind_namespace(
        &mut self,
        local: &str,
        id: &str,
        u: &xulo_core::ast::UseDecl,
    ) -> SResult<()> {
        if u.type_only {
            return Ok(());
        }
        // A root-level namespace binding named like a manifest path head
        // would shadow the package: decide the collision here rather than at
        // each path (spec `modules/import-export.md`, "Name collisions").
        // Binding the package itself (`use ui;`) is not a collision.
        if let Some(world) = &self.world
            && self.module_id.is_empty()
            && world.packages.get(local).is_some_and(|p| p != id)
        {
            return Err(self
                .err_code("E0104", format!("`{local}` is a manifest path name"))
                .at(u.span.clone())
                .with_note(
                    "rename the binding, or give the dependency an `alias` in the manifest",
                ));
        }
        if self.namespaces.contains_key(local) || self.table.lookup(local).is_some() {
            return Err(self
                .err_code("E0104", format!("`{local}` is already declared"))
                .at(u.span.clone()));
        }
        self.namespaces.insert(local.to_string(), id.to_string());
        self.use_bound
            .borrow_mut()
            .insert(local.to_string(), u.span.clone());
        self.use_bindings.push(UseBinding {
            local: local.to_string(),
            module_id: id.to_string(),
            member: None,
            is_pub: u.is_pub,
        });
        if u.is_pub {
            self.own_interface.insert(local.to_string());
        }
        Ok(())
    }

    /// Bind a member of an external manifest dependency (no local sources):
    /// the name is opaque in every position — a value that accepts any call
    /// and a type that accepts any annotation — so a package the compiler
    /// cannot see still type-checks at its use sites (spec `modules/
    /// import-export.md`: the manifest mapping, not the package, is given to
    /// the compiler).
    fn bind_external_name(&mut self, local: &str, module_id: &str, u: &UseDecl) -> SResult<()> {
        let entry = TypeEntry {
            type_params: Vec::new(),
            type_bounds: Vec::new(),
            kind: TypeEntryKind::Alias(Type::Any),
        };
        self.imported_types.insert(local.to_string(), entry.clone());
        if u.type_only {
            self.type_only.insert(local.to_string(), u.span.clone());
            return Ok(());
        }
        let symbol = Symbol {
            name: local.to_string(),
            type_: Type::Any,
            kind: SymbolKind::Function(Vec::new(), Vec::new(), Type::Any, Vec::new()),
            is_shared: false,
            is_param: false,
            is_const: true,
            is_mutable: false,
        };
        if !self.table.declare(symbol.clone()) {
            return Err(self
                .err_code("E0104", format!("`{local}` is already declared"))
                .at(u.span.clone()));
        }
        self.opaque.insert(local.to_string());
        self.use_origins
            .insert(local.to_string(), (module_id.to_string(), FaceKind::Value));
        self.use_bound
            .borrow_mut()
            .insert(local.to_string(), u.span.clone());
        self.use_bindings.push(UseBinding {
            local: local.to_string(),
            module_id: module_id.to_string(),
            member: None,
            is_pub: u.is_pub,
        });
        if u.is_pub {
            self.own_interface.insert(local.to_string());
            self.exported_symbols.push((local.to_string(), symbol));
            self.exported_types_for_iface
                .push((local.to_string(), entry));
            self.exported.push(local.to_string());
        }
        Ok(())
    }

    /// Bind `local` to `member` of the virtual module `module_id` when there
    /// is no world to inspect (lenient editor analysis): a seeded value
    /// symbol keeps its real type, a seeded type entry binds as a type-only
    /// name, and anything else degrades to an opaque `any` value — the same
    /// shapes the face-driven [`Self::bind_member`] produces, minus the
    /// publication walk that needs real faces.
    fn bind_degraded_member(
        &mut self,
        local: &str,
        member: &str,
        module_id: &str,
        u: &UseDecl,
    ) -> SResult<()> {
        let entry = TypeEntry {
            type_params: Vec::new(),
            type_bounds: Vec::new(),
            kind: TypeEntryKind::Alias(Type::Any),
        };
        if u.type_only {
            self.imported_types
                .entry(local.to_string())
                .or_insert(entry);
            self.type_only.insert(local.to_string(), u.span.clone());
            return Ok(());
        }
        let seeded = self.imports.get(member).cloned();
        let had_seed = seeded.is_some();
        if seeded.is_none() {
            if let Some(existing) = self.imported_types.get(local).cloned() {
                // A seeded type entry with no value symbol (an enum or alias
                // member): bind the type, never a value name.
                self.use_bound
                    .borrow_mut()
                    .insert(local.to_string(), u.span.clone());
                self.use_bindings.push(UseBinding {
                    local: local.to_string(),
                    module_id: module_id.to_string(),
                    member: Some(member.to_string()),
                    is_pub: u.is_pub,
                });
                if u.is_pub {
                    self.own_interface.insert(local.to_string());
                    self.exported_types_for_iface
                        .push((local.to_string(), existing));
                    self.exported.push(local.to_string());
                }
                return Ok(());
            }
            self.imported_types
                .entry(local.to_string())
                .or_insert_with(|| TypeEntry {
                    type_params: Vec::new(),
                    type_bounds: Vec::new(),
                    kind: TypeEntryKind::Alias(Type::Any),
                });
        }
        let symbol = match seeded {
            Some(seed) => Symbol {
                name: local.to_string(),
                type_: seed.type_.clone(),
                kind: seed.kind.clone(),
                is_shared: false,
                is_param: false,
                is_const: true,
                is_mutable: false,
            },
            None => Symbol {
                name: local.to_string(),
                type_: Type::Any,
                kind: SymbolKind::Function(Vec::new(), Vec::new(), Type::Any, Vec::new()),
                is_shared: false,
                is_param: false,
                is_const: true,
                is_mutable: false,
            },
        };
        if !self.table.declare(symbol.clone()) {
            return Err(self
                .err_code("E0104", format!("`{local}` is already declared"))
                .at(u.span.clone()));
        }
        if !had_seed {
            self.opaque.insert(local.to_string());
        }
        self.use_origins
            .insert(local.to_string(), (module_id.to_string(), FaceKind::Value));
        self.use_bound
            .borrow_mut()
            .insert(local.to_string(), u.span.clone());
        self.use_bindings.push(UseBinding {
            local: local.to_string(),
            module_id: module_id.to_string(),
            member: Some(member.to_string()),
            is_pub: u.is_pub,
        });
        if u.is_pub {
            self.own_interface.insert(local.to_string());
            self.exported_symbols.push((local.to_string(), symbol));
            self.exported.push(local.to_string());
        }
        Ok(())
    }

    /// The E0611 subset: a file-scope initializer that directly reads a
    /// `use`-bound name (or directly calls its function) from a module
    /// that initializes later (spec `modules/import-export.md`).
    fn check_init_order(&self, expr: &Expression) -> SResult<()> {
        if self.world.is_none() {
            return Ok(());
        }
        let mut violation: Option<XuloError> = None;
        self.walk_init_refs(expr, &mut |name, span| {
            if violation.is_some() {
                return;
            }
            let Some((origin, _)) = self.use_origins.get(name) else {
                return;
            };
            let Some(world) = &self.world else { return };
            let Some(cur) = world.init_order.get(&self.module_id) else {
                return;
            };
            let Some(ord) = world.init_order.get(origin) else {
                return;
            };
            if ord > cur {
                let short = origin.rsplit("::").next().unwrap_or(origin);
                violation = Some(
                    self.err_code(
                        "E0611",
                        format!("`{short}::{name}` is used before `{short}` initializes"),
                    )
                    .at(span.clone()),
                );
            }
        });
        match violation {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    /// Collect the direct identifier references of `expr` (reads and call
    /// callees), not descending into closures or nested function bodies.
    fn walk_init_refs(&self, expr: &Expression, f: &mut dyn FnMut(&str, &Range<usize>)) {
        match expr {
            Expression::Identifier { name, span } => f(name, span),
            Expression::Call(call) => {
                if let Some((head, _)) = call.callee.split_once("::") {
                    f(head, &call.span);
                } else {
                    f(&call.callee, &call.span);
                }
                for arg in &call.arguments {
                    self.walk_init_refs(&arg.value, f);
                }
            }
            Expression::BinaryOp(b) => {
                self.walk_init_refs(&b.left, f);
                self.walk_init_refs(&b.right, f);
            }
            Expression::Unary(u) => self.walk_init_refs(&u.operand, f),
            Expression::Index(ix) => {
                self.walk_init_refs(&ix.object, f);
                self.walk_init_refs(&ix.index, f);
            }
            Expression::Member(m) => self.walk_init_refs(&m.object, f),
            Expression::Await { expr, .. } => self.walk_init_refs(expr, f),
            Expression::Tuple { items, .. } => {
                for item in items {
                    self.walk_init_refs(item, f);
                }
            }
            Expression::Propagate { expr, .. } => self.walk_init_refs(expr, f),
            Expression::Panic { expr, .. } => self.walk_init_refs(expr, f),
            Expression::Spread { expr, .. } => self.walk_init_refs(expr, f),
            Expression::If(i) => {
                self.walk_init_refs(&i.condition, f);
                for s in &i.then_branch.statements {
                    if let Statement::Expr(e) = s {
                        self.walk_init_refs(&e.expr, f);
                    }
                }
                if let Some(else_branch) = &i.else_branch {
                    for s in &else_branch.statements {
                        if let Statement::Expr(e) = s {
                            self.walk_init_refs(&e.expr, f);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Mark a `use`-bound value/type name as used (W0102).
    fn mark_name_used(&self, name: &str) {
        self.use_bound.borrow_mut().remove(name);
    }

    fn check_export(&mut self, export: &xulo_core::ast::ExportStmt) -> SResult<()> {
        self.check_export_item(export)?;
        // E0604: every `pub` signature may name only universally visible
        // types, types this interface publishes, or `pub use` re-exports.
        if self.world.is_some() {
            match &export.item {
                xulo_core::ast::ExportItem::Fn(f) => {
                    let mut sig: Vec<&Type> = Vec::new();
                    for p in &f.params {
                        if let Some(t) = &p.type_annotation {
                            sig.push(t);
                        }
                    }
                    if let Some(rt) = &f.return_type {
                        sig.push(rt);
                    }
                    self.check_pub_interface_types(&sig, &f.name_span)?;
                }
                xulo_core::ast::ExportItem::Let(b) => {
                    if let Some(t) = &b.type_annotation {
                        let sig: [&Type; 1] = [t];
                        self.check_pub_interface_types(&sig, &b.name_span)?;
                    }
                }
                xulo_core::ast::ExportItem::Type(a) => {
                    let sig: [&Type; 1] = [&a.type_];
                    self.check_pub_interface_types(&sig, &a.name_span)?;
                }
                xulo_core::ast::ExportItem::Struct(st) => {
                    let types: Vec<&Type> = st.fields.iter().map(|f| &f.type_).collect();
                    self.check_pub_interface_types(&types, &st.name_span)?;
                }
                xulo_core::ast::ExportItem::Enum(e) => {
                    let mut types: Vec<&Type> = Vec::new();
                    for v in &e.variants {
                        for p in v.payload.iter().flatten() {
                            types.push(&p.type_);
                        }
                    }
                    self.check_pub_interface_types(&types, &e.name_span)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// E0604: each named module-local type in a `pub` signature must be
    /// part of this module's interface (`pub` here or `pub use`d).
    fn check_pub_interface_types(&self, types: &[&Type], span: &Range<usize>) -> SResult<()> {
        for ty in types {
            let mut named = Vec::new();
            collect_named_types(ty, &mut named);
            for name in named {
                if self.generics.contains(&name) {
                    continue;
                }
                if self.type_table.contains_key(&name) && !self.own_interface.contains(&name) {
                    return Err(self
                        .err_code(
                            "E0604",
                            format!("private type `{name}` in public interface"),
                        )
                        .at(span.clone()));
                }
            }
        }
        Ok(())
    }

    fn check_export_item(&mut self, export: &xulo_core::ast::ExportStmt) -> SResult<()> {
        match &export.item {
            xulo_core::ast::ExportItem::Fn(f) => {
                self.check_fn(f)?;
                self.exported.push(f.name.clone());
                if let Some(sym) = self.table.lookup(&f.name).cloned() {
                    self.exported_symbols.push((f.name.clone(), sym));
                }
                Ok(())
            }
            xulo_core::ast::ExportItem::Let(b) => {
                self.check_statement(&Statement::Let(b.clone()))?;
                self.exported.push(b.name.clone());
                if let Some(sym) = self.table.lookup(&b.name).cloned() {
                    self.exported_symbols.push((b.name.clone(), sym));
                }
                Ok(())
            }
            xulo_core::ast::ExportItem::Type(alias) => {
                self.check_type_alias(alias)?;
                self.exported.push(alias.name.clone());
                Ok(())
            }
            xulo_core::ast::ExportItem::Trait(t) => {
                self.check_trait(t)?;
                self.exported.push(t.name.clone());
                Ok(())
            }
            xulo_core::ast::ExportItem::Enum(e) => {
                self.check_enum(e)?;
                self.exported.push(e.name.clone());
                self.exported_symbols.push((
                    e.name.clone(),
                    Symbol {
                        name: e.name.clone(),
                        type_: Type::Named(e.name.clone(), Vec::new()),
                        kind: symbol_table::SymbolKind::Variable,
                        is_shared: false,
                        is_param: false,
                        is_const: true,
                        is_mutable: false,
                    },
                ));
                Ok(())
            }
            xulo_core::ast::ExportItem::Struct(s) => {
                self.check_struct(s)?;
                self.exported.push(s.name.clone());
                self.exported_symbols.push((
                    s.name.clone(),
                    Symbol {
                        name: s.name.clone(),
                        type_: Type::Named(s.name.clone(), Vec::new()),
                        kind: symbol_table::SymbolKind::Variable,
                        is_shared: false,
                        is_param: false,
                        is_const: true,
                        is_mutable: false,
                    },
                ));
                Ok(())
            }
            xulo_core::ast::ExportItem::Names(names) => {
                for name in names {
                    let is_enum = matches!(
                        self.type_table.get(name).map(|e| &e.kind),
                        Some(TypeEntryKind::Enum(_))
                    );
                    if self.table.lookup(name).is_none() && !is_enum {
                        return Err(if self.type_table.contains_key(name) {
                            // A type alias has no runtime value: it cannot be
                            // re-exported by bare name.
                            self.err(format!(
                                "cannot re-export `{name}`: it is a type with no runtime value; use `pub type` or `pub enum`"
                            ))
                        } else {
                            self.err(format!(
                                "cannot re-export `{name}`: it is not declared in this module"
                            ))
                        });
                    }
                    if is_enum {
                        // Enums have runtime value: re-export the name so
                        // importers can construct/match its variants.
                        self.exported_symbols.push((
                            name.clone(),
                            Symbol {
                                name: name.clone(),
                                type_: Type::Named(name.clone(), Vec::new()),
                                kind: symbol_table::SymbolKind::Variable,
                                is_shared: false,
                                is_param: false,
                                is_const: true,
                                is_mutable: false,
                            },
                        ));
                    }
                }
                for name in names {
                    if let Some(sym) = self.table.lookup(name).cloned() {
                        self.exported_symbols.push((name.clone(), sym));
                    }
                }
                self.exported.extend(names.iter().cloned());
                Ok(())
            }
        }
    }

    fn check_assign(&mut self, assign: &AssignStmt) -> SResult<()> {
        let target_type = self.assign_target_type(&assign.target)?;
        let value_type = self.check_expression(&assign.value)?;
        // A numeric literal reassigns a fixed-bit binding when it fits
        // (literal coercion, spec `type-system/coercion.md`); an out-of-
        // range one is E0209 rather than the generic mismatch.
        if !self.assignable(&value_type, &target_type)
            && !self.literal_matches(&assign.value, &target_type)
        {
            if let Some(e) =
                self.fixed_literal_error(&assign.value, &target_type, assign.value.span().clone())
            {
                return Err(e);
            }
            return Err(self.err(format!(
                "cannot assign a value of type `{}` to `{}: {}`",
                value_type.name(),
                assign_target_name(&assign.target),
                target_type.name()
            )));
        }
        Ok(())
    }

    /// Record the borrows a task-returning call holds: every argument that is
    /// a place, with the mutability its parameter declares.
    fn record_task_borrows(&mut self, call: &xulo_core::ast::Call, task: &str) {
        let params = match self.table.lookup(&call.callee).map(|s| &s.kind) {
            Some(SymbolKind::Function(_, params, _, _)) => params.clone(),
            _ => return,
        };
        for (i, arg) in call.arguments.iter().enumerate() {
            let param = match &arg.name {
                Some(name) => params.iter().find(|p| &p.name == name),
                None => params.get(i),
            };
            let Some(param) = param else { continue };
            let Some(place) = place_of(&arg.value) else {
                continue;
            };
            self.pending_borrows.push(PendingBorrow {
                place,
                is_mut: param.mode == Some(xulo_core::ast::ParamMode::Mut),
                task: task.to_string(),
                block: self.block_id,
            });
        }
    }

    /// A `spawn` block does not see the surrounding scope: an enclosing
    /// binding may appear only as a capture, `let local = move config` (spec
    /// concurrency.md, E0408). Declarations, `const`s, and `shared` regions
    /// are always visible.
    fn check_spawn_capture(&self, name: &str, span: &std::ops::Range<usize>) -> SResult<()> {
        if self.spawn_scopes.is_empty() || self.capture_operand {
            return Ok(());
        }
        let Some(&spawn_depth) = self.spawn_scopes.last() else {
            return Ok(());
        };
        let Some(sym) = self.table.lookup(name) else {
            return Ok(());
        };
        // Declared inside the spawn block (including nested scopes) — it is
        // the block's own binding.
        if let Some(index) = self.table.depth_of(name)
            && index >= spawn_depth
        {
            return Ok(());
        }
        match &sym.kind {
            // Declarations (`fn`, and the type forms) and prelude names are
            // always visible inside the block.
            SymbolKind::Function(..) => Ok(()),
            _ if sym.is_const => Ok(()),
            // A shared region needs no capture form: the block takes the
            // region itself.
            _ if sym.is_shared => Ok(()),
            _ => Err(self
                .err_code(
                    "E0408",
                    format!("`{name}` is not captured by the spawned block"),
                )
                .at(span.clone())),
        }
    }

    /// Is this root currently locked by a lexically enclosing `lock` on it?
    fn is_locked(&self, root: &str) -> bool {
        self.locked.iter().any(|n| n == root)
    }

    /// The contents of a shared binding — every field and subscript reached
    /// through it — may be reached only under a `lock` on that binding, or
    /// inside a function that receives the region as a `shared` parameter
    /// (spec concurrency.md, E0406).
    fn check_shared_contents(&self, place: &Place) -> SResult<()> {
        let Some(sym) = self.table.lookup(&place.root) else {
            return Ok(());
        };
        if !sym.is_shared || self.is_locked(&place.root) || sym.is_param {
            return Ok(());
        }
        Err(self
            .err_code(
                "E0406",
                format!("shared state `{}` is accessed outside `lock`", place.root),
            )
            .at(place.span.clone()))
    }

    /// A use of a place (read, write, move, or argument) while a borrow of it
    /// is outstanding (spec `memory-and-runtime.md`, rules 3 and Span end).
    /// While a **mutable** borrow is outstanding the place may not be used at
    /// all (`E0405`); while an **immutable** one is, it may not be written
    /// (`E0411`) and reads remain unrestricted.
    fn check_borrow_place(&self, place: &Place, is_write: bool) -> SResult<()> {
        for b in &self.pending_borrows {
            if !places_overlap(&b.place, place) {
                continue;
            }
            if b.is_mut {
                return Err(self
                    .err_code(
                        "E0405",
                        format!("cannot use `{}`: it is mutably borrowed", place.display),
                    )
                    .at(place.span.clone()));
            }
            if is_write {
                return Err(self
                    .err_code(
                        "E0411",
                        format!("cannot write `{}`: it is immutably borrowed", place.display),
                    )
                    .at(place.span.clone()));
            }
        }
        Ok(())
    }

    /// An `await` whose operand contains a task's binding is where a borrow
    /// that outlived its call ends (spec, Span end). Only an `await` in the
    /// same block releases it: on any other path the borrow is outstanding.
    fn release_borrows_of_awaited(&mut self, operand: &Expression) {
        let mut tasks = Vec::new();
        collect_task_names(operand, &mut tasks);
        if tasks.is_empty() {
            return;
        }
        self.pending_borrows
            .retain(|b| !(b.block == self.block_id && tasks.contains(&b.task)));
    }

    /// Rule 1 of aliasing XOR mutability: an argument passed to a `mut`
    /// parameter must be a place backed by a `mut` binding (spec
    /// `memory-and-runtime.md`, E0404). A `mut` parameter (or `mut self`)
    /// lends its own mutable borrow onward, so it qualifies here too.
    fn require_mutable_place(&self, expr: &Expression) -> SResult<Place> {
        let Some(place) = place_of(expr) else {
            return Err(self.err_code(
                "E0404",
                "cannot borrow as mutable: the argument is not a place",
            ));
        };
        let Some(sym) = self.table.lookup(&place.root) else {
            return Ok(place);
        };
        // Inside `lock` the contents of the shared binding are mutable places
        // whether or not the binding itself is `let mut`.
        if !sym.is_mutable && !self.is_locked(&place.root) {
            return Err(self
                .err_code(
                    "E0404",
                    format!("cannot borrow `{}` as mutable", place.display),
                )
                .at(place.span.clone()));
        }
        Ok(place)
    }

    /// Rules 1 and 2 for one call's argument list, in parameter order:
    /// every `mut` parameter receives a mutable place, and no two places
    /// overlap when either of the two parameters is `mut`.
    fn check_call_borrows(
        &self,
        args: &[(&Expression, Option<xulo_core::ast::ParamMode>)],
    ) -> SResult<()> {
        use xulo_core::ast::ParamMode;
        let mut places: Vec<(Place, bool)> = Vec::with_capacity(args.len());
        for (expr, mode) in args {
            match mode {
                Some(ParamMode::Mut) => {
                    let place = self.require_mutable_place(expr)?;
                    // Rule 3: a mutable borrow may not start while an
                    // immutable one of the same place is outstanding.
                    for b in &self.pending_borrows {
                        if !b.is_mut && places_overlap(&b.place, &place) {
                            return Err(self
                                .err_code(
                                    "E0404",
                                    format!(
                                        "cannot borrow `{}` as mutable: it is already immutably borrowed",
                                        place.display
                                    ),
                                )
                                .at(place.span.clone()));
                        }
                    }
                    places.push((place, true));
                }
                Some(ParamMode::Shared) => {
                    // A `shared` parameter receives the region itself: only a
                    // shared binding may be passed (spec concurrency.md, E0407).
                    let Some(place) = place_of(expr) else {
                        return Err(
                            self.err_code("E0407", "`shared` parameters take a shared binding")
                        );
                    };
                    let is_shared = self
                        .table
                        .lookup(&place.root)
                        .is_some_and(|sym| sym.is_shared);
                    if !is_shared {
                        return Err(self
                            .err_code(
                                "E0407",
                                format!(
                                    "`shared` parameter requires a shared binding, found `{}`",
                                    place.display
                                ),
                            )
                            .at(place.span.clone()));
                    }
                    places.push((place, false));
                }
                None => {
                    if let Some(place) = place_of(expr) {
                        places.push((place, false));
                    }
                }
            }
        }
        for (i, (a, a_mut)) in places.iter().enumerate() {
            for (b, b_mut) in places.iter().skip(i + 1) {
                if (*a_mut || *b_mut) && places_overlap(a, b) {
                    let culprit = if *a_mut { a } else { b };
                    return Err(self
                        .err_code(
                            "E0404",
                            format!(
                                "cannot borrow `{}` as mutable: it overlaps another argument of this call",
                                culprit.display
                            ),
                        )
                        .at(culprit.span.clone()));
                }
            }
        }
        Ok(())
    }

    /// The binding a place expression is rooted in: `a`, `a.b.c`, and
    /// `a[0].b` all root at `a`.
    fn place_root_name(expr: &Expression) -> Option<&str> {
        match expr {
            Expression::Identifier { name, .. } => Some(name),
            Expression::Member(m) => Self::place_root_name(&m.object),
            Expression::Index(i) => Self::place_root_name(&i.object),
            _ => None,
        }
    }

    /// A write through a place is legal only when its root binding is
    /// writable: not a `const`, and declared `mut` — and, inside a closure,
    /// only when it is a `let mut` binding rather than a capture of a
    /// parameter (spec `expressions/closures.md`, E0401).
    fn check_place_writable(&self, place: &Expression, display: &str) -> SResult<()> {
        let Some(name) = Self::place_root_name(place) else {
            return Ok(());
        };
        let Some(sym) = self.table.lookup(name) else {
            return Ok(());
        };
        if !matches!(sym.kind, SymbolKind::Variable | SymbolKind::State) {
            return Ok(());
        }
        if self.closure_depth > 0 && sym.is_param {
            return Err(self.err_code(
                "E0401",
                format!(
                    "cannot assign to `{name}`: a capture of a parameter is not a `let mut` binding"
                ),
            ));
        }
        if !sym.is_mutable && !self.is_locked(name) {
            return Err(self.err_code(
                "E0401",
                format!("cannot assign to `{display}`: variable is not declared `mut`"),
            ));
        }
        Ok(())
    }

    /// Type-check the left-hand side of an assignment and return the type its
    /// value must have.
    fn assign_target_type(&mut self, target: &AssignTarget) -> SResult<Type> {
        // A write is a use: it is barred by an outstanding mutable borrow
        // (`E0405`) and by an outstanding immutable one (`E0411`).
        if let Some(place) = assign_target_place(target) {
            self.check_borrow_place(&place, true)?;
            self.check_shared_contents(&place)?;
        }
        self.assign_target_type_inner(target)
    }

    fn assign_target_type_inner(&mut self, target: &AssignTarget) -> SResult<Type> {
        match target {
            AssignTarget::Name(name) => {
                let Some(sym) = self.table.lookup(name) else {
                    return Err(self.err(format!("undefined variable `{name}` cannot be assigned")));
                };
                if sym.is_const {
                    return Err(
                        self.err(format!("cannot assign to `{name}`: binding is immutable"))
                    );
                }
                if !sym.is_mutable {
                    return Err(self.err_code(
                        "E0401",
                        format!("cannot assign to `{name}`: variable is not declared `mut`"),
                    ));
                }
                match &sym.kind {
                    SymbolKind::Variable | SymbolKind::State => Ok(sym.type_.clone()),
                    SymbolKind::Store => Err(self.err(format!(
                        "cannot assign to `{name}`: store bindings are read-only"
                    ))),
                    SymbolKind::Function(_, _, _, _) => {
                        Err(self.err(format!("cannot assign to `{name}`: it is a function")))
                    }
                }
            }
            AssignTarget::Member(object, property) => {
                let object_type = self.check_expression(object)?;
                let place = format!("{}.{}", place_root_display(object), property);
                self.check_place_writable(object, &place)?;
                let resolved = self.resolve_alias(&object_type, 0);
                match resolved {
                    Type::Object | Type::Any | Type::Named(_, _) => Ok(Type::Any),
                    Type::ObjectType(fields) => fields
                        .iter()
                        .find(|(n, _)| n == property)
                        .map(|(_, t)| t.clone())
                        .ok_or_else(|| self.err(format!("type has no member `{property}`"))),
                    other => Err(self.err(format!(
                        "cannot assign member `{property}` of `{}`",
                        other.name()
                    ))),
                }
            }
            AssignTarget::Index(object, index) => {
                let object_type = self.check_expression(object)?;
                self.check_expression(index)?;
                let place = format!("{}[...]", place_root_display(object));
                self.check_place_writable(object, &place)?;
                let resolved = self.resolve_alias(&object_type, 0);
                match resolved {
                    Type::List(inner) => Ok(*inner),
                    Type::Object | Type::Any | Type::Named(_, _) => Ok(Type::Any),
                    other => {
                        Err(self.err(format!("cannot assign into `{}` by index", other.name())))
                    }
                }
            }
        }
    }

    /// `@State` / `@Store` / `@Effect` / `@Environment` are only valid at the
    /// top level of a function whose return type is `View` (docs §12).
    fn require_component_top_level(&self, what: &str) -> SResult<()> {
        if self.component_depth == 0 {
            return Err(self.err(format!(
                "`{what}` may only be used at the top level of a function returning `View`"
            )));
        }
        if self.block_depth > 0 {
            return Err(self.err(format!("`{what}` may not be used inside a nested block")));
        }
        Ok(())
    }

    /// `@Memo` is a general computation optimization: allowed at the top level
    /// of any function body (not only `View` components), never in a nested
    /// block or at module scope.
    fn require_fn_body_top_level(&self, what: &str) -> SResult<()> {
        if self.current_return.is_none() {
            return Err(self.err(format!(
                "`{what}` may only be used at the top level of a function body"
            )));
        }
        if self.block_depth > 0 {
            return Err(self.err(format!("`{what}` may not be used inside a nested block")));
        }
        Ok(())
    }

    /// Is this type `View` (optionally wrapped in `async`)?
    fn is_component_type(&self, ty: &Type) -> bool {
        match self.resolve_alias(ty, 0) {
            Type::Named(n, _) => n == "View",
            Type::Async(inner) => matches!(inner.as_ref(), Type::Named(n, _) if n == "View"),
            _ => false,
        }
    }

    fn check_state(&mut self, binding: &LetBinding) -> SResult<()> {
        self.require_component_top_level("@State")?;
        if let Some(annotation) = &binding.type_annotation {
            self.check_type(annotation)?;
        }
        let value_type = match &binding.value {
            Some(value) => self.check_expression(value)?,
            None => binding.type_annotation.clone().unwrap_or(Type::Any),
        };
        if let Some(annotation) = &binding.type_annotation {
            let literal_ok = match &binding.value {
                Some(value) => self.literal_matches(value, annotation),
                None => false,
            };
            if !self.assignable(&value_type, annotation) && !literal_ok {
                return Err(self.err_code(
                    "E0201",
                    format!(
                        "cannot bind a value of type `{}` to `@State {}: {}`",
                        value_type.name(),
                        binding.name,
                        annotation.name()
                    ),
                ));
            }
        }
        let declared = self.table.declare_with_def(
            Symbol {
                name: binding.name.clone(),
                type_: binding.type_annotation.clone().unwrap_or(value_type),
                kind: SymbolKind::State,
                is_shared: false,
                is_param: false,
                is_const: binding.is_const,
                is_mutable: true,
            },
            Some(binding.name_span.clone()),
        );
        if !declared {
            return Err(self.err(format!(
                "binding `{}` is already declared in this scope",
                binding.name
            )));
        }
        Ok(())
    }

    fn check_store(&mut self, store: &xulo_core::ast::StoreStmt) -> SResult<()> {
        self.require_component_top_level("@Store")?;
        let value_type = self.check_expression(&store.value)?;
        match &store.pattern {
            xulo_core::ast::BindingPattern::Ident(name) => {
                // A plain binding keeps the value's inferred type (like
                // `@State`); only destructured store fields stay `Any`, since
                // their shape is not statically known.
                if !self.table.declare(Symbol {
                    name: name.clone(),
                    type_: value_type.clone(),
                    kind: SymbolKind::Store,
                    is_shared: false,
                    is_param: false,
                    is_const: true,
                    is_mutable: false,
                }) {
                    return Err(self.err(format!(
                        "binding `{name}` is already declared in this scope"
                    )));
                }
            }
            xulo_core::ast::BindingPattern::Destructure(fields) => {
                for (name, alias) in fields {
                    let local = alias.clone().unwrap_or_else(|| name.clone());
                    if !self.table.declare(Symbol {
                        name: local.clone(),
                        type_: Type::Any,
                        kind: SymbolKind::Store,
                        is_shared: false,
                        is_param: false,
                        is_const: true,
                        is_mutable: false,
                    }) {
                        return Err(self.err(format!(
                            "binding `{local}` is already declared in this scope"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn check_effect(&mut self, effect: &xulo_core::ast::EffectStmt) -> SResult<()> {
        self.require_component_top_level("@Effect")?;
        // The runtime calls the closure with no arguments (`__effect(fn)`), so
        // parameters would silently bind `undefined`; reject them up front.
        if let Some(first) = effect.closure.params.first() {
            return Err(self.err(format!(
                "`@Effect` closures take no parameters (found `{}`): the runtime calls them without arguments",
                first.name
            )));
        }
        let saved = self.in_effect;
        self.in_effect = true;
        let result = (|| {
            self.check_fn_expr(&effect.closure)?;
            if let Some(deps) = &effect.deps {
                for dep in deps {
                    self.check_expression(dep)?;
                }
            }
            Ok(())
        })();
        self.in_effect = saved;
        result
    }

    fn check_environment(&mut self, env: &xulo_core::ast::EnvStmt) -> SResult<()> {
        self.require_component_top_level("@Environment")?;
        self.check_type(&env.type_)?;
        if !self.table.declare_with_def(
            Symbol {
                name: env.name.clone(),
                type_: env.type_.clone(),
                kind: SymbolKind::Store,
                is_shared: false,
                is_param: false,
                is_const: true,
                is_mutable: false,
            },
            Some(env.name_span.clone()),
        ) {
            return Err(self.err(format!(
                "binding `{}` is already declared in this scope",
                env.name
            )));
        }
        Ok(())
    }

    fn check_component(&mut self, component: &xulo_core::ast::ComponentStmt) -> SResult<()> {
        // Uppercase calls lower to UI components (`Name({ key: value })`);
        // props are not validated against a function signature (see
        // `component_call_props_are_loosely_typed`).
        // Components declare no type parameters — a written `CallTypeArgs`
        // list is `E0807` (spec `expressions/calls.md`, `ComponentCall`).
        if component.type_args.is_some() {
            return Err(self
                .err_code(
                    "E0807",
                    format!("`{}` accepts no type arguments", component.name),
                )
                .at(component.span.clone()));
        }
        self.record_component_use(component);
        for arg in &component.args {
            self.check_arg_value(&arg.value)?;
        }
        self.block_depth += 1;
        let mut result = Ok(());
        for child in &component.children {
            if let Err(e) = self.check_ui_element(child) {
                result = Err(e);
                break;
            }
        }
        self.block_depth -= 1;
        result
    }

    /// Record the component name as a name-use so editor tooling (hover /
    /// go-to-definition) can target it. A component whose name resolves to a
    /// user-declared factory function records that symbol (so the definition
    /// jumps to it); otherwise it is treated as a builtin component
    /// (`View`/`Text`/`Row`/...) with no definition.
    fn record_component_use(&mut self, component: &xulo_core::ast::ComponentStmt) {
        let name = &component.name;
        let span = component.name_span.clone();
        // A component invocation colors like `View` regardless of whether the
        // name resolves to a user factory function (its `def` still enables
        // go-to-definition). Only the *kind* differs from a plain call.
        let def = self.table.lookup_def(name).flatten().cloned();
        let type_ = self
            .lookup_symbol(name)
            .map(|symbol| symbol.type_)
            .unwrap_or_else(|_| Type::Named("View".to_string(), Vec::new()));
        self.resolutions.push(UseRecord {
            span,
            name: name.clone(),
            kind: UseKind::Component,
            type_: Some(type_),
            def,
        });
    }

    fn check_ui_element(&mut self, el: &xulo_core::ast::UiElement) -> SResult<()> {
        match el {
            xulo_core::ast::UiElement::Component(c) => self.check_component(c),
            xulo_core::ast::UiElement::Text(_) => Ok(()),
            xulo_core::ast::UiElement::Expr(e) => {
                let ty = self.check_expression(e)?;
                let allowed = match &ty {
                    Type::String | Type::Any => true,
                    other if self.is_component_type(other) => true,
                    // A list/optional child forwards ready-made children (e.g.
                    // a `children: list<View>` parameter); only element
                    // types that can themselves be children make sense.
                    Type::Optional(inner) => self.is_component_type(inner),
                    Type::List(inner) => {
                        self.is_component_type(inner)
                            || matches!(inner.as_ref(), Type::String | Type::Any)
                    }
                    _ => false,
                };
                if !allowed {
                    return Err(self.err(format!(
                        "component children must be strings, components, or lists of components, found `{}`",
                        ty.name()
                    )));
                }
                Ok(())
            }
            xulo_core::ast::UiElement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond = self.check_expression(condition)?;
                if !self.assignable(&cond, &Type::Boolean) {
                    return Err(self.err(format!(
                        "if condition must be a `bool`, found `{}`",
                        cond.name()
                    )));
                }
                for e in then_branch {
                    self.check_ui_element(e)?;
                }
                if let Some(els) = else_branch {
                    for e in els {
                        self.check_ui_element(e)?;
                    }
                }
                Ok(())
            }
            xulo_core::ast::UiElement::For {
                iter_var,
                iter_var_span,
                iterable,
                body,
            } => {
                let iterable_type = self.check_expression(iterable)?;
                match iterable_type {
                    Type::List(_) | Type::Any => {}
                    other => {
                        return Err(self.err(format!(
                            "for loop must iterate over a `List`, found `{}`",
                            other.name()
                        )));
                    }
                }
                self.table.push_scope();
                self.table.declare_with_def(
                    Symbol {
                        name: iter_var.clone(),
                        type_: Type::Any,
                        kind: SymbolKind::Variable,
                        is_shared: false,
                        is_param: false,
                        is_const: false,
                        is_mutable: true,
                    },
                    Some(iter_var_span.clone()),
                );
                let mut result = Ok(());
                for e in body {
                    if let Err(err) = self.check_ui_element(e) {
                        result = Err(err);
                        break;
                    }
                }
                self.table.pop_scope();
                result
            }
            xulo_core::ast::UiElement::Group(group) => {
                for e in group {
                    self.check_ui_element(e)?;
                }
                Ok(())
            }
        }
    }

    fn check_type_alias(&mut self, alias: &TypeAlias) -> SResult<()> {
        self.generics.extend(alias.type_params.iter().cloned());
        let result = self.check_type(&alias.type_);
        self.generics
            .truncate(self.generics.len().saturating_sub(alias.type_params.len()));
        result?;
        if self.type_table.contains_key(&alias.name) {
            return Err(self.err(format!("type `{}` is already defined", alias.name)));
        }
        self.type_table.insert(
            alias.name.clone(),
            TypeEntry {
                type_params: alias.type_params.clone(),
                type_bounds: alias.type_bounds.clone(),
                kind: TypeEntryKind::Alias(alias.type_.clone()),
            },
        );
        Ok(())
    }

    fn check_enum(&mut self, e: &xulo_core::ast::EnumDef) -> SResult<()> {
        if self.type_table.contains_key(&e.name) {
            return Err(self.err(format!("type `{}` is already defined", e.name)));
        }
        let mut seen = std::collections::HashSet::new();
        self.generics.extend(e.type_params.iter().cloned());
        for variant in &e.variants {
            if !seen.insert(variant.name.clone()) {
                self.generics
                    .truncate(self.generics.len().saturating_sub(e.type_params.len()));
                return Err(self.err(format!(
                    "enum `{}` has a duplicate member `{}`",
                    e.name, variant.name
                )));
            }
            if let Some(params) = &variant.payload
                && let Err(e2) = params.iter().try_for_each(|p| self.check_type(&p.type_))
            {
                self.generics
                    .truncate(self.generics.len().saturating_sub(e.type_params.len()));
                return Err(e2);
            }
        }
        self.generics
            .truncate(self.generics.len().saturating_sub(e.type_params.len()));
        self.type_table.insert(
            e.name.clone(),
            TypeEntry {
                type_params: e.type_params.clone(),
                type_bounds: e.type_bounds.clone(),
                kind: TypeEntryKind::Enum(e.variants.clone()),
            },
        );
        Ok(())
    }

    fn check_struct(&mut self, s: &xulo_core::ast::StructDef) -> SResult<()> {
        if self.type_table.contains_key(&s.name) {
            return Err(self.err(format!("type `{}` is already defined", s.name)));
        }
        // Register the struct name as a variable so `User.create(...)` /
        // `User.method(instance)` member calls type-check (the runtime keeps
        // the struct object in the global environment).
        self.table.declare_with_def(
            Symbol {
                name: s.name.clone(),
                type_: Type::Named(s.name.clone(), Vec::new()),
                kind: SymbolKind::Variable,
                is_shared: false,
                is_param: false,
                is_const: true,
                is_mutable: false,
            },
            Some(s.name_span.clone()),
        );
        self.generics.extend(s.type_params.iter().cloned());
        let mut fields = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for field in &s.fields {
            if !seen.insert(field.name.clone()) {
                self.generics
                    .truncate(self.generics.len().saturating_sub(s.type_params.len()));
                return Err(self.err(format!(
                    "struct `{}` has a duplicate field `{}`",
                    s.name, field.name
                )));
            }
            self.check_type(&field.type_)?;
            fields.push((
                field.name.clone(),
                field.type_.clone(),
                field.default.clone(),
            ));
        }
        self.generics
            .truncate(self.generics.len().saturating_sub(s.type_params.len()));
        self.type_table.insert(
            s.name.clone(),
            TypeEntry {
                type_params: s.type_params.clone(),
                type_bounds: s.type_bounds.clone(),
                kind: TypeEntryKind::Struct(fields),
            },
        );
        Ok(())
    }

    fn check_trait(&mut self, t: &TraitDecl) -> SResult<()> {
        // A trait is a fixed contract; the grammar accepts a parameter list
        // only so the attempt can be diagnosed (spec `types/traits.md`).
        if !t.type_params.is_empty() {
            return Err(self.err_code("E0810", "a trait may not declare type parameters"));
        }
        if self.type_table.contains_key(&t.name) {
            return Err(self.err(format!("type `{}` is already defined", t.name)));
        }
        // Register the trait before checking its methods so a method may
        // reference the trait itself as a return type (`fn scale(self): Shape`).
        self.type_table.insert(
            t.name.clone(),
            TypeEntry {
                type_params: t.type_params.clone(),
                type_bounds: Vec::new(),
                kind: TypeEntryKind::Trait(TraitDef {
                    methods: Vec::new(),
                }),
            },
        );
        let mut seen = std::collections::HashSet::new();
        self.generics.extend(t.type_params.iter().cloned());
        let mut check_all = |this: &Self| {
            for m in &t.methods {
                if !seen.insert(m.name.clone()) {
                    return Err(this.err(format!(
                        "trait `{}` has a duplicate method `{}`",
                        t.name, m.name
                    )));
                }
                for p in &m.params {
                    if let Some(ty) = &p.type_annotation {
                        this.check_type(ty)?;
                    }
                }
                if let Some(rt) = &m.return_type {
                    this.check_type(rt)?;
                }
            }
            Ok(())
        };
        let result = check_all(self);
        self.generics
            .truncate(self.generics.len().saturating_sub(t.type_params.len()));
        result?;
        let methods = t
            .methods
            .iter()
            .map(|m| TraitMethodSig {
                name: m.name.clone(),
                params: m
                    .params
                    .iter()
                    .map(|p| p.type_annotation.clone().unwrap_or(Type::Any))
                    .collect(),
                return_type: m.return_type.clone().unwrap_or(Type::Any),
                has_self: m.has_self,
                is_async: m.is_async,
            })
            .collect();
        if let Some(entry) = self.type_table.get_mut(&t.name) {
            entry.kind = TypeEntryKind::Trait(TraitDef { methods });
        }
        Ok(())
    }

    /// Fold the collected `reply` operand types into the service's reply
    /// type `R`: `Type::Null` (unit) when the method never replies, the operands'
    /// common type otherwise, and E0211 when they share none (spec
    /// `concurrency.md`).
    fn common_reply_type(&mut self, type_name: &str) -> SResult<()> {
        let mut common: Option<Type> = None;
        for operand in std::mem::take(&mut self.reply_operands) {
            common = Some(match common {
                None => operand,
                Some(prev) if self.assignable(&operand, &prev) => prev,
                Some(prev) if self.assignable(&prev, &operand) => operand,
                Some(prev) => {
                    return Err(self.err_code(
                        "E0211",
                        format!(
                            "`{}` and `{}` have no common type",
                            prev.name(),
                            operand.name()
                        ),
                    ));
                }
            });
        }
        self.service_reply_types
            .insert(type_name.to_string(), common.unwrap_or(Type::Null));
        Ok(())
    }

    /// `impl Trait for Type { ... }`: every method must have a signature
    /// compatible with the trait's, and its body is checked as a normal
    /// function (where `self` binds to the implemented type).
    fn check_impl(&mut self, imp: &ImplDecl) -> SResult<()> {
        // `impl<T: Bound> Receiver<T> { … }`: the receiver's type parameters
        // and their bounds are in scope while the method bodies are checked —
        // `self` carries them, so a field of type `T` has its bound's members
        // (spec `types/traits.md`, `types/generics.md`). Bounds are validated
        // first (E0806); the stack is popped on every exit, errors included.
        let mut bound_map = HashMap::new();
        let mut trait_map: HashMap<String, Vec<String>> = HashMap::new();
        for bound in &imp.type_bounds {
            let shape = self.bound_shape(bound)?;
            bound_map.insert(bound.param.clone(), shape);
            trait_map
                .entry(bound.param.clone())
                .or_default()
                .extend(bound.traits.iter().cloned());
        }
        self.generics.extend(imp.type_params.iter().cloned());
        self.bound_shapes.push(bound_map);
        self.bound_traits.push(trait_map);
        let result = self.check_impl_body(imp);
        let pushed = imp.type_params.len();
        self.generics.truncate(self.generics.len() - pushed);
        self.bound_shapes.pop();
        self.bound_traits.pop();
        result
    }

    fn check_impl_body(&mut self, imp: &ImplDecl) -> SResult<()> {
        // Inherent impl: `impl Type { ... }` — no trait, just methods on the type
        if imp.is_inherent {
            let ty_entry = self
                .type_entry(&imp.type_name)
                .ok_or_else(|| self.err(format!("unknown type `{}`", imp.type_name)))?;
            if matches!(ty_entry.kind, TypeEntryKind::Trait(_)) {
                return Err(self.err(format!(
                    "`{}` is a trait and cannot be the receiver type of an inherent `impl`",
                    imp.type_name
                )));
            }
            // The receiver must be a well-formed application of the
            // implemented type: the declaration's arity (E0230) and its
            // declared bounds apply to the written arguments, so a bare
            // `impl Palette` on `struct Palette<T>` is rejected here.
            self.check_type(&Type::Named(imp.type_name.clone(), imp.type_args.clone()))?;
            // The receiver's type as the impl declares it: `self` binds to
            // `Pair<T, T>` inside `impl<T> Pair<T, T>`, carrying the
            // parameters into the body.
            let self_ty = Type::Named(imp.type_name.clone(), imp.type_args.clone());
            for m in &imp.methods {
                // Check the method body with the receiver bound to the type.
                self.table.push_scope();
                for param in &m.params {
                    if param.name == "self" {
                        self.table.declare_with_def(
                            Symbol {
                                name: "self".to_string(),
                                type_: self_ty.clone(),
                                kind: SymbolKind::Variable,
                                is_shared: param.mode == Some(xulo_core::ast::ParamMode::Shared),
                                is_param: true,
                                is_const: true,
                                is_mutable: param.mode == Some(xulo_core::ast::ParamMode::Mut),
                            },
                            None,
                        );
                    } else {
                        let ty = param.type_annotation.clone().unwrap_or(Type::Any);
                        self.table.declare_with_def(
                            Symbol {
                                name: param.name.clone(),
                                type_: ty,
                                kind: SymbolKind::Variable,
                                is_shared: param.mode == Some(xulo_core::ast::ParamMode::Shared),
                                is_param: true,
                                is_const: true,
                                is_mutable: param.mode == Some(xulo_core::ast::ParamMode::Mut),
                            },
                            Some(param.span.clone()),
                        );
                    }
                }
                let impl_ret = m.return_type.clone().unwrap_or(Type::Any);
                let saved = self.current_return.replace(impl_ret);
                let saved_declared = self.declared_return;
                self.declared_return = m.return_type.is_some();
                let saved_async = self.async_depth;
                self.async_depth = if m.is_async { saved_async + 1 } else { 0 };
                // Inside `on_message`, `reply(...)` settles the send task;
                // the common type of its operands is the service's `R`.
                self.in_on_message = m.name == "on_message";
                self.reply_operands.clear();
                let result = self.check_block_implicit(&m.body);
                self.in_on_message = false;
                let reply_result = if m.name == "on_message" {
                    self.common_reply_type(&imp.type_name)
                } else {
                    Ok(())
                };
                self.reply_operands.clear();
                result?;
                reply_result?;
                self.async_depth = saved_async;
                self.declared_return = saved_declared;
                self.current_return = saved;
                self.table.pop_scope();
            }
            return Ok(());
        }

        // Trait impl: `impl Trait for Type { ... }`
        let entry = self
            .type_entry(&imp.trait_name)
            .cloned()
            .ok_or_else(|| self.err(format!("unknown trait `{}`", imp.trait_name)))?;
        let TypeEntryKind::Trait(trait_def) = &entry.kind else {
            return Err(self.err(format!("`{}` is not a trait", imp.trait_name)));
        };
        let ty_entry = self
            .type_entry(&imp.type_name)
            .ok_or_else(|| self.err(format!("unknown type `{}`", imp.type_name)))?;
        if matches!(ty_entry.kind, TypeEntryKind::Trait(_)) {
            return Err(self.err(format!(
                "`{}` is a trait and cannot be the receiver type of an `impl`",
                imp.type_name
            )));
        }
        // Same well-formedness for a trait impl's receiver: arity (E0230),
        // declared bounds, and well-formed arguments (`impl<T> Show for
        // Pair<T, T>` checks `Pair`'s application against `T, T`).
        self.check_type(&Type::Named(imp.type_name.clone(), imp.type_args.clone()))?;
        let self_ty = Type::Named(imp.type_name.clone(), imp.type_args.clone());
        // The `impl` must provide every method the trait declares; otherwise a
        // later `Trait::method` dispatch of the missing one would only fail at
        // the call site with a misleading "does not implement" error.
        {
            let implemented = imp
                .methods
                .iter()
                .map(|m| m.name.as_str())
                .collect::<std::collections::HashSet<_>>();
            let missing = trait_def
                .methods
                .iter()
                .filter(|m| !implemented.contains(m.name.as_str()))
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(self.err_code(
                    "E0802",
                    format!(
                        "`impl {0} for {1}` does not implement all of `{0}`'s methods: missing {2}",
                        imp.trait_name,
                        imp.type_name,
                        missing.join(", ")
                    ),
                ));
            }
        }
        for m in &imp.methods {
            let Some(sig) = trait_def.methods.iter().find(|s| s.name == m.name) else {
                return Err(self.err_code(
                    "E0804",
                    format!(
                        "`impl {0} for {1}` defines `{2}`, which `{0}` does not declare",
                        imp.trait_name, imp.type_name, m.name
                    ),
                ));
            };
            // The `self` receiver must match the trait declaration: dispatch
            // always passes the receiver as the impl function's first argument,
            // so a mismatch would silently drop it or mis-bind the parameters.
            let impl_has_self = m.params.iter().any(|p| p.name == "self");
            if impl_has_self != sig.has_self {
                return Err(self.err_code(
                    "E0803",
                    format!(
                        "`impl {0} for {1}` method `{2}` {3}",
                        imp.trait_name,
                        imp.type_name,
                        m.name,
                        if sig.has_self {
                            "must declare `self` as its first parameter, as the trait does"
                        } else {
                            "declares `self`, but the trait method has no receiver"
                        }
                    ),
                ));
            }
            // Async-ness must match too: dispatching is typed by the trait
            // signature, so a promise/value mismatch would break `await` at the
            // call site.
            if m.is_async != sig.is_async {
                return Err(self.err_code(
                    "E0803",
                    format!(
                        "`impl {0} for {1}` method `{2}` must be {3} to match the trait",
                        imp.trait_name,
                        imp.type_name,
                        m.name,
                        if sig.is_async { "async" } else { "synchronous" }
                    ),
                ));
            }
            // Signature compatibility: the impl method's non-`self` parameters
            // and return must match the trait binding.
            let impl_params = m
                .params
                .iter()
                .filter(|p| p.name != "self")
                .collect::<Vec<_>>();
            let sig_params = sig.params.clone();
            if impl_params.len() != sig_params.len() {
                return Err(self.err_code(
                    "E0803",
                    format!(
                        "`impl {0} for {1}` method `{2}` has the wrong arity (trait declares {3})",
                        imp.trait_name,
                        imp.type_name,
                        m.name,
                        sig_params.len()
                    ),
                ));
            }
            for (impl_p, sig_t) in impl_params.iter().zip(sig_params.iter()) {
                let impl_t = impl_p.type_annotation.clone().unwrap_or(Type::Any);
                let compatible = self.assignable(sig_t, &impl_t) && self.assignable(&impl_t, sig_t);
                if !compatible {
                    return Err(self.err_code(
                        "E0803",
                        format!(
                            "`impl {0} for {1}` method `{2}` parameter `{3}` must be `{4}`, found `{5}`",
                            imp.trait_name,
                            imp.type_name,
                            m.name,
                            impl_p.name,
                            sig_t.name(),
                            impl_t.name()
                        ),
                    ));
                }
            }
            let impl_ret = m.return_type.clone().unwrap_or(Type::Any);
            if !self.assignable(&sig.return_type, &impl_ret)
                && !self.assignable(&impl_ret, &sig.return_type)
            {
                return Err(self.err_code(
                    "E0803",
                    format!(
                        "`impl {0} for {1}` method `{2}` must return `{3}`, found `{4}`",
                        imp.trait_name,
                        imp.type_name,
                        m.name,
                        sig.return_type.name(),
                        impl_ret.name()
                    ),
                ));
            }
            // Register the dispatch target so `Trait::method` can resolve it.
            if !self.impls.insert((
                imp.trait_name.clone(),
                imp.type_name.clone(),
                m.name.clone(),
            )) {
                return Err(self.err_code(
                    "E0805",
                    format!(
                        "`{0}` is already implemented for `{1}` (method `{2}`)",
                        imp.trait_name, imp.type_name, m.name
                    ),
                ));
            }
            // Guard the mangled dispatch name: it joins `trait`/`type`/`method`
            // with `_`, so an `impl` whose names differ only by underscores (or
            // a user `fn` with the same name) would silently overwrite in the
            // JS and native runtimes.
            let mangled = impl_fn_name(&imp.trait_name, &imp.type_name, &m.name);
            if !self.impl_mangled.insert(mangled.clone()) {
                return Err(self.err(format!(
                    "trait dispatch name `{mangled}` is claimed by more than one `impl` (trait/type/method names that mangle to the same string collide, e.g. `a_b`/`c` vs `a`/`b_c`)"
                )));
            }
            if self.table.lookup(&mangled).is_some() {
                return Err(self.err(format!(
                    "trait dispatch name `{mangled}` collides with a function of the same name"
                )));
            }
            // Check the method body with the receiver bound to the type.
            self.table.push_scope();
            for param in &m.params {
                if param.name == "self" {
                    self.table.declare_with_def(
                        Symbol {
                            name: "self".to_string(),
                            type_: self_ty.clone(),
                            kind: SymbolKind::Variable,
                            is_shared: param.mode == Some(xulo_core::ast::ParamMode::Shared),
                            is_param: true,
                            is_const: true,
                            is_mutable: param.mode == Some(xulo_core::ast::ParamMode::Mut),
                        },
                        None,
                    );
                } else {
                    let ty = param.type_annotation.clone().unwrap_or(Type::Any);
                    self.table.declare_with_def(
                        Symbol {
                            name: param.name.clone(),
                            type_: ty,
                            kind: SymbolKind::Variable,
                            is_shared: param.mode == Some(xulo_core::ast::ParamMode::Shared),
                            is_param: true,
                            is_const: true,
                            is_mutable: param.mode == Some(xulo_core::ast::ParamMode::Mut),
                        },
                        Some(param.span.clone()),
                    );
                }
            }
            let saved = self.current_return.replace(impl_ret.clone());
            let saved_declared = self.declared_return;
            self.declared_return = m.return_type.is_some();
            let saved_async = self.async_depth;
            self.async_depth = if m.is_async { saved_async + 1 } else { 0 };
            // Inside `on_message`, `reply(...)` settles the send task.
            self.in_on_message = m.name == "on_message";
            let result = self.check_block_implicit(&m.body);
            self.in_on_message = false;
            self.async_depth = saved_async;
            self.declared_return = saved_declared;
            self.current_return = saved;
            result?;
            self.table.pop_scope();
        }
        Ok(())
    }

    /// Build the structural `ObjectType` shape of a trait: one field per method,
    /// typed as its `fn(...)` signature with `self` erased. Used for bound
    /// refinement and structural satisfaction checks.
    fn trait_shape(&self, def: &TraitDef) -> Type {
        let fields = def
            .methods
            .iter()
            .map(|m| {
                let params = m.params.clone();
                let bare = m.return_type.clone();
                let ret: Option<Box<Type>> = Some(Box::new(if m.is_async {
                    Type::Async(Box::new(bare))
                } else {
                    bare
                }));
                (m.name.clone(), Type::FnSig { params, ret })
            })
            .collect();
        Type::ObjectType(fields)
    }

    /// Resolve a bound clause's trait names (verifying each is a known trait)
    /// into the synthetic `ObjectType` combining their method fields.
    fn bound_shape(&self, bound: &FnBound) -> SResult<Type> {
        let mut fields = Vec::new();
        for trait_name in &bound.traits {
            let entry = self
                .type_entry(trait_name)
                .ok_or_else(|| self.err_code("E0806", format!("unknown trait `{trait_name}`")))?;
            let TypeEntryKind::Trait(def) = &entry.kind else {
                return Err(self.err_code("E0806", format!("`{trait_name}` is not a trait")));
            };
            for (name, ty) in match self.trait_shape(def) {
                Type::ObjectType(f) => f,
                _ => unreachable!(),
            } {
                if !fields.iter().any(|(n, _)| *n == name) {
                    fields.push((name, ty));
                }
            }
        }
        Ok(Type::ObjectType(fields))
    }

    /// Does `arg` satisfy the bound `trait_name`? (spec `types/traits.md`):
    /// exactly when a visible `impl` for that trait applies to it — there is
    /// no structural fallback and no second source of evidence.
    fn satisfies_bound(&self, arg: &Type, trait_name: &str) -> bool {
        // A type parameter is admissible when its own declaration grants this
        // bound; the evidence travels with the parameter into the body.
        if let Type::Named(name, _) = arg
            && self.generics.contains(name)
        {
            return self
                .bound_traits
                .iter()
                .rev()
                .find_map(|m| m.get(name))
                .is_some_and(|traits| traits.iter().any(|t| t == trait_name));
        }
        // An impl is registered under the implemented type's declared name;
        // an alias also answers under every name it expands to.
        let mut current = match arg {
            Type::Named(name, _) => Some(name.clone()),
            _ => None,
        };
        let mut hops = 0;
        while let Some(name) = current {
            if self
                .impls
                .iter()
                .any(|(t, ty, _)| t == trait_name && ty == &name)
            {
                // A generic `impl` that cannot be instantiated for this
                // receiver — or whose bounds that instantiation leaves
                // unsatisfied — contributes nothing for it (spec
                // `types/traits.md`, "When an impl applies"). Impls of this
                // module are pre-scanned, so the gate can decide; an `impl`
                // present only as an imported triple keeps its name-only
                // answer.
                let mut any_record = false;
                let mut applies = false;
                for record in self
                    .declared_impls
                    .iter()
                    .filter(|r| r.trait_name.as_deref() == Some(trait_name) && r.type_name == name)
                {
                    any_record = true;
                    if self.impl_applies(record, &name, arg).is_some() {
                        applies = true;
                        break;
                    }
                }
                if !any_record || applies {
                    return true;
                }
                // This name's impl does not apply: keep following the alias
                // chain in case another name answers.
            }
            hops += 1;
            if hops > 32 {
                break;
            }
            current = match self.type_entry(&name).map(|e| &e.kind) {
                Some(TypeEntryKind::Alias(Type::Named(target, _))) => Some(target.clone()),
                _ => None,
            };
        }
        false
    }

    /// Does the pre-scanned `impl` `record` apply to a receiver answering
    /// under `receiver_name` with type `receiver` (spec `types/traits.md`,
    /// "When an impl applies")? A non-generic `impl` applies to its receiver
    /// name alone; a generic one applies exactly when its written receiver
    /// arguments instantiate to the receiver's — every parameter binding to
    /// one type — and every bound of that instantiation is satisfied in
    /// turn. Returns that instantiation's bindings, or `None`.
    fn impl_applies(
        &self,
        record: &ImplRecord,
        receiver_name: &str,
        receiver: &Type,
    ) -> Option<HashMap<String, Type>> {
        if record.type_name != receiver_name {
            return None;
        }
        let Type::Named(_, receiver_args) = receiver else {
            return None;
        };
        if record.type_params.is_empty() {
            return Some(HashMap::new());
        }
        let mut bindings = HashMap::new();
        if !pattern_matches(
            &record.type_args,
            receiver_args,
            &record.type_params,
            &mut bindings,
        ) {
            return None;
        }
        for bound in &record.bounds {
            let Some(arg) = bindings.get(&bound.param) else {
                // The parameter never appears in the receiver pattern;
                // nothing selected a type to hold against its bound.
                continue;
            };
            for trait_name in &bound.traits {
                if !self.satisfies_bound(arg, trait_name) {
                    return None;
                }
            }
        }
        Some(bindings)
    }

    /// True when this module declares impls of `trait_name` for
    /// `receiver_name` yet none of them applies to `receiver`: the
    /// registered `(trait, type)` name alone must not answer for impls that
    /// contribute nothing here. No records at all (an imported impl) keeps
    /// the name-only answer, so `false`.
    fn impl_gate_blocks(&self, trait_name: &str, receiver_name: &str, receiver: &Type) -> bool {
        let mut any_record = false;
        for record in self
            .declared_impls
            .iter()
            .filter(|r| r.trait_name.as_deref() == Some(trait_name) && r.type_name == receiver_name)
        {
            any_record = true;
            if self.impl_applies(record, receiver_name, receiver).is_some() {
                return false;
            }
        }
        any_record
    }

    /// Check every inferred binding of a call against its parameter's bounds
    /// (spec `type-system/checking-rules.md`, E0801). A parameter no
    /// constraint reached is left alone: nothing selected a type argument for
    /// it to be checked against.
    fn check_inferred_bounds(
        &self,
        bounds: &[FnBound],
        bindings: &std::collections::HashMap<String, Type>,
    ) -> SResult<()> {
        for bound in bounds {
            let Some(arg) = bindings.get(&bound.param) else {
                continue;
            };
            for trait_name in &bound.traits {
                if !self.satisfies_bound(arg, trait_name) {
                    return Err(self.err_code(
                        "E0801",
                        format!("`{}` does not implement `{trait_name}`", arg.display()),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Validate a written call-site type-argument list (`first<int>(…)`): a
    /// list before a callee that declares no type parameters is `E0807`, a
    /// list of the wrong length is `E0808`, and an accepted list yields the
    /// bindings that take the place of inference for this call (spec
    /// `types/generics.md`).
    fn written_type_args(
        &self,
        call: &Call,
        type_params: &[String],
    ) -> SResult<Option<HashMap<String, Type>>> {
        let Some(written) = &call.type_args else {
            return Ok(None);
        };
        if type_params.is_empty() {
            return Err(self
                .err_code(
                    "E0807",
                    format!("`{}` accepts no type arguments", call.callee),
                )
                .at(call.span.clone()));
        }
        if written.len() != type_params.len() {
            let expected = type_params.len();
            return Err(self
                .err_code(
                    "E0808",
                    format!(
                        "expected {expected} type argument{}, found {}",
                        if expected == 1 { "" } else { "s" },
                        written.len()
                    ),
                )
                .at(call.span.clone()));
        }
        Ok(Some(
            type_params
                .iter()
                .cloned()
                .zip(written.iter().cloned())
                .collect(),
        ))
    }

    /// `E0807` for a callee that declares no type parameters at all — an
    /// intrinsic, a constructor, a method resolved without a generic
    /// signature (spec `types/generics.md`, registered message `` `f`
    /// accepts no type arguments ``).
    fn no_type_args(&self, call: &Call) -> SResult<()> {
        if call.type_args.is_some() {
            return Err(self
                .err_code(
                    "E0807",
                    format!("`{}` accepts no type arguments", call.callee),
                )
                .at(call.span.clone()));
        }
        Ok(())
    }

    /// Replace a callee's type parameters with `any` throughout a declared
    /// parameter type, so the structural argument check does not re-run the
    /// bound rule (the parameter's bound is what decides admissibility, and
    /// is checked by [`Analyzer::check_inferred_bounds`]).
    fn erase_generic_params(&self, ty: &Type, type_params: &[String]) -> Type {
        let bindings: std::collections::HashMap<String, Type> =
            type_params.iter().map(|p| (p.clone(), Type::Any)).collect();
        substitute_type(ty, &bindings)
    }

    fn check_block(&mut self, block: &Block) -> SResult<()> {
        self.check_block_tail(block).map(|_| ())
    }

    /// Check an expression statement and infer its value type. Statement-position
    /// `if` compiles to a direct `if` statement, so `await` in its arms is fine
    /// (unlike a value-position `if`, which `check_expression` wraps in
    /// `no_await`).
    fn check_expr_stmt(&mut self, stmt: &xulo_core::ast::ExprStmt) -> SResult<Type> {
        if let Expression::If(if_expr) = &stmt.expr {
            self.check_if(if_expr)
        } else {
            self.check_expression(&stmt.expr)
        }
    }

    /// Check a block and return the type of its trailing expression (the
    /// block's value), if any. Runs while the block scope is active. A
    /// trailing expression is the block's value, so it is checked via
    /// `check_expression` (a trailing `if`/`match` is therefore value
    /// position and `await` inside it is rejected).
    fn check_block_tail(&mut self, block: &Block) -> SResult<Option<Type>> {
        self.table.push_scope();
        self.block_depth += 1;
        // Every block owns its borrows: they are released at the block's end
        // (the scope exit has settled the task by then) and no `await` in a
        // sibling or outer block ends them.
        let saved_block = self.block_id;
        self.block_id = self.next_block_id;
        self.next_block_id += 1;
        let saved_borrows = self.pending_borrows.len();
        let tail = match block.statements.last() {
            Some(Statement::Expr(e)) => {
                self.check_statements_lenient(&block.statements[..block.statements.len() - 1])?;
                // A trailing `if`/`match` here is still statement position
                // (codegen emits it via `if_stmt`, whose `await` is fine). Only
                // an enclosing value-position context (`check_expression` for
                // `If`/`Match`) wraps arms in `no_await`.
                Some(self.check_expr_stmt(e)?)
            }
            _ => {
                self.check_statements_lenient(&block.statements)?;
                None
            }
        };
        self.pending_borrows.truncate(saved_borrows);
        self.block_id = saved_block;
        self.block_depth -= 1;
        self.table.pop_scope();
        Ok(tail)
    }

    /// Check a function body block, then validate a trailing expression as its
    /// implicit return (docs §21.2): when the enclosing function declares a
    /// return type, a trailing expression statement is that function's value.
    /// Mirrors codegen's `fn_def`/`fn_expr` rule and must run while the body
    /// scope is still active.
    fn check_block_implicit(&mut self, block: &Block) -> SResult<()> {
        self.table.push_scope();
        if self.declared_return
            && let Some(Statement::Expr(last)) = block.statements.last()
        {
            let expected = self.current_return.clone().unwrap();
            let target = match &expected {
                Type::Async(inner) => inner.as_ref(),
                other => other,
            };
            self.check_statements_lenient(&block.statements[..block.statements.len() - 1])?;
            if last.has_semicolon {
                // A trailing `expr;` is an ordinary statement, not an implicit
                // return (docs §21.2) → warn that its value is discarded.
                self.check_expr_stmt(last)?;
                self.warn(format!(
                    "ignored return value: trailing expression with `;` is not the function's return value (expected `{}`)",
                    expected.name()
                ));
            } else {
                // A trailing `expr` is the function's implicit return → value
                // position, so a trailing `if`/`match` rejects `await` inside.
                let value_type = self.check_expression(&last.expr)?;
                if !self.assignable(&value_type, target)
                    && !self.literal_matches(&last.expr, target)
                {
                    if let Some(e) =
                        self.fixed_literal_error(&last.expr, target, last.expr.span().clone())
                    {
                        return Err(e);
                    }
                    return Err(self.err(format!(
                        "return type mismatch: expected `{}`, found `{}`",
                        expected.name(),
                        value_type.name()
                    )));
                }
            }
        } else {
            self.check_statements_lenient(&block.statements)?;
        }
        self.table.pop_scope();
        Ok(())
    }

    /// Check a sequence of statements. In lenient (editor-tooling) mode each
    /// statement is checked independently — an error is remembered (once) and
    /// the rest keep checking, so sibling statements still resolve; in strict
    /// mode this is just the fail-fast loop.
    fn check_statements_lenient(&mut self, statements: &[Statement]) -> SResult<()> {
        if self.lenient {
            for statement in statements {
                self.check_statement_lenient(statement);
            }
            Ok(())
        } else {
            for statement in statements {
                self.check_statement(statement)?;
            }
            Ok(())
        }
    }

    /// Check an expression and infer its type, recording every checked
    /// expression's type out-of-band (`AnalysisResult.expr_types`, keyed by
    /// span) for editor tooling. The bulk of the work happens in
    /// [`Analyzer::check_expression_inner`]; the recording is deliberately
    /// done here, on success, so a failed check never leaves a type entry.
    /// Check one call/component argument value: the argument is a move
    /// position, so `move x` may appear whole inside it (spec
    /// `memory-and-runtime.md`, E0403).
    fn check_arg_value(&mut self, value: &Expression) -> SResult<Type> {
        self.in_move_position = true;
        self.check_expression(value)
    }

    fn check_expression(&mut self, expr: &Expression) -> SResult<Type> {
        self.current_span = expr.span().clone();
        // Only the outermost place of a chain is a use: the binding read of
        // `xs[0]` is part of reaching that one place, not a second use.
        if self.place_depth == 0
            && matches!(
                expr,
                Expression::Identifier { .. } | Expression::Member(_) | Expression::Index(_)
            )
            && let Some(place) = place_of(expr)
        {
            self.check_borrow_place(&place, false)?;
        }
        let ty = self.check_expression_inner(expr)?;
        self.expr_types.push((expr.span().clone(), ty.clone()));
        Ok(ty)
    }

    fn check_expression_inner(&mut self, expr: &Expression) -> SResult<Type> {
        // The move position set by a `let`/argument/`return` covers exactly
        // this node: children get no position of their own (spec
        // `memory-and-runtime.md`, E0403).
        self.cur_move_pos = std::mem::replace(&mut self.in_move_position, false);
        match expr {
            Expression::Literal { value: lit, .. } => self.check_literal(lit),
            Expression::Identifier { name, .. } => {
                if let Expression::Identifier { name, span } = expr {
                    self.check_spawn_capture(name, span)?;
                    if self.moved.contains_key(name) {
                        return Err(self
                            .err_code("E0403", format!("cannot use moved value `{name}`"))
                            .at(span.clone()));
                    }
                }
                let sym = self.lookup_symbol(name)?;
                // E0903 (spec ffi.md, "Calling"): an imported symbol is
                // callable in direct call position only — `check_call`
                // resolves a bare callee itself and never reaches this arm,
                // so anything else (binding it, passing it, comparing it)
                // is the error. Matched against the declaration span, so a
                // shadowing local binding of the same spelling is unaffected.
                if let Some(def) = self.extern_imports.get(name)
                    && self.table.lookup_def(name).flatten() == Some(def)
                {
                    return Err(self
                        .err_code(
                            "E0903",
                            format!("`{name}` is an extern import and can only be called directly"),
                        )
                        .at(expr.span().clone()));
                }
                self.record_use(expr.span().clone(), name, &sym);
                // A function name used as a value has its function type,
                // not its return type: `let f = double; f(21)`.
                if let SymbolKind::Function(type_params, params, ret, _) = &sym.kind
                    && type_params.is_empty()
                {
                    return Ok(Type::FnSig {
                        params: params
                            .iter()
                            .map(|p| p.type_annotation.clone().unwrap_or(Type::Any))
                            .collect(),
                        ret: Some(Box::new(ret.clone())),
                    });
                }
                Ok(sym.type_.clone())
            }
            Expression::BinaryOp(bin) => self.check_binary(bin),
            Expression::Unary(un) => self.check_unary(un),
            Expression::Call(call) => self.check_call(call),
            Expression::EnumRef(r) => self.check_enum_ref(r.enum_name.clone(), &r.variant),
            Expression::If(if_expr) => self.no_await(|this| this.check_if(if_expr)),
            Expression::Ternary(tr) => self.check_ternary(tr),
            Expression::Match(m) => self.no_await(|this| this.check_match(m)),
            Expression::Member(m) => self.check_member(m),
            Expression::Index(idx) => self.check_index(idx),
            Expression::Nullish(n) => self.check_nullish(n),
            Expression::Range(r) => {
                let start = self.check_expression(&r.start)?;
                let end = self.check_expression(&r.end)?;
                if !self.assignable(&start, &Type::Number) || !self.assignable(&end, &Type::Number)
                {
                    return Err(self.err(format!(
                        "range endpoints must be `number`, found `{}` and `{}`",
                        start.name(),
                        end.name()
                    )));
                }
                Ok(Type::List(Box::new(Type::Number)))
            }
            Expression::Await { expr: operand, .. } => self.check_await(operand),
            Expression::FnExpr(f) => self.check_fn_expr(f),
            Expression::Binding { name, .. } => {
                let sym = self.lookup_symbol(name)?;
                self.record_use(expr.span().clone(), name, &sym);
                match sym.kind {
                    SymbolKind::State | SymbolKind::Store => Ok(sym.type_.clone()),
                    _ => Err(self.err(format!(
                        "`$` binding requires a `@State` or `@Store` variable, but `{name}` is not"
                    ))),
                }
            }
            Expression::Spread { .. } => {
                Err(self.err("`...` spread is only allowed inside list or object literals"))
            }
            Expression::CallValue(cv) => {
                let callee_type = self.check_expression(&cv.callee)?;
                let callee_type = self.resolve_alias(&callee_type, 0);
                // A callee reached through a subscript, a call, or any other
                // postfix never declares type parameters: a written list on a
                // callable value is `E0807` (spec `types/generics.md`).
                if cv.type_args.is_some() && matches!(callee_type, Type::FnSig { .. } | Type::Any) {
                    return Err(self
                        .err_code(
                            "E0807",
                            format!(
                                "expression of type `{}` accepts no type arguments",
                                callee_type.name()
                            ),
                        )
                        .at(cv.span.clone()));
                }
                match callee_type {
                    Type::FnSig { params, ret } => {
                        self.check_fn_value_args(&params, ret, &cv.arguments)
                    }
                    Type::Any => {
                        for a in &cv.arguments {
                            self.check_expression(&a.value)?;
                        }
                        Ok(Type::Any)
                    }
                    other => Err(self.err(format!(
                        "expression of type `{}` is not callable",
                        other.name()
                    ))),
                }
            }
            Expression::Component(c) => {
                for arg in &c.args {
                    self.check_arg_value(&arg.value)?;
                }
                for el in &c.children {
                    self.check_ui_element(el)?;
                }
                Ok(Type::Named("View".to_string(), Vec::new()))
            }
            Expression::Tuple { items, .. } => {
                let mut types = Vec::with_capacity(items.len());
                for item in items {
                    types.push(self.check_expression(item)?);
                }
                Ok(Type::Tuple(types))
            }
            Expression::TypedMap(tm) => {
                for entry in &tm.entries {
                    self.check_expression(&entry.key)?;
                    self.check_expression(&entry.value)?;
                }
                Ok(Type::Named("Map".to_string(), Vec::new()))
            }
            Expression::Is(is) => {
                self.check_expression(&is.expr)?;
                self.check_type(&is.type_)?;
                Ok(Type::Boolean)
            }
            Expression::Panic { expr: inner, .. } => {
                self.check_expression(inner)?;
                Ok(Type::Any)
            }
            Expression::Propagate { expr: inner, .. } => {
                let inner_ty = self.check_expression(inner)?;
                let (success, failures) = self.propagate_shape(&inner_ty, inner.span().clone())?;
                for failure in &failures {
                    self.check_propagate_admission(failure, inner.span().clone())?;
                }
                Ok(success)
            }
            Expression::Arrow(arrow) => {
                self.table.push_scope();
                let param_types = self.declare_params_with_defaults(&arrow.params)?;
                let saved_return = self
                    .current_return
                    .replace(arrow.return_type.clone().unwrap_or(Type::Any));
                let saved_declared = self.declared_return;
                self.declared_return = arrow.return_type.is_some();
                let saved_async = self.async_depth;
                self.async_depth = if arrow.is_async { saved_async + 1 } else { 0 };
                let saved_component = self.component_depth;
                let saved_block = self.block_depth;
                let saved_no_await = self.no_await_depth;
                let saved_lock = self.lock_depth;
                // A closure's body is not the `defer` operand itself: it runs
                // whenever it is called, so `?` inside it is not E0229.
                let saved_defer = self.in_defer;
                self.in_defer = 0;
                self.closure_depth += 1;
                self.component_depth = 0;
                self.block_depth = 0;
                self.no_await_depth = 0;
                self.lock_depth = 0;
                let saved_moved = std::mem::take(&mut self.moved);
                let ret = match &arrow.body {
                    xulo_core::ast::ArrowBody::Expr(e) => Some(Box::new(self.check_expression(e)?)),
                    xulo_core::ast::ArrowBody::Block(b) => self.check_block_tail(b)?.map(Box::new),
                };
                self.moved = saved_moved;
                self.in_defer = saved_defer;
                self.closure_depth -= 1;
                self.no_await_depth = saved_no_await;
                self.lock_depth = saved_lock;
                self.component_depth = saved_component;
                self.block_depth = saved_block;
                self.async_depth = saved_async;
                self.current_return = saved_return;
                self.declared_return = saved_declared;
                self.table.pop_scope();
                let ret = match ret {
                    Some(r) if arrow.is_async => Some(Box::new(Type::Async(r))),
                    other => other,
                };
                Ok(Type::FnSig {
                    params: param_types,
                    ret,
                })
            }
            Expression::Spawn(spawn) => match &spawn.body {
                xulo_core::ast::SpawnBody::Async(b) => {
                    // `check_block` pushes the block's own scope: references
                    // to anything declared at a shallower scope must be
                    // captured first (spec concurrency.md).
                    let depth = self.table.depth();
                    self.spawn_scopes.push(depth);
                    let result = self.check_block(b);
                    self.spawn_scopes.pop();
                    result?;
                    Ok(Type::Named("Task".to_string(), Vec::new()))
                }
                xulo_core::ast::SpawnBody::Service { type_name, fields } => {
                    for field in fields {
                        self.check_expression(&field.value)?;
                    }
                    // The service type must declare `on_message` in an `impl`
                    // block, taking a receiver and exactly one message
                    // parameter (spec `concurrency.md`, E0409).
                    let methods = self.impl_methods.get(type_name).map(|ms| {
                        ms.iter()
                            .find(|m| m.name == "on_message")
                            .map(|m| (m.has_self, m.param_count))
                    });
                    match methods.flatten() {
                        None => {
                            return Err(self
                                .err_code(
                                    "E0409",
                                    format!("service `{type_name}` declares no `on_message`"),
                                )
                                .at(expr.span().clone()));
                        }
                        Some((has_self, param_count)) => {
                            if !has_self || param_count != 2 {
                                return Err(self
                                    .err_code(
                                        "E0409",
                                        format!(
                                            "`{type_name}.on_message` must take a receiver (`self` or `mut self`) and exactly one message parameter"
                                        ),
                                    )
                                    .at(expr.span().clone()));
                            }
                        }
                    }
                    // The handle is a structural value:
                    // `{ send: fn(M): Task<R>, stop: fn(): unit }` (spec
                    // `concurrency.md`). `M` comes from `on_message`'s message
                    // parameter; `R` is left loose (`any`) until reply-operand
                    // common typing lands (E0211).
                    let on_message = self
                        .impl_methods
                        .get(type_name)
                        .and_then(|ms| ms.iter().find(|m| m.name == "on_message"));
                    let message_type = on_message
                        .and_then(|m| m.param_types.get(1).cloned())
                        .unwrap_or(Type::Any);
                    // `R` is the common type of the `on_message` reply
                    // operands (spec `concurrency.md`); `Any` when the impl
                    // has not been checked yet (the spawn precedes it in the
                    // file), matching the loose `any` of unchecked bodies.
                    let reply_type = self
                        .service_reply_types
                        .get(type_name)
                        .cloned()
                        .unwrap_or(Type::Any);
                    Ok(Type::ObjectType(vec![
                        (
                            "send".to_string(),
                            Type::FnSig {
                                params: vec![message_type],
                                ret: Some(Box::new(Type::Named(
                                    "Task".to_string(),
                                    vec![reply_type],
                                ))),
                            },
                        ),
                        (
                            "stop".to_string(),
                            Type::FnSig {
                                params: vec![],
                                ret: Some(Box::new(Type::Any)),
                            },
                        ),
                    ]))
                }
            },
            Expression::Lock(lock) => {
                // The operand is the region the section guards: it must be a
                // `shared` binding (spec concurrency.md, E0407).
                let (is_shared, defined) = match self.table.lookup(&lock.name) {
                    Some(sym) => (sym.is_shared, true),
                    None => (false, false),
                };
                if !defined {
                    return Err(self
                        .err(format!("undefined variable `{}` in `lock`", lock.name))
                        .at(lock.name_span.clone()));
                }
                if !is_shared {
                    return Err(self
                        .err_code("E0407", "`lock` requires a shared binding")
                        .at(lock.name_span.clone()));
                }
                self.locked.push(lock.name.clone());
                self.lock_depth += 1;
                let result = self.check_block(&lock.body);
                self.lock_depth -= 1;
                self.locked.pop();
                result?;
                Ok(Type::Named("Lock".to_string(), Vec::new()))
            }
        }
    }

    /// Check a literal, recursively validating its contents (list items, object
    /// field values, spread operands) and inferring its type.
    fn check_literal(&mut self, lit: &Literal) -> SResult<Type> {
        match lit {
            Literal::List(items) => {
                if items.is_empty() {
                    return Ok(Type::List(Box::new(Type::Any)));
                }
                let mut element = Type::Any;
                let mut first = true;
                for item in items {
                    let item_type = match item {
                        Expression::Spread { expr: spread, .. } => {
                            let spread_type = self.check_expression(spread)?;
                            match spread_type {
                                Type::List(inner) => *inner,
                                Type::Any => Type::Any,
                                other => {
                                    return Err(self.err(format!(
                                        "spread operand must be a list, got `{}`",
                                        other.name()
                                    )));
                                }
                            }
                        }
                        other => self.check_expression(other)?,
                    };
                    if first {
                        element = item_type;
                        first = false;
                    } else if !self.assignable(&item_type, &element) {
                        // A list literal's elements need not share one type:
                        // `[1, "a"]` infers `list<number | string>` (an
                        // explicit `list<T | U>` annotation then type-checks,
                        // while an incompatible annotation like
                        // `list<number>` is rejected at the binding). This
                        // keeps mixed literals *typed* instead of silently
                        // collapsing them to the first element's type.
                        if self.assignable(&element, &item_type) {
                            element = item_type;
                        } else {
                            element = union_join(&element, &item_type);
                        }
                    }
                }
                Ok(Type::List(Box::new(element)))
            }
            Literal::Object(fields) => {
                let mut field_names = Vec::new();
                let mut field_types: Vec<(String, Type)> = Vec::new();
                for field in fields {
                    match field {
                        ObjectField::Field { name, value } => {
                            let ty = self.check_expression(value)?;
                            field_names.push(name.clone());
                            field_types.push((name.clone(), ty));
                        }
                        ObjectField::Spread { value } => {
                            let spread_type = self.check_expression(value)?;
                            if !matches!(spread_type, Type::Object | Type::Any) {
                                return Err(self.err(format!(
                                    "spread operand must be an object, got `{}`",
                                    spread_type.name()
                                )));
                            }
                            // Spread means we can't do exact struct matching
                            field_names.clear();
                            field_types.clear();
                        }
                    }
                }
                // Try to match against a known struct type, inferring the
                // struct's type arguments from the field values so a
                // generic struct literal carries them (`Palette(shape: rect)`
                // → `Palette<Rect>`), which is what impl applicability and
                // bound checking decide against (spec `types/traits.md`).
                // A struct declaring no parameters matches exactly as before;
                // a conflict keeps the argument-less type it always had.
                if !field_names.is_empty() {
                    let mut first_match: Option<String> = None;
                    for (name, entry) in &self.type_table {
                        let TypeEntryKind::Struct(declared) = &entry.kind else {
                            continue;
                        };
                        let struct_field_names: Vec<&str> =
                            declared.iter().map(|(n, _, _)| n.as_str()).collect();
                        let literal_field_names: Vec<&str> =
                            field_names.iter().map(|n| n.as_str()).collect();
                        if struct_field_names != literal_field_names {
                            continue;
                        }
                        if entry.type_params.is_empty() {
                            return Ok(Type::Named(name.clone(), Vec::new()));
                        }
                        if first_match.is_none() {
                            first_match = Some(name.clone());
                        }
                        let declared_types: Vec<Type> =
                            declared.iter().map(|(_, t, _)| t.clone()).collect();
                        let value_types: Vec<Type> =
                            field_types.iter().map(|(_, t)| t.clone()).collect();
                        if let Ok(bindings) =
                            infer_type_bindings(&entry.type_params, &declared_types, &value_types)
                        {
                            let args = entry
                                .type_params
                                .iter()
                                .map(|p| bindings.get(p).cloned().unwrap_or(Type::Any))
                                .collect();
                            return Ok(Type::Named(name.clone(), args));
                        }
                    }
                    if let Some(name) = first_match {
                        return Ok(Type::Named(name, Vec::new()));
                    }
                }
                Ok(Type::Object)
            }
            other => Ok(literal_type(other)),
        }
    }

    /// Run `f` with `no_await_depth` incremented, so `await` inside `if`/
    /// `match` arms is rejected (their arms compile to a plain function).
    fn no_await<T>(&mut self, f: impl FnOnce(&mut Self) -> SResult<T>) -> SResult<T> {
        self.no_await_depth += 1;
        let result = f(self);
        self.no_await_depth -= 1;
        result
    }

    fn check_await(&mut self, operand: &Expression) -> SResult<Type> {
        // A `lock` body holds a critical section from entry to the end of the
        // block; a suspension point would let another task interrupt it, so
        // the section is checked for absence of `await` (spec concurrency.md).
        if self.lock_depth > 0 {
            return Err(self.err_code("E0410", "`await` is not allowed inside `lock`"));
        }
        if self.no_await_depth > 0 {
            return Err(self.err(
                "`await` cannot be used inside an `if`/`match` expression; assign its value first with `let`",
            ));
        }
        if self.async_depth == 0 {
            return Err(self.err("`await` may only be used inside an `async` function"));
        }
        self.release_borrows_of_awaited(operand);
        let inner = self.check_expression(operand)?;
        match inner {
            Type::Async(inner) => Ok(*inner),
            // `await` unwraps `Task<T>` (spec: concurrency.md, lexical-structure.md).
            Type::Named(name, args) if name == "Task" => {
                Ok(args.into_iter().next().unwrap_or(Type::Any))
            }
            Type::Any => Ok(Type::Any),
            other => Err(self.err(format!(
                "cannot await a non-promise value of type `{}`",
                other.name()
            ))),
        }
    }

    fn check_unary(&mut self, un: &xulo_core::ast::UnaryOp) -> SResult<Type> {
        // Read before the operand's own check resets the position state.
        let move_pos = self.cur_move_pos;
        let operand = self.check_expression(&un.operand)?;
        match un.operator {
            xulo_core::ast::UnaryOperator::Not => {
                if self.assignable(&operand, &Type::Boolean) {
                    Ok(Type::Boolean)
                } else {
                    Err(self.err(format!(
                        "unary `!` requires a `bool` operand, found `{}`",
                        operand.name()
                    )))
                }
            }
            xulo_core::ast::UnaryOperator::Neg => {
                if self.assignable(&operand, &Type::Number) {
                    Ok(Type::Number)
                } else {
                    Err(self.err(format!(
                        "unary `-` requires a `number` operand, found `{}`",
                        operand.name()
                    )))
                }
            }
            xulo_core::ast::UnaryOperator::BitNot => {
                if self.assignable(&operand, &Type::Number) {
                    Ok(Type::Number)
                } else {
                    Err(self.err(format!(
                        "unary `~` requires a `number` operand, found `{}`",
                        operand.name()
                    )))
                }
            }
            xulo_core::ast::UnaryOperator::Move => {
                // A transfer is legal only in a `let` initializer, an
                // argument, or a `return` (spec `memory-and-runtime.md`,
                // E0403), and moves a whole, frame-owned binding: the operand
                // is a path naming one, and never a borrow.
                if !move_pos {
                    return Err(self.err_code(
                        "E0403",
                        "`move` is only allowed in a `let` initializer, an argument, or a `return`",
                    ));
                }
                let moved = match &un.operand {
                    Expression::Identifier { name, .. } => Some(name.clone()),
                    _ => None,
                };
                let Some(name) = moved else {
                    return Err(self.err_code(
                        "E0403",
                        "a transfer moves a whole binding, never a part of one or a computed value",
                    ));
                };
                if let Some(sym) = self.table.lookup(&name)
                    && sym.is_param
                {
                    return Err(self.err_code(
                        "E0403",
                        format!("cannot move `{name}`: it is a borrow, not an owned binding"),
                    ));
                }
                // A move is a consuming use: an outstanding borrow of the
                // place bars it (E0405 mutably, E0411 immutably).
                if let Some(place) = place_of(&un.operand) {
                    self.check_borrow_place(&place, true)?;
                }
                self.moved.insert(name, un.operand.span().clone());
                Ok(operand)
            }
            xulo_core::ast::UnaryOperator::Copy => Ok(operand),
        }
    }

    fn check_ternary(&mut self, tr: &xulo_core::ast::TernaryExpr) -> SResult<Type> {
        let condition = self.check_expression(&tr.condition)?;
        if !self.assignable(&condition, &Type::Boolean) {
            return Err(self.err(format!(
                "ternary condition must be a `bool`, found `{}`",
                condition.name()
            )));
        }
        let then_type = self.check_expression(&tr.then_value)?;
        let else_type = self.check_expression(&tr.else_value)?;
        if self.assignable(&then_type, &else_type) {
            Ok(else_type)
        } else if self.assignable(&else_type, &then_type) {
            Ok(then_type)
        } else {
            Ok(Type::Any)
        }
    }

    fn check_member(&mut self, m: &xulo_core::ast::MemberAccess) -> SResult<Type> {
        self.place_depth += 1;
        let object = self.check_expression(&m.object);
        self.place_depth -= 1;
        let object = object?;
        if let Some(place) = place_of(&m.object) {
            self.check_shared_contents(&place)?;
        }
        // A type parameter's members come from its bound, and the bound is
        // checked on the raw type: `resolve_alias` flattens `T` to the bound's
        // shape (or to `any` when unbounded), which would erase the parameter
        // name from the E0102 message. A member the bound does not declare —
        // or any member of an unbounded parameter — is E0102 (spec
        // `types/traits.md`).
        if let Type::Named(name, _) = &object
            && self.generics.contains(name)
        {
            let property = m.property.clone();
            return match self.bound_shape_of(name) {
                Some(Type::ObjectType(fields)) => fields
                    .iter()
                    .find(|(n, _)| *n == property)
                    .map(|(_, t)| t.clone())
                    .ok_or_else(|| {
                        self.err_code("E0102", format!("no member `{property}` on `{name}`"))
                    }),
                _ => Err(self.err_code("E0102", format!("no member `{property}` on `{name}`"))),
            };
        }
        let resolved = self.resolve_alias(&object, 0);
        let inner = match resolved {
            Type::Optional(inner) if m.optional => *inner,
            Type::Null if m.optional => return Ok(Type::Any),
            Type::Optional(_) => {
                return Err(self.err(format!(
                    "cannot access member of optional type `{}` without `?.`",
                    object.name()
                )));
            }
            other => other,
        };
        self.member_field_type(&inner, &m.property)
    }

    /// Look up a field type on an object-like type (`ObjectType`, the loose
    /// `object`, or `Any`).
    /// Structural shape for an in-scope bounded generic parameter (`T: Area`),
    /// searched from the innermost enclosing bound map outward.
    fn bound_shape_of(&self, name: &str) -> Option<Type> {
        self.bound_shapes
            .iter()
            .rev()
            .find_map(|map| map.get(name).cloned())
    }

    fn member_field_type(&self, ty: &Type, property: &str) -> SResult<Type> {
        match ty {
            Type::ObjectType(fields) => fields
                .iter()
                .find(|(n, _)| n == property)
                .map(|(_, t)| t.clone())
                .ok_or_else(|| self.err(format!("type has no member `{property}`"))),
            Type::Object | Type::Any => Ok(Type::Any),
            Type::List(_) => match property {
                "length" | "capacity" => Ok(Type::Number),
                _ => Ok(Type::Any),
            },
            Type::Named(name, _) => {
                // A declared struct's fields type their member access (`e`
                // : `Error` has `message: string`); a name the struct does
                // not declare is E0102 unless an `impl` method provides it —
                // methods are reached by call, but the name exists (spec
                // `expressions/path-and-access.md`).
                if !self.generics.contains(name)
                    && let Some(TypeEntryKind::Struct(fields)) =
                        self.type_entry(name).map(|e| &e.kind)
                {
                    if let Some((_, field_type, _)) = fields.iter().find(|(n, _, _)| n == property)
                    {
                        return Ok(field_type.clone());
                    }
                    if self
                        .impl_methods
                        .get(name)
                        .is_some_and(|ms| ms.iter().any(|m| m.name == property))
                    {
                        return Ok(Type::Any);
                    }
                    return Err(self.err_code(
                        "E0102",
                        format!("no member `{property}` on `{}`", ty.display()),
                    ));
                }
                // A bounded generic parameter (`T: Area`) exposes its bound's
                // members, so `t.area()` type-checks against the trait shape;
                // a type parameter has no members at all without a bound, and
                // a member the bound does not declare is E0102 either way
                // (spec `types/traits.md`).
                match self.bound_shape_of(name) {
                    Some(Type::ObjectType(fields)) => fields
                        .iter()
                        .find(|(n, _)| n == property)
                        .map(|(_, t)| t.clone())
                        .ok_or_else(|| {
                            self.err_code("E0102", format!("no member `{property}` on `{name}`"))
                        }),
                    _ if self.generics.contains(name) => {
                        Err(self.err_code("E0102", format!("no member `{property}` on `{name}`")))
                    }
                    _ => Ok(Type::Any),
                }
            }
            other => Err(self.err(format!(
                "cannot access member `{}` of a `{}`",
                property,
                other.name()
            ))),
        }
    }

    fn check_index(&mut self, idx: &xulo_core::ast::IndexExpr) -> SResult<Type> {
        self.place_depth += 1;
        let object = self.check_expression(&idx.object);
        self.place_depth -= 1;
        let object = object?;
        self.check_expression(&idx.index)?;
        if let Some(place) = place_of(&idx.object) {
            self.check_shared_contents(&place)?;
        }
        let resolved = self.resolve_alias(&object, 0);
        match resolved {
            Type::List(inner) => Ok(*inner),
            Type::Object | Type::Any | Type::String | Type::Null => Ok(Type::Any),
            Type::Named(_, _) => Ok(Type::Any),
            other => Err(self.err(format!("cannot index into `{}`", other.name()))),
        }
    }

    fn check_nullish(&mut self, n: &xulo_core::ast::NullishExpr) -> SResult<Type> {
        let left = self.check_expression(&n.left)?;
        let right = self.check_expression(&n.right)?;
        if let Type::Optional(inner) = left {
            Ok(*inner)
        } else if matches!(left, Type::Null) || matches!(left, Type::Any) {
            Ok(right)
        } else {
            Ok(left)
        }
    }

    fn check_match(&mut self, m: &xulo_core::ast::MatchExpr) -> SResult<Type> {
        let value_type = self.check_expression(&m.value)?;
        let mut arm_types = Vec::new();
        for arm in &m.arms {
            match &arm.pattern {
                xulo_core::ast::MatchPattern::Wildcard => {}
                xulo_core::ast::MatchPattern::Literal(lit) => {
                    let lit_type = literal_type(lit);
                    if !self.assignable(&lit_type, &value_type) {
                        return Err(self.err(format!(
                            "match arm pattern `{}` does not match value of type `{}`",
                            pattern_name(arm),
                            value_type.name()
                        )));
                    }
                }
                xulo_core::ast::MatchPattern::Range { .. } => {
                    if !self.assignable(&value_type, &Type::Number)
                        && !self.assignable(&value_type, &Type::String)
                    {
                        return Err(self.err(format!(
                            "match arm range pattern does not match value of type `{}` (expected `number` or `string`)",
                            value_type.name()
                        )));
                    }
                }
                xulo_core::ast::MatchPattern::Binding { name, span } => {
                    self.table.declare_with_def(
                        Symbol {
                            name: name.clone(),
                            type_: value_type.clone(),
                            kind: SymbolKind::Variable,
                            is_shared: false,
                            is_param: false,
                            is_const: true,
                            is_mutable: false,
                        },
                        Some(span.clone()),
                    );
                }
                xulo_core::ast::MatchPattern::TypePattern {
                    type_name,
                    binder,
                    span,
                    ..
                } => {
                    let pattern_ty = pattern_primitive_type(type_name)
                        .unwrap_or_else(|| Type::Named(type_name.clone(), Vec::new()));
                    self.check_type(&pattern_ty)?;
                    if let xulo_core::ast::PatternBinder::Binding(b) = binder {
                        self.table.declare_with_def(
                            Symbol {
                                name: b.clone(),
                                type_: pattern_ty,
                                kind: SymbolKind::Variable,
                                is_shared: false,
                                is_param: false,
                                is_const: true,
                                is_mutable: false,
                            },
                            Some(span.clone()),
                        );
                    }
                }
                xulo_core::ast::MatchPattern::Or { .. } => {}
                xulo_core::ast::MatchPattern::Struct { .. } => {}
                xulo_core::ast::MatchPattern::Enum(r) => {
                    let enum_type = self.check_enum_ref(r.enum_name.clone(), &r.variant)?;
                    if !self.assignable(&value_type, &enum_type) {
                        return Err(self.err(format!(
                            "match arm pattern `{}` does not match value of type `{}` (expected `{}`)",
                            pattern_name(arm),
                            value_type.name(),
                            enum_type.name()
                        )));
                    }
                }
                xulo_core::ast::MatchPattern::EnumPayload {
                    enum_name,
                    variant,
                    bindings,
                    span,
                } => {
                    let entry = self
                        .type_entry(enum_name)
                        .ok_or_else(|| self.err(format!("unknown enum `{enum_name}`")))?
                        .clone();
                    let enum_type = Type::Named(enum_name.clone(), Vec::new());
                    if !self.assignable(&value_type, &enum_type) {
                        return Err(self.err(format!(
                            "match arm pattern `{}` does not match value of type `{}` (expected `{}`)",
                            pattern_name(arm),
                            value_type.name(),
                            enum_type.name()
                        )));
                    }
                    let v = entry
                        .kind
                        .variants()
                        .and_then(|vs| vs.iter().find(|v| v.name == *variant))
                        .ok_or_else(|| {
                            self.err(format!("enum `{enum_name}` has no member `{variant}`"))
                        })?
                        .clone();
                    match &v.payload {
                        Some(params) => {
                            if params.len() != bindings.len() {
                                return Err(self
                                    .err(format!(
                                        "pattern binds {} values but `{enum_name}::{variant}` carries {}",
                                        bindings.len(),
                                        params.len()
                                    ))
                                    .at(span.clone()));
                            }
                            self.generics.extend(entry.type_params.iter().cloned());
                            self.table.push_scope();
                            for (param, binding) in params.iter().zip(bindings.iter()) {
                                if binding == "_" {
                                    continue;
                                }
                                self.table.declare_with_def(
                                    Symbol {
                                        name: binding.clone(),
                                        type_: self.resolve_alias(&param.type_, 0),
                                        kind: SymbolKind::Variable,
                                        is_shared: false,
                                        is_param: false,
                                        is_const: true,
                                        is_mutable: false,
                                    },
                                    Some(span.clone()),
                                );
                            }
                            let t = {
                                if let Some(guard) = &arm.guard {
                                    let guard_type = self.check_expression(guard)?;
                                    if !self.assignable(&guard_type, &Type::Boolean) {
                                        return Err(self.err(format!(
                                            "match guard must be a `bool`, found `{}`",
                                            guard_type.name()
                                        )));
                                    }
                                }
                                self.check_match_body(&arm.value)?
                            };
                            self.table.pop_scope();
                            self.generics.truncate(
                                self.generics.len().saturating_sub(entry.type_params.len()),
                            );
                            arm_types.push(t);
                            continue;
                        }
                        None => {
                            return Err(self.err(format!(
                                "enum member `{enum_name}::{variant}` has no payload to bind"
                            )));
                        }
                    }
                }
            }
            if let Some(guard) = &arm.guard {
                let guard_type = self.check_expression(guard)?;
                if !self.assignable(&guard_type, &Type::Boolean) {
                    return Err(self.err(format!(
                        "match guard must be a `bool`, found `{}`",
                        guard_type.name()
                    )));
                }
            }
            arm_types.push(self.check_match_body(&arm.value)?);
        }
        if arm_types.is_empty() {
            return Ok(Type::Any);
        }
        let first = &arm_types[0];
        if arm_types.iter().all(|t| t == first) {
            return Ok(first.clone());
        }
        // Arms must be mutually assignable; the first arm's type is the match
        // type (mirrors `check_if`'s branch typing).
        for t in &arm_types[1..] {
            if !self.assignable(t, first) && !self.assignable(first, t) {
                return Err(self.err(format!(
                    "match arms have incompatible types `{}` and `{}`",
                    first.name(),
                    t.name()
                )));
            }
        }
        Ok(first.clone())
    }

    /// Check a match arm body: an expression checks as an expression; a block
    /// checks as a block whose value is its trailing type (`Type::Any` for
    /// a unit-less block with no trailing expression).
    fn check_match_body(&mut self, body: &xulo_core::ast::MatchBody) -> SResult<Type> {
        match body {
            xulo_core::ast::MatchBody::Expr(e) => self.check_expression(e),
            xulo_core::ast::MatchBody::Block(b) => {
                Ok(self.check_block_tail(b)?.unwrap_or(Type::Any))
            }
        }
    }

    /// The fixed-bit operand rule of spec `types/primitive-types.md`
    /// ("Arithmetic result types", "Comparison operators"): `T op T` is `T`
    /// and a literal that coerces to `T` joins it (`u32 + 8` is `u32`);
    /// mixing a fixed-bit type with any other numeric type is `E0201`.
    /// `Ok(None)` leaves the caller's ordinary rules in charge: neither
    /// operand is fixed-bit, either is `any`, or the other operand is not
    /// numeric (the plain "cannot apply" / "cannot compare" error).
    fn fixed_bit_op(
        &self,
        bin: &xulo_core::ast::BinaryOp,
        left: &Type,
        right: &Type,
        comparison: bool,
    ) -> SResult<Option<Type>> {
        let l = self.resolve_alias(left, 0);
        let r = self.resolve_alias(right, 0);
        if matches!(l, Type::Any) || matches!(r, Type::Any) {
            return Ok(None);
        }
        let op = bin.operator.symbol();
        let mixing_err = |this: &Self, a: &Type, b: &Type| {
            let message = if comparison {
                format!("cannot compare `{}` with `{}`", a.name(), b.name())
            } else {
                format!("cannot apply `{op}` to `{}` and `{}`", a.name(), b.name())
            };
            this.err_code("E0201", message).at(bin.span.clone())
        };
        match (fixed_bit_name(&l), fixed_bit_name(&r)) {
            (Some(l_name), Some(r_name)) => {
                if l_name == r_name {
                    Ok(Some(if comparison { Type::Boolean } else { l.clone() }))
                } else {
                    Err(mixing_err(self, &l, &r))
                }
            }
            (Some(_), None) => {
                if self.literal_matches(&bin.right, &l) {
                    Ok(Some(if comparison { Type::Boolean } else { l.clone() }))
                } else if matches!(r, Type::Number) {
                    // A `number` operand that is not a literal coercing to
                    // the fixed type — a variable, or an out-of-range /
                    // non-integral literal — mixes (E0201).
                    Err(mixing_err(self, &l, &r))
                } else {
                    Ok(None)
                }
            }
            (None, Some(_)) => {
                if self.literal_matches(&bin.left, &r) {
                    Ok(Some(if comparison { Type::Boolean } else { r.clone() }))
                } else if matches!(l, Type::Number) {
                    Err(mixing_err(self, &l, &r))
                } else {
                    Ok(None)
                }
            }
            (None, None) => Ok(None),
        }
    }

    fn check_binary(&mut self, bin: &xulo_core::ast::BinaryOp) -> SResult<Type> {
        let left = self.check_expression(&bin.left)?;
        let right = self.check_expression(&bin.right)?;
        match bin.operator {
            BinaryOperator::Add => {
                let l = self.resolve_alias(&left, 0);
                let r = self.resolve_alias(&right, 0);
                // Fixed-bit arithmetic outranks the loose `number` rule: a
                // fixed type never widens to `number` (spec
                // `types/primitive-types.md`).
                if let Some(t) = self.fixed_bit_op(bin, &l, &r, false)? {
                    return Ok(t);
                }
                if matches!(l, Type::Number) && matches!(r, Type::Number) {
                    Ok(Type::Number)
                } else if self.is_stringish(&l) && self.is_stringish(&r) {
                    Ok(Type::String)
                } else if matches!(l, Type::List(_)) && matches!(r, Type::List(_)) {
                    // Record the site so codegen emits an array concatenation
                    // (`a.concat(b)`) instead of a JS `+`, which would coerce
                    // the lists into a string.
                    self.list_concat.push(bin.span.clone());
                    Ok(Type::List(Box::new(self.join_list(&l, &r))))
                } else if matches!(l, Type::Any) || matches!(r, Type::Any) {
                    Ok(Type::Any)
                } else {
                    Err(self
                        .err(format!(
                            "cannot apply `+` to `{}` and `{}`",
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
            BinaryOperator::Sub
            | BinaryOperator::Mul
            | BinaryOperator::Div
            | BinaryOperator::Mod
            | BinaryOperator::Pow => {
                if let Some(t) = self.fixed_bit_op(bin, &left, &right, false)? {
                    return Ok(t);
                }
                if self.assignable(&left, &Type::Number) && self.assignable(&right, &Type::Number) {
                    Ok(Type::Number)
                } else {
                    Err(self
                        .err(format!(
                            "cannot apply `{}` to `{}` and `{}`",
                            bin.operator.symbol(),
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
            BinaryOperator::BitAnd
            | BinaryOperator::BitOr
            | BinaryOperator::Xor
            | BinaryOperator::Shl
            | BinaryOperator::Shr => {
                if let Some(t) = self.fixed_bit_op(bin, &left, &right, false)? {
                    return Ok(t);
                }
                if self.assignable(&left, &Type::Number) && self.assignable(&right, &Type::Number) {
                    Ok(Type::Number)
                } else {
                    Err(self
                        .err(format!(
                            "cannot apply `{}` to `{}` and `{}`",
                            bin.operator.symbol(),
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
            BinaryOperator::Eq | BinaryOperator::Neq => {
                if let Some(t) = self.fixed_bit_op(bin, &left, &right, true)? {
                    return Ok(t);
                }
                let comparable = self.assignable(&left, &right) || self.assignable(&right, &left);
                if comparable {
                    Ok(Type::Boolean)
                } else {
                    Err(self
                        .err(format!(
                            "cannot compare `{}` with `{}`",
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
            BinaryOperator::Lt | BinaryOperator::Gt | BinaryOperator::Lte | BinaryOperator::Gte => {
                if let Some(t) = self.fixed_bit_op(bin, &left, &right, true)? {
                    return Ok(t);
                }
                // The only operators on `Ptr` are `==` and `!=` (spec
                // ffi.md): ordering never applies to an opaque handle.
                let l = self.resolve_alias(&left, 0);
                let r = self.resolve_alias(&right, 0);
                if is_ptr_type(&l) || is_ptr_type(&r) {
                    return Err(self
                        .err(format!(
                            "cannot order `{}` with `{}`: the only operators on `Ptr` are `==` and `!=`",
                            l.name(),
                            r.name()
                        ))
                        .at(bin.span.clone()));
                }
                let comparable = self.assignable(&left, &right) && self.assignable(&right, &left);
                if comparable {
                    Ok(Type::Boolean)
                } else {
                    Err(self
                        .err(format!(
                            "cannot compare `{}` with `{}`",
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
            BinaryOperator::And | BinaryOperator::Or => {
                if self.assignable(&left, &Type::Boolean) && self.assignable(&right, &Type::Boolean)
                {
                    Ok(Type::Boolean)
                } else {
                    Err(self
                        .err(format!(
                            "cannot apply `{}` to `{}` and `{}`",
                            bin.operator.symbol(),
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
        }
    }

    fn is_stringish(&self, ty: &Type) -> bool {
        matches!(ty, Type::String | Type::Literal(_))
    }

    fn join_list(&self, l: &Type, r: &Type) -> Type {
        let (Type::List(a), Type::List(b)) = (l, r) else {
            return Type::Any;
        };
        if self.assignable(a, b) {
            (**b).clone()
        } else if self.assignable(b, a) {
            (**a).clone()
        } else {
            Type::Any
        }
    }

    /// `RegExp::compile(pattern, flags)` — the regex module's only static
    /// member (spec `regular-expressions.mdx`): one or two string arguments,
    /// a `Result<RegExp, RegexError>` back. Arity is `E0207`, an
    /// inadmissible argument `E0202` (spec `expressions/calls.md`).
    fn check_regex_static_call(&mut self, call: &Call, head: &str, variant: &str) -> SResult<Type> {
        if (head, variant) != ("RegExp", "compile") {
            return Err(self
                .err(format!("`{head}` has no member `{variant}`"))
                .at(call.span.clone()));
        }
        if call.arguments.iter().any(|a| a.name.is_some()) {
            return Err(self
                .err("`RegExp::compile` takes positional arguments")
                .at(call.span.clone()));
        }
        if !(1..=2).contains(&call.arguments.len()) {
            return Err(self
                .err_code(
                    "E0207",
                    format!("expected 1 or 2 arguments, found {}", call.arguments.len()),
                )
                .at(call.span.clone()));
        }
        for arg in &call.arguments {
            let found = self.check_arg_value(&arg.value)?;
            if !self.assignable(&found, &Type::String)
                && !self.literal_matches(&arg.value, &Type::String)
            {
                return Err(self
                    .err_code(
                        "E0202",
                        format!(
                            "argument to `RegExp::compile` must be `string`, found `{}`",
                            found.name()
                        ),
                    )
                    .at(arg.value.span().clone()));
            }
        }
        Ok(regex_compile_result_type())
    }

    /// Type-check a call to a member of the built-in `Task` namespace.
    /// Returns `Task<T>` (spec signatures), which `await` unwraps to `T`.
    /// `Math::max(a, b)` / `Time::sleep(ms)`: check a call to an intrinsic
    /// namespace function against its registered signature (spec
    /// `builtins/intrinsic-functions.md`). Arity is `E0207` and an
    /// inadmissible argument `E0202` (spec `expressions/calls.md`).
    fn check_builtin_fn_call(&mut self, ns: &str, member: &str, call: &Call) -> SResult<Type> {
        // Namespace members of the intrinsic tables declare no type
        // parameters; the `Head::member` dispatch already guards this, kept
        // here so any future entry point is covered too.
        self.no_type_args(call)?;
        let registered = builtin_member(ns, member);
        let sig = match &registered {
            Some(BuiltinMember::Fn(sig)) => sig,
            Some(BuiltinMember::Constant) => {
                return Err(self
                    .err(format!("member `{ns}::{member}` is not a function"))
                    .at(call.span.clone()));
            }
            None => {
                return Err(self
                    .err(format!("`{ns}` has no member `{member}`"))
                    .at(call.span.clone()));
            }
        };
        if call.arguments.len() != sig.params.len() {
            let expected = sig.params.len();
            return Err(self
                .err_code(
                    "E0207",
                    format!(
                        "expected {} argument{}, found {}",
                        expected,
                        if expected == 1 { "" } else { "s" },
                        call.arguments.len()
                    ),
                )
                .at(call.span.clone()));
        }
        for (arg, param) in call.arguments.iter().zip(sig.params.iter()) {
            let found = self.check_arg_value(&arg.value)?;
            let expected = param.ty();
            if !self.assignable(&found, &expected) && !self.literal_matches(&arg.value, &expected) {
                return Err(self
                    .err_code(
                        "E0202",
                        format!(
                            "argument to `{ns}::{member}` must be `{}`, found `{}`",
                            param.display(),
                            found.name()
                        ),
                    )
                    .at(arg.value.span().clone()));
            }
        }
        Ok(sig.ret.ty())
    }

    /// A qualified call inside the built-in `std` tree: `rest` is the callee
    /// with the `std::` prefix removed (`math::floor`, `regex::RegExp::compile`).
    fn check_std_call(&mut self, rest: &str, call: &Call) -> SResult<Type> {
        if rest.starts_with("regex") && rest.split("::").nth(1).is_some() {
            // Any path under `std::regex` names a type reached through its
            // imported name, never a static call.
            return Err(self
                .err(
                    "`std::regex` types are reached through their imported name: use `use std::regex::{RegExp, RegexError}` and call `RegExp::compile(…)`",
                )
                .at(call.span.clone()));
        }
        let Some((module, member)) = rest.split_once("::") else {
            return Err(self
                .err_code(
                    "E0607",
                    format!("`std::{rest}` is a namespace: call one of its members, e.g. `std::math::floor(…)`"),
                )
                .at(call.span.clone()));
        };
        if member.contains("::") {
            return Err(self
                .err_code("E0607", format!("cannot resolve `std::{rest}`"))
                .at(call.span.clone()));
        }
        if !matches!(module, "math" | "time" | "fs" | "io" | "os") {
            return Err(self
                .err_code("E0607", format!("cannot resolve `std::{module}`"))
                .at(call.span.clone()));
        }
        self.check_builtin_fn_call(&format!("std::{module}"), member, call)
    }

    fn check_task_utility(&mut self, call: &Call, method: &str) -> SResult<Type> {
        self.no_type_args(call)?;
        match method {
            "resolve" => {
                if call.arguments.len() != 1 {
                    return Err(self
                        .err(format!(
                            "`Task::resolve` expects exactly one argument, got {}",
                            call.arguments.len()
                        ))
                        .at(call.span.clone()));
                }
                let inner = self.check_expression(&call.arguments[0].value)?;
                Ok(Type::Named("Task".to_string(), vec![inner]))
            }
            "all" | "race" => {
                if call.arguments.len() != 1 {
                    return Err(self
                        .err(format!(
                            "`Task::{method}` expects exactly one argument (a list of tasks), got {}",
                            call.arguments.len()
                        ))
                        .at(call.span.clone()));
                }
                let arg_span = call.arguments[0].value.span().clone();
                let arg = self.check_expression(&call.arguments[0].value)?;
                let elem = match arg {
                    Type::List(inner) => match *inner {
                        // `Async(T)` is an in-flight async call; `Task<T>` a
                        // spawn or combinator result. Both await to `T`.
                        Type::Async(inner) => *inner,
                        Type::Named(ref name, ref args) if name == "Task" => {
                            args.first().cloned().unwrap_or(Type::Any)
                        }
                        Type::Any => Type::Any,
                        other => other,
                    },
                    Type::Any => Type::Any,
                    other => {
                        return Err(self
                            .err(format!(
                                "`Task::{method}` expects a list of tasks, found `{}`",
                                other.name()
                            ))
                            .at(arg_span));
                    }
                };
                let combined = if method == "all" {
                    Type::List(Box::new(elem))
                } else {
                    elem
                };
                Ok(Type::Named("Task".to_string(), vec![combined]))
            }
            "timeout" => {
                if call.arguments.len() != 2 {
                    return Err(self
                        .err(format!(
                            "`Task::timeout` expects exactly two arguments (a task and a millisecond count), got {}",
                            call.arguments.len()
                        ))
                        .at(call.span.clone()));
                }
                let task_span = call.arguments[0].value.span().clone();
                let task = self.check_expression(&call.arguments[0].value)?;
                // `timeout` wraps one task: an in-flight async call (`Async`)
                // or any task result (`Task<T>`), unwrapped the same way
                // `await` unwraps it.
                let inner = match task {
                    Type::Async(inner) => *inner,
                    Type::Named(ref name, ref args) if name == "Task" => {
                        args.first().cloned().unwrap_or(Type::Any)
                    }
                    Type::Any => Type::Any,
                    other => {
                        return Err(self
                            .err(format!(
                                "`Task::timeout` expects a task, found `{}`",
                                other.name()
                            ))
                            .at(task_span));
                    }
                };
                let ms_span = call.arguments[1].value.span().clone();
                let ms = self.check_expression(&call.arguments[1].value)?;
                if !self.assignable(&ms, &Type::Number) {
                    return Err(self
                        .err(format!(
                            "`Task::timeout` millisecond count must be a `number`, found `{}`",
                            ms.name()
                        ))
                        .at(ms_span));
                }
                // The combined task carries the outcome as data: `Ok` of the
                // wrapped result, `Err(Error("timed out"))` past the deadline
                // (spec `error-handling.md`).
                Ok(Type::Named(
                    "Task".to_string(),
                    vec![Type::Named(
                        "Result".to_string(),
                        vec![inner, Type::Named("Error".to_string(), Vec::new())],
                    )],
                ))
            }
            other => Err(self
                .err(format!("`Task` has no member `{other}`"))
                .at(call.span.clone())),
        }
    }

    fn check_call(&mut self, call: &xulo_core::ast::Call) -> SResult<Type> {
        if call.is_enum() {
            // The built-in `std` tree splits at the last `::` of its module
            // path (`std::math::floor`), so the first-`::` enum split below
            // cannot see it — dispatch on the raw callee first.
            if let Some(rest) = call.callee.strip_prefix("std::") {
                self.no_type_args(call)?;
                return self.check_std_call(rest, call);
            }
            let (enum_name, variant) = call.enum_parts().unwrap();
            // Trait dispatch, built-in namespace members, and enum
            // construction alike declare no type parameters, so a written
            // list anywhere on a `Head::member` call is `E0807`.
            self.no_type_args(call)?;
            // `Area::area(recv)` is a trait dispatch call when `Area` is a
            // registered trait; otherwise it is an enum construction.
            if let Some(entry) = self.type_entry(enum_name)
                && matches!(entry.kind, TypeEntryKind::Trait(_))
            {
                if call.arguments.iter().any(|a| a.name.is_some()) {
                    return Err(self.err(format!(
                        "trait dispatch `{enum_name}::{variant}` takes positional arguments: the receiver is the first argument, and the method's parameters follow in the trait declaration's order"
                    )));
                }
                let args: Vec<&Expression> = call.arguments.iter().map(|a| &a.value).collect();
                let (ret, mangled) =
                    self.check_trait_call(enum_name.to_string(), variant, &args)?;
                self.trait_dispatch.push((call.span.clone(), mangled));
                return Ok(ret);
            }
            // Built-in namespaces (`Task::all`, `std::math::max`,
            // `std::time::sleep`, `std::fs::read`) are not enums: their heads
            // are the intrinsic tables of spec `builtins/intrinsic-functions.md`.
            // A program-declared type of the same name shadows the
            // namespace, as elsewhere.
            if self.type_entry(enum_name).is_none() {
                if enum_name == "Task" {
                    return self.check_task_utility(call, variant);
                }
                if let Some(ns) = std_ns_key(enum_name) {
                    return self.check_builtin_fn_call(ns, variant, call);
                }
                if matches!(enum_name, "Math" | "Time" | "Fs" | "Io" | "Os") {
                    return Err(self
                        .err(format!(
                            "`{enum_name}` lives under `std`: use `std::{0}::{variant}` or import it with `use std::{0}::*`",
                            enum_name.to_lowercase()
                        ))
                        .at(call.span.clone()));
                }
            }
            // A module or package path (`form::validate(…)`,
            // `lsp-server::run()`): the head resolves as an ordinary name
            // (use-alias or `mod` binding), then a manifest package, then a
            // root module (spec names.md). The member is a value of that
            // module's interface.
            if let Some(ret) = self.check_module_path_call(call, enum_name, variant)? {
                return Ok(ret);
            }
            // The `regex` module's types reached through their imported name
            // (`RegExp::compile(pattern, flags)`): `regex_names` records what
            // `use regex::…` bound, so a program-declared type of the same
            // name — or a module that resolved above — shadows the builtin.
            if self.regex_names.contains(enum_name) {
                return self.check_regex_static_call(call, enum_name, variant);
            }
            // The same heads without their import: name the fix instead of
            // reporting an unknown enum.
            if matches!(enum_name, "RegExp" | "RegexError" | "RegexMatch")
                && self.type_entry(enum_name).is_none()
            {
                return Err(self
                    .err(format!(
                        "`{enum_name}` belongs to the built-in `std::regex` module: import it with `use std::regex::{{{enum_name}}}` or `use std::regex::{{RegExp, RegexError}}`"
                    ))
                    .at(call.span.clone()));
            }
            if enum_name.starts_with("std::regex::") {
                return Err(self
                    .err(
                        "`std::regex` types are reached through their imported name: use `use std::regex::{RegExp, RegexError}` and call `RegExp::compile(…)`",
                    )
                    .at(call.span.clone()));
            }
            let args: Vec<&Expression> = call.arguments.iter().map(|a| &a.value).collect();
            return self.check_enum_call(enum_name.to_string(), variant.to_string(), &args);
        }
        if let Some(object) = &call.object {
            // Method call `obj.method(args)`.
            let receiver = self.check_expression(object)?;
            // No method declared in this language is generic — impl methods,
            // trait methods, bound methods, and loose member signatures all
            // take no type parameters — so a written list is `E0807` as soon
            // as the receiver itself checks.
            self.no_type_args(call)?;
            // A type parameter's method members are granted by its bound
            // (spec `types/generics.md`): the bound's declared signature types
            // the call; without a bound — or with a method the bound does not
            // declare — the member does not exist (E0102, spec
            // `types/traits.md`). The raw receiver type is checked before
            // `resolve_alias` flattens `T` to its shape or to `any`.
            if let Type::Named(name, _) = &receiver
                && self.generics.contains(name)
            {
                let method = call.method.as_deref().unwrap_or(&call.callee);
                if let Some(Type::ObjectType(fields)) = self.bound_shape_of(name)
                    && let Some(Type::FnSig { ret, .. }) =
                        fields.iter().find(|(n, _)| n == method).map(|(_, t)| t)
                {
                    for arg in &call.arguments {
                        self.check_arg_value(&arg.value)?;
                    }
                    return Ok(ret.clone().map(|r| *r).unwrap_or(Type::Any));
                }
                return Err(self.err_code("E0102", format!("no member `{method}` on `{name}`")));
            }
            for arg in &call.arguments {
                self.check_arg_value(&arg.value)?;
            }
            // A structural object member with a function type (`send` on a
            // service handle) yields its declared return type; otherwise the
            // call stays `any`.
            if let Some(method) = &call.method
                && let Some(Type::FnSig { ret, .. }) = self
                    .member_field_type(&self.resolve_alias(&receiver, 0), method)
                    .ok()
                    .filter(|t| matches!(t, Type::FnSig { .. }))
            {
                return Ok(ret.clone().map(|r| *r).unwrap_or(Type::Any));
            }
            // Member-call resolution for a user-declared struct/enum
            // receiver (spec `types/traits.md`, "Member calls"): inherent
            // `impl` methods first, then trait `impl` methods applicable to
            // the receiver; none found is E0102, several trait candidates
            // are E0809. Field members were handled above; builtin heads
            // (`Map`, `Task`, …) have no type entry and keep the loose
            // `any` this call always had.
            if let Some(method) = &call.method {
                let resolved = self.resolve_alias(&receiver, 0);
                if let Type::Named(recv_name, _) = &resolved
                    && matches!(
                        self.type_entry(recv_name).map(|e| &e.kind),
                        Some(TypeEntryKind::Struct(_)) | Some(TypeEntryKind::Enum(_))
                    )
                {
                    // A declared field that is not callable keeps the loose
                    // `any`: the field exists, so E0102 would be a lie.
                    let is_field = matches!(
                        self.type_entry(recv_name).map(|e| &e.kind),
                        Some(TypeEntryKind::Struct(fields))
                            if fields.iter().any(|(n, _, _)| n == method)
                    );
                    if is_field {
                        return Ok(Type::Any);
                    }
                    // Every name the receiver answers under: the name it was
                    // written as, each alias hop, and the resolved name —
                    // an `impl` is registered under the name it was written
                    // for.
                    let mut names: Vec<String> = Vec::new();
                    if let Type::Named(orig, _) = &receiver {
                        names.push(orig.clone());
                    }
                    let mut target = self.alias_target(&receiver);
                    let mut hops = 0;
                    while let Some(t) = target {
                        if hops > 32 || names.contains(&t) {
                            break;
                        }
                        hops += 1;
                        let next = self.alias_target(&Type::Named(t.clone(), Vec::new()));
                        names.push(t);
                        target = next;
                    }
                    if !names.contains(recv_name) {
                        names.push(recv_name.clone());
                    }
                    // Step 1: inherent impl methods of the receiver's type.
                    for name in &names {
                        for record in self
                            .declared_impls
                            .iter()
                            .filter(|r| r.trait_name.is_none() && r.type_name == *name)
                        {
                            let Some(bindings) = self.impl_applies(record, name, &resolved) else {
                                continue;
                            };
                            if let Some(m) = record.methods.iter().find(|m| &m.name == method) {
                                return Ok(substitute_type(&m.ret, &bindings));
                            }
                        }
                    }
                    // Step 2: methods of trait impls applicable to the
                    // receiver; exactly one candidate may result.
                    let mut candidates: Vec<Type> = Vec::new();
                    for name in &names {
                        for record in self
                            .declared_impls
                            .iter()
                            .filter(|r| r.trait_name.is_some() && r.type_name == *name)
                        {
                            let Some(bindings) = self.impl_applies(record, name, &resolved) else {
                                continue;
                            };
                            if let Some(m) = record.methods.iter().find(|m| &m.name == method) {
                                candidates.push(substitute_type(&m.ret, &bindings));
                            }
                        }
                    }
                    match candidates.len() {
                        1 => return Ok(candidates.pop().unwrap()),
                        0 => {
                            return Err(self.err_code(
                                "E0102",
                                format!("no member `{method}` on `{}`", resolved.display()),
                            ));
                        }
                        _ => {
                            return Err(self.err_code(
                                "E0809",
                                format!(
                                    "multiple applicable implementations of `{method}` for `{}`",
                                    resolved.display()
                                ),
                            ));
                        }
                    }
                }
            }
            // Infer return type for known list methods
            if let Type::List(inner) = &receiver
                && let Some(method) = &call.method
            {
                return Ok(match method.as_str() {
                    "length" | "capacity" | "indexOf" => Type::Number,
                    "isEmpty" => Type::Boolean,
                    "first" | "last" | "get" => Type::Optional(inner.clone()),
                    "pop" | "remove" => *inner.clone(),
                    "push" | "set" | "insert" | "reverse" | "sort" | "shrink" | "clear" => {
                        receiver.clone()
                    }
                    "removeValue" | "contains" | "some" | "every" => Type::Boolean,
                    "filter" | "slice" | "concat" | "map" | "flat" => Type::List(inner.clone()),
                    "join" => Type::String,
                    "reduce" | "find" | "forEach" => Type::Any,
                    _ => Type::Any,
                });
            }
            // Infer the return type of the known string methods (spec
            // `regular-expressions.mdx`): the two that consume a pattern or
            // reshape the text both yield a new string.
            if matches!(receiver, Type::String)
                && let Some(method) = &call.method
            {
                return Ok(match method.as_str() {
                    "replace" | "to_uppercase" => Type::String,
                    _ => Type::Any,
                });
            }
            return Ok(Type::Any);
        }
        // A user-declared `str`/`print` shadows the builtin of the same name
        // (the builtins only apply when no user symbol matches).
        if let Some(sym) = self.table.lookup(&call.callee).cloned() {
            // A std intrinsic imported bare (`use std::math::*` → `floor(…)`):
            // type it from the builtin table so its diagnostics match the
            // qualified `std::math::floor(…)` form.
            if let Some((ns, member)) = self.std_fns.get(&call.callee).cloned() {
                // The call reads the `use` binding it came from (W0102), and
                // the IR backend rewrites by the whole call span — see
                // [`apply_std_calls`], which matches `Call.span`.
                self.mark_name_used(&call.callee);
                self.std_calls
                    .push((call.span.clone(), format!("{ns}::{member}")));
                return self.check_builtin_fn_call(&ns, &member, call);
            }
            self.record_call_callee(call, &sym);
            return self.check_call_symbol(call, &sym);
        }
        if call.callee == "reply" {
            self.no_type_args(call)?;
            // `reply(e)` settles the current message's send task; it exists
            // only while checking an `on_message` body (spec concurrency.md).
            if !self.in_on_message {
                return Err(self
                    .err("`reply` may only be called inside `on_message`")
                    .at(call.span.clone()));
            }
            if call.arguments.len() != 1 {
                return Err(self
                    .err(format!(
                        "`reply` expects exactly one argument, got {}",
                        call.arguments.len()
                    ))
                    .at(call.span.clone()));
            }
            let operand = self.check_arg_value(&call.arguments[0].value)?;
            self.reply_operands.push(operand);
            return Ok(Type::Any);
        }
        if call.callee == "print" || call.callee == "println" || call.callee == "repl_echo" {
            self.no_type_args(call)?;
            for arg in &call.arguments {
                self.check_arg_value(&arg.value)?;
            }
            return Ok(Type::Any);
        }
        if call.callee == "str" {
            self.no_type_args(call)?;
            if call.arguments.len() != 1 {
                return Err(self
                    .err(format!(
                        "`str` expects exactly one argument, got {}",
                        call.arguments.len()
                    ))
                    .at(call.span.clone()));
            }
            let operand = self.check_expression(&call.arguments[0].value)?;
            // `str(v)` and `${v}` accept exactly the same operands: a base
            // type or a value whose type implements `ToString`; anything
            // else is `E0212` (spec `builtins/intrinsic-functions.md`,
            // `expressions/literals.md`).
            if !self.type_has_string_form(&operand) {
                return Err(self
                    .err_code(
                        "E0212",
                        format!(
                            "`{}` has no string form; implement `ToString`",
                            operand.display()
                        ),
                    )
                    .at(call.arguments[0].value.span().clone()));
            }
            // A `ToString` implementor renders through `toString`: the call
            // is annotated for dispatch exactly like `Trait::method(recv)`.
            if let Type::Named(name, _) = &operand
                && self.string_forms.contains(name)
            {
                self.trait_dispatch.push((
                    call.span.clone(),
                    impl_fn_name("ToString", name, "toString"),
                ));
            }
            return Ok(Type::String);
        }
        // `Error(message)` constructs the built-in base error type (spec
        // `error-handling.md`): one `string` argument, typed `Error`. Like
        // `print`/`str`, a user-declared `fn Error` shadows it (the lookup
        // above runs first).
        if call.callee == "Error" {
            self.no_type_args(call)?;
            if call.arguments.len() != 1 {
                return Err(self
                    .err(format!(
                        "`Error` expects exactly one argument (a message), got {}",
                        call.arguments.len()
                    ))
                    .at(call.span.clone()));
            }
            let message = self.check_expression(&call.arguments[0].value)?;
            if !self.assignable(&message, &Type::String) {
                return Err(self
                    .err(format!(
                        "`Error` message must be a `string`, found `{}`",
                        message.name()
                    ))
                    .at(call.arguments[0].value.span().clone()));
            }
            return Ok(Type::Named("Error".to_string(), Vec::new()));
        }
        if call.callee == "assert" {
            self.no_type_args(call)?;
            if call.arguments.len() != 2 {
                return Err(self
                    .err(format!(
                        "`assert` expects exactly two arguments (condition, message), got {}",
                        call.arguments.len()
                    ))
                    .at(call.span.clone()));
            }
            let cond = self.check_expression(&call.arguments[0].value)?;
            if !self.assignable(&cond, &Type::Boolean) {
                return Err(self
                    .err(format!(
                        "`assert` condition must be a `bool`, found `{}`",
                        cond.name()
                    ))
                    .at(call.arguments[0].value.span().clone()));
            }
            let message = self.check_expression(&call.arguments[1].value)?;
            if !self.assignable(&message, &Type::String) {
                return Err(self
                    .err(format!(
                        "`assert` message must be a `string`, found `{}`",
                        message.name()
                    ))
                    .at(call.arguments[1].value.span().clone()));
            }
            return Ok(Type::Any);
        }
        if self.test_names.contains(&call.callee) {
            return Err(self
                .err_code(
                    "E0107",
                    format!(
                        "`{}` is a test function and cannot be referenced",
                        call.callee
                    ),
                )
                .at(call.span.clone()));
        }
        let Some(sym) = self.table.lookup(&call.callee).cloned() else {
            return Err(self.err(format!("unknown function `{}`", call.callee)));
        };
        self.record_call_callee(call, &sym);
        self.check_call_symbol(call, &sym)
    }

    /// Shared call validation against a resolved symbol: effect-scope guard,
    /// then per-symbol-kind argument/arity/type checks.
    fn check_call_symbol(&mut self, call: &Call, sym: &Symbol) -> SResult<Type> {
        if self.in_effect
            && self
                .render_locals
                .last()
                .is_some_and(|locals| locals.contains(&call.callee))
        {
            return Err(self.err(format!(
                "`@Effect` closures cannot reference `{}`: it is declared inside the component body and is out of scope when the effect runs",
                call.callee
            )));
        }
        match &sym.kind {
            SymbolKind::Function(type_params, params, return_type, callee_bounds) => {
                // An unseeded import (no module graph) is opaque: accept any
                // argument list and return `any`. Local functions never reach
                // here, even when un-annotated (zero params, no return type).
                if self.opaque.contains(&call.callee) {
                    for arg in &call.arguments {
                        self.check_arg_value(&arg.value)?;
                    }
                    return Ok(Type::Any);
                }
                let named = call
                    .arguments
                    .iter()
                    .filter(|a| a.name.is_some())
                    .collect::<Vec<_>>();
                let positional = call
                    .arguments
                    .iter()
                    .filter(|a| a.name.is_none())
                    .collect::<Vec<_>>();
                let all_named = named.len() == call.arguments.len() && !call.arguments.is_empty();
                let type_params = type_params.clone();
                let params = params.clone();
                let return_type = return_type.clone();
                let callee_bounds = callee_bounds.clone();
                // A written type-argument list replaces inference for this
                // call; arity (`E0808`) and the non-generic case (`E0807`)
                // are settled before any call-site state is pushed.
                let written = self.written_type_args(call, &type_params)?;
                self.generics.extend(type_params.iter().cloned());
                // Call-site bounds: the callee's `T: Trait` clauses constrain
                // its argument checks the same way they do inside its body.
                let bound_map = match callee_bounds.iter().try_fold(
                    HashMap::new(),
                    |mut map, bound| -> SResult<HashMap<String, Type>> {
                        let shape = self.bound_shape(bound)?;
                        map.insert(bound.param.clone(), shape);
                        Ok(map)
                    },
                ) {
                    Ok(map) => map,
                    Err(e) => {
                        self.generics
                            .truncate(self.generics.len().saturating_sub(type_params.len()));
                        return Err(e);
                    }
                };
                self.bound_shapes.push(bound_map);
                let mut trait_map: HashMap<String, Vec<String>> = HashMap::new();
                for bound in &callee_bounds {
                    trait_map
                        .entry(bound.param.clone())
                        .or_default()
                        .extend(bound.traits.iter().cloned());
                }
                self.bound_traits.push(trait_map);
                let result = (|| {
                    if all_named {
                        // Named arguments must cover every parameter (defaults
                        // may be omitted) exactly once.
                        let mut seen = std::collections::HashSet::new();
                        let mut borrow_args: Vec<(&Expression, Option<xulo_core::ast::ParamMode>)> =
                            Vec::with_capacity(named.len());
                        for arg in &named {
                            let name = arg.name.as_ref().unwrap();
                            let Some(param) = params.iter().find(|p| &p.name == name) else {
                                return Err(self.err(format!(
                                    "function `{}` has no parameter `{name}`",
                                    call.callee
                                )));
                            };
                            if !seen.insert(name.clone()) {
                                return Err(self.err(format!(
                                    "argument `{name}` to `{}` is provided twice",
                                    call.callee
                                )));
                            }
                            let arg_type = self.check_arg_value(&arg.value)?;
                            // The structural check does not re-run the bound
                            // rule: a parameter's bound is what decides
                            // admissibility, and it is checked once per
                            // binding below. Written type arguments substitute
                            // into the annotation first (spec `types/generics.md`);
                            // otherwise the annotation is erased.
                            let ann = param.type_annotation.as_ref().unwrap_or(&Type::Any);
                            let expected = match &written {
                                Some(w) => substitute_type(ann, w),
                                None => self.erase_generic_params(ann, &type_params),
                            };
                            if !self.assignable(&arg_type, &expected)
                                && !self.literal_matches(&arg.value, &expected)
                            {
                                // An out-of-range numeric argument literal is
                                // E0209 (spec `type-system/coercion.md`),
                                // ahead of the argument mismatch.
                                if let Some(e) = self.fixed_literal_error(
                                    &arg.value,
                                    &expected,
                                    arg.value.span().clone(),
                                ) {
                                    return Err(e);
                                }
                                return Err(if written.is_some() {
                                    self.err_code(
                                        "E0202",
                                        format!(
                                            "expected `{}`, found `{}`",
                                            expected.name(),
                                            arg_type.name()
                                        ),
                                    )
                                } else {
                                    self.err_code(
                                        "E0202",
                                        format!(
                                            "argument to `{}` must be `{}`, found `{}`",
                                            call.callee,
                                            expected.name(),
                                            arg_type.name()
                                        ),
                                    )
                                });
                            }
                            borrow_args.push((&arg.value, param.mode));
                        }
                        self.check_call_borrows(&borrow_args)?;
                        let required = params
                            .iter()
                            .filter(|p| p.default.is_none() && !param_optional(p))
                            .map(|p| p.name.clone())
                            .collect::<Vec<_>>();
                        let missing = required.iter().find(|name| !seen.contains(*name)).cloned();
                        if let Some(name) = missing {
                            return Err(self.err(format!(
                                "function `{}` is missing required argument `{name}`",
                                call.callee
                            )));
                        }
                        // Call-site inference for generic functions, mirroring
                        // the positional path: bind `T` from the named args
                        // (in parameter order, using defaults when omitted).
                        // A written list suppresses inference entirely (spec:
                        // no unification against the value arguments).
                        let mut resolved = return_type;
                        if let Some(w) = &written {
                            self.check_inferred_bounds(&callee_bounds, w)?;
                            resolved = substitute_type(&resolved, w);
                        } else if !type_params.is_empty() && !params.is_empty() {
                            let mut positional_types = Vec::with_capacity(params.len());
                            let mut unbounded = false;
                            for param in &params {
                                let arg = call
                                    .arguments
                                    .iter()
                                    .find(|a| a.name.as_ref() == Some(&param.name));
                                if let Some(arg) = arg {
                                    let arg_type = self.check_arg_value(&arg.value)?;
                                    positional_types.push(arg_type);
                                } else if let Some(default) = &param.default {
                                    let default_type = self.check_expression(default)?;
                                    positional_types.push(default_type);
                                } else {
                                    // A required parameter is absent; the call
                                    // is already rejected by the arity check
                                    // that ran above, so skip inference.
                                    unbounded = true;
                                    break;
                                }
                            }
                            if !unbounded {
                                let param_types = params
                                    .iter()
                                    .map(|p| p.type_annotation.clone().unwrap_or(Type::Any))
                                    .collect::<Vec<_>>();
                                let bindings = match infer_type_bindings(
                                    &type_params,
                                    &param_types,
                                    &positional_types,
                                ) {
                                    Ok(b) => b,
                                    Err((param, existing, found)) => {
                                        return Err(self.err_code(
                                            "E0202",
                                            format!(
                                                "expected `{}` for `{param}`, found `{}`",
                                                existing.name(),
                                                found.name()
                                            ),
                                        ));
                                    }
                                };
                                self.check_inferred_bounds(&callee_bounds, &bindings)?;
                                resolved = substitute_type(&resolved, &bindings);
                            }
                        }
                        Ok(resolved)
                    } else {
                        if !named.is_empty() {
                            // Codegen emits argument lists in source order for
                            // non-all-named calls, so a positional/named mix
                            // would silently land in the wrong parameter slot.
                            return Err(self.err(format!(
                                "call to `{}` cannot mix positional and named arguments",
                                call.callee
                            )));
                        }
                        let expected = params.len();
                        let required = params
                            .iter()
                            .filter(|p| p.default.is_none() && !param_optional(p))
                            .count();
                        let actual = positional.len();
                        if actual < required || actual > expected {
                            let range = if required == expected {
                                format!("{expected}")
                            } else {
                                format!("{required} to {expected}")
                            };
                            return Err(self.err(format!(
                                "function `{}` expects {range} argument(s), but {actual} were provided",
                                call.callee
                            )));
                        }
                        let mut positional_types = Vec::with_capacity(positional.len());
                        let mut borrow_args: Vec<(&Expression, Option<xulo_core::ast::ParamMode>)> =
                            Vec::with_capacity(positional.len());
                        for (arg, param) in positional.iter().zip(params.iter()) {
                            let arg_type = self.check_arg_value(&arg.value)?;
                            positional_types.push(arg_type.clone());
                            // The structural check does not re-run the bound
                            // rule; it is checked once per binding below.
                            // Written type arguments substitute into the
                            // annotation first, so `first<int>(["a"])` fails
                            // with the substituted `List<number>` (spec
                            // `types/generics.md`); otherwise it is erased.
                            let ann = param.type_annotation.as_ref().unwrap_or(&Type::Any);
                            let expected = match &written {
                                Some(w) => substitute_type(ann, w),
                                None => self.erase_generic_params(ann, &type_params),
                            };
                            if !self.assignable(&arg_type, &expected)
                                && !self.literal_matches(&arg.value, &expected)
                            {
                                // An out-of-range numeric argument literal is
                                // E0209 (spec `type-system/coercion.md`),
                                // ahead of the argument mismatch.
                                if let Some(e) = self.fixed_literal_error(
                                    &arg.value,
                                    &expected,
                                    arg.value.span().clone(),
                                ) {
                                    return Err(e);
                                }
                                return Err(if written.is_some() {
                                    self.err_code(
                                        "E0202",
                                        format!(
                                            "expected `{}`, found `{}`",
                                            expected.name(),
                                            arg_type.name()
                                        ),
                                    )
                                } else {
                                    self.err_code(
                                        "E0202",
                                        format!(
                                            "argument to `{}` must be `{}`, found `{}`",
                                            call.callee,
                                            expected.name(),
                                            arg_type.name()
                                        ),
                                    )
                                });
                            }
                            borrow_args.push((&arg.value, param.mode));
                        }
                        self.check_call_borrows(&borrow_args)?;
                        // Call-site inference for generic functions
                        // (`first([1, 2])` binds `T = number`); a written
                        // list takes the place of inference for the call.
                        let mut resolved = return_type;
                        if let Some(w) = &written {
                            self.check_inferred_bounds(&callee_bounds, w)?;
                            resolved = substitute_type(&resolved, w);
                        } else if !type_params.is_empty() {
                            let param_types = params
                                .iter()
                                .map(|p| p.type_annotation.clone().unwrap_or(Type::Any))
                                .collect::<Vec<_>>();
                            let bindings = match infer_type_bindings(
                                &type_params,
                                &param_types,
                                &positional_types,
                            ) {
                                Ok(b) => b,
                                Err((param, existing, found)) => {
                                    return Err(self.err_code(
                                        "E0202",
                                        format!(
                                            "expected `{}` for `{param}`, found `{}`",
                                            existing.name(),
                                            found.name()
                                        ),
                                    ));
                                }
                            };
                            self.check_inferred_bounds(&callee_bounds, &bindings)?;
                            resolved = substitute_type(&resolved, &bindings);
                        }
                        Ok(resolved)
                    }
                })();
                self.bound_shapes.pop();
                self.bound_traits.pop();
                self.generics
                    .truncate(self.generics.len().saturating_sub(type_params.len()));
                result
            }
            SymbolKind::Variable => {
                // A function value: `let f = fn() {...}; f(x)`. The type may be
                // a `type Handler = fn(...)` alias, so resolve it first.
                let resolved = self.resolve_alias(&sym.type_, 0);
                // A struct constructor: the name resolves to a nominal struct
                // type and the call supplies its fields as named arguments
                // (spec `types/composite-types.md`: `User(name: "Alice",
                // age: 30)`).
                if let Type::Named(name, _) = &resolved
                    && self
                        .type_entry(name)
                        .is_some_and(|e| matches!(e.kind, TypeEntryKind::Struct(_)))
                {
                    // Constructors declare no type parameters (spec change
                    // `add-explicit-type-arguments`: a struct's type arguments
                    // come from its field initializers) — `E0807`.
                    self.no_type_args(call)?;
                    return self.check_struct_ctor_call(call, name);
                }
                let sig = match &resolved {
                    Type::FnSig { params, ret } => Some((params.clone(), ret.clone())),
                    _ => None,
                };
                let Some((params, ret)) = sig else {
                    return Err(self.err(format!("`{}` is not a function", call.callee)));
                };
                // A call through a function value takes no type arguments.
                self.no_type_args(call)?;
                self.check_fn_value_args(&params, ret, &call.arguments)
            }
            SymbolKind::State | SymbolKind::Store => {
                Err(self.err(format!("`{}` is not a function", call.callee)))
            }
        }
    }

    /// Validate a call against a function-value signature (`Type::FnSig`):
    /// positional arguments only, at most the declared arity, per-parameter
    /// type checks. Fewer arguments than declared are allowed because a
    /// signature erases default parameters (`fn(a, b = a)` types as
    /// `fn(number, number)`) and both runtimes bind missing trailing
    /// parameters to their default (`undefined` in JS, `null`/default in the
    /// native runtime) — exactly like a plain call.
    fn check_fn_value_args(
        &mut self,
        params: &[Type],
        ret: Option<Box<Type>>,
        arguments: &[CallArg],
    ) -> SResult<Type> {
        if arguments.iter().any(|a| a.name.is_some()) {
            return Err(self.err("named arguments are not supported when calling a function value"));
        }
        let expected = params.len();
        let actual = arguments.len();
        if actual > expected {
            return Err(self.err(format!(
                "function values expect at most {expected} argument(s), but {actual} were provided",
            )));
        }
        for (arg, param) in arguments.iter().zip(params.iter()) {
            let arg_type = self.check_arg_value(&arg.value)?;
            if !self.assignable(&arg_type, param) && !self.literal_matches(&arg.value, param) {
                // Out-of-range numeric literals are E0209, ahead of the
                // mismatch (spec `type-system/coercion.md`).
                if let Some(e) =
                    self.fixed_literal_error(&arg.value, param, arg.value.span().clone())
                {
                    return Err(e);
                }
                return Err(self.err(format!(
                    "argument to function value must be `{}`, found `{}`",
                    param.name(),
                    arg_type.name()
                )));
            }
        }
        Ok(ret.map(|r| *r).unwrap_or(Type::Any))
    }

    /// Type-check a struct constructor — `User(name: "Alice", age: 30)`:
    /// named arguments only (spec `types/composite-types.md`), no field the
    /// struct does not declare (`E0206`), each argument given once, every
    /// non-optional field without a default supplied (`E0205`), and every
    /// value fitting its field's type (`E0202`). The call's type is the
    /// struct with its type arguments inferred from the field values.
    fn check_struct_ctor_call(&mut self, call: &Call, name: &str) -> SResult<Type> {
        let entry = self
            .type_entry(name)
            .cloned()
            .ok_or_else(|| self.err(format!("unknown type `{name}`")))?;
        let TypeEntryKind::Struct(declared) = entry.kind else {
            return Err(self.err(format!("`{name}` is not a struct")));
        };
        let type_params = entry.type_params;
        let type_bounds = entry.type_bounds;
        for arg in &call.arguments {
            if arg.name.is_none() {
                return Err(self
                    .err_code(
                        "E0208",
                        format!("struct `{name}` is constructed with named arguments"),
                    )
                    .at(arg.value.span().clone()));
            }
        }
        let mut provided: HashMap<String, (Type, &Expression)> = HashMap::new();
        for arg in &call.arguments {
            let label = arg.name.as_deref().unwrap_or_default();
            if !declared.iter().any(|(fname, _, _)| fname == label) {
                return Err(self
                    .err_code("E0206", format!("`{name}` has no field `{label}`"))
                    .at(arg.value.span().clone()));
            }
            if provided.contains_key(label) {
                return Err(self.err(format!("argument `{label}` to `{name}` is provided twice")));
            }
            let arg_type = self.check_expression(&arg.value)?;
            provided.insert(label.to_string(), (arg_type, &arg.value));
        }
        for (fname, ftype, fdefault) in &declared {
            match provided.get(fname) {
                Some((arg_type, expr)) => {
                    if !self.assignable(arg_type, ftype) && !self.literal_matches(expr, ftype) {
                        return Err(self
                            .err_code(
                                "E0202",
                                format!(
                                    "field `{fname}` of `{name}` expects `{}`, found `{}`",
                                    ftype.name(),
                                    arg_type.name()
                                ),
                            )
                            .at(expr.span().clone()));
                    }
                }
                None => {
                    if fdefault.is_none() && !matches!(ftype, Type::Optional(_)) {
                        return Err(self
                            .err_code(
                                "E0205",
                                format!("missing field `{fname}` in initializer of `{name}`"),
                            )
                            .at(call.span.clone()));
                    }
                }
            }
        }
        let mut type_args = Vec::with_capacity(type_params.len());
        if !type_params.is_empty() {
            let mut param_types = Vec::new();
            let mut arg_types = Vec::new();
            for (fname, ftype, _) in &declared {
                if let Some((arg_type, _)) = provided.get(fname) {
                    param_types.push(ftype.clone());
                    arg_types.push(arg_type.clone());
                }
            }
            let bindings = match infer_type_bindings(&type_params, &param_types, &arg_types) {
                Ok(b) => b,
                Err((param, existing, found)) => {
                    return Err(self.err_code(
                        "E0202",
                        format!(
                            "expected `{}` for `{param}`, found `{}`",
                            existing.name(),
                            found.name()
                        ),
                    ));
                }
            };
            self.check_inferred_bounds(&type_bounds, &bindings)?;
            for p in &type_params {
                type_args.push(bindings.get(p).cloned().unwrap_or(Type::Any));
            }
        }
        Ok(Type::Named(name.to_string(), type_args))
    }

    fn check_enum_ref(&self, enum_name: String, variant: &str) -> SResult<Type> {
        if enum_name == "std" {
            return Err(self.err_code(
                "E0607",
                "`std` is a namespace root: pick a module, e.g. `std::math::PI`",
            ));
        }
        if enum_name == "std::regex" || enum_name.starts_with("std::regex::") {
            return Err(self.err(
                "`std::regex` types are reached through their imported name: use `use std::regex::{RegExp, RegexError}`",
            ));
        }
        // The `regex` module's static member as a value (`let f =
        // RegExp::compile`), reached through the imported name — only what
        // `use regex::…` bound, so a program's own type of the name shadows
        // it like any other declaration.
        if self.regex_names.contains(&enum_name) {
            return match (enum_name.as_str(), variant) {
                ("RegExp", "compile") => Ok(Type::FnSig {
                    params: vec![Type::String, Type::String],
                    ret: Some(Box::new(regex_compile_result_type())),
                }),
                _ => Err(self.err(format!("`{enum_name}` has no member `{variant}`"))),
            };
        }
        // A built-in namespace's members are constants (`Math::PI`) or
        // functions reached by call (`Math::max(...)`); a bare function
        // reference types as a function value (spec
        // `builtins/intrinsic-functions.md`). A program-declared type of the
        // head's name shadows the namespace, as elsewhere.
        if self.type_entry(&enum_name).is_none() {
            if let Some(ns) = std_ns_key(&enum_name) {
                return match builtin_member(ns, variant) {
                    Some(BuiltinMember::Constant) => Ok(Type::Number),
                    Some(BuiltinMember::Fn(sig)) => Ok(Type::FnSig {
                        params: sig.params.iter().map(|p| p.ty()).collect(),
                        ret: Some(Box::new(sig.ret.ty())),
                    }),
                    None => Err(self.err(format!("`{ns}` has no member `{variant}`"))),
                };
            }
            if matches!(enum_name.as_str(), "Math" | "Time" | "Fs" | "Io" | "Os") {
                return Err(self.err(format!(
                    "`{enum_name}` lives under `std`: use `std::{0}::{variant}` or import it with `use std::{0}::*`",
                    enum_name.to_lowercase()
                )));
            }
            if enum_name == "Task" {
                return match task_bare_member_sig(variant) {
                    Some(sig) => Ok(sig),
                    None => Err(self.err(format!("`Task` has no member `{variant}`"))),
                };
            }
        }
        // A module path (`form::CONF`, `form::fields::Point`) resolves
        // before the enum fallback: its head is a namespace binding.
        if let Some(ty) = self.module_path_value_type(&enum_name, variant)? {
            return Ok(ty);
        }
        // The regex module's heads without their import: name the fix
        // instead of reporting an unknown enum.
        if matches!(enum_name.as_str(), "RegExp" | "RegexError" | "RegexMatch")
            && self.type_entry(&enum_name).is_none()
        {
            return Err(self.err(format!(
                "`{enum_name}` belongs to the built-in `std::regex` module: import it with `use std::regex::{{{enum_name}}}` or `use std::regex::{{RegExp, RegexError}}`"
            )));
        }
        let Some(entry) = self.type_entry(&enum_name) else {
            return Err(self.err(format!("unknown enum `{enum_name}`")));
        };
        match &entry.kind {
            TypeEntryKind::Enum(variants) => {
                if !variants.iter().any(|v| v.name == variant) {
                    return Err(self.err(format!("enum `{enum_name}` has no member `{variant}`")));
                }
                Ok(Type::Named(enum_name, Vec::new()))
            }
            TypeEntryKind::Alias(_) | TypeEntryKind::Trait(_) | TypeEntryKind::Struct(_) => {
                Err(self.err(format!("`{enum_name}` is not an enum")))
            }
        }
    }

    /// Resolve `head::...::member` as a module-path call, returning the
    /// member function's return type when the head names a module
    /// (`Ok(None)` when the path is not a module path at all).
    fn check_module_path_call(
        &mut self,
        call: &xulo_core::ast::Call,
        head: &str,
        member: &str,
    ) -> SResult<Option<Type>> {
        let module = match self.namespaces.get(head) {
            Some(id) => id.clone(),
            None => {
                let Some(world) = &self.world else {
                    return Ok(None);
                };
                let pkg_root = ModuleWorld::package_root_of(&self.module_id);
                match world
                    .packages
                    .get(head)
                    .filter(|_| pkg_root.is_empty())
                    .or_else(|| {
                        world
                            .modules
                            .get(pkg_root)
                            .and_then(|f| f.children.get(head))
                    }) {
                    Some(id) => id.clone(),
                    None => return Ok(None),
                }
            }
        };
        self.mark_name_used(head);
        // A std alias/namespace head (`use std::math as m` → `m::floor(…)`):
        // the synthetic modules carry no world face — dispatch on the table.
        if let Some(std_module) = std_module_name(&module) {
            if member.contains("::") || std_module == "regex" {
                return Err(self
                    .err(
                        "`std::regex` types are reached through their imported name: use `use std::regex::{RegExp, RegexError}` and call `RegExp::compile(…)`",
                    )
                    .at(call.span.clone()));
            }
            let ns = format!("std::{std_module}");
            let ty = self.check_builtin_fn_call(&ns, member, call)?;
            // The IR backend only keys the intrinsic table on the qualified
            // `std::…` form: record the rewrite like the bare-name path does
            // so an alias head (`m::floor(…)` / `io::println(…)`) lowers once
            // `apply_std_calls` has run. The interpreter resolves the alias
            // binding itself and never applies the rewrite.
            self.std_calls
                .push((call.span.clone(), format!("{ns}::{member}")));
            return Ok(Some(ty));
        }
        let Some(face) = self
            .world
            .as_ref()
            .and_then(|w| w.modules.get(&module))
            .cloned()
        else {
            if self.lenient {
                // Editor partial mode: no face to consult. Record the path
                // head (its span starts the call) so go-to-definition can
                // follow the import edge, then check the call opaquely like
                // an external dependency would.
                self.resolutions.push(UseRecord {
                    span: call.span.start..call.span.start + head.len(),
                    name: head.to_string(),
                    kind: UseKind::Variable,
                    type_: Some(Type::Any),
                    def: None,
                });
                for arg in &call.arguments {
                    self.check_arg_value(&arg.value)?;
                }
                return Ok(Some(Type::Any));
            }
            return Err(self
                .err_code("E0607", format!("cannot resolve `{head}`"))
                .at(call.span.clone()));
        };
        if face.external {
            // An external dependency's exports are invisible: any member call
            // is checked opaquely (arguments still check, result is `any`).
            self.no_type_args(call)?;
            for arg in &call.arguments {
                self.check_arg_value(&arg.value)?;
            }
            return Ok(Some(Type::Any));
        }
        // Descend child namespaces: a multi-segment member like `m::inner::v`
        // walks `inner` down to the child module's face before `v` is looked
        // up on it — a flat lookup would miss and report E0601.
        let mut module = module;
        let mut member = member.to_string();
        let mut face = face;
        loop {
            if face.names.contains_key(&member) {
                break;
            }
            let Some((first, rest)) = member.split_once("::") else {
                break;
            };
            let Some(child) = face.children.get(first).cloned() else {
                break;
            };
            if module != self.module_id && face.names.get(first).is_some_and(|info| !info.is_pub) {
                return Err(self
                    .err_code(
                        "E0602",
                        format!("`{first}` is private to `{}`", module_display(&module)),
                    )
                    .at(call.span.clone()));
            }
            let Some(next) = self
                .world
                .as_ref()
                .and_then(|w| w.modules.get(&child))
                .cloned()
            else {
                break;
            };
            module = child;
            member = rest.to_string();
            face = next;
        }
        let info = face.names.get(&member).copied();
        let info = match info {
            Some(info) => {
                if module != self.module_id && !info.is_pub {
                    return Err(self
                        .err_code(
                            "E0602",
                            format!("`{member}` is private to `{}`", module_display(&module)),
                        )
                        .at(call.span.clone()));
                }
                info
            }
            None => {
                return Err(self
                    .err_code(
                        "E0601",
                        format!(
                            "module `{}` does not export `{member}`",
                            module_display(&module)
                        ),
                    )
                    .at(call.span.clone()));
            }
        };
        match info.kind {
            FaceKind::Module => Err(self
                .err_code(
                    "E0606",
                    format!("`{head}::{member}` is a module, not a value"),
                )
                .at(call.span.clone())),
            FaceKind::Type => Err(self
                .err_code(
                    "E0606",
                    format!("`{head}::{member}` is a type-only binding and is not a value"),
                )
                .at(call.span.clone())),
            FaceKind::Fn | FaceKind::Let | FaceKind::Value => {
                let symbol = face
                    .exports
                    .iter()
                    .find(|(n, _)| n == &member)
                    .map(|(_, sym)| sym.clone())
                    .unwrap_or(Symbol {
                        name: member.to_string(),
                        type_: Type::Any,
                        kind: SymbolKind::Function(Vec::new(), Vec::new(), Type::Any, Vec::new()),
                        is_shared: false,
                        is_param: false,
                        is_const: true,
                        is_mutable: false,
                    });
                if matches!(info.kind, FaceKind::Fn) {
                    self.no_type_args(call)?;
                    return Ok(Some(self.check_call_symbol(call, &symbol)?));
                }
                let resolved = self.resolve_alias(&symbol.type_, 0);
                let sig = match &resolved {
                    Type::FnSig { params, ret } => Some((params.clone(), ret.clone())),
                    _ => None,
                };
                self.no_type_args(call)?;
                let Some((params, ret)) = sig else {
                    return Err(self.err(format!("`{head}::{member}` is not a function")));
                };
                Ok(Some(self.check_fn_value_args(
                    &params,
                    ret,
                    &call.arguments,
                )?))
            }
        }
    }

    /// Resolve `head::...::member` as a module path in a value position
    /// (`Ok(None)` when the head is not a module namespace).
    fn module_path_value_type(&self, head: &str, member: &str) -> SResult<Option<Type>> {
        let module = match self.namespaces.get(head) {
            Some(id) => id.clone(),
            None => {
                let Some(world) = &self.world else {
                    return Ok(None);
                };
                let pkg_root = ModuleWorld::package_root_of(&self.module_id);
                match world
                    .packages
                    .get(head)
                    .filter(|_| pkg_root.is_empty())
                    .or_else(|| {
                        world
                            .modules
                            .get(pkg_root)
                            .and_then(|f| f.children.get(head))
                    }) {
                    Some(id) => id.clone(),
                    None => return Ok(None),
                }
            }
        };
        // A std alias/namespace head in value position (`m::PI`).
        if let Some(std_module) = std_module_name(&module) {
            if std_module == "regex" || member.contains("::") {
                return Err(self.err(
                    "`std::regex` types are reached through their imported name: use `use std::regex::{RegExp, RegexError}`",
                ));
            }
            let Some(registered) = builtin_member(&format!("std::{std_module}"), member) else {
                return Err(self.err(format!("`std::{std_module}` has no member `{member}`")));
            };
            return Ok(Some(match registered {
                BuiltinMember::Constant => Type::Number,
                BuiltinMember::Fn(sig) => Type::FnSig {
                    params: sig.params.iter().map(|p| p.ty()).collect(),
                    ret: Some(Box::new(sig.ret.ty())),
                },
            }));
        }
        let Some(face) = self
            .world
            .as_ref()
            .and_then(|w| w.modules.get(&module))
            .cloned()
        else {
            if self.lenient {
                // Editor partial mode: the head is a namespace but no face
                // exists to read the member from — read opaquely instead of
                // collapsing the path into an unknown-enum error.
                return Ok(Some(Type::Any));
            }
            return Ok(None);
        };
        if face.external {
            // External dependency: any member reads as `any`.
            return Ok(Some(Type::Any));
        }
        // Descend child namespaces (mirroring `check_module_path_call`):
        // `m::inner::v` walks `inner` to the child module's face before `v`
        // is looked up on it.
        let mut module = module;
        let mut member = member.to_string();
        let mut face = face;
        loop {
            if face.names.contains_key(&member) {
                break;
            }
            let Some((first, rest)) = member.split_once("::") else {
                break;
            };
            let Some(child) = face.children.get(first).cloned() else {
                break;
            };
            if module != self.module_id && face.names.get(first).is_some_and(|info| !info.is_pub) {
                return Err(self.err_code(
                    "E0602",
                    format!("`{first}` is private to `{}`", module_display(&module)),
                ));
            }
            let Some(next) = self
                .world
                .as_ref()
                .and_then(|w| w.modules.get(&child))
                .cloned()
            else {
                break;
            };
            module = child;
            member = rest.to_string();
            face = next;
        }
        let Some(info) = face.names.get(&member).copied() else {
            return Err(self.err_code(
                "E0601",
                format!(
                    "module `{}` does not export `{member}`",
                    module_display(&module)
                ),
            ));
        };
        if module != self.module_id && !info.is_pub {
            return Err(self.err_code(
                "E0602",
                format!("`{member}` is private to `{}`", module_display(&module)),
            ));
        }
        let value = face
            .exports
            .iter()
            .find(|(n, _)| n == &member)
            .map(|(_, sym)| sym.type_.clone());
        match (info.kind, value) {
            (_, Some(ty)) => Ok(Some(ty)),
            (FaceKind::Type, None) => Ok(Some(Type::Named(member.to_string(), Vec::new()))),
            (FaceKind::Module, _) => Err(self.err_code(
                "E0606",
                format!("`{head}::{member}` is a module, not a value"),
            )),
            (_, None) => Ok(Some(Type::Any)),
        }
    }

    fn check_enum_call(
        &mut self,
        enum_name: String,
        variant: String,
        arguments: &[&Expression],
    ) -> SResult<Type> {
        let Some(entry) = self.type_entry(&enum_name) else {
            return Err(self.err(format!("unknown enum `{enum_name}`")));
        };
        let type_params = entry.type_params.clone();
        let TypeEntryKind::Enum(variants) = &entry.kind else {
            return Err(self.err(format!("`{enum_name}` is not an enum")));
        };
        let Some(v) = variants.iter().find(|v| v.name == variant) else {
            return Err(self.err(format!("enum `{enum_name}` has no member `{variant}`")));
        };
        let payload = v.payload.clone();
        self.generics.extend(type_params.clone());
        let result = match payload {
            Some(params) => {
                if arguments.len() != params.len() {
                    Err(self.err(format!(
                        "enum member `{enum_name}::{variant}` expects {} argument(s), got {}",
                        params.len(),
                        arguments.len()
                    )))
                } else {
                    let mut errs = Vec::new();
                    for (arg, param) in arguments.iter().zip(params.iter()) {
                        match self.check_expression(arg) {
                            Ok(arg_type) => {
                                if !self.assignable(&arg_type, &param.type_)
                                    && !self.literal_matches(arg, &param.type_)
                                {
                                    errs.push(self.err(format!(
                                        "argument to `{enum_name}::{variant}` must be `{}`, found `{}`",
                                        param.type_.name(),
                                        arg_type.name()
                                    )));
                                }
                            }
                            Err(e) => errs.push(e),
                        }
                    }
                    if let Some(e) = errs.into_iter().next() {
                        Err(e)
                    } else {
                        Ok(Type::Named(enum_name, Vec::new()))
                    }
                }
            }
            None => {
                if !arguments.is_empty() {
                    Err(self.err(format!(
                        "enum member `{enum_name}::{variant}` takes no payload"
                    )))
                } else {
                    Ok(Type::Named(enum_name, Vec::new()))
                }
            }
        };
        match result {
            Ok(t) => {
                self.generics
                    .truncate(self.generics.len().saturating_sub(type_params.len()));
                Ok(t)
            }
            Err(e) => {
                self.generics
                    .truncate(self.generics.len().saturating_sub(type_params.len()));
                Err(e)
            }
        }
    }

    /// `Trait::method(receiver, ...)` explicit dispatch (design B1): the method
    /// is resolved to the registered `impl Trait for Type` whose receiver type
    /// matches the first argument's static type, and the call's arguments are
    /// checked against the trait binding's parameter list.
    fn check_trait_call(
        &mut self,
        trait_name: String,
        method: &str,
        arguments: &[&Expression],
    ) -> SResult<(Type, String)> {
        let entry = self
            .type_entry(&trait_name)
            .cloned()
            .ok_or_else(|| self.err(format!("unknown trait `{trait_name}`")))?;
        let TypeEntryKind::Trait(trait_def) = &entry.kind else {
            unreachable!("check_trait_call is only entered for trait entries")
        };
        let Some(sig) = trait_def.methods.iter().find(|s| s.name == method) else {
            return Err(self.err(format!("trait `{trait_name}` has no method `{method}`")));
        };
        // Dispatch always passes the receiver as the impl function's first
        // argument and binds it to `self`; a trait method without `self` would
        // mis-bind the receiver into its first real parameter at run time.
        if !sig.has_self {
            return Err(self.err(format!(
                "trait method `{trait_name}::{method}` has no `self` receiver, so it cannot be dispatched: add `self` as the first parameter to use `{trait_name}::{method}(receiver, ...)`"
            )));
        }
        // The receiver is the first argument; it must be statically typed by a
        // named type with a registered `impl`, so dispatch is fully static.
        let Some(receiver) = arguments.first() else {
            return Err(self.err(format!(
                "trait method `{trait_name}::{method}` requires a receiver argument"
            )));
        };
        let receiver_type = self.check_expression(receiver)?;
        // Dispatch always receives the receiver first, followed by the trait
        // method's declared parameters.
        let expected_slots = 1 + sig.params.len();
        if arguments.len() != expected_slots {
            return Err(self.err(format!(
                "trait method `{trait_name}::{method}` expects {expected_slots} argument(s), got {}",
                arguments.len()
            )));
        }
        for (arg, expected) in arguments[1..].iter().zip(sig.params.iter()) {
            let arg_type = self.check_expression(arg)?;
            if !self.assignable(&arg_type, expected) {
                return Err(self.err(format!(
                    "argument to `{trait_name}::{method}` must be `{}`, found `{}`",
                    expected.name(),
                    arg_type.name()
                )));
            }
        }
        // Resolve the receiver type to its concrete name for impl lookup.
        let concrete = match &receiver_type {
            Type::Named(name, _) => Some(name.clone()),
            Type::Optional(_) => {
                return Err(self.err(format!(
                    "cannot dispatch `{trait_name}::{method}` on an optional receiver: it may be `null` at run time, and dispatch has no null-safe form; pass a concrete, non-nullable receiver"
                )));
            }
            _ => None,
        };
        // Aliases are followed one level at a time to their target before
        // concluding "no impl": `type RectAlias = Rect` (where `Rect` itself
        // may alias `object`) dispatches to `impl Area for Rect`. The original
        // name is checked first, and a generic parameter is never erased
        // (dispatch on generics is rejected with its own message).
        match concrete {
            Some(name) if self.impls.contains(&(trait_name.clone(), name.clone(), method.to_string())) => {
                // An impl declared in this module that cannot be
                // instantiated for this receiver (or whose bounds fail for
                // it) contributes nothing: dispatch reports the same
                // "does not implement" a missing impl would (spec
                // `types/traits.md`, "When an impl applies").
                if self.impl_gate_blocks(&trait_name, &name, &receiver_type) {
                    return Err(self.err(format!(
                        "type `{name}` does not implement trait `{trait_name}` (add `impl {trait_name} for {name}`)"
                    )));
                }
                Ok((sig.return_type.clone(), impl_fn_name(&trait_name, &name, method)))
            }
            Some(name) if self.generics.contains(&name) => {
                // Explicit dispatch through a parameter is well-formed when
                // the parameter's declaration bounds it by this trait (spec
                // `types/generics.md`, "Generic constraints and dispatch").
                // The impl is not static here — it depends on the type the
                // parameter is instantiated with — so the call is annotated
                // for run-time resolution under the bound's trait instead of
                // a mangled name.
                let granted = self
                    .bound_traits
                    .iter()
                    .rev()
                    .find_map(|m| m.get(&name))
                    .is_some_and(|traits| traits.iter().any(|t| t == &trait_name));
                if granted {
                    Ok((
                        sig.return_type.clone(),
                        format!("dyn:{trait_name}:{method}"),
                    ))
                } else {
                    Err(self.err(format!(
                        "cannot dispatch `{trait_name}::{method}` on generic parameter `{name}`: a parameter is a dispatch receiver only when its declaration bounds it by the trait, so add `{name}: {trait_name}` to the parameter list"
                    )))
                }
            }
            Some(name) => {
                let mut target = self.alias_target(&receiver_type);
                while let Some(t) = &target {
                    if self
                        .impls
                        .contains(&(trait_name.clone(), t.clone(), method.to_string()))
                    {
                        return Ok((
                            sig.return_type.clone(),
                            impl_fn_name(&trait_name, t, method),
                        ));
                    }
                    target = self.alias_target(&Type::Named(t.clone(), Vec::new()));
                }
                Err(self.err(format!(
                    "type `{name}` does not implement trait `{trait_name}` (add `impl {trait_name} for {name}`)"
                )))
            }
            None => Err(self.err(format!(
                "cannot dispatch `{trait_name}::{method}` on type `{}`: provide a named type with `impl {trait_name} for Type`",
                receiver_type.name()
            ))),
        }
    }

    /// One step of alias resolution: if `ty` names a type alias whose
    /// underlying type is itself a named type, that name. `None` when `ty` is
    /// not a named alias or its target is not named (e.g. `type X = object`).
    fn alias_target(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Named(name, _) => match self.type_entry(name).map(|e| &e.kind) {
                Some(TypeEntryKind::Alias(Type::Named(target, _))) => Some(target.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// The success type of `operand?` and the failure values it may leave the
    /// function with (spec `error-handling.md`): `T?` succeeds with `T` and
    /// fails with `null`; `Result<T, E>` succeeds with `T` and fails with the
    /// whole `Result::Err`. `Result<T, E>?` composes the two — the Ok payload
    /// succeeds, and both `null` and the `Result` are possible failures. Any
    /// other operand is `E0217`.
    fn propagate_shape(&self, ty: &Type, span: Range<usize>) -> SResult<(Type, Vec<Type>)> {
        let resolved = self.resolve_alias(ty, 0);
        match &resolved {
            Type::Named(name, args) if name == "Result" && args.len() == 2 => {
                Ok((args[0].clone(), vec![resolved.clone()]))
            }
            Type::Optional(inner) => {
                let inner_resolved = self.resolve_alias(inner, 0);
                if matches!(&inner_resolved, Type::Named(n, a) if n == "Result" && a.len() == 2) {
                    let (success, mut failures) = self.propagate_shape(inner, span)?;
                    failures.push(Type::Null);
                    Ok((success, failures))
                } else {
                    Ok((inner_resolved, vec![Type::Null]))
                }
            }
            other => Err(self
                .err_code(
                    "E0217",
                    format!(
                        "`?` requires a `T?` or `Result<T, E>` operand, found `{}`",
                        other.name()
                    ),
                )
                .at(span)),
        }
    }

    /// A `?` needs an enclosing function whose declared return type accepts
    /// this failure value (spec `error-handling.md`, E0217/E0229): `null` for
    /// an optional operand, the whole `Result` for a `Result` operand — and
    /// never inside `defer`, whose operand runs as the block is already
    /// leaving.
    fn check_propagate_admission(&self, failure: &Type, span: Range<usize>) -> SResult<()> {
        if self.in_defer > 0 {
            return Err(self
                .err_code(
                    "E0229",
                    "`?` may not appear in a `defer` operand: control is already leaving the block when it runs",
                )
                .at(span));
        }
        let Some(ret) = self.current_return.clone() else {
            return Err(self
                .err_code(
                    "E0217",
                    "cannot use `?` here: there is no enclosing function return type",
                )
                .at(span));
        };
        if !self.declared_return {
            return Err(self
                .err_code(
                    "E0217",
                    "cannot use `?` in a function with no declared return type",
                )
                .at(span));
        }
        let mut ret = self.resolve_alias(&ret, 0);
        if let Type::Async(inner) = &ret {
            ret = self.resolve_alias(inner, 0);
        }
        if !self.assignable(failure, &ret) {
            return Err(self
                .err_code(
                    "E0217",
                    format!(
                        "`?` failure value of type `{}` is not assignable to the declared return type `{}`",
                        failure.name(),
                        ret.name()
                    ),
                )
                .at(span));
        }
        Ok(())
    }

    /// Narrow the binding `name` for one `if` branch: `to_null` selects the
    /// `null` type, otherwise the optional's inner type. Returns the name
    /// and the symbol to restore once the branch has checked. Nothing to
    /// restore means nothing narrowed (not a `T?` binding, or unbound).
    fn narrow_binding(&mut self, name: &str, to_null: bool) -> Option<(String, Symbol)> {
        let sym = self.table.lookup(name)?.clone();
        let resolved = self.resolve_alias(&sym.type_, 0);
        let Type::Optional(inner) = resolved else {
            return None;
        };
        let mut narrowed = sym;
        narrowed.type_ = if to_null { Type::Null } else { *inner };
        let old = self.table.replace_symbol(name, narrowed)?;
        Some((name.to_string(), old))
    }

    fn check_if(&mut self, if_expr: &IfExpr) -> SResult<Type> {
        let condition = self.check_expression(&if_expr.condition)?;
        if !self.assignable(&condition, &Type::Boolean) {
            return Err(self.err(format!(
                "if condition must be a `bool`, found `{}`",
                condition.name()
            )));
        }
        // Flow-sensitive narrowing (spec `types/composite-types.md`): a
        // comparison against `null` splits a `T?` binding — `x != null`
        // gives `x` type `T` in then and `null` in else, `x == null` the
        // reverse. The narrowed symbol shadows the declaration for the
        // branch check and is restored right after, so nested and sibling
        // `if`s compose. Only `T?` narrows: `unknown`, `any`, and plain `T`
        // are left untouched.
        let null_test = match_null_test(&if_expr.condition);
        let then_narrow = null_test
            .as_ref()
            .and_then(|(name, eq)| self.narrow_binding(name, *eq));
        let then_res = self.check_block_tail(&if_expr.then_branch);
        if let Some((name, old)) = then_narrow {
            self.table.replace_symbol(&name, old);
        }
        let then_type = then_res?;
        let else_res = match &if_expr.else_branch {
            Some(branch) => {
                let else_narrow = null_test
                    .as_ref()
                    .and_then(|(name, eq)| self.narrow_binding(name, !*eq));
                let res = self.check_block_tail(branch);
                if let Some((name, old)) = else_narrow {
                    self.table.replace_symbol(&name, old);
                }
                res
            }
            None => Ok(None),
        };
        let else_type = else_res?;
        // An `if` in value position takes the then-branch tail type; when both
        // branches have values they must be compatible (Rust/Swift style).
        match (then_type, else_type) {
            (Some(then_t), Some(else_t))
                if !self.assignable(&else_t, &then_t) && !self.assignable(&then_t, &else_t) =>
            {
                Err(self.err(format!(
                    "if branches have incompatible types `{}` and `{}`",
                    then_t.name(),
                    else_t.name()
                )))
            }
            (Some(then_t), _) => Ok(then_t),
            _ => Ok(Type::Any),
        }
    }

    /// Validate that every named type referenced by `ty` is defined.
    fn check_type(&self, ty: &Type) -> SResult<()> {
        match ty {
            Type::Named(name, args) => self.check_type_application(name, args),
            Type::List(inner) | Type::Optional(inner) => self.check_type(inner),
            Type::Union(parts) | Type::Intersection(parts) => {
                for p in parts {
                    self.check_type(p)?;
                }
                Ok(())
            }
            Type::Tuple(parts) => {
                for p in parts {
                    self.check_type(p)?;
                }
                Ok(())
            }
            Type::ObjectType(fields) => {
                for (_, t) in fields {
                    self.check_type(t)?;
                }
                Ok(())
            }
            Type::Async(inner) => self.check_type(inner),
            Type::FnSig { params, ret } => {
                for p in params {
                    self.check_type(p)?;
                }
                if let Some(r) = ret {
                    self.check_type(r)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Check one type application `Name<Args>` for well-formedness (spec
    /// `types/generics.md`, E0230): the head takes no applied type parameter,
    /// the argument count matches the head's declaration, and every argument
    /// is itself a well-formed type.
    fn check_type_application(&self, name: &str, args: &[Type]) -> SResult<()> {
        if self.generics.iter().any(|g| g == name) {
            // A parameter stands for a proper type, never a family of them.
            if !args.is_empty() {
                return Err(self.err_code(
                    "E0230",
                    format!("`{name}` is a type parameter, so it takes no type arguments"),
                ));
            }
            return Ok(());
        }
        if let Some(expected) = self.declared_type_arity(name)
            && args.len() != expected
        {
            return Err(self.err_code(
                "E0230",
                format!(
                    "`{name}` expects {expected} type argument{}, found {}",
                    if expected == 1 { "" } else { "s" },
                    args.len()
                ),
            ));
        }
        // Every position a type argument is written is checked against the
        // bound its parameter declares (spec `type-system/checking-rules.md`).
        if let Some(entry) = self.type_entry(name) {
            for (i, arg) in args.iter().enumerate() {
                let Some(param) = entry.type_params.get(i) else {
                    break;
                };
                for bound in entry.type_bounds.iter().filter(|b| &b.param == param) {
                    for trait_name in &bound.traits {
                        if !self.satisfies_bound(arg, trait_name) {
                            return Err(self.err_code(
                                "E0801",
                                format!("`{}` does not implement `{trait_name}`", arg.display()),
                            ));
                        }
                    }
                }
            }
        }
        for arg in args {
            self.check_type(arg)?;
        }
        // A name is known when the program declares it (a type table entry,
        // an imported face type), when it is one of the prelude's own entries,
        // or when it is a fixed-arity prelude/primitive name (spec
        // `types/primitive-types.md`, `builtins/prelude.md`).
        if self.type_table.contains_key(name)
            || self.imported_types.contains_key(name)
            || self.builtins.contains_key(name)
            || builtin_type_arity(name).is_some()
        {
            Ok(())
        } else {
            Err(self.err(format!("unknown type `{name}`")))
        }
    }

    /// How many type arguments a named type's declaration takes, if that is
    /// known statically: a user `struct`/`enum`/`type`/`trait`, or a prelude
    /// type whose arity the language fixes.
    fn declared_type_arity(&self, name: &str) -> Option<usize> {
        if let Some(entry) = self.type_entry(name) {
            return Some(entry.type_params.len());
        }
        builtin_type_arity(name)
    }

    /// Resolve a `Named` type to its underlying type. Generic type parameters
    /// erase to `Any`; enums are returned unchanged. A depth cap guards against
    /// self-referential aliases.
    fn resolve_alias(&self, ty: &Type, depth: usize) -> Type {
        if depth > 32 {
            return ty.clone();
        }
        match ty {
            Type::Named(name, _) => {
                if self.generics.contains(name) {
                    // A bounded generic erases to its bound's structural shape
                    // (`T: Area` behaves like `{ area: fn(): number }`); an
                    // unbounded generic erases to `Any`.
                    self.bound_shape_of(name).unwrap_or(Type::Any)
                } else {
                    match self.type_entry(name).map(|e| &e.kind) {
                        Some(TypeEntryKind::Alias(inner)) => self.resolve_alias(inner, depth + 1),
                        _ => ty.clone(),
                    }
                }
            }
            _ => ty.clone(),
        }
    }

    /// Look up a type by name, consulting locally-declared types, then types
    /// imported from other modules, then the built-ins: a declaration at any
    /// of the first two levels shadows a builtin of the same name.
    fn type_entry(&self, name: &str) -> Option<&TypeEntry> {
        if let Some(e) = self.type_table.get(name) {
            return Some(e);
        }
        if let Some(e) = self.imported_types.get(name) {
            self.mark_name_used(name);
            return Some(e);
        }
        self.builtins.get(name)
    }

    /// Is a value of type `from` acceptable where `to` is expected?
    fn assignable(&self, from: &Type, to: &Type) -> bool {
        let from = self.resolve_alias(from, 0);
        let to = self.resolve_alias(to, 0);
        if matches!(from, Type::Any) || matches!(to, Type::Any) {
            return true;
        }
        if from == to {
            return true;
        }
        match (&from, &to) {
            (Type::Null, Type::Optional(_)) => true,
            (inner, Type::Optional(expected)) => {
                self.assignable(inner, expected) || matches!(inner, Type::Null)
            }
            // Loose typing (docs §10): an optional cascades to its inner type,
            // mirroring how `T?` is a shorthand the checker treats permissively.
            (Type::Optional(inner), expected) => self.assignable(inner, expected),
            (Type::Union(parts), expected) => parts.iter().all(|p| self.assignable(p, expected)),
            (actual, Type::Union(parts)) => parts.iter().any(|p| self.assignable(actual, p)),
            (Type::Intersection(parts), expected) => {
                parts.iter().all(|p| self.assignable(p, expected))
            }
            (actual, Type::Intersection(parts)) => parts.iter().all(|p| self.assignable(actual, p)),
            (Type::List(a), Type::List(b)) => self.assignable(a, b),
            (Type::Object, Type::ObjectType(_)) => true,
            (Type::ObjectType(_), Type::Object) => true,
            // A brace literal stands in for any map shape when the
            // annotation is a map (spec `types/composite-types.md`: the
            // expected type is applied to the literal — `let same:
            // Map<string, int> = { a: 1 }`). `Type::Object` carries no field
            // types to compare, and map types unify loosely by name the same
            // way the typed form does.
            (Type::Object, Type::Named(n, _)) if n == "Map" => true,
            (Type::Named(a, _), Type::Named(b, _)) if a == "Map" && b == "Map" => true,
            (Type::ObjectType(from_fields), Type::ObjectType(to_fields)) => {
                // Structural typing: every field the target requires must be
                // present in the source with a compatible type. Extra source
                // fields are allowed (`{ a, b }` is assignable to `{ a }`).
                to_fields.iter().all(|(to_name, to_ty)| {
                    from_fields
                        .iter()
                        .find(|(from_name, _)| from_name == to_name)
                        .is_some_and(|(_, from_ty)| self.assignable(from_ty, to_ty))
                })
            }
            (Type::Literal(_), Type::String) => true,
            (Type::Async(a), Type::Async(b)) => self.assignable(a, b),
            (Type::Async(a), expected) => {
                matches!(a.as_ref(), Type::Any) || self.assignable(a, expected)
            }
            (actual, Type::Async(b)) => {
                matches!(b.as_ref(), Type::Any) || self.assignable(actual, b)
            }
            (
                Type::FnSig {
                    params: ap,
                    ret: ar,
                },
                Type::FnSig {
                    params: bp,
                    ret: br,
                },
            ) => {
                ap.len() == bp.len()
                    && ap.iter().zip(bp).all(|(a, b)| self.assignable(a, b))
                    && match (ar, br) {
                        (None, None) => true,
                        (Some(a), Some(b)) => self.assignable(a, b),
                        (Some(_), None) => true,
                        (None, Some(_)) => false,
                    }
            }
            _ => false,
        }
    }

    /// Value-aware check for string-literal types: does a direct literal
    /// expression belong to the set of string-literal types in `annotation`?
    fn literal_matches(&self, value: &Expression, annotation: &Type) -> bool {
        match annotation {
            Type::Optional(inner) => match value {
                Expression::Literal {
                    value: Literal::Null,
                    ..
                } => true,
                _ => self.literal_matches(value, inner),
            },
            Type::Union(parts) => parts.iter().any(|p| self.literal_matches(value, p)),
            // A numeric literal within a fixed-bit type's range coerces to it
            // (spec `type-system/coercion.md`, literal coercion; the f32/f64
            // rows take any in-range value, the integer rows only an
            // integral one — a float literal never reaches an integer type).
            // The AST carries every number as `Literal::Number`, so `3.0`
            // writes the integer literal `3`, and the sign of `-1` lives in
            // the unary operator ([`numeric_literal_of`]); an overflowing
            // f64 literal parses to infinity and is out of range.
            Type::Named(name, args) if args.is_empty() && fixed_bit(name).is_some() => {
                match numeric_literal_of(value) {
                    None => false,
                    Some(v) => match fixed_bit(name).expect("guarded above") {
                        FixedBit::Int { bits, signed } => int_fixed_fits(v, bits, signed),
                        FixedBit::Float32 => v.is_finite() && v.abs() <= f32::MAX as f64,
                        FixedBit::Float64 => v.is_finite(),
                    },
                }
            }
            Type::Named(_, _) => {
                let resolved = self.resolve_alias(annotation, 0);
                if resolved != *annotation {
                    self.literal_matches(value, &resolved)
                } else {
                    false
                }
            }
            Type::Literal(expected) => match value {
                Expression::Literal {
                    value: Literal::String(s),
                    ..
                } => s == expected,
                _ => false,
            },
            _ => false,
        }
    }

    /// `E0209`: a numeric literal written where a fixed-bit annotation is
    /// required does not fit that type (spec `type-system/coercion.md`:
    /// "a literal that does not fit a fixed-bit type it is coerced into").
    /// Returns `None` when the value is not such a literal or it fits —
    /// the caller raises its own mismatch error then — and for a
    /// non-integral literal aimed at an integer type (float-literal-to-
    /// integer is the caller's `E0201`, not a range failure).
    fn fixed_literal_error(
        &self,
        value: &Expression,
        annotation: &Type,
        span: Range<usize>,
    ) -> Option<XuloError> {
        let v = numeric_literal_of(value)?;
        let target = self.resolve_alias(annotation, 0);
        let Type::Named(name, args) = &target else {
            return None;
        };
        if !args.is_empty() {
            return None;
        }
        let text = num_literal_text(v);
        match fixed_bit(name)? {
            FixedBit::Int { bits, signed } => match int_literal_fit(v, bits, signed) {
                LiteralFit::OutOfRange => Some(
                    self.err_code("E0209", format!("literal `{text}` does not fit `{name}`"))
                        .at(span),
                ),
                LiteralFit::Fits | LiteralFit::NotAnInteger => None,
            },
            FixedBit::Float32 => {
                if !v.is_finite() || v.abs() > f32::MAX as f64 {
                    Some(
                        self.err_code("E0209", format!("literal `{text}` does not fit `f32`"))
                            .at(span),
                    )
                } else {
                    None
                }
            }
            FixedBit::Float64 => {
                if !v.is_finite() {
                    Some(
                        self.err_code("E0209", format!("literal `{text}` does not fit `f64`"))
                            .at(span),
                    )
                } else {
                    None
                }
            }
        }
    }
}

/// True when a parameter is declared optional (`name: T?`), meaning callers may
/// omit its argument (docs §6 / §15).
fn param_optional(param: &xulo_core::ast::Param) -> bool {
    matches!(param.type_annotation, Some(Type::Optional(_)))
}

/// A type parameter that two constraints settled differently: the parameter
/// name, the type the earlier constraint fixed, and the type the later one
/// demanded. Reported as a mismatch (spec `types/generics.md`, E0202).
type InferenceConflict = (String, Type, Type);

/// Infer a function's type-parameter bindings by unifying each declared
/// parameter type against its (positionally aligned) argument type, e.g.
/// `first([1, 2])` binds `T = number` when the parameter is `list<T>`.
///
/// A parameter receives one solution: when a later constraint demands a
/// different type for one already fixed, inference fails with the conflict
/// instead of keeping the first answer.
fn infer_type_bindings(
    type_params: &[String],
    param_types: &[Type],
    arg_types: &[Type],
) -> Result<std::collections::HashMap<String, Type>, InferenceConflict> {
    let mut bindings = std::collections::HashMap::new();
    for (param_ty, arg_ty) in param_types.iter().zip(arg_types.iter()) {
        unify_param(param_ty, arg_ty, type_params, &mut bindings)?;
    }
    Ok(bindings)
}

fn unify_param(
    expected: &Type,
    actual: &Type,
    type_params: &[String],
    bindings: &mut std::collections::HashMap<String, Type>,
) -> Result<(), InferenceConflict> {
    match expected {
        Type::Named(name, _) if type_params.contains(name) => match bindings.get(name) {
            Some(existing) if existing != actual => {
                Err((name.clone(), existing.clone(), actual.clone()))
            }
            Some(_) => Ok(()),
            None => {
                bindings.insert(name.clone(), actual.clone());
                Ok(())
            }
        },
        Type::List(inner) => {
            if let Type::List(element) = actual {
                unify_param(inner, element, type_params, bindings)?;
            }
            Ok(())
        }
        Type::Optional(inner) => match actual {
            Type::Optional(a) => unify_param(inner, a, type_params, bindings),
            Type::Null => Ok(()),
            other => unify_param(inner, other, type_params, bindings),
        },
        Type::Union(parts) => {
            for p in parts {
                unify_param(p, actual, type_params, bindings)?;
            }
            Ok(())
        }
        Type::Intersection(parts) => {
            for p in parts {
                unify_param(p, actual, type_params, bindings)?;
            }
            Ok(())
        }
        Type::Async(inner) => {
            if let Type::Async(a) = actual {
                unify_param(inner, a, type_params, bindings)?;
            }
            Ok(())
        }
        Type::FnSig { params, ret } => {
            if let Type::FnSig {
                params: actual_params,
                ret: actual_ret,
            } = actual
            {
                for (p, a) in params.iter().zip(actual_params.iter()) {
                    unify_param(p, a, type_params, bindings)?;
                }
                if let (Some(pe), Some(ae)) = (ret, actual_ret) {
                    unify_param(pe, ae, type_params, bindings)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Substitute inferred type-parameter bindings throughout a type.
fn substitute_type(ty: &Type, bindings: &std::collections::HashMap<String, Type>) -> Type {
    match ty {
        Type::Named(name, _) => {
            if let Some(bound) = bindings.get(name) {
                bound.clone()
            } else {
                ty.clone()
            }
        }
        Type::List(inner) => Type::List(Box::new(substitute_type(inner, bindings))),
        Type::Optional(inner) => Type::Optional(Box::new(substitute_type(inner, bindings))),
        Type::Union(parts) => {
            Type::Union(parts.iter().map(|p| substitute_type(p, bindings)).collect())
        }
        Type::Intersection(parts) => {
            Type::Intersection(parts.iter().map(|p| substitute_type(p, bindings)).collect())
        }
        Type::Async(inner) => Type::Async(Box::new(substitute_type(inner, bindings))),
        Type::FnSig { params, ret } => Type::FnSig {
            params: params
                .iter()
                .map(|p| substitute_type(p, bindings))
                .collect(),
            ret: ret.as_ref().map(|r| Box::new(substitute_type(r, bindings))),
        },
        other => other.clone(),
    }
}

/// Strict structural match of an `impl` receiver pattern against a
/// receiver's written type arguments (spec `types/traits.md`, "When an impl
/// applies"): equal arity, every parameter position binding to exactly one
/// type — so the repeated parameter of `Pair<T, T>` must see one type in
/// both slots — and every concrete position equal, including its own type
/// arguments, which `Type`'s name-based `PartialEq` ignores.
fn pattern_matches(
    pattern: &[Type],
    receiver: &[Type],
    params: &[String],
    bindings: &mut std::collections::HashMap<String, Type>,
) -> bool {
    if pattern.len() != receiver.len() {
        return false;
    }
    pattern
        .iter()
        .zip(receiver.iter())
        .all(|(p, r)| pattern_type_matches(p, r, params, bindings))
}

fn pattern_type_matches(
    pattern: &Type,
    actual: &Type,
    params: &[String],
    bindings: &mut std::collections::HashMap<String, Type>,
) -> bool {
    match pattern {
        Type::Named(name, args) if params.contains(name) && args.is_empty() => {
            match bindings.get(name) {
                Some(prev) => type_same(prev, actual),
                None => {
                    bindings.insert(name.clone(), actual.clone());
                    true
                }
            }
        }
        Type::Named(name, pargs) => match actual {
            Type::Named(aname, aargs) => {
                name == aname
                    && (pargs.is_empty() || pattern_matches(pargs, aargs, params, bindings))
            }
            _ => false,
        },
        Type::List(inner) => match actual {
            Type::List(a) => pattern_type_matches(inner, a, params, bindings),
            _ => false,
        },
        Type::Optional(inner) => match actual {
            Type::Optional(a) => pattern_type_matches(inner, a, params, bindings),
            Type::Null => true,
            other => pattern_type_matches(inner, other, params, bindings),
        },
        Type::Async(inner) => match actual {
            Type::Async(a) => pattern_type_matches(inner, a, params, bindings),
            _ => false,
        },
        Type::Union(parts) => parts
            .iter()
            .all(|p| pattern_type_matches(p, actual, params, bindings)),
        Type::Intersection(parts) => parts
            .iter()
            .all(|p| pattern_type_matches(p, actual, params, bindings)),
        Type::Any => true,
        other => other == actual,
    }
}

/// Type equality that sees type arguments (`Type`'s `PartialEq` compares
/// named types by name only), used when one parameter must bind to a single
/// type in every position it appears.
fn type_same(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Named(x, xa), Type::Named(y, ya)) => {
            x == y && xa.len() == ya.len() && xa.iter().zip(ya.iter()).all(|(p, q)| type_same(p, q))
        }
        (Type::List(x), Type::List(y))
        | (Type::Optional(x), Type::Optional(y))
        | (Type::Async(x), Type::Async(y)) => type_same(x, y),
        (Type::Union(x), Type::Union(y))
        | (Type::Intersection(x), Type::Intersection(y))
        | (Type::Tuple(x), Type::Tuple(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| type_same(p, q))
        }
        (
            Type::FnSig {
                params: px,
                ret: rx,
            },
            Type::FnSig {
                params: py,
                ret: ry,
            },
        ) => {
            px.len() == py.len()
                && px.iter().zip(py.iter()).all(|(p, q)| type_same(p, q))
                && match (rx, ry) {
                    (Some(x), Some(y)) => type_same(x, y),
                    (None, None) => true,
                    _ => false,
                }
        }
        _ => a == b,
    }
}

fn literal_type(lit: &Literal) -> Type {
    match lit {
        Literal::String(_) => Type::String,
        Literal::Number(_) => Type::Number,
        Literal::Boolean(_) => Type::Boolean,
        Literal::Null => Type::Null,
        Literal::List(items) => {
            let element = items
                .iter()
                .find(|e| !matches!(e, Expression::Spread { .. }))
                .map(expr_type_hint)
                .unwrap_or(Type::Any);
            Type::List(Box::new(element))
        }
        Literal::Object(_) => Type::Object,
    }
}

/// Best-effort type of an expression without semantic checks (used to infer
/// list element types from literal syntax).
fn expr_type_hint(expr: &Expression) -> Type {
    match expr {
        Expression::Literal { value: lit, .. } => literal_type(lit),
        _ => Type::Any,
    }
}

/// Join two types into a union for list-element inference: `[1, "a"]` becomes
/// `list<number | string>`. Flattens and deduplicates existing unions and
/// collapses to a plain type when the parts coincide.
fn union_join(a: &Type, b: &Type) -> Type {
    let mut parts: Vec<Type> = match a {
        Type::Union(ps) => ps.clone(),
        other => vec![other.clone()],
    };
    let mut additions: Vec<Type> = match b {
        Type::Union(ps) => ps.clone(),
        other => vec![other.clone()],
    };
    for part in additions.drain(..) {
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    if parts.len() == 1 {
        parts.pop().expect("one part")
    } else {
        Type::Union(parts)
    }
}

/// Render a match arm pattern for error messages.
fn pattern_name(arm: &xulo_core::ast::MatchArm) -> String {
    match &arm.pattern {
        xulo_core::ast::MatchPattern::Literal(Literal::String(s)) => format!("\"{s}\""),
        xulo_core::ast::MatchPattern::Literal(Literal::Number(n)) => n.to_string(),
        xulo_core::ast::MatchPattern::Literal(Literal::Boolean(b)) => b.to_string(),
        xulo_core::ast::MatchPattern::Literal(Literal::Null) => "null".into(),
        xulo_core::ast::MatchPattern::Literal(Literal::List(_)) => "[...]".into(),
        xulo_core::ast::MatchPattern::Literal(Literal::Object(_)) => "{...}".into(),
        xulo_core::ast::MatchPattern::Enum(r) => format!("{}::{}", r.enum_name, r.variant),
        xulo_core::ast::MatchPattern::EnumPayload {
            enum_name, variant, ..
        } => format!("{enum_name}::{variant}"),
        xulo_core::ast::MatchPattern::Wildcard => "_".into(),
        xulo_core::ast::MatchPattern::Range { start, end, .. } => {
            format!("{}..{}", start, end)
        }
        xulo_core::ast::MatchPattern::Binding { name, .. } => name.clone(),
        xulo_core::ast::MatchPattern::TypePattern {
            type_name, binder, ..
        } => match binder {
            xulo_core::ast::PatternBinder::Wildcard => type_name.clone(),
            xulo_core::ast::PatternBinder::Binding(b) => format!("{type_name} {b}"),
        },
        xulo_core::ast::MatchPattern::Or { .. } => ".. | ..".into(),
        xulo_core::ast::MatchPattern::Struct { name, .. } => format!("{name} {{ .. }}"),
    }
}
