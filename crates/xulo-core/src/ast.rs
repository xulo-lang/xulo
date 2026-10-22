use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Fn(FnDef),
    /// `#Test [ "(" [ StringLiteral ] ")" ] [async] fn name() { … }` — an
    /// always-checked, inert test body (spec testing.md). The `FnDef` carries
    /// the fixed shape: empty params, no return type, no generics; `is_async`
    /// is the optional marker and `title` the optional display title.
    Test(FnDef),
    Let(LetBinding),
    Return(ReturnStmt),
    For(ForStmt),
    While(WhileStmt),
    Assign(AssignStmt),
    TypeAlias(TypeAlias),
    Enum(EnumDef),
    Struct(StructDef),
    Expr(ExprStmt),
    Block(Block),
    Try(TryStmt),
    Throw(Expression),
    Export(ExportStmt),
    State(StateStmt),
    Store(StoreStmt),
    Effect(EffectStmt),
    Environment(EnvStmt),
    Component(ComponentStmt),
    Trait(TraitDecl),
    Impl(ImplDecl),
    Break,
    Continue,
    /// `mod name;` / `mod name { … }` — claims a file, or nests a module.
    Mod(ModDecl),
    /// `use …;` / `pub use …;` — binds names of a path in this file's scope.
    Use(UseDecl),
    /// `[pub] extern "abi" { fn name(…) [: T] [as local]; … }` — an FFI
    /// import block (spec ffi.md). Items declare signatures and bind their
    /// symbol (or `local` alias) as callables; the block's `pub` exports
    /// every item. The companion `pub extern "abi" fn` export form lowers
    /// to `ExportItem::Fn` with `FnDef::abi` set.
    ExternBlock(ExternBlockDecl),
    /// `defer <block|expression>`: runs its operand when the enclosing block
    /// exits (spec: statements/return-and-block.md).
    Defer(DeferStmt),
}

/// An expression used as a statement. `has_semicolon` records whether it ended
/// with a `;`: a trailing expression *without* a semicolon is a function's
/// implicit return; with one it is an ordinary (value-ignored) statement and
/// triggers an "ignored return value" warning (docs §21.2).
#[derive(Debug, Clone, PartialEq)]
pub struct ExprStmt {
    pub expr: Expression,
    pub has_semicolon: bool,
    pub span: Range<usize>,
}

/// One declaration attribute: `#Name` with an optional parenthesized,
/// comma-separated list of identifier arguments (`#Derive(ToString)`,
/// `#Test`). Attributes are written on the five item declarations — `fn`,
/// `struct`, `enum`, `trait`, `impl` — and on the test form; which names are
/// defined and where they may appear is a checking rule (spec
/// `types/traits.md`, "Attributes"), not a grammatical one.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// The attribute name as written after `#` (`Test`, `Derive`).
    pub name: String,
    /// Source span of the attribute name (diagnostics point here).
    pub name_span: Range<usize>,
    /// Parenthesized argument names, in order; empty for `#Name` and for
    /// `#Name()` alike (registry entries distinguish the two shapes, which is
    /// `E0110` where a list is required).
    pub args: Vec<(String, Range<usize>)>,
    /// Full span from `#` through the closing `)` (or the name when bare).
    pub span: Range<usize>,
    /// Whether the argument list was written at all — `#Derive` and
    /// `#Derive()` are both an E0110 for `Derive`, but the registry reports
    /// them uniformly from the list being empty.
    pub has_parens: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDef {
    pub name: String,
    /// Source span of the function name (for name-related diagnostics and
    /// editor tooling's go-to-definition).
    pub name_span: Range<usize>,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub type_params: Vec<String>,
    /// Generic bounds on `type_params`: `T: Area` constrains the parameter.
    pub bounds: Vec<FnBound>,
    pub is_async: bool,
    pub body: Block,
    pub span: Range<usize>,
    /// Declaration attributes written directly before the `fn` keyword
    /// (empty for methods, closures, and the fixed-shape test form, whose
    /// `#Test` is part of the `TestDecl` production).
    pub attributes: Vec<Attribute>,
    /// The `#Test` display title — `#Test("parse test")` — as written,
    /// `None` when the group is absent or empty and for every declaration
    /// that is not a test (spec testing.md, "The title"). It is metadata for
    /// a tool's reporting: not a name, never referenceable.
    pub title: Option<String>,
    /// The FFI ABI tag of a `pub extern "abi" fn` export (spec ffi.md) —
    /// `Some("C")` only on that form; `None` for every ordinary function.
    /// The exported symbol is the declared name and the compiler lays the
    /// function out under the registered ABI's calling convention.
    pub abi: Option<String>,
}

/// `[pub] extern "abi" { ExternItem* }` — an FFI import block (spec ffi.md).
#[derive(Debug, Clone, PartialEq)]
pub struct ExternBlockDecl {
    /// The block's `pub`: exports every item of the block.
    pub is_pub: bool,
    /// The `AbiTag` as written (e.g. `"C"`); registration is a checking
    /// rule, not a grammatical one (E0902).
    pub abi: String,
    pub items: Vec<ExternItem>,
    pub span: Range<usize>,
}

