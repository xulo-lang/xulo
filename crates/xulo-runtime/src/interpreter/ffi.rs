//! FFI 导入桥（spec ffi.md）：extern 块在任何语句执行前注册成 Native 绑定，
//! 调用时经 `dlsym` 拿到的地址走 build.rs 生成的 C trampoline。
//!
//! 边界表示：
//! - 实参窄整类/`Ptr` 统一以 i64 携带（x86-64 SysV 下 callee 只读低位）；
//! - 返回按宽度读出 `Wide` 位模式，符号扩展在此按 AST 边界类型解释
//!   （窄返回的高位未定义，trampoline 必须按窄类型声明）；
//! - `f32`/`f64` 全程数值传递（不是位模式，与打印槽的位打包区分）；
//! - `Ptr` 零地址表示为 `Null`（只有 `Ptr?` 可空），`Ptr` 结果非零。

use std::cell::RefCell;
use std::ffi::{CString, c_char, c_void};
use std::rc::Rc;

use xulo_core::ast::{ExternBlockDecl, Type};
use xulo_core::error::{ErrorKind, XuloError};

use crate::env::Env;
use crate::interpreter::{Interpreter, NativeFn, RunError};
use crate::value::{PtrValue, Value};

/// 解释器支持的最大 extern 参数个数（与 build.rs 的生成上限同步）
const MAX_PARAMS: usize = 6;

/// 调用侧实参：参数寄存器类（i64 / f32 / f64）
pub(crate) enum FfiArg {
    Int(i64),
    F32(f32),
    F64(f64),
}

/// 调用侧返回：`Wide` 是按返回宽度读出的位模式，符号/宽度解释在
/// [`ret_to_value`] 按 AST 类型完成
pub(crate) enum FfiRet {
    Void,
    Wide(u64),
    F32(f32),
    F64(f64),
}

pub(crate) fn arg_i64(args: &[FfiArg], i: usize) -> Option<i64> {
    match args.get(i) {
        Some(FfiArg::Int(v)) => Some(*v),
        _ => None,
    }
}

pub(crate) fn arg_f32(args: &[FfiArg], i: usize) -> Option<f32> {
    match args.get(i) {
        Some(FfiArg::F32(v)) => Some(*v),
        _ => None,
    }
}

pub(crate) fn arg_f64(args: &[FfiArg], i: usize) -> Option<f64> {
    match args.get(i) {
        Some(FfiArg::F64(v)) => Some(*v),
        _ => None,
    }
}

// build.rs 按 (参数类, 个数, 返回类) 穷举生成的 trampoline 表
include!(concat!(env!("OUT_DIR"), "/ffi_trampolines.rs"));

/// glibc：在进程全局符号表里查符号（`RTLD_DEFAULT = (void *) -1`）。
/// xulo 运行在 Linux/glibc；符号在链接期已可见（libc/libm 与所依赖的动态库）。
const RTLD_DEFAULT: *const c_void = usize::MAX as *const c_void;

unsafe extern "C" {
    fn dlsym(handle: *const c_void, symbol: *const c_char) -> *mut c_void;
}

fn lookup_symbol(symbol: &str) -> Result<usize, XuloError> {
    let c = CString::new(symbol).map_err(|_| {
        XuloError::new(
            ErrorKind::Runtime,
            format!("FFI symbol `{symbol}` contains a NUL byte"),
        )
    })?;
    let addr = unsafe { dlsym(RTLD_DEFAULT, c.as_ptr()) } as usize;
    if addr == 0 {
        return Err(XuloError::new(
            ErrorKind::Runtime,
            format!("FFI symbol `{symbol}` not found in the process"),
        ));
    }
    Ok(addr)
}

/// 边界类型的参数寄存器类：0 = i64（整/布尔/Ptr），1 = f32，2 = f64。
/// E0901 之外的拼写按整类兜底（语义层已拒绝，不可达）。
fn param_class(ty: &Type) -> u8 {
    match ty {
        Type::Optional(inner) => param_class(inner),
        Type::Named(n, _) => match n.as_str() {
            "f32" => 1,
            "f64" => 2,
            _ => 0,
        },
        _ => 0,
    }
}

