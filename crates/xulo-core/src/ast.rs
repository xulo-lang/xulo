use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Fn(FnDef),
    Let(LetBinding),
    Return(ReturnStmt),
    For(ForStmt),
    While(WhileStmt),
    Assign(AssignStmt),
    TypeAlias(TypeAlias),
    Enum(EnumDef),
    Expr(ExprStmt),
    Block(Block),
    Try(TryStmt),
    Throw(Expression),
    Import(ImportStmt),
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub type_annotation: Option<Type>,
    pub default: Option<Expression>,
    pub span: Range<usize>,
}

/// A `let` or `const` binding. `const` bindings may not be reassigned.
#[derive(Debug, Clone, PartialEq)]
pub struct LetBinding {
    pub name: String,
    /// Source span of the binding name (for name-related diagnostics).
    pub name_span: Range<usize>,
    pub type_annotation: Option<Type>,
    pub value: Option<Expression>,
    pub is_const: bool,
    /// `@Memo` value memoization: `@Memo([deps]) let x = expr`. The native
    /// interpreter caches the computed value per (site, deps) and reuses it
    /// when the deps are unchanged; the JS target recomputes on every render
    /// (documented no-op for now).
    pub memo: bool,
    /// The memo dependency expressions (`None` only when `memo` is `false`; a
    /// bare `@Memo` is normalized to an empty list = cache forever).
    pub memo_deps: Option<Vec<Expression>>,
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

/// An `impl Trait for Type` block: provides method bodies for a concrete named
/// type. `self` may be the first parameter of each method and is bound to the
/// receiver's type during checking.
#[derive(Debug, Clone, PartialEq)]
pub struct ImplDecl {
    pub trait_name: String,
    pub type_name: String,
    pub methods: Vec<FnDef>,
    pub span: Range<usize>,
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
    pub variants: Vec<EnumVariant>,
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

/// A `try { ... } catch (e) { ... }` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct TryStmt {
    pub try_block: Block,
    pub catch_var: String,
    /// Source span of the catch-variable name (for name-related diagnostics).
    pub catch_var_span: Range<usize>,
    pub catch_block: Block,
}

/// An `import` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportStmt {
    pub source: String,
    pub spec: ImportSpec,
    /// `import type { ... }`: erased at runtime, only feeds the type checker.
    pub type_only: bool,
}

/// What `import` brings in: a namespace (`* as ns`), named bindings
/// (`{ a, b as c }`), or a bare side-effect import (`import "..."`).
#[derive(Debug, Clone, PartialEq)]
pub enum ImportSpec {
    Namespace(String),
    Named(Vec<(String, Option<String>)>),
    Bare,
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
    pub value: Expression,
    /// Source span of the whole arm (pattern through value).
    pub span: Range<usize>,
}

/// A `match` arm pattern: literal, enum member, enum member with a payload
/// binding, or the `_` wildcard.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchPattern {
    Literal(Literal),
    Enum(EnumRef),
    EnumPayload {
        enum_name: String,
        variant: String,
        /// One binding per payload parameter, in order; `_` discards a slot.
        bindings: Vec<String>,
        span: Range<usize>,
    },
    Wildcard,
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

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    String,
    Number,
    Boolean,
    Null,
    /// The loose, dynamic `object` type (object literals).
    Object,
    /// A named type: an alias, an enum, or a stdlib type (`User`, `View`).
    Named(String),
    /// A string literal type: `"active"`.
    Literal(String),
    List(Box<Type>),
    Optional(Box<Type>),
    Union(Vec<Type>),
    Intersection(Vec<Type>),
    /// A structural object type: `{ width: number }`.
    ObjectType(Vec<(String, Type)>),
    /// A function *type*: `fn(a: string): boolean`.
    FnSig {
        params: Vec<Type>,
        ret: Option<Box<Type>>,
    },
    /// An async (promise) result: the return type annotation `async`.
    Async(Box<Type>),
    Any,
}

impl Type {
    pub fn name(&self) -> String {
        match self {
            Type::String => "string".into(),
            Type::Number => "number".into(),
            Type::Boolean => "boolean".into(),
            Type::Null => "null".into(),
            Type::Object => "object".into(),
            Type::Named(n) => n.clone(),
            Type::Literal(s) => format!("\"{s}\""),
            Type::List(inner) => format!("list<{}>", inner.name()),
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    Not,
    Neg,
}

impl UnaryOperator {
    pub fn symbol(&self) -> &'static str {
        match self {
            UnaryOperator::Not => "!",
            UnaryOperator::Neg => "-",
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
        }
    }
}