/// One `fn name(…) [: T] [as local];` inside an extern block — a signature
/// declaration with no body (spec ffi.md).
#[derive(Debug, Clone, PartialEq)]
pub struct ExternItem {
    /// The declared symbol name (what gets looked up in the process).
    pub name: String,
    pub name_span: Range<usize>,
    /// The local binding name: the `as` alias when written, otherwise the
    /// declared name.
    pub local_name: String,
    /// Source span of the local binding (the alias when written, otherwise
    /// the declared name's span).
    pub local_name_span: Range<usize>,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub type_annotation: Option<Type>,
    pub default: Option<Expression>,
    /// Parameter mode: `mut T` / `shared T` (spec: types/function-types.md).
    /// `None` for the ordinary (default) mode.
    pub mode: Option<ParamMode>,
    pub span: Range<usize>,
}

/// A parameter mode: `mut T` (exclusive mutable access) or `shared T`
/// (concurrent read access). Modes are exact — they do not coerce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamMode {
    Mut,
    Shared,
}

/// A `let` or `const` binding. `const` bindings may not be reassigned.
/// `is_mutable` controls whether reassignment is allowed: `let x = 1` is
/// immutable by default; `let mut x = 1` or `let x := 1` sets `is_mutable`.
#[derive(Debug, Clone, PartialEq)]
pub struct LetBinding {
    pub name: String,
    /// Source span of the binding name (for name-related diagnostics).
    pub name_span: Range<usize>,
    pub type_annotation: Option<Type>,
    pub value: Option<Expression>,
    pub is_const: bool,
    /// When true, the variable can be reassigned after declaration.
    pub is_mutable: bool,
    /// `let x = shared expr`: the initializer is a shared (concurrently
    /// accessible) value (spec: concurrency.md).
    pub is_shared: bool,
    /// `@Memo` value memoization: `@Memo([deps]) let x = expr`. The native
    /// interpreter caches the computed value per (site, deps) and reuses it
    /// when the deps are unchanged; the JS target recomputes on every render
    /// (documented no-op for now).
    pub memo: bool,
    /// The memo dependency expressions (`None` only when `memo` is `false`; a
    /// bare `@Memo` is normalized to an empty list = cache forever).
    pub memo_deps: Option<Vec<Expression>>,
    /// Tuple destructuring: `let (a, b, c) = expr`. When `Some`, the binding
    /// destructures the right-hand side into the listed names. `name` is set
    /// to a synthetic empty string for tuple patterns.
    pub tuple_names: Option<Vec<String>>,
    /// Object destructuring: `let { name, age: user_age } = expr`. When `Some`,
    /// the binding destructures an object's fields. Each entry is
    /// `(field_name, alias)`: `None` alias means the variable name matches the
    /// field name.
    pub object_destructuring: Option<Vec<(String, Option<String>)>>,
}

/// A `@State` declaration: reactive local state inside a `View` function.
#[derive(Debug, Clone, PartialEq)]
pub struct StateStmt {
    pub binding: LetBinding,
}

/// A `@Store` declaration: destructure reactive store bindings.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreStmt {
    pub pattern: BindingPattern,
    pub value: Expression,
}

/// The left-hand side of a `@Store` binding: `name` or `{ a, b: c }`.
#[derive(Debug, Clone, PartialEq)]
pub enum BindingPattern {
    Ident(String),
    Destructure(Vec<(String, Option<String>)>),
}

/// A `@Effect` declaration: a closure plus an optional dependency array.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectStmt {
    pub closure: FnExpr,
    pub deps: Option<Vec<Expression>>,
}

/// An `@Environment` declaration: reads an injected value of the given type.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvStmt {
    pub name: String,
    /// Source span of the binding name (for name-related diagnostics).
    pub name_span: Range<usize>,
    pub type_: Type,
}

/// A UI component invocation: `Component(args) { children }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentStmt {
    pub name: String,
    /// The byte span of `name` (for editor tooling: hover/go-to-definition on
    /// component sites).
    pub name_span: Range<usize>,
    pub args: Vec<CallArg>,
    pub children: Vec<UiElement>,
    /// Source span of the whole invocation (name through children block).
    pub span: Range<usize>,
    /// A written `CallTypeArgs` list (`Text<int>("Hi") { … }`); components
    /// declare no type parameters, so the checker rejects it with `E0807`.
    pub type_args: Option<Vec<Type>>,
}