/// 返回类：与 build.rs 的 `ret` 索引一一对应 —— 0 = void，1..=4 = 按宽度
/// （u8/u16/u32/u64，符号由调用侧按 AST 解释），5 = f32，6 = f64。
/// `Ptr`/`Ptr?` 是地址，按 u64 返回。
fn ret_class_of(ty: Option<&Type>) -> u8 {
    let Some(ty) = ty else {
        return 0;
    };
    match ty {
        Type::Boolean => 1,
        Type::Optional(_) => 4,
        Type::Named(n, _) => match n.as_str() {
            "f32" => 5,
            "f64" => 6,
            "i8" | "u8" | "bool" => 1,
            "i16" | "u16" => 2,
            "i32" | "u32" => 3,
            "unit" => 0,
            // i64/u64/usize/isize/Ptr —— 默认走宽返回
            _ => 4,
        },
        // E0901 之外不可达；按地址宽度兜底
        _ => 4,
    }
}

fn is_ptr(ty: &Type) -> bool {
    match ty {
        Type::Named(n, _) => n == "Ptr",
        Type::Optional(inner) => is_ptr(inner),
        _ => false,
    }
}

fn err(message: String) -> RunError {
    RunError::Err(XuloError::new(ErrorKind::Runtime, message))
}

/// 一个已解析的 extern 导入：地址、边界类型与预计算的 trampoline 索引
struct ExternDecl {
    /// 本地绑定名（错误信息用）
    local: String,
    params: Vec<Type>,
    ret: Option<Type>,
    addr: usize,
    param_classes: Vec<u8>,
    ret_class: u8,
}

fn value_to_arg(v: &Value, ty: &Type, local: &str, idx: usize) -> Result<FfiArg, RunError> {
    let where_ = format!("extern `{local}` argument {}", idx + 1);
    match ty {
        Type::Boolean => match v {
            Value::Boolean(b) => Ok(FfiArg::Int(i64::from(*b))),
            other => Err(err(format!(
                "{where_} must be a `bool`, found {}",
                other.kind_name()
            ))),
        },
        Type::Optional(_) if is_ptr(ty) => match v {
            Value::Ptr(p) => Ok(FfiArg::Int(p.0 as i64)),
            Value::Null => Ok(FfiArg::Int(0)),
            other => Err(err(format!(
                "{where_} must be a `Ptr?`, found {}",
                other.kind_name()
            ))),
        },
        Type::Named(n, _) => match n.as_str() {
            "Ptr" => match v {
                Value::Ptr(p) => Ok(FfiArg::Int(p.0 as i64)),
                // `Ptr` 非空由语义保证；防御性接受 null 当地址 0
                Value::Null => Ok(FfiArg::Int(0)),
                other => Err(err(format!(
                    "{where_} must be a `Ptr`, found {}",
                    other.kind_name()
                ))),
            },
            "f32" => match v {
                Value::Number(n) => Ok(FfiArg::F32(*n as f32)),
                other => Err(err(format!(
                    "{where_} must be a `f32`, found {}",
                    other.kind_name()
                ))),
            },
            "f64" => match v {
                Value::Number(n) => Ok(FfiArg::F64(*n)),
                other => Err(err(format!(
                    "{where_} must be a `f64`, found {}",
                    other.kind_name()
                ))),
            },
            // i8…u64 / usize / isize：值域由 E0209/E0201 保证
            _ => match v {
                Value::Number(n) => Ok(FfiArg::Int(*n as i64)),
                other => Err(err(format!(
                    "{where_} must be a number, found {}",
                    other.kind_name()
                ))),
            },
        },
        _ => Err(err(format!("{where_} has a non-ABI type"))),
    }
}

