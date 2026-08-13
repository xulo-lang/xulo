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
    Expr(Expression),
    Block(Block),
    Try(TryStmt),
    Throw(Expression),
    Import(ImportStmt),
    Export(ExportStmt),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub body: Block,
    pub is_async: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub type_annotation: Option<Type>,
    pub default: Option<Expression>,
}

/// A `let` or `const` binding. `const` bindings may not be reassigned.
#[derive(Debug, Clone, PartialEq)]
pub struct LetBinding {
    pub name: String,
    pub type_annotation: Option<Type>,
    pub value: Option<Expression>,
    pub is_const: bool,
}

/// A reassignment statement: `name = expr`.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignStmt {
    pub name: String,
    pub value: Expression,
}

/// A `type` alias declaration. Type parameters are parsed and erased.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeAlias {
    pub name: String,
    pub type_params: Vec<String>,
    pub type_: Type,
}

/// An `enum` declaration with optional per-variant payload types.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub variants: Vec<EnumVariant>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub payload: Option<Type>,
    /// Optional field name for the payload: `Submit(data: object)`.
    pub payload_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStmt {
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForStmt {
    pub iter_var: String,
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
/// (`{ a, b as c }`), a default binding (`Foo from "..."`), or a bare
/// side-effect import (`import "..."`).
#[derive(Debug, Clone, PartialEq)]
pub enum ImportSpec {
    Namespace(String),
    Named(Vec<(String, Option<String>)>),
    Default(String),
    Bare,
}

/// An `export` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportStmt {
    pub item: ExportItem,
}

/// What `export` exposes: a declaration (`export fn/let/const`), a default
/// export, a bare name list (`export { a, b }`), or a type/enum/alias.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportItem {
    Fn(FnDef),
    Let(LetBinding),
    Default(Box<ExportItem>),
    Names(Vec<String>),
    Type(TypeAlias),
    Enum(EnumDef),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Literal(Literal),
    Identifier(String),
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
    Await(Box<Expression>),
    /// An anonymous function literal: `fn(a: number): number { ... }`.
    FnExpr(Box<FnExpr>),
    /// A spread element in a list literal: `[1, ...rest, 3]`.
    Spread(Box<Expression>),
    /// Calling a function value held in an arbitrary expression:
    /// `xs[0](10)`, `getHandler()(event)`, `(fn() {...})(5)`.
    CallValue(Box<CallValue>),
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct NullishExpr {
    pub left: Box<Expression>,
    pub right: Box<Expression>,
}

/// A member access `object.prop` or optional access `object?.prop`.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberAccess {
    pub object: Expression,
    pub property: String,
    pub optional: bool,
}

/// An indexing expression `object[index]`.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexExpr {
    pub object: Box<Expression>,
    pub index: Box<Expression>,
}

/// A range literal `start..<end` (exclusive upper bound).
#[derive(Debug, Clone, PartialEq)]
pub struct RangeExpr {
    pub start: Box<Expression>,
    pub end: Box<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TernaryExpr {
    pub condition: Expression,
    pub then_value: Expression,
    pub else_value: Expression,
}

/// A `match` expression with a wildcard-capable arm list.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchExpr {
    pub value: Expression,
    pub arms: Vec<MatchArm>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    pub value: Expression,
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
        binding: String,
    },
    Wildcard,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnaryOp {
    pub operator: UnaryOperator,
    pub operand: Expression,
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
}

/// A plain function call (`foo(args)`), an enum payload construction
/// (`Result::Success(args)`, where `callee` is `"Result::Success"`), or a
/// method call (`obj.method(args)` where `object` is the receiver).
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub callee: String,
    /// The receiver of a method call (`obj.method(...)`); `None` for a plain
    /// or enum call.
    pub object: Option<Box<Expression>>,
    /// Method name when `object` is present.
    pub method: Option<String>,
    pub arguments: Vec<CallArg>,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfExpr {
    pub condition: Expression,
    pub then_branch: Block,
    pub else_branch: Option<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    String,
    Number,
    Boolean,
    Null,
    /// The loose, dynamic `object` type (object literals).
    Object,
    /// A named type: an alias, an enum, or a stdlib type (`User`, `Component`).
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
    FnSig { params: Vec<Type>, ret: Option<Box<Type>> },
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