/// A single element in a UI block (component, text, expression, if, for, or grouping).
#[derive(Debug, Clone, PartialEq)]
pub enum UiElement {
    Component(ComponentStmt),
    Text(String),
    /// A bare expression child (e.g. a forwarded `children` variable, a member
    /// access, or a call). The semantic phase constrains its type to a string,
    /// a component, or a list of components; a list value renders as a nested
    /// array that the consumer flattens (the same convention the compiled
    /// `if`/`for` output uses).
    Expr(Expression),
    If {
        condition: Expression,
        then_branch: Vec<UiElement>,
        else_branch: Option<Vec<UiElement>>,
    },
    For {
        iter_var: String,
        /// Source span of the loop variable name (for name-related diagnostics).
        iter_var_span: Range<usize>,
        iterable: Expression,
        body: Vec<UiElement>,
    },
    Group(Vec<UiElement>),
}

/// The left-hand side of an assignment: a plain name, a member access, or an
/// index into a list/object (`user.name = x`, `xs[0] = y`).
#[derive(Debug, Clone, PartialEq)]
pub enum AssignTarget {
    Name(String),
    Member(Box<Expression>, String),
    Index(Box<Expression>, Box<Expression>),
}

/// A reassignment statement: `target = expr`.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignStmt {
    pub target: AssignTarget,
    pub value: Expression,
    pub span: Range<usize>,
}

/// A `type` alias declaration. Type parameters are parsed and erased.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeAlias {
    pub name: String,
    /// Source span of the alias name (for name-related diagnostics).
    pub name_span: Range<usize>,
    pub type_params: Vec<String>,
    /// Inline bounds on `type_params`, parallel to it and no longer
    /// (`type Params` has no `where` clause; spec `types/generics.md`).
    pub type_bounds: Vec<FnBound>,
    pub type_: Type,
}

/// A generic bound on one type parameter: `T: Area & Comparable` becomes
/// `FnBound { param: "T", traits: ["Area", "Comparable"] }`.
#[derive(Debug, Clone, PartialEq)]
pub struct FnBound {
    pub param: String,
    pub traits: Vec<String>,
}

/// A `trait` declaration: a named, structural contract of method signatures.
/// The receiver `self` marks a member as an instance method; trait members are
/// satisfied structurally (a type whose object literal carries the matching
/// function field) or by an `impl` block.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitDecl {
    pub name: String,
    /// Source span of the trait name (for name-related diagnostics).
    pub name_span: Range<usize>,
    pub type_params: Vec<String>,
    pub methods: Vec<TraitMethod>,
    pub span: Range<usize>,
    /// Declaration attributes written directly before the `trait` keyword.
    pub attributes: Vec<Attribute>,
}

/// One method signature in a `trait` declaration. `self` is the receiver and
/// never appears in `params`; remaining parameters are positional.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethod {
    pub name: String,
    pub name_span: Range<usize>,
    pub has_self: bool,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub is_async: bool,
    pub span: Range<usize>,
}

/// An `impl Trait for Type` block or an inherent `impl Type` block: provides
/// method bodies for a concrete named type. `self` may be the first parameter
/// of each method and is bound to the receiver's type during checking.
/// When `is_inherent` is true, this is an inherent impl (no trait).
#[derive(Debug, Clone, PartialEq)]
pub struct ImplDecl {
    pub trait_name: String,
    pub type_name: String,
    /// Type parameters of a generic `impl` (`impl<T> …`), in order.
    pub type_params: Vec<String>,
    /// Inline bounds on `type_params`, parallel to it (spec `types/generics.md`).
    pub type_bounds: Vec<FnBound>,
    /// The receiver's written type arguments (`Pair<T, T>` in
    /// `impl<T> Pair<T, T> { … }`), in declaration order; empty when the
    /// receiver is written without arguments (`impl Area for Rect`). Together
    /// with `type_params` this is the pattern applicability is decided against
    /// (spec `types/traits.md`, "When an impl applies").
    pub type_args: Vec<Type>,
    pub methods: Vec<FnDef>,
    pub span: Range<usize>,
    /// `true` for `impl Type { ... }` (inherent), `false` for `impl Trait for Type { ... }`.
    pub is_inherent: bool,
    /// Declaration attributes written directly before the `impl` keyword.
    pub attributes: Vec<Attribute>,
}

/// Mangle an `impl` method into the module-level function name that codegen
/// and the native runtime both define and dispatch to.
pub fn impl_fn_name(trait_name: &str, type_name: &str, method: &str) -> String {
    format!("impl_{trait_name}_{type_name}_{method}")
}

/// An `enum` declaration with optional per-variant payload types.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    /// Source span of the enum name (for name-related diagnostics).
    pub name_span: Range<usize>,
    pub type_params: Vec<String>,
    /// Inline bounds on `type_params`, parallel to it (spec `types/generics.md`).
    pub type_bounds: Vec<FnBound>,
    pub variants: Vec<EnumVariant>,
    /// Declaration attributes written directly before the `enum` keyword.
    pub attributes: Vec<Attribute>,
}

