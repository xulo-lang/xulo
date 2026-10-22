use std::fmt;

/// xulo IR 类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrType {
    Void,
    Bool,
    I64,
    F64,
    Pointer,
}

impl IrType {
    pub fn is_void(&self) -> bool {
        matches!(self, IrType::Void)
    }
}

/// IR 标签（用于控制流）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label(pub usize);

/// 局部变量ID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);

/// 函数ID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuncId(pub usize);

/// IR 操作数
#[derive(Debug, Clone)]
pub enum Operand {
    /// 常量
    Const(IrValue),
    /// 局部变量
    Local(LocalId),
}

/// IR 值
#[derive(Debug, Clone)]
pub enum IrValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    String(usize), // 字符串表索引
    Null,
}

/// 运行时函数
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeFn {
    Print,
    PrintInt,
    PrintFloat,
    PrintValue,
    Panic,
    AllocObject,
    AllocArray,
    StringConcat,
    /// 带标签的值格式化成字符串（多参数 print 的参数拼接 / str() 转换）
    Stringify,
    ArrayPush,
    ArrayLen,
    ArrayGet,
    ArrayGetTag,
    ArraySet,
    ArrayConcat,
    ObjectGet,
    /// 读对象字段的运行时标签（派生 ToString 按字段真实标签渲染，
    /// 字段值 0 分不清 null 与整数 0）
    ObjectGetTag,
    ObjectSet,
    /// 枚举值：分配 + payload 写入 + match 的 tag/payload 读取
    AllocEnum,
    EnumSet,
    EnumVariant,
    EnumGet,
    EnumGetTag,
    /// 动态数值转 f64（match 绑定等标签未知的操作数）
    AsF64,
    /// 注册 f64 位模式，供打印端的自动检测（tag=-1）还原小数
    RegisterFloat,
    /// 闭包捕获单元：分配一个 cell（持有原始位模式的 i64 槽）
    AllocCell,
    /// 读 cell 的原始位模式
    CellGet,
    /// 读 cell 中的 f64 位模式（返回真正的 f64，避免按位读回时走整数槽）
    CellGetF64,
    /// 写 cell
    CellSet,
    /// 分配捕获环境（容纳 n 个 cell 指针）
    AllocEnv,
    /// 向环境槽位写入 cell 指针
    EnvSet,
    /// 读环境槽位的 cell 指针（env 为空/越界时返回 0）
    EnvGet,
    /// 分配闭包值（函数地址 + 环境指针）
    AllocClosure,
    /// 读闭包的函数地址
    ClosureFunc,
    /// 读闭包的环境指针
    ClosureEnv,
    /// 校验值是可调用的闭包（否则 panic），返回闭包指针本身
    CheckCallable,
    /// 正则：`RegExp::compile` / 实例查询 / 字符串替换 / 大写
    RegexCompile,
    RegexTest,
    RegexFind,
    RegexFindAll,
    RegexMatchGroup,
    RegexReplace,
    RegexReplaceWith,
    StringToUppercase,
    /// match 字面量 pattern 判定（返回 1/0）
    LiteralMatch,
    /// 区间表达式 `0..<3` / `1...4` 求值为列表
    RangeList,
    /// 命名空间内建（`std::math::*` / `std::time::*`）：携带已映射好的运行时符号名
    Intrinsic(&'static str),
}

/// FFI 边界参数的 C ABI 类（spec ffi.md 的 ABI-safe 表逐项落到 SysV 参数类）。
/// xulo 内部整数一律 I64 槽，窄类只存在于 extern 边界上：调用点按此类
/// 截断/扩展，callee 按 C 签名读写。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrAbiParam {
    /// C `_Bool`（1 字节，0/1）
    Bool,
    S8,
    S16,
    S32,
    S64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
}

/// 一条 extern 导入声明（`extern "C" { fn … }` 内的函数签名）
#[derive(Debug, Clone)]
pub struct IrExternFn {
    /// 要在进程/链接器里查找的 C 符号名（`as` 别名的目标名）
    pub symbol: String,
    pub params: Vec<IrAbiParam>,
    /// `None` = `unit` 返回
    pub return_type: Option<IrAbiParam>,
    /// ABI 标签（`"C"` / `"system"`；当前注册表两者都是 SysV）
    pub abi: String,
}

/// IR 指令
#[derive(Debug, Clone)]
pub enum Instruction {
    // 常量加载
    Const {
        dst: LocalId,
        value: IrValue,
    },

