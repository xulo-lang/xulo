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
    ToString,
    ArrayPush,
    ArrayLen,
    ArrayGet,
    ArrayGetTag,
    ArraySet,
    ArrayConcat,
    ObjectGet,
    ObjectSet,
}

/// IR 指令
#[derive(Debug, Clone)]
pub enum Instruction {
    // 常量加载
    Const { dst: LocalId, value: IrValue },

    // 算术运算
    Add { dst: LocalId, left: Operand, right: Operand },
    Sub { dst: LocalId, left: Operand, right: Operand },
    Mul { dst: LocalId, left: Operand, right: Operand },
    Div { dst: LocalId, left: Operand, right: Operand },
    Neg { dst: LocalId, operand: Operand },

    // 比较运算
    Eq { dst: LocalId, left: Operand, right: Operand },
    Neq { dst: LocalId, left: Operand, right: Operand },
    Lt { dst: LocalId, left: Operand, right: Operand },
    Gt { dst: LocalId, left: Operand, right: Operand },
    Lte { dst: LocalId, left: Operand, right: Operand },
    Gte { dst: LocalId, left: Operand, right: Operand },

    // 逻辑运算
    And { dst: LocalId, left: Operand, right: Operand },
    Or { dst: LocalId, left: Operand, right: Operand },
    Not { dst: LocalId, operand: Operand },

    // 控制流
    Branch { condition: Operand, then_label: Label, else_label: Label },
    Jump(Label),
    Return(Option<Operand>),

    // 函数调用
    Call { dst: Option<LocalId>, func: FuncId, args: Vec<Operand> },
    RuntimeCall { dst: Option<LocalId>, func: RuntimeFn, args: Vec<Operand> },

    // 局部变量
    GetLocal { dst: LocalId, src: LocalId },
    SetLocal { dst: LocalId, src: Operand },

    // 对象操作
    NewObject { dst: LocalId, fields: Vec<(String, Operand, i64)> },
    GetField { dst: LocalId, object: Operand, field: String },
    SetField { object: Operand, field: String, value: Operand, tag: i64 },

    // 数组操作
    NewArray { dst: LocalId, elements: Vec<(Operand, i64)> },
    GetIndex { dst: LocalId, array: Operand, index: Operand },
    SetIndex { array: Operand, index: Operand, value: Operand, tag: i64 },

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
    /// 下一个可用的 LocalId，从 params.len() 开始以避免与参数冲突
    next_local_id: usize,
}

/// IR 模块
#[derive(Debug, Clone)]
pub struct IrModule {
    pub functions: Vec<IrFunction>,
    pub entry_point: FuncId,
    pub strings: Vec<String>,
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