/// A single positional payload parameter of an enum variant:
/// `Submit(data: object)` or `Point(number, number)`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumPayloadParam {
    /// Optional field name for the parameter, e.g. `data` in `Submit(data: object)`.
    pub name: Option<String>,
    pub type_: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    /// Source span of the variant name (for name-related diagnostics).
    pub name_span: Range<usize>,
    /// Payload parameters, in declaration order. `None` when the variant
    /// carries no payload.
    pub payload: Option<Vec<EnumPayloadParam>>,
}

/// A `struct` definition: `struct User { name: string, age: number = 0 }`
#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: String,
    /// Source span of the struct name (for name-related diagnostics).
    pub name_span: Range<usize>,
    pub type_params: Vec<String>,
    /// Inline bounds on `type_params`, parallel to it (spec `types/generics.md`).
    pub type_bounds: Vec<FnBound>,
    pub fields: Vec<StructField>,
    pub span: Range<usize>,
    /// Declaration attributes written directly before the `struct` keyword.
    pub attributes: Vec<Attribute>,
}

/// A single field in a `struct` definition.
#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub name: String,
    pub name_span: Range<usize>,
    pub type_: Type,
    /// Optional default value expression. `None` means the field is required.
    pub default: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStmt {
    pub value: Option<Expression>,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForStmt {
    pub iter_var: String,
    /// Source span of the loop variable name (for name-related diagnostics).
    pub iter_var_span: Range<usize>,
    pub iterable: Expression,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhileStmt {
    pub condition: Expression,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Statement>,
}

/// A `try { ... } catch (e) { ... }` statement. (Removed from the language by
/// the current spec — kept so legacy ASTs still round-trip through tooling.)
#[derive(Debug, Clone, PartialEq)]
pub struct TryStmt {
    pub try_block: Block,
    pub catch_var: String,
    /// Source span of the catch-variable name (for name-related diagnostics).
    pub catch_var_span: Range<usize>,
    pub catch_block: Block,
}

/// A `defer <block|expression>` statement: the operand runs when control
/// leaves the enclosing block (spec: statements/return-and-block.md).
#[derive(Debug, Clone, PartialEq)]
pub struct DeferStmt {
    pub body: DeferBody,
}

/// The operand of a `defer`: a block or an expression. An operand beginning
/// with `{` is always a block, never a map literal.
#[derive(Debug, Clone, PartialEq)]
pub enum DeferBody {
    Block(Block),
    Expr(Expression),
}

/// `mod name;` / `pub mod name { … }` — a file-backed module claims a file in
/// the declaring module's home directory; an inline module carries its own
/// body (header forms plus declarations).
#[derive(Debug, Clone, PartialEq)]
pub struct ModDecl {
    pub name: String,
    pub is_pub: bool,
    /// `Some(body)` for an inline `mod name { … }`.
    pub body: Option<Vec<Statement>>,
    pub span: Range<usize>,
}

/// `use …;` and `pub use …;` — binds names of a path in this file's scope
/// (and, for `pub use`, adds them to the module's interface).
#[derive(Debug, Clone, PartialEq)]
pub struct UseDecl {
    pub is_pub: bool,
    /// `use type …`: binds for type positions only, erased everywhere else.
    pub type_only: bool,
    pub target: UseTarget,
    pub span: Range<usize>,
}

/// What a `use`/`pub use` targets: a single (possibly aliased) path, a group
/// of entries under a prefix, or a glob of the prefix's exports.
#[derive(Debug, Clone, PartialEq)]
pub enum UseTarget {
    Path {
        head: PathHead,
        segments: Vec<String>,
        alias: Option<String>,
    },
    Group {
        head: PathHead,
        entries: Vec<UseEntry>,
    },
    Glob {
        head: PathHead,
        /// Middle segments between the head and `*` (`std::math` in
        /// `use std::math::*`); empty for a single-segment glob.
        segments: Vec<String>,
    },
    /// `pub use { a, b }` — re-exports names already bound in this module.
    Local(Vec<String>),
}

/// One entry of a grouped use: a path relative to the group's prefix
/// (`fields::Field` in `use form::{validate, fields::Field}`), optionally
/// renamed locally.
#[derive(Debug, Clone, PartialEq)]
pub struct UseEntry {
    pub segments: Vec<String>,
    pub alias: Option<String>,
}

/// The head of a qualified path: a manifest package name (kebab-case
/// allowed), an ordinary identifier, or the `self`/`super` module keywords.
#[derive(Debug, Clone, PartialEq)]
pub enum PathHead {
    Package(String),
    Name(String),
    SelfModule,
    Super,
}

/// A `pub` declaration statement (also the target of `pub use`). Internal
/// name kept as "export" for historical continuity.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportStmt {
    pub item: ExportItem,
}

/// What a `pub` export exposes: a declaration (`pub fn/let/const/type`), a
/// bare name list (`pub use { a, b }`), or a trait/enum/alias.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportItem {
    Fn(FnDef),
    Let(LetBinding),
    Names(Vec<String>),
    Type(TypeAlias),
    Enum(EnumDef),
    Struct(StructDef),
    Trait(TraitDecl),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Literal {
        value: Literal,
        span: Range<usize>,
    },
    Identifier {
        name: String,
        span: Range<usize>,
    },
    BinaryOp(Box<BinaryOp>),
    Unary(Box<UnaryOp>),
    Call(Call),
    EnumRef(EnumRef),
    If(Box<IfExpr>),
    Ternary(Box<TernaryExpr>),
    Match(Box<MatchExpr>),
    Member(Box<MemberAccess>),
    Index(Box<IndexExpr>),
    Nullish(Box<NullishExpr>),
    Range(Box<RangeExpr>),
    Await {
        expr: Box<Expression>,
        span: Range<usize>,
    },
    /// An anonymous function literal: `fn(a: number): number { ... }`.
    FnExpr(Box<FnExpr>),
    /// A spread element in a list literal: `[1, ...rest, 3]`.
    Spread {
        expr: Box<Expression>,
        span: Range<usize>,
    },
    /// Calling a function value held in an arbitrary expression:
    /// `xs[0](10)`, `getHandler()(event)`, `(fn() {...})(5)`.
    CallValue(Box<CallValue>),
    /// A `$name` binding reference to a `@State`/`@Store` variable.
    Binding {
        name: String,
        span: Range<usize>,
    },
    /// A UI component invocation with a children block, used as a value:
    /// `VStack { Text("a") }`. In statement and child position the parser
    /// lowers this to `Statement::Component` / `UiElement::Component`.
    Component(ComponentStmt),
    /// A tuple literal: `(a, b, c)` (at least two elements).
    Tuple {
        items: Vec<Expression>,
        span: Range<usize>,
    },
    /// A typed map literal: `Map<K, V>{ key: value, … }`.
    TypedMap(Box<TypedMap>),
    /// A type test: `x is T` / `x is not T`.
    Is(Box<IsExpr>),
    /// The `panic(expr)` expression: stops the program with an error.
    Panic {
        expr: Box<Expression>,
        span: Range<usize>,
    },
    /// The postfix failure-propagation operator `expr?`.
    Propagate {
        expr: Box<Expression>,
        span: Range<usize>,
    },
    /// An arrow closure: `(a, b) => expr`, `x => { … }`, `async x => expr`.
    Arrow(Box<ArrowClosure>),
    /// A `spawn` expression: starts a task, thread, process, or service.
    Spawn(Box<SpawnExpr>),
    /// A `lock` expression: runs a block over a shared binding.
    Lock(Box<LockExpr>),
}