fn ret_to_value(ret: FfiRet, ty: Option<&Type>) -> Value {
    match ret {
        FfiRet::Void => Value::Null,
        FfiRet::F32(v) => Value::Number(v as f64),
        FfiRet::F64(v) => Value::Number(v),
        FfiRet::Wide(w) => match ty {
            Some(Type::Boolean) => Value::Boolean(w != 0),
            // E0901 只放行 `Ptr?` 一种可选拼写
            Some(Type::Optional(_)) => ptr_or_null(w),
            Some(Type::Named(n, _)) => match n.as_str() {
                "Ptr" => ptr_or_null(w),
                "i8" => Value::Number((w as u8 as i8) as f64),
                "i16" => Value::Number((w as u16 as i16) as f64),
                "i32" => Value::Number((w as u32 as i32) as f64),
                "i64" | "isize" => Value::Number(w as i64 as f64),
                "u8" => Value::Number((w as u8) as f64),
                "u16" => Value::Number((w as u16) as f64),
                "u32" => Value::Number((w as u32) as f64),
                "u64" | "usize" => Value::Number(w as f64),
                _ => Value::Number(w as f64),
            },
            _ => Value::Number(w as f64),
        },
    }
}

fn ptr_or_null(w: u64) -> Value {
    if w == 0 {
        Value::Null
    } else {
        Value::Ptr(PtrValue(w as usize))
    }
}

impl Interpreter {
    /// 把一个 extern 块的每条 item 注册成 Native 绑定（在任何语句执行前，
    /// 与声明注册同一遍；spec ffi.md）。符号在此时解析，签名超出 trampoline
    /// 上限也在此时报错——错误先于程序任何副作用。
    pub(crate) fn register_externs(
        &self,
        b: &ExternBlockDecl,
        env: &Rc<RefCell<Env>>,
    ) -> Result<(), XuloError> {
        for item in &b.items {
            if item.params.len() > MAX_PARAMS {
                return Err(XuloError::new(
                    ErrorKind::Runtime,
                    format!(
                        "extern `{}` has {} parameters; the interpreter supports at most {MAX_PARAMS}",
                        item.local_name,
                        item.params.len()
                    ),
                ));
            }
            let addr = lookup_symbol(&item.name)?;
            let params: Vec<Type> = item
                .params
                .iter()
                .map(|p| {
                    p.type_annotation
                        .clone()
                        .unwrap_or(Type::Named("i64".to_string(), Vec::new()))
                })
                .collect();
            let param_classes: Vec<u8> = params.iter().map(param_class).collect();
            let ret_class = ret_class_of(item.return_type.as_ref());
            let decl = Rc::new(ExternDecl {
                local: item.local_name.clone(),
                params,
                ret: item.return_type.clone(),
                addr,
                param_classes,
                ret_class,
            });
            let bound = decl.clone();
            let local_name = item.local_name.clone();
            let native: NativeFn = Rc::new(move |_interp, args| {
                if args.len() != bound.params.len() {
                    return Err(err(format!(
                        "extern `{}` expects {} arguments, got {}",
                        bound.local,
                        bound.params.len(),
                        args.len()
                    )));
                }
                let mut ffi_args = Vec::with_capacity(args.len());
                for (i, (v, ty)) in args.iter().zip(&bound.params).enumerate() {
                    ffi_args.push(value_to_arg(v, ty, &bound.local, i)?);
                }
                let ret = invoke(bound.addr, &bound.param_classes, bound.ret_class, &ffi_args)
                    .ok_or_else(|| {
                        err(format!(
                            "unsupported extern signature for `{}`",
                            bound.local
                        ))
                    })?;
                Ok(ret_to_value(ret, bound.ret.as_ref()))
            });
            env.borrow_mut()
                .define(&local_name, Value::Native(native), false);
            // 具名参数/省略补 null 的绑定依据（spec ffi.md Calling）
            self.extern_params
                .borrow_mut()
                .insert(local_name, params_declared_names(item));
        }
        Ok(())
    }
}

fn params_declared_names(item: &xulo_core::ast::ExternItem) -> Vec<String> {
    item.params.iter().map(|p| p.name.clone()).collect()
}
