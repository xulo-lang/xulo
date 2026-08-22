use std::collections::HashMap;

use xulo_core::ast::*;
use xulo_core::ir::*;
use xulo_core::error::{XuloError, ErrorKind};

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
    label_map: HashMap<String, Label>,
    loop_stack: Vec<LoopContext>,
}

struct LoopContext {
    break_label: Label,
    continue_label: Label,
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
            label_map: HashMap::new(),
            loop_stack: Vec::new(),
        }
    }

    pub fn generate(&mut self, program: &Program) -> Result<IrModule, XuloError> {
        // 首先注册所有函数（但不生成函数体）
        let mut main_fn_def: Option<FnDef> = None;
        for stmt in &program.statements {
            match stmt {
                Statement::Fn(f) => {
                    if f.name == "main" {
                        main_fn_def = Some(f.clone());
                    }
                    let _func_id = self.register_function(f)?;
                }
                Statement::Export(export) => {
                    if let ExportItem::Fn(f) = &export.item {
                        self.register_function(f)?;
                    }
                }
                _ => {}
            }
        }

        // 查找 main 函数索引
        let main_idx = self.module.functions.iter().position(|f| f.name == "main");
        
        if main_idx.is_none() {
            // 没有 main 函数，创建一个空的
            let main_func = IrFunction::new(
                "main".to_string(),
                Vec::new(),
                IrType::Void,
            );
            self.module.add_function(main_func);
        }

        self.module.entry_point = FuncId(main_idx.unwrap_or(self.module.functions.len() - 1));

        // Generate IR for ALL functions
        let func_defs: Vec<(usize, FnDef)> = program.statements.iter().filter_map(|stmt| {
            match stmt {
                Statement::Fn(f) => {
                    let idx = self.module.functions.iter().position(|ir_f| ir_f.name == f.name);
                    idx.map(|i| (i, f.clone()))
                }
                Statement::Export(export) => {
                    if let ExportItem::Fn(f) = &export.item {
                        let idx = self.module.functions.iter().position(|ir_f| ir_f.name == f.name);
                        idx.map(|i| (i, f.clone()))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }).collect();

        for (idx, func_def) in func_defs {
            self.current_func = Some(idx);
            self.locals.clear();
            self.local_types.clear();
            self.source_types.clear();
            self.label_map.clear();
            self.generate_function_body(&func_def)?;
        }

        Ok(self.module.clone())
    }

    fn register_function(&mut self, f: &FnDef) -> Result<FuncId, XuloError> {
        let params: Vec<(LocalId, IrType)> = f.params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let ty = self.type_to_ir(p.type_annotation.as_ref());
                (LocalId(i), ty)
            })
            .collect();

        let return_type = f.return_type.as_ref()
            .map(|t| self.type_to_ir(Some(t)))
            .unwrap_or(IrType::Void);

        // 保存 AST 返回类型注解（用于泛型函数推断）
        self.func_return_types.insert(f.name.clone(), f.return_type.clone());

        let func = IrFunction::new(f.name.clone(), params, return_type);
        Ok(self.module.add_function(func))
    }

    fn generate_function_body(&mut self, f: &FnDef) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        
        // 使用 register_function 中已创建的参数 LocalId
        for (i, param) in f.params.iter().enumerate() {
            let local_id = self.module.functions[func_idx].params[i].0;
            self.locals.insert(param.name.clone(), local_id);
            // 记录参数的源级类型
            if let Some(ann) = &param.type_annotation {
                self.source_types.insert(param.name.clone(), ann.clone());
            }
        }
        
        // 注册函数体中定义的函数
        for stmt in &f.body.statements {
            if let Statement::Fn(inner_f) = stmt {
                let _func_id = self.register_function(inner_f)?;
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.locals.insert(inner_f.name.clone(), local);
            }
        }
        
        // 生成函数体语句
        let last_stmt_is_return = matches!(f.body.statements.last(), Some(Statement::Return(_)));
        for stmt in &f.body.statements {
            if !matches!(stmt, Statement::Fn(_)) {
                self.generate_statement(stmt)?;
            }
        }
        
        // 如果没有显式的 return，根据返回类型处理
        if !last_stmt_is_return {
            // Check if last instruction is already a terminator (e.g. from if/else returning in both branches)
            let last_is_terminator = matches!(
                self.module.functions[func_idx].instructions.last(),
                Some(Instruction::Return(_))
            );
            if !last_is_terminator {
                let ret_type = self.type_to_ir(f.return_type.as_ref());
                if !ret_type.is_void() {
                    if let Some(Statement::Expr(expr_stmt)) = f.body.statements.last() {
                        let operand = self.generate_expression(&expr_stmt.expr)?;
                        self.module.functions[func_idx].instructions.push(Instruction::Return(Some(operand)));
                    } else {
                        self.module.functions[func_idx].instructions.push(Instruction::Return(None));
                    }
                } else {
                    self.module.functions[func_idx].instructions.push(Instruction::Return(None));
                }
            }
        }
        
        Ok(())
    }

    fn generate_program_body(&mut self, program: &Program) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        
        // 注册所有函数定义
        for stmt in &program.statements {
            if let Statement::Fn(f) = stmt {
                let _func_id = self.register_function(f)?;
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.locals.insert(f.name.clone(), local);
            }
        }

        // 生成语句
        for stmt in &program.statements {
            match stmt {
                Statement::Fn(_) => {} // 已经注册过了
                Statement::Export(export) => {
                    if let ExportItem::Fn(f) = &export.item {
                        let _func_id = self.register_function(f)?;
                        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                        self.locals.insert(f.name.clone(), local);
                    }
                }
                _ => {
                    self.generate_statement(stmt)?;
                }
            }
        }

        // 添加默认返回
        let func = &mut self.module.functions[func_idx];
        func.instructions.push(Instruction::Return(None));

        Ok(())
    }

    fn generate_statement(&mut self, stmt: &Statement) -> Result<(), XuloError> {
        match stmt {
            Statement::Let(binding) => self.generate_let(binding),
            Statement::Return(r) => self.generate_return(r),
            Statement::Expr(e) => {
                if let Expression::If(if_expr) = &e.expr {
                    self.generate_if(if_expr)
                } else {
                    self.generate_expression(&e.expr)?;
                    Ok(())
                }
            }
            Statement::For(for_stmt) => self.generate_for(for_stmt),
            Statement::While(while_stmt) => self.generate_while(while_stmt),
            Statement::Assign(assign) => self.generate_assign(assign),
            Statement::Fn(_) => Ok(()), // 已处理
            Statement::Export(_) => Ok(()), // 已处理
            _ => Ok(()), // 其他语句暂时忽略
        }
    }

    fn generate_let(&mut self, binding: &LetBinding) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        
        // 在生成表达式之前，先记录源级类型
        if let Some(ann) = &binding.type_annotation {
            self.source_types.insert(binding.name.clone(), ann.clone());
        } else if let Some(ref expr) = binding.value {
            if let Some(src_ty) = self.infer_source_type_from_expr(expr) {
                self.source_types.insert(binding.name.clone(), src_ty);
            }
        }

        // 先生成表达式以获取实际类型
        let init_val = if let Some(ref expr) = binding.value {
            self.generate_expression(expr)?
        } else {
            Operand::Const(IrValue::Null)
        };
        
        // 推断类型
        let ty = if let Some(ann) = &binding.type_annotation {
            self.type_to_ir(Some(ann))
        } else {
            // 根据表达式推断类型
            self.infer_type_from_operand(&init_val, func_idx)
        };

        let local = self.module.functions[func_idx].add_local(ty.clone());
        self.locals.insert(binding.name.clone(), local);
        self.local_types.insert(local, ty.clone());
        
        self.module.functions[func_idx].instructions.push(Instruction::SetLocal {
            dst: local,
            src: init_val,
        });
        
        Ok(())
    }

    fn infer_type_from_operand(&self, operand: &Operand, func_idx: usize) -> IrType {
        match operand {
            Operand::Const(val) => {
                match val {
                    IrValue::I64(_) => IrType::I64,
                    IrValue::F64(_) => IrType::F64,
                    IrValue::Bool(_) => IrType::Bool,
                    IrValue::String(_) => IrType::Pointer,
                    IrValue::Null => IrType::Pointer,
                }
            }
            Operand::Local(local_id) => {
                // 查找局部变量的类型
                if let Some((_, ty)) = self.module.functions[func_idx].locals.iter().find(|(id, _)| id == local_id) {
                    ty.clone()
                } else {
                    IrType::Pointer
                }
            }
        }
    }

    /// 检查操作数是否是数组类型（通过源级类型推断和指令分析）
    fn is_array_operand(&self, operand: &Operand, func_idx: usize) -> bool {
        match operand {
            Operand::Local(local_id) => {
                // 从 source_types 查找变量名
                for (name, lid) in &self.locals {
                    if lid == local_id {
                        if let Some(src_ty) = self.source_types.get(name) {
                            return matches!(src_ty, Type::List(_));
                        }
                    }
                }
                // 检查是否是 NewArray 指令的结果
                let func = &self.module.functions[func_idx];
                for instr in &func.instructions {
                    match instr {
                        Instruction::NewArray { dst, .. } if dst == local_id => return true,
                        Instruction::RuntimeCall { dst: Some(d), func: RuntimeFn::ArrayConcat, .. } if d == local_id => return true,
                        Instruction::RuntimeCall { dst: Some(d), func: RuntimeFn::ArrayPush, .. } if d == local_id => return true,
                        Instruction::RuntimeCall { dst: Some(d), func: RuntimeFn::AllocArray, .. } if d == local_id => return true,
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// 检查表达式是否产生数组（列表字面量、列表变量、数组拼接等）
    fn is_array_like_expr(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Literal { value, .. } => matches!(value, Literal::List(_)),
            Expression::Identifier { name, .. } => {
                if let Some(src_ty) = self.source_types.get(name) {
                    matches!(src_ty, Type::List(_))
                } else {
                    false
                }
            }
            Expression::BinaryOp(binop) => {
                matches!(binop.operator, BinaryOperator::Add)
                    && (self.is_array_like_expr(&binop.left) || self.is_array_like_expr(&binop.right))
            }
            Expression::Call(call) => {
                // 方法调用返回数组的情况
                if let Some(obj) = &call.object {
                    if call.method.as_deref() == Some("push") {
                        return true; // push 返回新数组
                    }
                    self.is_array_like_expr(obj)
                } else if let Some(Some(ast_ret)) = self.func_return_types.get(&call.callee) {
                    matches!(ast_ret, Type::List(_))
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// 从操作数推断源级类型（用于 print 类型标签）
    fn infer_source_type(&self, operand: &Operand) -> Option<Type> {
        match operand {
            Operand::Const(val) => {
                match val {
                    IrValue::I64(_) => Some(Type::Number),
                    IrValue::F64(_) => Some(Type::Number),
                    IrValue::Bool(_) => Some(Type::Boolean),
                    IrValue::String(_) => Some(Type::String),
                    IrValue::Null => Some(Type::Null),
                }
            }
            Operand::Local(local_id) => {
                // 从 source_types 查找
                for (name, lid) in &self.locals {
                    if lid == local_id {
                        return self.source_types.get(name).cloned();
                    }
                }
                None
            }
        }
    }

    /// 从 AST 表达式直接推断源级类型（不依赖已生成的 IR）
    fn infer_source_type_from_expr(&self, expr: &Expression) -> Option<Type> {
        match expr {
            Expression::Literal { value, .. } => {
                match value {
                    Literal::Number(n) => {
                        if n.fract() == 0.0 { Some(Type::Number) } else { Some(Type::Number) }
                    }
                    Literal::Boolean(_) => Some(Type::Boolean),
                    Literal::String(_) => Some(Type::String),
                    Literal::Null => Some(Type::Null),
                    Literal::List(elems) => {
                        // 推断元素类型
                        if let Some(first) = elems.first() {
                            let elem_ty = self.infer_source_type_from_expr(first)
                                .unwrap_or(Type::Any);
                            Some(Type::List(Box::new(elem_ty)))
                        } else {
                            Some(Type::List(Box::new(Type::Any)))
                        }
                    }
                    Literal::Object(fields) => {
                        let field_types: Vec<(String, Type)> = fields.iter().filter_map(|f| {
                            match f {
                                ObjectField::Field { name, value } => {
                                    let ty = self.infer_source_type_from_expr(value)
                                        .unwrap_or(Type::Any);
                                    Some((name.clone(), ty))
                                }
                                _ => None,
                            }
                        }).collect();
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
                }
                None
            }
            Expression::Identifier { name, .. } => {
                self.source_types.get(name).cloned()
            }
            Expression::Call(call) => {
                // 查找被调用函数的返回类型
                for func in &self.module.functions {
                    if func.name == call.callee {
                        // 返回类型为 Pointer 的可能是 array/object
                        // 这里无法精确判断，返回 None 让调用方处理
                        break;
                    }
                }
                None
            }
            Expression::If(if_expr) => {
                // 取 then 分支的类型
                if let Some(last) = if_expr.then_branch.statements.last() {
                    if let Statement::Expr(e) = last {
                        return self.infer_source_type_from_expr(&e.expr);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// 确定 print 的类型标签
    /// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null
    fn determine_print_tag(&self, operand: &Operand, func_idx: usize) -> i64 {
        match operand {
            Operand::Const(val) => {
                match val {
                    IrValue::String(_) => 0,
                    IrValue::I64(_) => 1,
                    IrValue::F64(_) => 2,
                    IrValue::Bool(_) => 5,
                    IrValue::Null => 6,
                }
            }
            Operand::Local(local_id) => {
                // 先检查 IR 类型
                let ir_ty = self.module.functions[func_idx].locals.iter()
                    .find(|(id, _)| id == local_id)
                    .map(|(_, ty)| ty.clone());
                match ir_ty {
                    Some(IrType::I64) => 1,
                    Some(IrType::F64) => 2,
                    Some(IrType::Bool) => 5,
                    Some(IrType::Pointer) => {
                        // Pointer 类型 - 需要区分 string/array/object
                        // 从 source_types 查找
                        for (name, lid) in &self.locals {
                            if lid == local_id {
                                if let Some(src_ty) = self.source_types.get(name) {
                                    return match src_ty {
                                        Type::List(_) => 3,
                                        Type::Object | Type::ObjectType(_) => 4,
                                        Type::Named(_) => -1, // 泛型参数，运行时自动检测
                                        _ => 0, // 默认为 string
                                    };
                                }
                            }
                        }
                        // 无法确定，运行时自动检测
                        -1
                    }
                    _ => 0,
                }
            }
        }
    }

    /// 确定表达式的类型标签（用于数组/对象元素存储）
    /// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null
    fn determine_element_tag(&self, expr: &Expression) -> i64 {
        match expr {
            Expression::Literal { value, .. } => match value {
                Literal::String(_) => 0,
                Literal::Number(n) => {
                    if n.fract() == 0.0 { 1 } else { 2 }
                }
                Literal::Boolean(_) => 5,
                Literal::Null => 6,
                Literal::List(_) => 3,
                Literal::Object(_) => 4,
            },
            Expression::Identifier { name, .. } => {
                // 优先从 source_types 查找
                if let Some(src_ty) = self.source_types.get(name) {
                    match src_ty {
                        Type::String => 0,
                        Type::Number => 1,
                        Type::Boolean => 5,
                        Type::Null => 6,
                        Type::List(_) => 3,
                        Type::Object | Type::ObjectType(_) => 4,
                        Type::Named(_) => -1, // 泛型参数，运行时自动检测
                        _ => 1,
                    }
                } else if let Some(local_id) = self.locals.get(name) {
                    // 从 IR 本地变量类型推断
                    if let Some(func_idx) = self.current_func {
                        if let Some((_, ir_ty)) = self.module.functions[func_idx].locals.iter()
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
                        if self.is_array_like_expr(&binop.left) || self.is_array_like_expr(&binop.right) {
                            3 // array
                        } else if self.has_float_operand(&binop.left) || self.has_float_operand(&binop.right) {
                            2
                        } else {
                            1
                        }
                    }
                    BinaryOperator::Sub
                    | BinaryOperator::Mul | BinaryOperator::Div => {
                        if self.has_float_operand(&binop.left) || self.has_float_operand(&binop.right) {
                            2
                        } else {
                            1
                        }
                    }
                    BinaryOperator::Eq | BinaryOperator::Neq
                    | BinaryOperator::Lt | BinaryOperator::Gt
                    | BinaryOperator::Lte | BinaryOperator::Gte
                    | BinaryOperator::And | BinaryOperator::Or => 5,
                }
            }
            Expression::Unary(unop) => {
                match &unop.operator {
                    UnaryOperator::Not => 5,
                    UnaryOperator::Neg => self.determine_element_tag(&unop.operand),
                }
            }
            Expression::Call(call) => {
                // 优先使用 AST 返回类型注解推断 tag
                if let Some(Some(ast_ret)) = self.func_return_types.get(&call.callee) {
                    match ast_ret {
                        Type::String => 0,
                        Type::Number => 1,
                        Type::Boolean => 5,
                        Type::Null => 6,
                        Type::List(_) => 3,
                        Type::Object | Type::ObjectType(_) => 4,
                        Type::Named(_) => {
                            // 泛型参数：无法在编译期确定具体类型，使用运行时自动检测
                            -1
                        }
                        _ => 1,
                    }
                } else {
                    // 退化到 IR 类型
                    if let Some(func_idx) = self.module.functions.iter().position(|f| f.name == call.callee) {
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
            Expression::Index(_) => -1, // 数组元素类型未知，运行时自动检测
            Expression::Member(_) => -1, // 对象字段类型未知，运行时自动检测
            _ => 1,
        }
    }

    fn has_float_operand(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Literal { value, .. } => matches!(value, Literal::Number(n) if n.fract() != 0.0),
            Expression::Unary(unop) => self.has_float_operand(&unop.operand),
            Expression::BinaryOp(binop) => self.has_float_operand(&binop.left) || self.has_float_operand(&binop.right),
            _ => false,
        }
    }

    fn generate_return(&mut self, r: &ReturnStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        
        if let Some(value) = &r.value {
            let operand = self.generate_expression(value)?;
            self.module.functions[func_idx].instructions.push(Instruction::Return(Some(operand)));
        } else {
            self.module.functions[func_idx].instructions.push(Instruction::Return(None));
        }

        Ok(())
    }

    fn generate_expression(&mut self, expr: &Expression) -> Result<Operand, XuloError> {
        match expr {
            Expression::Literal { value, .. } => self.generate_literal(value),
            Expression::Identifier { name, .. } => {
                if let Some(local) = self.locals.get(name) {
                    Ok(Operand::Local(*local))
                } else {
                    Err(XuloError::new(ErrorKind::Semantic, format!("undefined variable: {}", name)))
                }
            }
            Expression::BinaryOp(binop) => self.generate_binary_op(binop),
            Expression::Unary(unary) => self.generate_unary(unary),
            Expression::Call(call) => self.generate_call(call),
            Expression::If(if_expr) => self.generate_if_expr(if_expr),
            Expression::Member(member) => self.generate_member_access(member),
            Expression::Index(index) => self.generate_index(index),
            _ => {
                // 其他表达式返回 null
                let func_idx = self.current_func.unwrap();
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx].instructions.push(Instruction::Const {
                    dst: local,
                    value: IrValue::Null,
                });
                Ok(Operand::Local(local))
            }
        }
    }

    fn generate_literal(&mut self, lit: &Literal) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let (value, ty) = match lit {
            Literal::Number(n) => {
                if n.fract() == 0.0 {
                    (IrValue::I64(*n as i64), IrType::I64)
                } else {
                    (IrValue::F64(*n), IrType::F64)
                }
            }
            Literal::Boolean(b) => (IrValue::Bool(*b), IrType::Bool),
            Literal::String(s) => {
                let idx = self.module.add_string(s.clone());
                (IrValue::String(idx), IrType::Pointer)
            }
            Literal::Null => (IrValue::Null, IrType::Pointer),
            Literal::List(elements) => {
                // 创建数组
                let mut element_ops = Vec::new();
                for elem in elements {
                    let tag = self.determine_element_tag(elem);
                    element_ops.push((self.generate_expression(elem)?, tag));
                }
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx].instructions.push(Instruction::NewArray {
                    dst: local,
                    elements: element_ops,
                });
                return Ok(Operand::Local(local));
            }
            Literal::Object(fields) => {
                // 创建对象
                let mut field_ops = Vec::new();
                for field in fields {
                    match field {
                        ObjectField::Field { name, value } => {
                            let tag = self.determine_element_tag(value);
                            field_ops.push((name.clone(), self.generate_expression(value)?, tag));
                        }
                        ObjectField::Spread { .. } => {
                            // Spread 暂不支持
                        }
                    }
                }
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx].instructions.push(Instruction::NewObject {
                    dst: local,
                    fields: field_ops,
                });
                return Ok(Operand::Local(local));
            }
        };

        let local = self.module.functions[func_idx].add_local(ty);
        self.module.functions[func_idx].instructions.push(Instruction::Const {
            dst: local,
            value,
        });
        Ok(Operand::Local(local))
    }

    fn generate_binary_op(&mut self, binop: &BinaryOp) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let left = self.generate_expression(&binop.left)?;
        let right = self.generate_expression(&binop.right)?;
        
        // Check if this is a string concatenation or array concatenation
        let left_ty = self.infer_type_from_operand(&left, func_idx);
        let right_ty = self.infer_type_from_operand(&right, func_idx);
        let is_pointer_op = matches!(binop.operator, BinaryOperator::Add) 
            && (matches!(left_ty, IrType::Pointer) || matches!(right_ty, IrType::Pointer));
        
        if is_pointer_op {
            // 检查是否是数组拼接
            let left_is_array = self.is_array_operand(&left, func_idx);
            let right_is_array = self.is_array_operand(&right, func_idx);
            
            if left_is_array || right_is_array {
                // Array concatenation: call runtime function
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx].instructions.push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::ArrayConcat,
                    args: vec![left, right],
                });
                return Ok(Operand::Local(local));
            } else {
                // String concatenation: call runtime function
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx].instructions.push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::StringConcat,
                    args: vec![left, right],
                });
                return Ok(Operand::Local(local));
            }
        }
        
        let ty = match binop.operator {
            BinaryOperator::Add | BinaryOperator::Sub | 
            BinaryOperator::Mul | BinaryOperator::Div => IrType::I64,
            _ => IrType::Bool,
        };

        let local = self.module.functions[func_idx].add_local(ty);
        
        let instr = match binop.operator {
            BinaryOperator::Add => Instruction::Add { dst: local, left, right },
            BinaryOperator::Sub => Instruction::Sub { dst: local, left, right },
            BinaryOperator::Mul => Instruction::Mul { dst: local, left, right },
            BinaryOperator::Div => Instruction::Div { dst: local, left, right },
            BinaryOperator::Eq => Instruction::Eq { dst: local, left, right },
            BinaryOperator::Neq => Instruction::Neq { dst: local, left, right },
            BinaryOperator::Lt => Instruction::Lt { dst: local, left, right },
            BinaryOperator::Gt => Instruction::Gt { dst: local, left, right },
            BinaryOperator::Lte => Instruction::Lte { dst: local, left, right },
            BinaryOperator::Gte => Instruction::Gte { dst: local, left, right },
            BinaryOperator::And => Instruction::And { dst: local, left, right },
            BinaryOperator::Or => Instruction::Or { dst: local, left, right },
        };

        self.module.functions[func_idx].instructions.push(instr);
        Ok(Operand::Local(local))
    }

    fn generate_unary(&mut self, unary: &UnaryOp) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let operand = self.generate_expression(&unary.operand)?;
        
        let ty = match unary.operator {
            UnaryOperator::Not => IrType::Bool,
            UnaryOperator::Neg => IrType::I64,
        };

        let local = self.module.functions[func_idx].add_local(ty);
        
        let instr = match unary.operator {
            UnaryOperator::Not => Instruction::Not { dst: local, operand },
            UnaryOperator::Neg => Instruction::Neg { dst: local, operand },
        };

        self.module.functions[func_idx].instructions.push(instr);
        Ok(Operand::Local(local))
    }

    fn generate_call(&mut self, call: &Call) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let mut args = Vec::new();

        // 对于方法调用，先生成 receiver 并作为第一个参数
        if let Some(obj) = &call.object {
            let receiver = self.generate_expression(obj)?;
            args.push(receiver);
        }

        for arg in &call.arguments {
            args.push(self.generate_expression(&arg.value)?);
        }

        // 检查是否是内置函数或内置方法
        let runtime_fn = if call.object.is_some() {
            // 方法调用：根据方法名匹配
            match call.method.as_deref().unwrap_or(&call.callee) {
                "push" => {
                    // push 需要类型标签作为第三个参数
                    if let Some(arg) = call.arguments.first() {
                        let tag = self.determine_element_tag(&arg.value);
                        args.push(Operand::Const(IrValue::I64(tag)));
                    } else {
                        args.push(Operand::Const(IrValue::I64(6))); // null
                    }
                    Some(RuntimeFn::ArrayPush)
                }
                _ => None,
            }
        } else {
            // 普通函数调用
            match call.callee.as_str() {
                "print" | "println" => {
                    // 使用 PrintValue 带类型标签
                    if args.is_empty() {
                        // print() 无参数: 传 (0, 6) 打印空行
                        args.push(Operand::Const(IrValue::I64(0)));
                        args.push(Operand::Const(IrValue::I64(6)));
                    } else {
                        // 从原始 AST 表达式推断类型标签（而非生成后的 operand）
                        let tag = self.determine_element_tag(&call.arguments[0].value);
                        args.push(Operand::Const(IrValue::I64(tag)));
                    }
                    Some(RuntimeFn::PrintValue)
                }
                "panic" => Some(RuntimeFn::Panic),
                "str" => Some(RuntimeFn::ToString),
                _ => None,
            }
        };

        if let Some(rf) = runtime_fn {
            // PrintValue 和 Print 不返回值，不需要 dst
            let no_return = matches!(rf, RuntimeFn::PrintValue | RuntimeFn::Print | RuntimeFn::Panic | RuntimeFn::ArrayPush | RuntimeFn::ArraySet | RuntimeFn::ObjectSet);
            if no_return {
                self.module.functions[func_idx].instructions.push(Instruction::RuntimeCall {
                    dst: None,
                    func: rf,
                    args,
                });
                // 返回一个虚拟值（不会被使用）
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                Ok(Operand::Local(local))
            } else {
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx].instructions.push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: rf,
                    args,
                });
                Ok(Operand::Local(local))
            }
        } else {
            // 查找用户定义的函数
            let callee_name = if call.object.is_some() {
                call.method.as_deref().unwrap_or(&call.callee)
            } else {
                &call.callee
            };
            let callee_idx = self.module.functions.iter().position(|f| f.name == callee_name)
                .ok_or_else(|| XuloError::new(ErrorKind::Semantic, format!("undefined function: {}", callee_name)))?;
            let ret_type = self.module.functions[callee_idx].return_type.clone();
            let local = self.module.functions[func_idx].add_local(ret_type);
            self.module.functions[func_idx].instructions.push(Instruction::Call {
                dst: Some(local),
                func: FuncId(callee_idx),
                args,
            });
            Ok(Operand::Local(local))
        }
    }

    fn generate_if(&mut self, if_expr: &IfExpr) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        let condition = self.generate_expression(&if_expr.condition)?;
        
        let then_label = self.module.functions[func_idx].add_label();
        let else_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();

        self.module.functions[func_idx].instructions.push(Instruction::Branch {
            condition,
            then_label,
            else_label,
        });

        self.module.functions[func_idx].instructions.push(Instruction::Label(then_label));
        for stmt in &if_expr.then_branch.statements {
            self.generate_statement(stmt)?;
        }
        let then_has_return = matches!(
            self.module.functions[func_idx].instructions.last(),
            Some(Instruction::Return(_))
        );
        if !then_has_return {
            self.module.functions[func_idx].instructions.push(Instruction::Jump(end_label));
        }

        self.module.functions[func_idx].instructions.push(Instruction::Label(else_label));
        if let Some(else_branch) = &if_expr.else_branch {
            for stmt in &else_branch.statements {
                self.generate_statement(stmt)?;
            }
        }
        let else_has_return = matches!(
            self.module.functions[func_idx].instructions.last(),
            Some(Instruction::Return(_))
        );
        if !else_has_return {
            self.module.functions[func_idx].instructions.push(Instruction::Jump(end_label));
        }

        // Only emit end_label if at least one branch jumps to it
        if !then_has_return || !else_has_return {
            self.module.functions[func_idx].instructions.push(Instruction::Label(end_label));
        } else {
            // Both branches returned - emit end_label with a dead return so it's not empty
            let is_void = self.module.functions[func_idx].return_type.is_void();
            self.module.functions[func_idx].instructions.push(Instruction::Label(end_label));
            if !is_void {
                self.module.functions[func_idx].instructions.push(Instruction::Return(Some(Operand::Const(IrValue::I64(0)))));
            } else {
                self.module.functions[func_idx].instructions.push(Instruction::Return(None));
            }
        }

        Ok(())
    }

    fn generate_if_expr(&mut self, if_expr: &IfExpr) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let condition = self.generate_expression(&if_expr.condition)?;
        
        let then_label = self.module.functions[func_idx].add_label();
        let else_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();
        let result_local = self.module.functions[func_idx].add_local(IrType::Pointer);

        self.module.functions[func_idx].instructions.push(Instruction::Branch {
            condition,
            then_label,
            else_label,
        });

        self.module.functions[func_idx].instructions.push(Instruction::Label(then_label));
        // 简化处理：只取最后一个表达式
        if let Some(last_stmt) = if_expr.then_branch.statements.last() {
            if let Statement::Expr(e) = last_stmt {
                let val = self.generate_expression(&e.expr)?;
                self.module.functions[func_idx].instructions.push(Instruction::SetLocal {
                    dst: result_local,
                    src: val,
                });
            }
        }
        self.module.functions[func_idx].instructions.push(Instruction::Jump(end_label));

        self.module.functions[func_idx].instructions.push(Instruction::Label(else_label));
        if let Some(else_branch) = &if_expr.else_branch {
            if let Some(last_stmt) = else_branch.statements.last() {
                if let Statement::Expr(e) = last_stmt {
                    let val = self.generate_expression(&e.expr)?;
                    self.module.functions[func_idx].instructions.push(Instruction::SetLocal {
                        dst: result_local,
                        src: val,
                    });
                }
            }
        }
        self.module.functions[func_idx].instructions.push(Instruction::Label(end_label));

        Ok(Operand::Local(result_local))
    }

    fn generate_for(&mut self, for_stmt: &ForStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        
        // 创建循环变量
        let iter_local = self.module.functions[func_idx].add_local(IrType::I64);
        self.locals.insert(for_stmt.iter_var.clone(), iter_local);

        // 推断迭代变量的源级类型
        if let Some(iterable_ty) = self.infer_source_type_from_expr(&for_stmt.iterable) {
            match iterable_ty {
                Type::List(inner) => {
                    self.source_types.insert(for_stmt.iter_var.clone(), *inner);
                }
                _ => {}
            }
        }

        let start_label = self.module.functions[func_idx].add_label();
        let body_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();
        let continue_label = self.module.functions[func_idx].add_label();

        self.loop_stack.push(LoopContext {
            break_label: end_label,
            continue_label,
        });

        // 生成范围循环的 IR
        if let Expression::Range(range) = &for_stmt.iterable {
            let start = self.generate_expression(&range.start)?;
            let end = self.generate_expression(&range.end)?;
            
            // 初始化循环变量
            self.module.functions[func_idx].instructions.push(Instruction::SetLocal {
                dst: iter_local,
                src: start,
            });

            // 跳转到循环开始
            self.module.functions[func_idx].instructions.push(Instruction::Jump(start_label));

            // 循环开始
            self.module.functions[func_idx].instructions.push(Instruction::Label(start_label));
            
            // 检查条件
            let cond_local = self.module.functions[func_idx].add_local(IrType::Bool);
            if range.end_inclusive {
                self.module.functions[func_idx].instructions.push(Instruction::Lte {
                    dst: cond_local,
                    left: Operand::Local(iter_local),
                    right: end,
                });
            } else {
                self.module.functions[func_idx].instructions.push(Instruction::Lt {
                    dst: cond_local,
                    left: Operand::Local(iter_local),
                    right: end,
                });
            }

            self.module.functions[func_idx].instructions.push(Instruction::Branch {
                condition: Operand::Local(cond_local),
                then_label: body_label,
                else_label: end_label,
            });

            // 循环体
            self.module.functions[func_idx].instructions.push(Instruction::Label(body_label));
            for stmt in &for_stmt.body.statements {
                self.generate_statement(stmt)?;
            }

            // 跳转到 continue 标签
            self.module.functions[func_idx].instructions.push(Instruction::Jump(continue_label));

            // continue 标签
            self.module.functions[func_idx].instructions.push(Instruction::Label(continue_label));
            
            // 递增
            let one = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx].instructions.push(Instruction::Const {
                dst: one,
                value: IrValue::I64(1),
            });
            let new_val = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx].instructions.push(Instruction::Add {
                dst: new_val,
                left: Operand::Local(iter_local),
                right: Operand::Local(one),
            });
            self.module.functions[func_idx].instructions.push(Instruction::SetLocal {
                dst: iter_local,
                src: Operand::Local(new_val),
            });

            self.module.functions[func_idx].instructions.push(Instruction::Jump(start_label));
        } else {
            // Generic iteration over any expression (array or variable)
            let array_operand = self.generate_expression(&for_stmt.iterable)?;
            let array_local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx].instructions.push(Instruction::SetLocal {
                dst: array_local,
                src: array_operand,
            });

            // Get array length
            let len_local = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx].instructions.push(Instruction::RuntimeCall {
                dst: Some(len_local),
                func: RuntimeFn::ArrayLen,
                args: vec![Operand::Local(array_local)],
            });

            // index = 0
            let index_local = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx].instructions.push(Instruction::Const {
                dst: index_local,
                value: IrValue::I64(0),
            });

            // Jump to start
            self.module.functions[func_idx].instructions.push(Instruction::Jump(start_label));

            // Loop start: check index < len
            self.module.functions[func_idx].instructions.push(Instruction::Label(start_label));
            let cond_local = self.module.functions[func_idx].add_local(IrType::Bool);
            self.module.functions[func_idx].instructions.push(Instruction::Lt {
                dst: cond_local,
                left: Operand::Local(index_local),
                right: Operand::Local(len_local),
            });
            self.module.functions[func_idx].instructions.push(Instruction::Branch {
                condition: Operand::Local(cond_local),
                then_label: body_label,
                else_label: end_label,
            });

            // Body
            self.module.functions[func_idx].instructions.push(Instruction::Label(body_label));
            // iter_var = array[index]
            self.module.functions[func_idx].instructions.push(Instruction::GetIndex {
                dst: iter_local,
                array: Operand::Local(array_local),
                index: Operand::Local(index_local),
            });
            for stmt in &for_stmt.body.statements {
                self.generate_statement(stmt)?;
            }

            // Jump to continue
            self.module.functions[func_idx].instructions.push(Instruction::Jump(continue_label));

            // Continue: index++
            self.module.functions[func_idx].instructions.push(Instruction::Label(continue_label));
            let one = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx].instructions.push(Instruction::Const {
                dst: one,
                value: IrValue::I64(1),
            });
            let new_index = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx].instructions.push(Instruction::Add {
                dst: new_index,
                left: Operand::Local(index_local),
                right: Operand::Local(one),
            });
            self.module.functions[func_idx].instructions.push(Instruction::SetLocal {
                dst: index_local,
                src: Operand::Local(new_index),
            });
            self.module.functions[func_idx].instructions.push(Instruction::Jump(start_label));
        }

        self.module.functions[func_idx].instructions.push(Instruction::Label(end_label));
        self.loop_stack.pop();

        Ok(())
    }

    fn generate_while(&mut self, while_stmt: &WhileStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        
        let start_label = self.module.functions[func_idx].add_label();
        let body_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();
        let continue_label = self.module.functions[func_idx].add_label();

        self.loop_stack.push(LoopContext {
            break_label: end_label,
            continue_label,
        });

        // 循环开始
        self.module.functions[func_idx].instructions.push(Instruction::Label(start_label));
        
        // 检查条件
        let condition = self.generate_expression(&while_stmt.condition)?;
        self.module.functions[func_idx].instructions.push(Instruction::Branch {
            condition,
            then_label: body_label,
            else_label: end_label,
        });

        // 循环体
        self.module.functions[func_idx].instructions.push(Instruction::Label(body_label));
        for stmt in &while_stmt.body.statements {
            self.generate_statement(stmt)?;
        }

        // continue 标签
        self.module.functions[func_idx].instructions.push(Instruction::Label(continue_label));
        self.module.functions[func_idx].instructions.push(Instruction::Jump(start_label));

        self.module.functions[func_idx].instructions.push(Instruction::Label(end_label));
        self.loop_stack.pop();

        Ok(())
    }

    fn generate_assign(&mut self, assign: &AssignStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        let value = self.generate_expression(&assign.value)?;

        match &assign.target {
            AssignTarget::Name(name) => {
                if let Some(local) = self.locals.get(name) {
                    self.module.functions[func_idx].instructions.push(Instruction::SetLocal {
                        dst: *local,
                        src: value,
                    });
                }
            }
            AssignTarget::Member(obj, field) => {
                let obj_operand = self.generate_expression(obj)?;
                let tag = self.determine_element_tag(&assign.value);
                self.module.functions[func_idx].instructions.push(Instruction::SetField {
                    object: obj_operand,
                    field: field.clone(),
                    value,
                    tag,
                });
            }
            AssignTarget::Index(obj, idx) => {
                let obj_operand = self.generate_expression(obj)?;
                let idx_operand = self.generate_expression(idx)?;
                let tag = self.determine_element_tag(&assign.value);
                self.module.functions[func_idx].instructions.push(Instruction::SetIndex {
                    array: obj_operand,
                    index: idx_operand,
                    value,
                    tag,
                });
            }
        }

        Ok(())
    }

    fn generate_member_access(&mut self, member: &MemberAccess) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let object = self.generate_expression(&member.object)?;
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        
        self.module.functions[func_idx].instructions.push(Instruction::GetField {
            dst: local,
            object,
            field: member.property.clone(),
        });

        Ok(Operand::Local(local))
    }

    fn generate_index(&mut self, index: &IndexExpr) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let array = self.generate_expression(&index.object)?;
        let idx = self.generate_expression(&index.index)?;
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        
        self.module.functions[func_idx].instructions.push(Instruction::GetIndex {
            dst: local,
            array,
            index: idx,
        });

        Ok(Operand::Local(local))
    }

    fn type_to_ir(&self, ty: Option<&Type>) -> IrType {
        match ty {
            None => IrType::Void,
            Some(t) => match t {
                Type::Boolean => IrType::Bool,
                Type::String | Type::Null | Type::Object | Type::List(_)
                | Type::Named(_) | Type::Literal(_) | Type::Optional(_)
                | Type::Union(_) | Type::Intersection(_) | Type::ObjectType(_)
                | Type::FnSig { .. } | Type::Async(_) | Type::Any => IrType::Pointer,
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