impl Expression {
    /// Source span of this expression (its first to last token).
    pub fn span(&self) -> &Range<usize> {
        match self {
            Expression::Literal { span, .. }
            | Expression::Identifier { span, .. }
            | Expression::Binding { span, .. }
            | Expression::Await { span, .. }
            | Expression::Spread { span, .. } => span,
            Expression::BinaryOp(b) => &b.span,
            Expression::Unary(u) => &u.span,
            Expression::Call(c) => &c.span,
            Expression::EnumRef(r) => &r.span,
            Expression::If(e) => &e.span,
            Expression::Ternary(t) => &t.span,
            Expression::Match(m) => &m.span,
            Expression::Member(m) => &m.span,
            Expression::Index(i) => &i.span,
            Expression::Nullish(n) => &n.span,
            Expression::Range(r) => &r.span,
            Expression::FnExpr(f) => &f.span,
            Expression::CallValue(c) => &c.span,
            Expression::Component(c) => &c.span,
            Expression::Tuple { span, .. } => span,
            Expression::TypedMap(t) => &t.span,
            Expression::Is(i) => &i.span,
            Expression::Panic { span, .. } => span,
            Expression::Propagate { span, .. } => span,
            Expression::Arrow(a) => &a.span,
            Expression::Spawn(s) => &s.span,
            Expression::Lock(l) => &l.span,
        }
    }
}

/// An anonymous function expression (`fn(a, b) { ... }`), usable wherever a
/// value is expected. Captures variables from the enclosing scope like a JS
/// closure.
#[derive(Debug, Clone, PartialEq)]
pub struct FnExpr {
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub body: Block,
    pub is_async: bool,
    pub span: Range<usize>,
}

/// An arrow closure: `(a, b) => expr`, `x => { … }`, or the async forms.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrowClosure {
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub body: ArrowBody,
    pub is_async: bool,
    pub span: Range<usize>,
}

/// The body of an arrow closure: an expression or a block.
#[derive(Debug, Clone, PartialEq)]
pub enum ArrowBody {
    Expr(Expression),
    Block(Block),
}

/// A typed map literal: `Map<K, V>{ key: value, … }`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedMap {
    pub key_type: Type,
    pub value_type: Type,
    pub entries: Vec<TypedMapEntry>,
    pub span: Range<usize>,
}

/// One `key: value` entry of a typed map literal.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedMapEntry {
    pub key: Expression,
    pub value: Expression,
}

