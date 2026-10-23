//! IR 生成：AST -> xulo IR 的 lowering。
//!
//! 拆分：`mod.rs` 入口与共享推断，`expr`/`stmt`/`func`/`decl`/`consteval`
//! 按 lowering 面分工（方法以 `impl IrGenerator` 分块定义在各文件）。
mod consteval;
mod decl;
mod expr;
mod func;
mod stmt;
mod traits;
mod types;

use std::collections::{HashMap, HashSet};

use xulo_core::ast::*;
use xulo_core::error::{ErrorKind, XuloError};
use xulo_core::ir::*;

use crate::module::LoadedModules;

/// IR 生成器
pub struct IrGenerator {
    module: IrModule,
    current_func: Option<usize>,
    locals: HashMap<String, LocalId>,
    local_types: HashMap<LocalId, IrType>,
    /// 跟踪变量的源级类型（用于 print 等需要类型区分的场景）
    source_types: HashMap<String, Type>,
    /// 函数名 -> AST 返回类型注解（用于泛型函数的返回类型推断）
    func_return_types: HashMap<String, Option<Type>>,
    /// 用户函数的形参表（名字 + 默认表达式）：命名参数重排与缺省参数补齐
    func_params: HashMap<String, Vec<(String, Option<Expression>)>>,
    label_map: HashMap<String, Label>,
    loop_stack: Vec<LoopContext>,
    /// struct 定义（结构体构造需要字段顺序与默认值）
    structs: HashMap<String, Vec<StructField>>,
    /// `Type::method` -> IR 函数（impl 固有方法）
    methods: HashMap<String, FuncId>,
    /// 派发名 `impl_{Trait}_{Type}_{method}` -> impl 函数的 IR 函数。
    /// 语义层把 `Trait::method(recv)` 与 `str(v)` 标注成派发名；AOT 在此
    /// 静态解析（派生 `#Derive(ToString)` 注册到同一张表）。
    trait_impls: HashMap<String, FuncId>,
    /// 方法名 -> 定义它的类型列表（唯一时可按方法名分派）
    method_types: HashMap<String, Vec<String>>,
    /// IR 函数名 -> self 的源级类型（方法体里 self 字段的类型推断）
    self_types: HashMap<String, Type>,
    /// 返回值以 F64 位模式穿过整数签名的函数（Math::sqrt 落点，print 需要 tag=2）
    float_bit_returns: HashSet<String>,
    /// 枚举定义：类型名 -> 按声明序的变体名（序号即 match/构造的 tag）。
    /// 内置 `Result` 预置，模块自己的 `enum Result` 覆盖之。
    enums: HashMap<String, Vec<String>>,
    /// match 绑定名 -> 运行时标签操作数（payload 是 int 还是 f64 位模式，
    /// 编译期不知道，构造侧的 tag 在绑定时摘出来伴随使用）
    dyn_tag_of: HashMap<String, Operand>,
    /// 本作用域需要装箱（cell 承载）的名字：被嵌套字面量自由引用的绑定，
    /// 以及闭包体里来自 env 的捕获名
    boxed_names: HashSet<String>,
    /// 装箱名字的逻辑类型（I64/F64/Bool/Pointer）：cell 的本地槽是
    /// Pointer，读出时按逻辑类型还原算术/标签语义
    boxed_logical: HashMap<String, IrType>,
    /// 闭包体当前的 env 参数（LocalId(0)）；顶层函数体为 None
    closure_env: Option<LocalId>,
    /// 顶层函数名 -> 惰性生成的 thunk 闭包（把直接调用协议适配成
    /// CallIndirect 的 `(env, a0..a7)` 协议）
    func_thunks: HashMap<String, FuncId>,
    /// 内嵌 fn / 闭包体的 IR 名计数（同名函数在不同作用域需要唯一名）
    inner_fn_counter: usize,
    /// 模块 id -> 名字解析上下文（单文件程序只有入口模块 `""`）
    modules: HashMap<String, ModuleCtx>,
    /// 当前正在注册/生成正文的模块 id
    current_module: String,
    /// 模块级 `const`：IR 名 -> 定义（使用点内联初始化表达式）
    consts: HashMap<String, ConstDef>,
    /// const 内联的递归栈（自引用检测）
    const_stack: Vec<String>,
    /// FFI 导入：解析后的本地名 -> `IrModule.externs` 下标（extern 块按
    /// 模块解析前缀后登记；调用点在局部绑定之后、用户函数之前查这张表）
    extern_map: HashMap<String, usize>,
}

struct LoopContext {
    break_label: Label,
    continue_label: Label,
}

/// 进入闭包体前保存的外层作用域状态
struct ScopeSnapshot {
    current_func: Option<usize>,
    locals: HashMap<String, LocalId>,
    local_types: HashMap<LocalId, IrType>,
    source_types: HashMap<String, Type>,
    label_map: HashMap<String, Label>,
    dyn_tag_of: HashMap<String, Operand>,
    loop_stack: Vec<LoopContext>,
    boxed_names: HashSet<String>,
    boxed_logical: HashMap<String, IrType>,
    closure_env: Option<LocalId>,
}

/// 一个字面量函数（fn 表达式 / 箭头 / 内嵌 fn 语句）的捕获分析结果
struct LiteralCapture {
    /// 自由变量（按首次出现顺序去重）——创建时推入 env，体入口处加载
    free: Vec<String>,
    /// 本作用域绑定且被嵌套字面量自由引用 → 需要装箱成 cell
    boxed: HashSet<String>,
}