    // 算术运算
    Add {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Sub {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Mul {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Div {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Mod {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Pow {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Neg {
        dst: LocalId,
        operand: Operand,
    },

    // 位运算
    BitAnd {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    BitOr {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Xor {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Shl {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Shr {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    BitNot {
        dst: LocalId,
        operand: Operand,
    },

    // 比较运算
    Eq {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Neq {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Lt {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Gt {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Lte {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Gte {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },

    // 逻辑运算
    And {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Or {
        dst: LocalId,
        left: Operand,
        right: Operand,
    },
    Not {
        dst: LocalId,
        operand: Operand,
    },

    // 控制流
    Branch {
        condition: Operand,
        then_label: Label,
        else_label: Label,
    },
    Jump(Label),
    Return(Option<Operand>),

    // 函数调用
    Call {
        dst: Option<LocalId>,
        func: FuncId,
        args: Vec<Operand>,
    },
    RuntimeCall {
        dst: Option<LocalId>,
        func: RuntimeFn,
        args: Vec<Operand>,
    },

    // FFI 导入调用：按 `extern_idx` 索引 `IrModule.externs` 的签名走 C ABI
    // （JIT 经 dlsym 解析符号，AOT 留成 undefined 由系统链接器解析）
    ExternCall {
        dst: Option<LocalId>,
        extern_idx: usize,
        args: Vec<Operand>,
    },

    // 闭包
    /// dst = MakeClosure(func, env)：打包函数地址与捕获环境
    MakeClosure {
        dst: LocalId,
        func: FuncId,
        env: Operand,
    },
    /// dst = callee(env, a0..aN)：经函数值间接调用（callee 是闭包指针）
    CallIndirect {
        dst: Option<LocalId>,
        callee: Operand,
        args: Vec<Operand>,
    },

    // 局部变量
    GetLocal {
        dst: LocalId,
        src: LocalId,
    },
    SetLocal {
        dst: LocalId,
        src: Operand,
    },

    // 对象操作
    NewObject {
        dst: LocalId,
        fields: Vec<(String, Operand, i64)>,
    },
    GetField {
        dst: LocalId,
        object: Operand,
        field: String,
    },
    SetField {
        object: Operand,
        field: String,
        value: Operand,
        tag: i64,
    },

    // 数组操作
    NewArray {
        dst: LocalId,
        elements: Vec<(Operand, i64)>,
    },
    GetIndex {
        dst: LocalId,
        array: Operand,
        index: Operand,
    },
    SetIndex {
        array: Operand,
        index: Operand,
        value: Operand,
        tag: i64,
    },

    // 标签
    Label(Label),

    // 空操作
    Nop,
}

/// IR 函数
#[derive(Debug, Clone)]
pub struct IrFunction {
    pub name: String,
    pub params: Vec<(LocalId, IrType)>,
    pub return_type: IrType,
    pub locals: Vec<(LocalId, IrType)>,
    pub instructions: Vec<Instruction>,
    pub labels: Vec<Label>,
    /// 闭包入口：签名固定为 `(env, a0..a7) -> i64`，经 CallIndirect 间接调用。
    /// 实参少于 8 个时由调用方以 0 补齐；无返回值时也返回 0。
    pub closure_entry: bool,
    /// 下一个可用的 LocalId，从 params.len() 开始以避免与参数冲突
    next_local_id: usize,
}

/// IR 模块
#[derive(Debug, Clone)]
pub struct IrModule {
    pub functions: Vec<IrFunction>,
    pub entry_point: FuncId,
    pub strings: Vec<String>,
    /// FFI 导入声明（extern 块按出现顺序收集；`Instruction::ExternCall` 按下标引用）
    pub externs: Vec<IrExternFn>,
}

impl Default for IrModule {
    fn default() -> Self {
        Self::new()
    }
}

impl IrModule {
    pub fn new() -> Self {
        IrModule {
            functions: Vec::new(),
            entry_point: FuncId(0),
            strings: Vec::new(),
            externs: Vec::new(),
        }
    }

    pub fn add_function(&mut self, func: IrFunction) -> FuncId {
        let id = FuncId(self.functions.len());
        self.functions.push(func);
        id
    }

    pub fn add_string(&mut self, s: String) -> usize {
        let idx = self.strings.len();
        self.strings.push(s);
        idx
    }
}

impl IrFunction {
    pub fn new(name: String, params: Vec<(LocalId, IrType)>, return_type: IrType) -> Self {
        let next_local_id = params.len();
        IrFunction {
            name,
            params,
            return_type,
            locals: Vec::new(),
            instructions: Vec::new(),
            labels: Vec::new(),
            closure_entry: false,
            next_local_id,
        }
    }

    pub fn add_local(&mut self, ty: IrType) -> LocalId {
        let id = LocalId(self.next_local_id);
        self.next_local_id += 1;
        self.locals.push((id, ty));
        id
    }

    pub fn add_label(&mut self) -> Label {
        let label = Label(self.labels.len());
        self.labels.push(label);
        label
    }
}

impl fmt::Display for IrType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrType::Void => write!(f, "void"),
            IrType::Bool => write!(f, "bool"),
            IrType::I64 => write!(f, "i64"),
            IrType::F64 => write!(f, "f64"),
            IrType::Pointer => write!(f, "ptr"),
        }
    }
}

impl fmt::Display for IrValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrValue::Bool(b) => write!(f, "{}", b),
            IrValue::I64(n) => write!(f, "{}", n),
            IrValue::F64(n) => write!(f, "{}", n),
            IrValue::String(idx) => write!(f, "str[{}]", idx),
            IrValue::Null => write!(f, "null"),
        }
    }
}