/// A type test: `expr is Type` / `expr is not Type`.
#[derive(Debug, Clone, PartialEq)]
pub struct IsExpr {
    pub expr: Box<Expression>,
    pub negated: bool,
    pub type_: Type,
    pub span: Range<usize>,
}

/// A `spawn` expression (spec: concurrency.md).
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnExpr {
    pub target: SpawnTarget,
    pub body: SpawnBody,
    pub span: Range<usize>,
}

/// The dispatch target of a `spawn` task form.
#[derive(Debug, Clone, PartialEq)]
pub enum SpawnTarget {
    /// `spawn async { … }`
    Default,
    /// `spawn .thread async { … }`
    Thread,
    /// `spawn .process async { … }` (task form; the service form uses
    /// `SpawnBody::Service`).
    Process,
    /// `spawn .on(expr) async { … }`
    On(Box<Expression>),
}

/// The body of a `spawn` expression: an async block, or the service form
/// `spawn .process Type { fields }`.
#[derive(Debug, Clone, PartialEq)]
pub enum SpawnBody {
    Async(Block),
    Service {
        type_name: String,
        fields: Vec<SpawnField>,
    },
}

/// One `name: value` field initializer of a spawn service.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnField {
    pub name: String,
    pub value: Expression,
}

/// A `lock name { … }` expression: a critical section over a shared binding.
#[derive(Debug, Clone, PartialEq)]
pub struct LockExpr {
    pub name: String,
    /// Source span of the locked binding name.
    pub name_span: Range<usize>,
    pub body: Block,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NullishExpr {
    pub left: Box<Expression>,
    pub right: Box<Expression>,
    pub span: Range<usize>,
}

/// A member access `object.prop` or optional access `object?.prop`.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberAccess {
    pub object: Expression,
    pub property: String,
    pub optional: bool,
    pub span: Range<usize>,
}

/// An indexing expression `object[index]`.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexExpr {
    pub object: Box<Expression>,
    pub index: Box<Expression>,
    pub span: Range<usize>,
}

/// A range literal `start..<end` (exclusive upper bound) or `start...end`
/// (inclusive upper bound, `end_inclusive`).
#[derive(Debug, Clone, PartialEq)]
pub struct RangeExpr {
    pub start: Box<Expression>,
    pub end: Box<Expression>,
    pub end_inclusive: bool,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TernaryExpr {
    pub condition: Expression,
    pub then_value: Expression,
    pub else_value: Expression,
    pub span: Range<usize>,
}

/// A `match` expression with a wildcard-capable arm list.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchExpr {
    pub value: Expression,
    pub arms: Vec<MatchArm>,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    /// Optional guard: `pattern if condition => body` (spec: control-flow.md).
    /// Evaluated only when the pattern has matched.
    pub guard: Option<Expression>,
    /// The arm body: an expression or a block.
    pub value: MatchBody,
    /// Source span of the whole arm (pattern through value).
    pub span: Range<usize>,
}

/// The body of a `match` arm: `pattern => Expression` or `pattern => { … }`.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchBody {
    Expr(Expression),
    Block(Block),
}

/// A `match` arm pattern: literal, range, enum member, enum member with a
/// payload binding, struct pattern, type pattern, binding name, or-pattern,
/// parenthesized alternative, or the `_` wildcard.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchPattern {
    Literal(Literal),
    Enum(EnumRef),
    EnumPayload {
        enum_name: String,
        variant: String,
        /// One binding per payload parameter, in order; `_` discards a slot.
        /// Only plain identifiers and `_` are supported in payload slots.
        bindings: Vec<String>,
        span: Range<usize>,
    },
    /// A numeric range pattern: `0..<5` or `1...3`.
    Range {
        start: f64,
        end: f64,
        end_inclusive: bool,
        span: Range<usize>,
    },
    /// A struct pattern: `Rect(w, h)` (positional bindings).
    Struct {
        name: String,
        bindings: Vec<MatchPattern>,
        span: Range<usize>,
    },
    /// A type pattern: `string s`, `unknown _`, or `string? p`; with a union
    /// designator `string | int v`. `type_name` is the (possibly rendered
    /// union) designator text.
    TypePattern {
        type_name: String,
        optional: bool,
        binder: PatternBinder,
        span: Range<usize>,
    },
    /// A binding pattern: a bare identifier binds the matched value.
    Binding {
        name: String,
        span: Range<usize>,
    },
    /// An or-pattern: `0 | 1`, `Shape::Circle(_) | Shape::Dot(_)`.
    Or {
        patterns: Vec<MatchPattern>,
        span: Range<usize>,
    },
    Wildcard,
}