impl Default for IrGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// 带 `#Derive` 声明的可派生目标
enum DerivedDecl<'a> {
    Struct(&'a [StructField]),
    Enum(&'a [EnumVariant]),
}

/// 一个模块在 IR 生成期的名字解析上下文。入口模块（id `""`）的顶层声明
/// 保留原名；其余模块按 `{模块 id}::名字` 加前缀，避免跨模块同名冲突。
/// `use` 成员导入与 `use … as`/`mod` 命名空间绑定把本地名映射到目标模块
/// （语义层已解析好，见 `Module::uses` / `Module::mods`）。
struct ModuleCtx {
    prefix: String,
    /// 命名空间本地名 -> 目标模块 id
    namespaces: HashMap<String, String>,
    /// 成员导入本地名 -> (目标模块 id, 导出成员名)
    aliases: HashMap<String, (String, String)>,
    /// 本模块声明的值名（函数/常量）
    own_values: HashSet<String>,
    /// 本模块声明的类型名（struct/enum/trait/alias）
    own_types: HashSet<String>,
}

impl ModuleCtx {
    fn new(module_id: &str, program: &Program) -> Self {
        let prefix = if module_id.is_empty() {
            String::new()
        } else {
            format!("{module_id}::")
        };
        let mut ctx = ModuleCtx {
            prefix,
            namespaces: HashMap::new(),
            aliases: HashMap::new(),
            own_values: HashSet::new(),
            own_types: HashSet::new(),
        };
        for stmt in &program.statements {
            match stmt {
                Statement::Fn(f) => {
                    ctx.own_values.insert(f.name.clone());
                }
                Statement::Let(b) => {
                    ctx.own_values.insert(b.name.clone());
                }
                Statement::Struct(def) => {
                    ctx.own_types.insert(def.name.clone());
                }
                Statement::Enum(def) => {
                    ctx.own_types.insert(def.name.clone());
                }
                Statement::Trait(t) => {
                    ctx.own_types.insert(t.name.clone());
                }
                Statement::TypeAlias(t) => {
                    ctx.own_types.insert(t.name.clone());
                }
                Statement::Export(export) => match &export.item {
                    ExportItem::Fn(f) => {
                        ctx.own_values.insert(f.name.clone());
                    }
                    ExportItem::Let(b) => {
                        ctx.own_values.insert(b.name.clone());
                    }
                    ExportItem::Struct(def) => {
                        ctx.own_types.insert(def.name.clone());
                    }
                    ExportItem::Enum(def) => {
                        ctx.own_types.insert(def.name.clone());
                    }
                    ExportItem::Trait(t) => {
                        ctx.own_types.insert(t.name.clone());
                    }
                    ExportItem::Type(t) => {
                        ctx.own_types.insert(t.name.clone());
                    }
                    ExportItem::Names(names) => {
                        ctx.own_values.extend(names.iter().cloned());
                    }
                },
                _ => {}
            }
        }
        ctx
    }
}

/// 模块级 `const`：不可变，AOT 在使用点内联初始化表达式（在定义模块的
/// 上下文里生成，因此跨模块 `use` 的常量也能解析）。
#[derive(Clone)]
struct ConstDef {
    module_id: String,
    expr: Expression,
    /// 初始化表达式可静态判定的值标签（-1 = 运行时检测）
    tag: i64,
    /// 初始化表达式可静态判定的源级类型（仅字面量/取负）
    ty: Option<Type>,
}

impl IrGenerator {
    pub fn new() -> Self {
        IrGenerator {
            module: IrModule::new(),
            current_func: None,
            locals: HashMap::new(),
            local_types: HashMap::new(),
            source_types: HashMap::new(),
            func_return_types: HashMap::new(),
            func_params: HashMap::new(),
            label_map: HashMap::new(),
            loop_stack: Vec::new(),
            structs: HashMap::new(),
            methods: HashMap::new(),
            trait_impls: HashMap::new(),
            method_types: HashMap::new(),
            self_types: HashMap::new(),
            float_bit_returns: HashSet::new(),
            enums: HashMap::new(),
            dyn_tag_of: HashMap::new(),
            boxed_names: HashSet::new(),
            boxed_logical: HashMap::new(),
            closure_env: None,
            func_thunks: HashMap::new(),
            inner_fn_counter: 0,
            modules: HashMap::new(),
            current_module: String::new(),
            consts: HashMap::new(),
            const_stack: Vec::new(),
            extern_map: HashMap::new(),
        }
    }

    /// 单文件入口：一个 id 为 `""` 的入口模块。
    pub fn generate(&mut self, program: &Program) -> Result<IrModule, XuloError> {
        self.modules
            .insert(String::new(), ModuleCtx::new("", program));
        self.compile_programs(&[(String::new(), program)])
    }

    /// 多模块入口：编译整棵模块树（入口模块 id 为 `""`，子模块/内联模块
    /// 按 `mod` 路径命名）。模块按加载器的初始化顺序传入。
    pub fn generate_modules(&mut self, loaded: &LoadedModules) -> Result<IrModule, XuloError> {
        for m in &loaded.modules {
            let mut ctx = ModuleCtx::new(&m.id, &m.program);
            for (local, child_idx) in &m.mods {
                ctx.namespaces
                    .insert(local.clone(), loaded.modules[*child_idx].id.clone());
            }
            for u in &m.uses {
                match &u.member {
                    Some(member) => {
                        ctx.aliases
                            .insert(u.local.clone(), (u.module_id.clone(), member.clone()));
                    }
                    None => {
                        ctx.namespaces.insert(u.local.clone(), u.module_id.clone());
                    }
                }
            }
            self.modules.insert(m.id.clone(), ctx);
        }
        let programs: Vec<(String, &Program)> = loaded
            .modules
            .iter()
            .map(|m| (m.id.clone(), &m.program))
            .collect();
        self.compile_programs(&programs)
    }

    /// 模块集合的编译流水线：注册 -> 定义收集 -> 派生/impl 注册 ->
    /// 函数体 -> 派生正文。每个阶段切换 `current_module` 以解析本地名。
    fn compile_programs(&mut self, programs: &[(String, &Program)]) -> Result<IrModule, XuloError> {
        // 1. 注册所有函数（不生成函数体）
        for (id, program) in programs {
            self.current_module = id.clone();
            for stmt in &program.statements {
                match stmt {
                    Statement::Fn(f) => {
                        self.register_function(f)?;
                    }
                    Statement::Export(export) => {
                        if let ExportItem::Fn(f) = &export.item {
                            self.register_function(f)?;
                        }
                    }
                    // FFI 导入先于函数体登记：正文里的直接调用要能解析
                    Statement::ExternBlock(b) => self.collect_externs(b),
                    _ => {}
                }
            }
        }

        // 2. struct / enum / 模块级 const 定义（键为规范化后的名字）
        //    内置 Result 预置，程序自己的 enum 覆盖同名（spec：Result 既是
        //    prelude 类型，也是程序可以重定义的名字）
        self.enums.insert(
            "Result".to_string(),
            vec!["Ok".to_string(), "Err".to_string()],
        );
        for (id, program) in programs {
            self.current_module = id.clone();
            for stmt in &program.statements {
                match stmt {
                    Statement::Struct(def) => self.collect_struct(def),
                    Statement::Enum(def) => self.collect_enum(def),
                    Statement::Let(b) => self.collect_const(id, b),
                    Statement::Export(export) => match &export.item {
                        ExportItem::Struct(def) => self.collect_struct(def),
                        ExportItem::Enum(def) => self.collect_enum(def),
                        ExportItem::Let(b) => self.collect_const(id, b),
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        // 3. `#Derive(ToString)`：语义层只登记派发名，不生成 AST impl；
        //    这里先注册派生函数（正文在所有注册完成后统一生成，可互引）。
        //    先于 impl 方法注册：手写固有方法同名时覆盖派发表项，与解释器
        //    「派生只在未有 toString 时落到结构体对象」的优先级一致。
        self.register_derived_decls(programs);

        // 4. 注册 impl 方法：按 `{规范化类型}::{method}` 命名，trait impl 另有
        //    包全局唯一的派发名（语义层的 `Trait::method`/`str` 标注用它，
        //    用源级类型名，与跨模块 impl 表一致）。
        self.register_impl_methods(programs)?;

        // 5. 返回 Math::sqrt 位模式的函数：print 端要用 tag=2 还原小数
        for (id, program) in programs {
            self.current_module = id.clone();
            for stmt in &program.statements {
                match stmt {
                    Statement::Fn(f) => {
                        let name = self.resolve_value(&f.name);
                        self.note_float_bit_returns(&name, f);
                    }
                    Statement::Export(export) => {
                        if let ExportItem::Fn(f) = &export.item {
                            let name = self.resolve_value(&f.name);
                            self.note_float_bit_returns(&name, f);
                        }
                    }
                    Statement::Impl(imp) => {
                        let type_key = self.resolve_type(&imp.type_name);
                        for m in &imp.methods {
                            let key = format!("{type_key}::{}", m.name);
                            self.note_float_bit_returns(&key, m);
                        }
                    }
                    _ => {}
                }
            }
        }

        // 6. 入口：入口模块的 `main`（其余模块的函数名带模块前缀）
        self.current_module = programs
            .iter()
            .find(|(id, _)| id.is_empty())
            .map(|(id, _)| id.clone())
            .unwrap_or_default();
        let main_idx = self.module.functions.iter().position(|f| f.name == "main");
        if main_idx.is_none() {
            // 没有 main 函数，创建一个空的
            let main_func = IrFunction::new("main".to_string(), Vec::new(), IrType::Void);
            self.module.add_function(main_func);
        }
        self.module.entry_point = FuncId(main_idx.unwrap_or(self.module.functions.len() - 1));

        // 7. 生成所有函数体（含 impl 方法），记录每个函数所属模块
        let mut func_defs: Vec<(usize, FnDef, String)> = Vec::new();
        for (id, program) in programs {
            self.current_module = id.clone();
            for stmt in &program.statements {
                match stmt {
                    Statement::Fn(f) => {
                        let name = self.resolve_value(&f.name);
                        if let Some(idx) = self
                            .module
                            .functions
                            .iter()
                            .position(|ir_f| ir_f.name == name)
                        {
                            func_defs.push((idx, f.clone(), id.clone()));
                        }
                    }
                    Statement::Export(export) => {
                        if let ExportItem::Fn(f) = &export.item {
                            let name = self.resolve_value(&f.name);
                            if let Some(idx) = self
                                .module
                                .functions
                                .iter()
                                .position(|ir_f| ir_f.name == name)
                            {
                                func_defs.push((idx, f.clone(), id.clone()));
                            }
                        }
                    }
                    Statement::Impl(imp) => {
                        let type_key = self.resolve_type(&imp.type_name);
                        for m in &imp.methods {
                            let key = format!("{type_key}::{}", m.name);
                            if let Some(idx) = self
                                .module
                                .functions
                                .iter()
                                .position(|ir_f| ir_f.name == key)
                            {
                                func_defs.push((idx, m.clone(), id.clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        for (idx, func_def, module_id) in func_defs {
            self.current_module = module_id;
            self.current_func = Some(idx);
            self.locals.clear();
            self.local_types.clear();
            self.source_types.clear();
            self.label_map.clear();
            // impl 方法：self 的源级类型
            let fn_name = self.module.functions[idx].name.clone();
            if let Some(ty) = self.self_types.get(&fn_name) {
                self.source_types.insert("self".to_string(), ty.clone());
            }
            self.generate_function_body(&func_def)?;
        }

        // 8. 派生 `#Derive(ToString)` 正文：所有目标注册完成后生成，
        //    嵌套派生/手写 ToString 的互相引用都能解析。
        self.generate_derived_bodies(programs)?;

        Ok(self.module.clone())
    }

    fn ctx(&self) -> Option<&ModuleCtx> {
        self.modules.get(&self.current_module)
    }

    /// 模块 id + 成员名 -> 规范化名（入口模块无前缀）
    fn join_item(module_id: &str, member: &str) -> String {
        if module_id.is_empty() {
            member.to_string()
        } else {
            format!("{module_id}::{member}")
        }
    }

    /// 值名（函数/常量）-> IR 名：成员导入按别名目标解析，本模块声明加前缀
    fn resolve_value(&self, name: &str) -> String {
        let Some(ctx) = self.ctx() else {
            return name.to_string();
        };
        if let Some((mid, member)) = ctx.aliases.get(name) {
            return Self::join_item(mid, member);
        }
        if ctx.own_values.contains(name) {
            return format!("{}{}", ctx.prefix, name);
        }
        name.to_string()
    }

    /// 类型名 -> 规范化名：导入的类型按目标解析，本模块声明加前缀；
    /// 限定名把模块命名空间头换成模块 id。
    fn resolve_type(&self, name: &str) -> String {
        let Some(ctx) = self.ctx() else {
            return name.to_string();
        };
        if let Some((mid, member)) = ctx.aliases.get(name) {
            return Self::join_item(mid, member);
        }
        if ctx.own_types.contains(name) {
            return format!("{}{}", ctx.prefix, name);
        }
        if let Some((head, rest)) = name.split_once("::")
            && let Some(target) = ctx.namespaces.get(head)
        {
            return Self::join_item(target, rest);
        }
        name.to_string()
    }

    /// 调用名 -> IR 名：限定调用先解析模块命名空间头，其余按值名解析。
    /// `std::` 内建与 `Trait::method`（带 `trait_impl` 标注、不走这里）原样返回。
    fn resolve_callee_name(&self, callee: &str) -> String {
        if let Some(ctx) = self.ctx()
            && let Some((head, rest)) = callee.split_once("::")
        {
            if let Some(target) = ctx.namespaces.get(head) {
                return Self::join_item(target, rest);
            }
            return callee.to_string();
        }
        self.resolve_value(callee)
    }

    /// 物化一个字符串字面量到本地槽（Const String 不能直接作 RuntimeCall
    /// 实参，只有本地槽能跨指令传递）。
    fn const_string_local(&mut self, s: &str) -> Operand {
        let func_idx = self.current_func.unwrap();
        let idx = self.module.add_string(s.to_string());
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Const {
                dst: local,
                value: IrValue::String(idx),
            });
        Operand::Local(local)
    }

    /// 字符串拼接（`+` 的字符串路径与派生渲染共用）
    fn string_concat(&mut self, left: Operand, right: Operand) -> Operand {
        let func_idx = self.current_func.unwrap();
        let dst = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(dst),
                func: RuntimeFn::StringConcat,
                args: vec![left, right],
            });
        Operand::Local(dst)
    }

    fn infer_type_from_operand(&self, operand: &Operand, func_idx: usize) -> IrType {
        match operand {
            Operand::Const(val) => match val {
                IrValue::I64(_) => IrType::I64,
                IrValue::F64(_) => IrType::F64,
                IrValue::Bool(_) => IrType::Bool,
                IrValue::String(_) => IrType::Pointer,
                IrValue::Null => IrType::Pointer,
            },
            Operand::Local(local_id) => {
                // 查找局部变量的类型
                if let Some((_, ty)) = self.module.functions[func_idx]
                    .locals
                    .iter()
                    .find(|(id, _)| id == local_id)
                {
                    ty.clone()
                } else {
                    IrType::Pointer
                }
            }
        }
    }

    /// 检查操作数是否是数组类型（通过源级类型推断和指令分析）
    /// 检查表达式是否产生数组（列表字面量、列表变量、数组拼接等）
    fn is_array_like_expr(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Literal { value, .. } => matches!(value, Literal::List(_)),
            Expression::Range(_) => true,
            Expression::Identifier { name, .. } => {
                if let Some(src_ty) = self.source_types.get(name) {
                    matches!(src_ty, Type::List(_))
                } else {
                    matches!(
                        self.const_def(name).and_then(|d| d.ty.as_ref()),
                        Some(Type::List(_))
                    )
                }
            }
            Expression::BinaryOp(binop) => {
                matches!(binop.operator, BinaryOperator::Add)
                    && (self.is_array_like_expr(&binop.left)
                        || self.is_array_like_expr(&binop.right))
            }
            Expression::Call(call) => {
                // 方法调用返回数组的情况
                if let Some(obj) = &call.object {
                    if call.method.as_deref() == Some("push") {
                        return true; // push 返回新数组
                    }
                    self.is_array_like_expr(obj)
                } else if let Some(Some(ast_ret)) = self
                    .func_return_types
                    .get(&self.resolve_callee_name(&call.callee))
                {
                    matches!(ast_ret, Type::List(_))
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// 从 AST 表达式直接推断源级类型（不依赖已生成的 IR）
    fn infer_source_type_from_expr(&self, expr: &Expression) -> Option<Type> {
        match expr {
            Expression::Literal { value, .. } => {
                match value {
                    Literal::Number(_) => Some(Type::Number),
                    Literal::Boolean(_) => Some(Type::Boolean),
                    Literal::String(_) => Some(Type::String),
                    Literal::Null => Some(Type::Null),
                    Literal::List(elems) => {
                        // 推断元素类型
                        if let Some(first) = elems.first() {
                            let elem_ty =
                                self.infer_source_type_from_expr(first).unwrap_or(Type::Any);
                            Some(Type::List(Box::new(elem_ty)))
                        } else {
                            Some(Type::List(Box::new(Type::Any)))
                        }
                    }
                    Literal::Object(fields) => {
                        let field_types: Vec<(String, Type)> = fields
                            .iter()
                            .filter_map(|f| match f {
                                ObjectField::Field { name, value } => {
                                    let ty = self
                                        .infer_source_type_from_expr(value)
                                        .unwrap_or(Type::Any);
                                    Some((name.clone(), ty))
                                }
                                _ => None,
                            })
                            .collect();
                        Some(Type::ObjectType(field_types))
                    }
                }
            }
            Expression::BinaryOp(binop) => {
                // + on two lists = list concat
                if matches!(binop.operator, BinaryOperator::Add) {
                    let left_ty = self.infer_source_type_from_expr(&binop.left);
                    let right_ty = self.infer_source_type_from_expr(&binop.right);
                    if let (Some(Type::List(_)), _) = (&left_ty, &right_ty) {
                        return left_ty;
                    }
                    if let (_, Some(Type::List(_))) = (&left_ty, &right_ty) {
                        return right_ty;
                    }
                    // 字符串拼接（模板字面量 desugar 成 +）结果是字符串
                    if left_ty == Some(Type::String) || right_ty == Some(Type::String) {
                        return Some(Type::String);
                    }
                }
                None
            }
            Expression::Identifier { name, .. } => self
                .source_types
                .get(name)
                .cloned()
                .or_else(|| self.const_def(name).and_then(|d| d.ty.clone())),
            Expression::Call(call) => {
                if call.object.is_none() {
                    if call.callee == "str" {
                        return Some(Type::String);
                    }
                    // `std::fs::exists` 是布尔谓词（可失败成员返回 Result，
                    // 由 print/tag 路径按枚举处理，类型推断只需 bool）
                    if call.callee.starts_with("std::fs::") {
                        return (call.callee == "std::fs::exists").then_some(Type::Boolean);
                    }
                    // `std::io` 的七个成员一律返回 Result 枚举，没有可标的
                    // 原始类型（打印端按 lowering 表的 tag=7 当枚举处理）
                    if call.callee.starts_with("std::io::") {
                        return None;
                    }
                    // `std::os`：可失败成员返回 Result 枚举（同 io，无原始
                    // 类型）；pid/platform/arch 有原始返回类型，按 lowering
                    // 表的 tag 映射
                    if call.callee.starts_with("std::os::") {
                        return match call.callee.as_str() {
                            "std::os::pid" => Some(Type::Number),
                            "std::os::platform" | "std::os::arch" => Some(Type::String),
                            _ => None,
                        };
                    }
                    // Math::/Time:: 内建：按 lowering 表的 tag 映射返回类型
                    if let Some((_, _, tag)) = Self::intrinsic_lowering(&call.callee) {
                        return Some(if tag == 0 { Type::String } else { Type::Number });
                    }
                    // 结构体构造：Point(x: 1) -> Point
                    let ctor = self.resolve_type(&call.callee);
                    if self.structs.contains_key(&ctor) {
                        return Some(Type::Named(ctor, Vec::new()));
                    }
                }
                // 方法调用用 `Type::method` 作键；否则用（规范化后的）函数名
                let key = if let Some(recv) = &call.object
                    && let Some(recv_ty) = self.struct_name_of_expr(recv)
                    && let Some(method) = call.method.as_deref()
                {
                    format!("{}::{}", recv_ty, method)
                } else {
                    self.resolve_callee_name(&call.callee)
                };
                // Trait 派发：按派发目标对应的 `Type::method` 查返回类型
                let key = if let Some(mangled) = &call.trait_impl
                    && let Some(&fid) = self.trait_impls.get(mangled)
                {
                    self.module.functions[fid.0].name.clone()
                } else {
                    key
                };
                self.func_return_types.get(&key).cloned().flatten()
            }
            Expression::If(if_expr) => {
                // 取 then 分支的类型
                if let Some(Statement::Expr(e)) = if_expr.then_branch.statements.last() {
                    return self.infer_source_type_from_expr(&e.expr);
                }
                None
            }
            Expression::Member(member) => {
                // 结构体字段类型
                if let Some(recv_ty) = self.struct_name_of_expr(&member.object)
                    && let Some(fields) = self.structs.get(&recv_ty)
                    && let Some(field) = fields.iter().find(|f| f.name == member.property)
                {
                    return Some(field.type_.clone());
                }
                None
            }
            // `std::math` 常量（PI/E/TAU/INFINITY/NAN）都是 float
            Expression::EnumRef(e) if e.enum_name == "std::math" => Some(Type::Number),
            // 命名空间成员引用（`m::PI`）：常量/函数的静态类型
            Expression::EnumRef(e) => {
                let target = self.ctx()?.namespaces.get(&e.enum_name)?.clone();
                let canonical = Self::join_item(&target, &e.variant);
                if let Some(def) = self.consts.get(&canonical) {
                    return def.ty.clone();
                }
                self.func_return_types.get(&canonical).cloned().flatten()
            }
            // 区间表达式求值为数字列表
            Expression::Range(_) => Some(Type::List(Box::new(Type::Number))),
            _ => None,
        }
    }

    /// 确定表达式的类型标签（用于数组/对象元素存储）
    /// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null
    fn determine_element_tag(&self, expr: &Expression) -> i64 {
        match expr {
            Expression::Literal { value, .. } => match value {
                Literal::String(_) => 0,
                Literal::Number(n) => {
                    if n.fract() == 0.0 {
                        1
                    } else {
                        2
                    }
                }
                Literal::Boolean(_) => 5,
                Literal::Null => 6,
                Literal::List(_) => 3,
                Literal::Object(_) => 4,
            },
            Expression::Identifier { name, .. } => {
                // match 绑定：payload 标签运行时才知道
                if self.is_dyn_name(name) {
                    return -1;
                }
                // 模块级 const：按初始化表达式的静态标签（字面量）打 tag
                if !self.locals.contains_key(name)
                    && let Some(def) = self.const_def(name)
                {
                    return def.tag;
                }
                // `use std::math::*` 导入的裸名常量走 F64 槽（tag=2）
                if Self::bare_math_const(name).is_some() && !self.locals.contains_key(name) {
                    return 2;
                }
                // 装箱变量按逻辑类型打标签（cell 本地槽是 Pointer，不能直接用）
                if let Some(logical) = self.boxed_logical.get(name) {
                    return match logical {
                        IrType::F64 => 2,
                        IrType::I64 => 1,
                        IrType::Bool => 5,
                        IrType::Pointer => -1,
                        IrType::Void => 6,
                    };
                }
                // 优先从 source_types 查找
                if let Some(src_ty) = self.source_types.get(name) {
                    match src_ty {
                        Type::String => 0,
                        Type::Number => {
                            // number 绑定若以 F64 槽承载（小数字面量），值是位模式，按 tag=2 打印
                            if let Some(local_id) = self.locals.get(name)
                                && let Some(func_idx) = self.current_func
                                && self.module.functions[func_idx]
                                    .locals
                                    .iter()
                                    .any(|(id, ty)| id == local_id && matches!(ty, IrType::F64))
                            {
                                2
                            } else {
                                1
                            }
                        }
                        Type::Boolean => 5,
                        Type::Null => 6,
                        Type::List(_) => 3,
                        Type::Object | Type::ObjectType(_) => 4,
                        Type::Named(_, _) => -1, // 泛型参数，运行时自动检测
                        _ => 1,
                    }
                } else if let Some(local_id) = self.locals.get(name) {
                    // 从 IR 本地变量类型推断
                    if let Some(func_idx) = self.current_func {
                        if let Some((_, ir_ty)) = self.module.functions[func_idx]
                            .locals
                            .iter()
                            .find(|(id, _)| id == local_id)
                        {
                            match ir_ty {
                                IrType::I64 => 1,
                                IrType::F64 => 2,
                                IrType::Bool => 5,
                                IrType::Pointer => -1, // 泛型指针类型，运行时自动检测
                                IrType::Void => 6,
                            }
                        } else {
                            1
                        }
                    } else {
                        1
                    }
                } else {
                    1
                }
            }
            Expression::BinaryOp(binop) => {
                match &binop.operator {
                    BinaryOperator::Add => {
                        // 检查是否是数组拼接
                        if self.is_array_like_expr(&binop.left)
                            || self.is_array_like_expr(&binop.right)
                        {
                            3 // array
                        } else if self.is_string_like(&binop.left)
                            || self.is_string_like(&binop.right)
                        {
                            0 // string（模板/字符串拼接）
                        } else if self.expr_is_dyn(&binop.left)
                            || self.expr_is_dyn(&binop.right)
                            || self.has_float_operand(&binop.left)
                            || self.has_float_operand(&binop.right)
                        {
                            2
                        } else {
                            1
                        }
                    }
                    BinaryOperator::Sub
                    | BinaryOperator::Mul
                    | BinaryOperator::Div
                    | BinaryOperator::Mod
                    | BinaryOperator::Pow
                    | BinaryOperator::BitAnd
                    | BinaryOperator::BitOr
                    | BinaryOperator::Xor
                    | BinaryOperator::Shl
                    | BinaryOperator::Shr => {
                        // 动态绑定参与的数值运算走 f64，结果按位模式承载
                        if self.expr_is_dyn(&binop.left)
                            || self.expr_is_dyn(&binop.right)
                            || self.has_float_operand(&binop.left)
                            || self.has_float_operand(&binop.right)
                        {
                            2
                        } else {
                            1
                        }
                    }
                    BinaryOperator::Eq
                    | BinaryOperator::Neq
                    | BinaryOperator::Lt
                    | BinaryOperator::Gt
                    | BinaryOperator::Lte
                    | BinaryOperator::Gte
                    | BinaryOperator::And
                    | BinaryOperator::Or => 5,
                }
            }
            Expression::Unary(unop) => match &unop.operator {
                UnaryOperator::Not => 5,
                UnaryOperator::Neg | UnaryOperator::BitNot => {
                    self.determine_element_tag(&unop.operand)
                }
                UnaryOperator::Move | UnaryOperator::Copy => {
                    self.determine_element_tag(&unop.operand)
                }
            },
            Expression::Call(call) => {
                // 经局部变量的间接调用：返回标签运行时才知道
                if call.object.is_none() && self.locals.contains_key(&call.callee) {
                    return -1;
                }
                // 方法调用用 `Type::method` 作键查找返回类型注解
                let key = if call.object.is_some() {
                    let method = call.method.as_deref().unwrap_or(&call.callee);
                    match call
                        .object
                        .as_ref()
                        .and_then(|o| self.struct_name_of_expr(o))
                    {
                        Some(ty) => format!("{}::{}", ty, method),
                        None => method.to_string(),
                    }
                } else {
                    self.resolve_callee_name(&call.callee)
                };
                // Trait 派发：返回类型按派发目标的 `Type::method` 查表，
                // 否则 `Area::area(...)` 会退化成未知标签（小数被当整数打印）
                let key = if call.callee != "str"
                    && let Some(mangled) = &call.trait_impl
                    && let Some(&fid) = self.trait_impls.get(mangled)
                {
                    self.module.functions[fid.0].name.clone()
                } else {
                    key
                };
                // 结构体构造产出对象（tag=4）
                if call.object.is_none()
                    && self.structs.contains_key(&self.resolve_type(&call.callee))
                {
                    return 4;
                }
                // str() 产出字符串；Math::Time:: 内建与 float 位模式返回按表打 tag
                if key == "str" {
                    return 0;
                }
                if let Some((_, _, tag)) = Self::intrinsic_lowering(&key) {
                    return tag;
                }
                if self.float_bit_returns.contains(&key) {
                    return 2;
                }
                // FFI 导入返回 f32/f64：调用点拿到的是真浮点槽，直接按 tag=2
                // 打印——否则 bitcast 成 i64 后不在 FLOATS 注册表里，运行时
                // 检测会把它当整数
                if call.object.is_none()
                    && let Some(&ex_idx) = self.extern_map.get(&key)
                    && matches!(
                        self.module.externs[ex_idx].return_type,
                        Some(IrAbiParam::F32 | IrAbiParam::F64)
                    )
                {
                    return 2;
                }
                // RegExp / 字符串正则方法
                if call.object.is_some()
                    && call
                        .object
                        .as_ref()
                        .is_none_or(|o| self.struct_name_of_expr(o).is_none())
                {
                    match key.as_str() {
                        "test" => return 5,
                        "group" | "replace" | "to_uppercase" => return 0,
                        "find" => return -1, // match 对象或 null
                        "find_all" => return 3,
                        _ => {}
                    }
                }
                // `RegExp::compile` -> Result::Ok/Err（运行时检测）
                if call.object.is_none() && key == "RegExp::compile" {
                    return -1;
                }
                // `Enum::Variant(...)` 构造值
                if call.object.is_none()
                    && let Some((enum_name, variant)) = key.split_once("::")
                    && self.enum_variant_index(enum_name, variant).is_some()
                {
                    return 7;
                }
                // 优先使用 AST 返回类型注解推断 tag
                if let Some(Some(ast_ret)) = self.func_return_types.get(&key) {
                    match ast_ret {
                        Type::String => 0,
                        // number 的返回可能是 f64 位模式（float 结果在返回边界
                        // 按位打包），交给运行时自动检测
                        Type::Number => -1,
                        Type::Boolean => 5,
                        Type::Null => 6,
                        Type::List(_) => 3,
                        Type::Object | Type::ObjectType(_) => 4,
                        Type::Named(_, _) => {
                            // 泛型参数：无法在编译期确定具体类型，使用运行时自动检测
                            -1
                        }
                        _ => 1,
                    }
                } else {
                    // 退化到 IR 类型
                    if let Some(func_idx) = self.module.functions.iter().position(|f| f.name == key)
                    {
                        match self.module.functions[func_idx].return_type {
                            IrType::I64 => 1,
                            IrType::F64 => 2,
                            IrType::Bool => 5,
                            IrType::Void => 6,
                            IrType::Pointer => {
                                // 指针类型可能是 string/array/object/泛型，使用运行时自动检测
                                -1
                            }
                        }
                    } else {
                        1
                    }
                }
            }
            // `std::math` 常量（PI/E/TAU/INFINITY/NAN）都是 float
            Expression::EnumRef(e) if e.enum_name == "std::math" => 2,
            // 命名空间成员引用（`m::PI`）：常量按定义标签，函数按返回标签
            Expression::EnumRef(e) => {
                if let Some(ctx) = self.ctx()
                    && let Some(target) = ctx.namespaces.get(&e.enum_name)
                {
                    let canonical = Self::join_item(target, &e.variant);
                    if let Some(def) = self.consts.get(&canonical) {
                        return def.tag;
                    }
                    if let Some(Some(ast_ret)) = self.func_return_types.get(&canonical) {
                        return match ast_ret {
                            Type::String => 0,
                            Type::Number => -1,
                            Type::Boolean => 5,
                            Type::Null => 6,
                            Type::List(_) => 3,
                            Type::Object | Type::ObjectType(_) => 4,
                            _ => -1,
                        };
                    }
                }
                // 其余枚举值（含无 payload 的变体引用）
                7
            }
            // match 表达式：分支结果各异，运行时检测
            Expression::Match(_) => -1,
            // 区间表达式求值为列表
            Expression::Range(_) => 3,
            // `expr?`：Ok payload 类型不定，运行时检测
            Expression::Propagate { .. } => -1,
            // 函数值（闭包/箭头字面量）：tag=8
            Expression::FnExpr(_) | Expression::Arrow(_) => 8,
            // 经任意表达式的函数值调用：返回标签未知
            Expression::CallValue(_) => -1,
            Expression::Nullish(nullish) => self.determine_element_tag(&nullish.right),
            Expression::Index(_) => -1, // 数组元素类型未知，运行时自动检测
            Expression::Member(member) => {
                // 结构体字段类型已知
                if let Some(recv_ty) = self.struct_name_of_expr(&member.object)
                    && let Some(fields) = self.structs.get(&recv_ty)
                    && let Some(field) = fields.iter().find(|f| f.name == member.property)
                {
                    return match &field.type_ {
                        Type::String => 0,
                        // number 字段可能以 f64 位模式承载（整数/小数由构造侧
                        // 标签决定），静态无法区分，交给运行时自动检测
                        Type::Number => -1,
                        Type::Boolean => 5,
                        Type::Null => 6,
                        Type::List(_) => 3,
                        Type::Object | Type::ObjectType(_) => 4,
                        // 可选字段可能是 null，运行时自动检测
                        _ => -1,
                    };
                }
                -1 // 对象字段类型未知，运行时自动检测
            }
            _ => 1,
        }
    }

    /// 名字是否是 match 绑定（payload 标签运行时才知道）
    fn is_dyn_name(&self, name: &str) -> bool {
        self.dyn_tag_of.contains_key(name)
    }

    fn expr_is_dyn(&self, expr: &Expression) -> bool {
        matches!(expr, Expression::Identifier { name, .. } if self.is_dyn_name(name))
    }

    /// 动态绑定的操作数经 `xulo_as_f64` 转成 f64 槽；tag 取绑定时摘出的
    /// companion 槽（构造侧记录的是真实 payload 标签）
    fn as_f64_operand(&mut self, operand: Operand, name: &str) -> Result<Operand, XuloError> {
        let Some(tag_op) = self.dyn_tag_of.get(name).cloned() else {
            return Ok(operand);
        };
        let func_idx = self.current_func.unwrap();
        let local = self.module.functions[func_idx].add_local(IrType::F64);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(local),
                func: RuntimeFn::AsF64,
                args: vec![operand, tag_op],
            });
        Ok(Operand::Local(local))
    }

    /// f64 值在按位模式进 I64/Pointer 槽（return / match 结果）前注册位模式，
    /// 打印端的自动检测（tag=-1）才能还原成小数
    fn emit_register_float(&mut self, operand: Operand) {
        let func_idx = self.current_func.unwrap();
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: None,
                func: RuntimeFn::RegisterFloat,
                args: vec![operand],
            });
    }

    fn has_float_operand(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Literal { value, .. } => {
                matches!(value, Literal::Number(n) if n.fract() != 0.0)
            }
            Expression::Identifier { name, .. } => {
                // F64 槽承载的变量（装箱逻辑类型或本地槽）参与算术走浮点
                if self.boxed_logical.get(name) == Some(&IrType::F64) {
                    return true;
                }
                if let Some(local_id) = self.locals.get(name)
                    && let Some(func_idx) = self.current_func
                {
                    return self.module.functions[func_idx]
                        .locals
                        .iter()
                        .any(|(id, ty)| id == local_id && matches!(ty, IrType::F64));
                }
                false
            }
            Expression::Unary(unop) => self.has_float_operand(&unop.operand),
            Expression::BinaryOp(binop) => {
                self.has_float_operand(&binop.left) || self.has_float_operand(&binop.right)
            }
            _ => false,
        }
    }

    /// 表达式是否是字符串（用于 `+` 拼接与 str() 直通的 tag 判断）
    fn is_string_like(&self, expr: &Expression) -> bool {
        if matches!(self.infer_source_type_from_expr(expr), Some(Type::String)) {
            return true;
        }
        // str() 的返回值
        matches!(expr, Expression::Call(call)
            if call.object.is_none() && call.callee == "str")
    }

    /// IR 操作数的槽类型（Neg 等按类型选择本地变量类型与指令）
    fn operand_slot_type(&self, op: &Operand) -> IrType {
        match op {
            Operand::Const(IrValue::F64(_)) => IrType::F64,
            Operand::Const(IrValue::String(_)) | Operand::Const(IrValue::Null) => IrType::Pointer,
            Operand::Const(_) => IrType::I64,
            Operand::Local(id) => {
                let Some(func_idx) = self.current_func else {
                    return IrType::I64;
                };
                self.module.functions[func_idx]
                    .locals
                    .iter()
                    .find(|(lid, _)| lid == id)
                    .map(|(_, ty)| ty.clone())
                    .unwrap_or(IrType::I64)
            }
        }
    }

    /// Math/Time 命名空间内建的 lowering 表（对齐语义 `builtin_member`，spec
    /// `builtins/intrinsic-functions.md`）：返回 (运行时符号, 结果槽类型, 打印 tag)。
    /// float 组返回 f64，但 xulo 的 number 在 IR 层是整数域——按 F64 位模式放进
    /// I64 槽（打印端 tag=2 还原小数）；toFixed 返回字符串；Time 读数返回整数。
    /// `use std::math::*` 导入的裸名常量在 locals 缺席时的 F64 值。
    fn bare_math_const(name: &str) -> Option<f64> {
        match name {
            "PI" => Some(std::f64::consts::PI),
            "E" => Some(std::f64::consts::E),
            "TAU" => Some(std::f64::consts::TAU),
            "INFINITY" => Some(f64::INFINITY),
            "NAN" => Some(f64::NAN),
            _ => None,
        }
    }

    fn intrinsic_lowering(callee: &str) -> Option<(&'static str, IrType, i64)> {
        // `std::math::abs` / `std::fs::read` / `std::io::println` 归一到表键
        // `Math::abs` / `Fs::read` / `Io::println` 等
        let normalized: std::borrow::Cow<str> =
            if let Some(rest) = callee.strip_prefix("std::math::") {
                std::borrow::Cow::Owned(format!("Math::{rest}"))
            } else if let Some(rest) = callee.strip_prefix("std::time::") {
                std::borrow::Cow::Owned(format!("Time::{rest}"))
            } else if let Some(rest) = callee.strip_prefix("std::fs::") {
                std::borrow::Cow::Owned(format!("Fs::{rest}"))
            } else if let Some(rest) = callee.strip_prefix("std::io::") {
                std::borrow::Cow::Owned(format!("Io::{rest}"))
            } else if let Some(rest) = callee.strip_prefix("std::os::") {
                std::borrow::Cow::Owned(format!("Os::{rest}"))
            } else if let Some(rest) = callee.strip_prefix("std::path::") {
                std::borrow::Cow::Owned(format!("Path::{rest}"))
            } else {
                std::borrow::Cow::Borrowed(callee)
            };
        let lowering = match normalized.as_ref() {
            "Math::abs" => ("xulo_math_abs", IrType::I64, 2),
            "Math::min" => ("xulo_math_min", IrType::I64, 2),
            "Math::max" => ("xulo_math_max", IrType::I64, 2),
            "Math::floor" => ("xulo_math_floor", IrType::I64, 2),
            "Math::ceil" => ("xulo_math_ceil", IrType::I64, 2),
            "Math::round" => ("xulo_math_round", IrType::I64, 2),
            "Math::trunc" => ("xulo_math_trunc", IrType::I64, 2),
            "Math::sqrt" => ("xulo_math_sqrt", IrType::I64, 2),
            "Math::cbrt" => ("xulo_math_cbrt", IrType::I64, 2),
            "Math::pow" => ("xulo_math_pow", IrType::I64, 2),
            "Math::sin" => ("xulo_math_sin", IrType::I64, 2),
            "Math::cos" => ("xulo_math_cos", IrType::I64, 2),
            "Math::tan" => ("xulo_math_tan", IrType::I64, 2),
            "Math::asin" => ("xulo_math_asin", IrType::I64, 2),
            "Math::acos" => ("xulo_math_acos", IrType::I64, 2),
            "Math::atan" => ("xulo_math_atan", IrType::I64, 2),
            "Math::atan2" => ("xulo_math_atan2", IrType::I64, 2),
            "Math::log" => ("xulo_math_log", IrType::I64, 2),
            "Math::log2" => ("xulo_math_log2", IrType::I64, 2),
            "Math::log10" => ("xulo_math_log10", IrType::I64, 2),
            "Math::exp" => ("xulo_math_exp", IrType::I64, 2),
            "Math::random" => ("xulo_math_random", IrType::I64, 2),
            "Math::toFixed" => ("xulo_math_to_fixed", IrType::Pointer, 0),
            "Time::now" => ("xulo_time_now", IrType::I64, 1),
            "Time::now_nanos" => ("xulo_time_now_nanos", IrType::I64, 1),
            "Time::monotonic" => ("xulo_time_monotonic", IrType::I64, 1),
            "Time::monotonic_nanos" => ("xulo_time_monotonic_nanos", IrType::I64, 1),
            // `std::fs`：可失败操作返回内置 Result 枚举（tag=7），exists 是 bool
            "Fs::read" => ("xulo_fs_read", IrType::Pointer, 7),
            "Fs::write" => ("xulo_fs_write", IrType::Pointer, 7),
            "Fs::exists" => ("xulo_fs_exists", IrType::Bool, 5),
            "Fs::remove" => ("xulo_fs_remove", IrType::Pointer, 7),
            "Fs::mkdir" => ("xulo_fs_mkdir", IrType::Pointer, 7),
            // `std::io`：七个成员全部可失败，一律返回内置 Result 枚举
            // （tag=7）；读入结果由运行时按载荷 tag 区分字符串与 null(EOF)
            "Io::print" => ("xulo_io_print", IrType::Pointer, 7),
            "Io::println" => ("xulo_io_println", IrType::Pointer, 7),
            "Io::eprint" => ("xulo_io_eprint", IrType::Pointer, 7),
            "Io::eprintln" => ("xulo_io_eprintln", IrType::Pointer, 7),
            "Io::flush" => ("xulo_io_flush", IrType::Pointer, 7),
            "Io::readLine" => ("xulo_io_read_line", IrType::Pointer, 7),
            "Io::read" => ("xulo_io_read", IrType::Pointer, 7),
            // `std::os`：可失败成员返回内置 Result 枚举（tag=7，getenv 载荷
            // 区分字符串/null）；pid 是数字，platform/arch 是字符串
            "Os::getenv" => ("xulo_os_getenv", IrType::Pointer, 7),
            "Os::setenv" => ("xulo_os_setenv", IrType::Pointer, 7),
            "Os::cwd" => ("xulo_os_cwd", IrType::Pointer, 7),
            "Os::chdir" => ("xulo_os_chdir", IrType::Pointer, 7),
            "Os::pid" => ("xulo_os_pid", IrType::I64, 1),
            "Os::platform" => ("xulo_os_platform", IrType::Pointer, 0),
            "Os::arch" => ("xulo_os_arch", IrType::Pointer, 0),
            // `std::path`：全成员返回驻留字符串（tag=0，总谓词不失败）
            "Path::join" => ("xulo_path_join", IrType::Pointer, 0),
            "Path::dirname" => ("xulo_path_dirname", IrType::Pointer, 0),
            "Path::basename" => ("xulo_path_basename", IrType::Pointer, 0),
            "Path::ext" => ("xulo_path_ext", IrType::Pointer, 0),
            "Path::normalize" => ("xulo_path_normalize", IrType::Pointer, 0),
            _ => return None,
        };
        Some(lowering)
    }

    fn type_to_ir(&self, ty: Option<&Type>) -> IrType {
        match ty {
            None => IrType::Void,
            Some(t) => match t {
                Type::Boolean => IrType::Bool,
                Type::String
                | Type::Null
                | Type::Object
                | Type::List(_)
                | Type::Named(_, _)
                | Type::Literal(_)
                | Type::Optional(_)
                | Type::Union(_)
                | Type::Intersection(_)
                | Type::ObjectType(_)
                | Type::FnSig { .. }
                | Type::Async(_)
                | Type::Any
                | Type::Tuple(_) => IrType::Pointer,
                Type::Number => IrType::I64,
            },
        }
    }
}

/// 从 AST 生成 IR 模块
pub fn generate_ir(program: &Program) -> Result<IrModule, XuloError> {
    let mut generator = IrGenerator::new();
    generator.generate(program)
}

/// 有序去重插入（自由变量/绑定收集保持首次出现顺序）
fn push_unique(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|n| n == name) {
        list.push(name.to_string());
    }
}