/// The binder of a type pattern: `_` (discard) or a name.
#[derive(Debug, Clone, PartialEq)]
pub enum PatternBinder {
    Wildcard,
    Binding(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnaryOp {
    pub operator: UnaryOperator,
    pub operand: Expression,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    String(String),
    Number(f64),
    Boolean(bool),
    Null,
    List(Vec<Expression>),
    Object(Vec<ObjectField>),
}

/// A field in an object literal: `key: value` or a spread `...expr`.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectField {
    Field { name: String, value: Expression },
    Spread { value: Expression },
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinaryOp {
    pub left: Expression,
    pub operator: BinaryOperator,
    pub right: Expression,
    pub span: Range<usize>,
    /// Set by the semantic phase's list-concat annotation pass when both sides
    /// are statically `list` types: the JS codegen then emits an array
    /// concatenation instead of the bare `+` (which would stringify). Mirrors
    /// how `Call.trait_impl` carries a codegen hint back from checking.
    pub list_concat: bool,
}

/// A plain function call (`foo(args)`), an enum payload construction
/// (`Result::Success(args)`, where `callee` is `"Result::Success"`), a trait
/// dispatch call (`Area::area(recv)`, where `callee` is `"Area::area"`), or a
/// method call (`obj.method(args)` where `object` is the receiver).
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub callee: String,
    /// Source span of the bare callee name in a plain call (`foo(...)`); `None`
    /// for enum-constructing (`Color::Red(...)`) and method (`obj.m(...)`)
    /// calls, whose callee has no single identifier span.
    pub callee_span: Option<Range<usize>>,
    /// The receiver of a method call (`obj.method(...)`); `None` for a plain
    /// or enum call.
    pub object: Option<Box<Expression>>,
    /// Method name when `object` is present.
    pub method: Option<String>,
    /// When the callee was written as `obj?.method(...)`: emit `obj?.method()`
    /// so a null receiver short-circuits instead of throwing.
    pub optional: bool,
    pub arguments: Vec<CallArg>,
    pub span: Range<usize>,
    /// When the call is `Trait::method(receiver, ...)` dispatch, the semantic
    /// phase annotates the mangled impl name (`impl_Area_Rectangle_area`) here.
    /// `None` for plain/enum/method calls.
    pub trait_impl: Option<String>,
    /// A written call-site type-argument list — `first<int>(…)` — between the
    /// callee and the argument list (spec `expressions/calls.md`
    /// `CallTypeArgs`). `None` when no list is written; `Some(vec![])` for
    /// `f<>(…)`, which must reach the arity diagnostic, not a comparison.
    pub type_args: Option<Vec<Type>>,
}

/// A single call argument, optionally labeled (`name: expr`).
#[derive(Debug, Clone, PartialEq)]
pub struct CallArg {
    pub name: Option<String>,
    pub value: Expression,
}

/// A call where the callee is an arbitrary expression evaluating to a function
/// value (`xs[0](10)`), as opposed to `Call` which has a static name.
#[derive(Debug, Clone, PartialEq)]
pub struct CallValue {
    pub callee: Box<Expression>,
    pub arguments: Vec<CallArg>,
    pub span: Range<usize>,
    /// Call-site type arguments on a callee that is an arbitrary expression
    /// (`xs[0]<int>(1)`); always `E0807` — a value of function type declares
    /// no type parameters (spec `types/generics.md`).
    pub type_args: Option<Vec<Type>>,
}

impl Call {
    pub fn is_enum(&self) -> bool {
        self.object.is_none() && self.callee.contains("::")
    }

    /// Split `"Result::Success"` into `("Result", "Success")`.
    pub fn enum_parts(&self) -> Option<(&str, &str)> {
        if self.object.is_some() {
            return None;
        }
        self.callee.split_once("::")
    }
}

/// A bare enum member reference: `Theme::Dark`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumRef {
    pub enum_name: String,
    pub variant: String,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfExpr {
    pub condition: Expression,
    pub then_branch: Block,
    pub else_branch: Option<Block>,
    pub span: Range<usize>,
}

#[derive(Debug, Clone)]
pub enum Type {
    String,
    Number,
    Boolean,
    Null,
    /// The loose, dynamic `object` type (object literals).
    Object,
    /// A named type: an alias, an enum, a generic application, or a stdlib
    /// type (`User`, `View`). The second field carries the written type
    /// arguments of an application — `Map<string, int>` is
    /// `Named("Map", [Type::String, Type::Number])` — and is empty when no argument list
    /// was written. Argument well-formedness (count, bare names, bounds) is
    /// checked by the semantic analyzer (spec `types/generics.md`).
    Named(String, Vec<Type>),
    /// A string literal type: `"active"`.
    Literal(String),
    List(Box<Type>),
    Optional(Box<Type>),
    Union(Vec<Type>),
    Intersection(Vec<Type>),
    /// A structural object type: `{ width: number }`. (Removed from the
    /// language by the current spec — maps replaced object types; kept for
    /// legacy ASTs.)
    ObjectType(Vec<(String, Type)>),
    /// A positional tuple type: `(int, string)` (at least two elements).
    Tuple(Vec<Type>),
    /// A function *type*: `fn(a: string): boolean`.
    FnSig {
        params: Vec<Type>,
        ret: Option<Box<Type>>,
    },
    /// An async (promise) result: the return type annotation `async`.
    Async(Box<Type>),
    Any,
}

/// Two named types are the same type when their *names* are: the written type
/// arguments do not take part in type identity, because they are validated
/// separately (arity and bounds, spec E0230/E0801) and because a value's type
/// carries no arguments — `Palette<int>` and the `Palette` a value of it has
/// must still compare equal.
impl PartialEq for Type {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Type::String, Type::String) => true,
            (Type::Number, Type::Number) => true,
            (Type::Boolean, Type::Boolean) => true,
            (Type::Null, Type::Null) => true,
            (Type::Object, Type::Object) => true,
            (Type::Any, Type::Any) => true,
            (Type::Named(a, _), Type::Named(b, _)) => a == b,
            (Type::Literal(a), Type::Literal(b)) => a == b,
            (Type::List(a), Type::List(b)) => a == b,
            (Type::Optional(a), Type::Optional(b)) => a == b,
            (Type::Union(a), Type::Union(b)) => a == b,
            (Type::Intersection(a), Type::Intersection(b)) => a == b,
            (Type::ObjectType(a), Type::ObjectType(b)) => a == b,
            (Type::Tuple(a), Type::Tuple(b)) => a == b,
            (
                Type::FnSig {
                    params: ap,
                    ret: ar,
                },
                Type::FnSig {
                    params: bp,
                    ret: br,
                },
            ) => ap == bp && ar == br,
            (Type::Async(a), Type::Async(b)) => a == b,
            _ => false,
        }
    }
}

impl Type {
    pub fn name(&self) -> String {
        match self {
            Type::String => "string".into(),
            Type::Number => "number".into(),
            Type::Boolean => "bool".into(),
            Type::Null => "null".into(),
            Type::Object => "object".into(),
            Type::Named(n, _) => n.clone(),
            Type::Literal(s) => format!("\"{s}\""),
            Type::List(inner) => format!("List<{}>", inner.name()),
            Type::Optional(inner) => format!("{}?", inner.name()),
            Type::Union(parts) => parts
                .iter()
                .map(|p| p.name())
                .collect::<Vec<_>>()
                .join(" | "),
            Type::Intersection(parts) => parts
                .iter()
                .map(|p| p.name())
                .collect::<Vec<_>>()
                .join(" & "),
            Type::ObjectType(fields) => format!(
                "{{{}}}",
                fields
                    .iter()
                    .map(|(n, t)| format!("{n}: {}", t.name()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Type::Tuple(parts) => format!(
                "({})",
                parts
                    .iter()
                    .map(|p| p.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Type::FnSig { params, ret } => format!(
                "fn({}){}",
                params
                    .iter()
                    .map(|p| p.name())
                    .collect::<Vec<_>>()
                    .join(", "),
                ret.as_ref()
                    .map(|r| format!(": {}", r.name()))
                    .unwrap_or_default()
            ),
            Type::Async(_) => "async".into(),
            Type::Any => "any".into(),
        }
    }

    /// Render a type the way it was written, including its type arguments
    /// (`Map<string, int>`). Diagnostics that quote an application use this;
    /// [`Type::name`] stays the bare head so existing messages keep reading
    /// the same.
    pub fn display(&self) -> String {
        match self {
            Type::Named(n, args) if !args.is_empty() => format!(
                "{n}<{}>",
                args.iter()
                    .map(|a| a.display())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => self.name(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    Not,
    Neg,
    BitNot,
    /// `move expr`: transfer ownership out of a binding.
    Move,
    /// `copy expr`: deep-copy the value.
    Copy,
}

impl UnaryOperator {
    pub fn symbol(&self) -> &'static str {
        match self {
            UnaryOperator::Not => "!",
            UnaryOperator::Neg => "-",
            UnaryOperator::BitNot => "~",
            UnaryOperator::Move => "move",
            UnaryOperator::Copy => "copy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    And,
    Or,
    BitAnd,
    BitOr,
    Xor,
    Shl,
    Shr,
}

impl BinaryOperator {
    pub fn symbol(&self) -> &'static str {
        match self {
            BinaryOperator::Add => "+",
            BinaryOperator::Sub => "-",
            BinaryOperator::Mul => "*",
            BinaryOperator::Div => "/",
            BinaryOperator::Mod => "%",
            BinaryOperator::Pow => "**",
            BinaryOperator::Eq => "==",
            BinaryOperator::Neq => "!=",
            BinaryOperator::Lt => "<",
            BinaryOperator::Gt => ">",
            BinaryOperator::Lte => "<=",
            BinaryOperator::Gte => ">=",
            BinaryOperator::And => "and",
            BinaryOperator::Or => "or",
            BinaryOperator::BitAnd => "&",
            BinaryOperator::BitOr => "|",
            BinaryOperator::Xor => "^",
            BinaryOperator::Shl => "<<",
            BinaryOperator::Shr => ">>",
        }
    }
}
