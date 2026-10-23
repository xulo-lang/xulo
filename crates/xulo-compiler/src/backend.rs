//! 后端抽象：JIT（进程内执行）与 AOT（产出目标文件）的差异只落在这一层。
//!
//! 职责分层：
//! - `irgen`：xulo 源码 -> xulo IR（`xulo_core::ir`）
//! - `codegen`：把 xulo IR lowering 成 Cranelift 函数体（两后端共用一份逻辑）
//! - `backend`（本文件）：模块管理——声明/定义函数、字符串常量、运行时符号、收尾
//! - `aot`：目标文件与可执行文件的输出
//!
//! 将来若接 LLVM 或并行支持多后端，改动集中在这里的 `BackendMode`/`CompilerModule`，
//! `codegen`/`aot` 不用跟着改。

use std::collections::HashMap;
use std::ffi::CString;

use cranelift::codegen::Context;
use cranelift::codegen::ir::types::{F64, I64, Type};
use cranelift::codegen::ir::{FuncRef, Function, InstBuilder, Signature};
use cranelift::codegen::isa::TargetIsa;
use cranelift::codegen::settings;
use cranelift::frontend::FunctionBuilder;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule, ObjectProduct};

use xulo_core::error::{ErrorKind, XuloError};

/// 后端模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendMode {
    /// 进程内 JIT：finalize 后直接拿到入口函数指针
    Jit,
    /// AOT：产出 ObjectProduct（.o），由系统链接器生成可执行文件
    Aot,
}

/// 运行时函数描述：JIT 用 `addr` 注册进程内符号，
/// 两后端都按 `params`/`returns` 声明 import。
pub(crate) struct RuntimeFnDesc {
    pub(crate) name: &'static str,
    pub(crate) addr: *const u8,
    pub(crate) params: &'static [Type],
    pub(crate) returns: &'static [Type],
}

/// xulo-runtime / xulo-std 导出函数表（JIT 符号注册与两后端 import 声明共用一份事实）
pub(crate) const RUNTIME_FNS: &[RuntimeFnDesc] = &[
    RuntimeFnDesc {
        name: "xulo_print",
        addr: xulo_runtime::runtime::xulo_print as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_print_int",
        addr: xulo_runtime::runtime::xulo_print_int as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_print_float",
        addr: xulo_runtime::runtime::xulo_print_float as *const u8,
        params: &[cranelift::codegen::ir::types::F64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_print_value",
        addr: xulo_runtime::runtime::xulo_print_value as *const u8,
        params: &[I64, I64],
        returns: &[],
    },
    RuntimeFnDesc {
        name: "xulo_panic",
        addr: xulo_runtime::runtime::xulo_panic as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_string_concat",
        addr: xulo_runtime::runtime::xulo_string_concat as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_stringify",
        addr: xulo_runtime::runtime::xulo_stringify as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_alloc_object",
        addr: xulo_runtime::runtime::xulo_alloc_object as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_alloc_array",
        addr: xulo_runtime::runtime::xulo_alloc_array as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_array_push",
        addr: xulo_runtime::runtime::xulo_array_push as *const u8,
        params: &[I64, I64, I64],
        returns: &[],
    },
    RuntimeFnDesc {
        name: "xulo_array_len",
        addr: xulo_runtime::runtime::xulo_array_len as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_array_get",
        addr: xulo_runtime::runtime::xulo_array_get as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_array_set",
        addr: xulo_runtime::runtime::xulo_array_set as *const u8,
        params: &[I64, I64, I64, I64],
        returns: &[],
    },
    RuntimeFnDesc {
        name: "xulo_array_concat",
        addr: xulo_runtime::runtime::xulo_array_concat as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_object_get",
        addr: xulo_runtime::runtime::xulo_object_get as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_object_get_tag",
        addr: xulo_runtime::runtime::xulo_object_get_tag as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_object_set",
        addr: xulo_runtime::runtime::xulo_object_set as *const u8,
        params: &[I64, I64, I64, I64],
        returns: &[],
    },
    RuntimeFnDesc {
        name: "xulo_alloc_enum",
        addr: xulo_runtime::runtime::xulo_alloc_enum as *const u8,
        params: &[I64, I64, I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_enum_set",
        addr: xulo_runtime::runtime::xulo_enum_set as *const u8,
        params: &[I64, I64, I64, I64],
        returns: &[],
    },
    RuntimeFnDesc {
        name: "xulo_enum_variant",
        addr: xulo_runtime::runtime::xulo_enum_variant as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_enum_get",
        addr: xulo_runtime::runtime::xulo_enum_get as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_enum_get_tag",
        addr: xulo_runtime::runtime::xulo_enum_get_tag as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_as_f64",
        addr: xulo_runtime::runtime::xulo_as_f64 as *const u8,
        params: &[I64, I64],
        returns: &[cranelift::codegen::ir::types::F64],
    },
    RuntimeFnDesc {
        name: "xulo_register_float",
        addr: xulo_runtime::runtime::xulo_register_float as *const u8,
        params: &[I64],
        returns: &[],
    },
    RuntimeFnDesc {
        name: "xulo_register_string",
        addr: xulo_runtime::runtime::xulo_register_string as *const u8,
        params: &[I64],
        returns: &[],
    },
    // 闭包运行时：捕获单元 / 环境 / 闭包值
    RuntimeFnDesc {
        name: "xulo_alloc_cell",
        addr: xulo_runtime::runtime::xulo_alloc_cell as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_cell_get",
        addr: xulo_runtime::runtime::xulo_cell_get as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_cell_get_f64",
        addr: xulo_runtime::runtime::xulo_cell_get_f64 as *const u8,
        params: &[I64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_cell_set",
        addr: xulo_runtime::runtime::xulo_cell_set as *const u8,
        params: &[I64, I64],
        returns: &[],
    },
    RuntimeFnDesc {
        name: "xulo_alloc_env",
        addr: xulo_runtime::runtime::xulo_alloc_env as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_env_set",
        addr: xulo_runtime::runtime::xulo_env_set as *const u8,
        params: &[I64, I64, I64],
        returns: &[],
    },
    RuntimeFnDesc {
        name: "xulo_env_get",
        addr: xulo_runtime::runtime::xulo_env_get as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_alloc_closure",
        addr: xulo_runtime::runtime::xulo_alloc_closure as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_closure_func",
        addr: xulo_runtime::runtime::xulo_closure_func as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_closure_env",
        addr: xulo_runtime::runtime::xulo_closure_env as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_check_callable",
        addr: xulo_runtime::runtime::xulo_check_callable as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    // 正则：指针一律按 I64 传递（与 string/array/object 同规）
    RuntimeFnDesc {
        name: "xulo_regex_compile",
        addr: xulo_runtime::runtime::xulo_regex_compile as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_regex_test",
        addr: xulo_runtime::runtime::xulo_regex_test as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_regex_find",
        addr: xulo_runtime::runtime::xulo_regex_find as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_regex_find_all",
        addr: xulo_runtime::runtime::xulo_regex_find_all as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_regex_match_group",
        addr: xulo_runtime::runtime::xulo_regex_match_group as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_regex_replace",
        addr: xulo_runtime::runtime::xulo_regex_replace as *const u8,
        params: &[I64, I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_regex_replace_with",
        addr: xulo_runtime::runtime::xulo_regex_replace_with as *const u8,
        params: &[I64, I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_string_to_uppercase",
        addr: xulo_runtime::runtime::xulo_string_to_uppercase as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    // match 字面量 pattern / 区间表达式求值
    RuntimeFnDesc {
        name: "xulo_literal_match",
        addr: xulo_runtime::runtime::xulo_literal_match as *const u8,
        params: &[I64, I64, I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_range_list",
        addr: xulo_runtime::runtime::xulo_range_list as *const u8,
        params: &[F64, F64, I64],
        returns: &[I64],
    },
    // Math 命名空间：float 组返回 f64（对齐语义 builtin_member 的 BuiltinRet）
    RuntimeFnDesc {
        name: "xulo_math_abs",
        addr: xulo_std::ffi::xulo_math_abs as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_min",
        addr: xulo_std::ffi::xulo_math_min as *const u8,
        params: &[F64, F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_max",
        addr: xulo_std::ffi::xulo_math_max as *const u8,
        params: &[F64, F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_floor",
        addr: xulo_std::ffi::xulo_math_floor as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_ceil",
        addr: xulo_std::ffi::xulo_math_ceil as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_round",
        addr: xulo_std::ffi::xulo_math_round as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_trunc",
        addr: xulo_std::ffi::xulo_math_trunc as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_sqrt",
        addr: xulo_std::ffi::xulo_math_sqrt as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_cbrt",
        addr: xulo_std::ffi::xulo_math_cbrt as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_pow",
        addr: xulo_std::ffi::xulo_math_pow as *const u8,
        params: &[F64, F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_sin",
        addr: xulo_std::ffi::xulo_math_sin as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_cos",
        addr: xulo_std::ffi::xulo_math_cos as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_tan",
        addr: xulo_std::ffi::xulo_math_tan as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_asin",
        addr: xulo_std::ffi::xulo_math_asin as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_acos",
        addr: xulo_std::ffi::xulo_math_acos as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_atan",
        addr: xulo_std::ffi::xulo_math_atan as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_atan2",
        addr: xulo_std::ffi::xulo_math_atan2 as *const u8,
        params: &[F64, F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_log",
        addr: xulo_std::ffi::xulo_math_log as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_log2",
        addr: xulo_std::ffi::xulo_math_log2 as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_log10",
        addr: xulo_std::ffi::xulo_math_log10 as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_exp",
        addr: xulo_std::ffi::xulo_math_exp as *const u8,
        params: &[F64],
        returns: &[F64],
    },
    RuntimeFnDesc {
        name: "xulo_math_to_fixed",
        addr: xulo_std::ffi::xulo_math_to_fixed as *const u8,
        params: &[F64, I64],
        returns: &[I64],
    },
    // Time 命名空间：返回自 epoch/进程原点的整数读数
    RuntimeFnDesc {
        name: "xulo_time_now",
        addr: xulo_std::ffi::xulo_time_now as *const u8,
        params: &[],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_time_now_nanos",
        addr: xulo_std::ffi::xulo_time_now_nanos as *const u8,
        params: &[],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_time_monotonic",
        addr: xulo_std::ffi::xulo_time_monotonic as *const u8,
        params: &[],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_time_monotonic_nanos",
        addr: xulo_std::ffi::xulo_time_monotonic_nanos as *const u8,
        params: &[],
        returns: &[I64],
    },
    // `std::fs`：可失败操作返回 Result 枚举指针，exists 返回 0/1
    RuntimeFnDesc {
        name: "xulo_fs_read",
        addr: xulo_runtime::runtime::xulo_fs_read as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_fs_write",
        addr: xulo_runtime::runtime::xulo_fs_write as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_fs_exists",
        addr: xulo_runtime::runtime::xulo_fs_exists as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_fs_remove",
        addr: xulo_runtime::runtime::xulo_fs_remove as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_fs_mkdir",
        addr: xulo_runtime::runtime::xulo_fs_mkdir as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    // `std::io`：写真实 stdout/stderr、读真实 stdin，七个成员一律返回
    // Result 枚举指针（载荷 tag 区分 null/字符串）
    RuntimeFnDesc {
        name: "xulo_io_print",
        addr: xulo_runtime::runtime::xulo_io_print as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_io_println",
        addr: xulo_runtime::runtime::xulo_io_println as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_io_eprint",
        addr: xulo_runtime::runtime::xulo_io_eprint as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_io_eprintln",
        addr: xulo_runtime::runtime::xulo_io_eprintln as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_io_flush",
        addr: xulo_runtime::runtime::xulo_io_flush as *const u8,
        params: &[],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_io_read_line",
        addr: xulo_runtime::runtime::xulo_io_read_line as *const u8,
        params: &[],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_io_read",
        addr: xulo_runtime::runtime::xulo_io_read as *const u8,
        params: &[],
        returns: &[I64],
    },
    // `std::os`：可失败成员返回 Result 枚举指针（getenv 载荷区分 null），
    // pid 返回数字，platform/arch 返回驻留字符串
    RuntimeFnDesc {
        name: "xulo_os_getenv",
        addr: xulo_runtime::runtime::xulo_os_getenv as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_os_setenv",
        addr: xulo_runtime::runtime::xulo_os_setenv as *const u8,
        params: &[I64, I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_os_cwd",
        addr: xulo_runtime::runtime::xulo_os_cwd as *const u8,
        params: &[],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_os_chdir",
        addr: xulo_runtime::runtime::xulo_os_chdir as *const u8,
        params: &[I64],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_os_pid",
        addr: xulo_runtime::runtime::xulo_os_pid as *const u8,
        params: &[],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_os_platform",
        addr: xulo_runtime::runtime::xulo_os_platform as *const u8,
        params: &[],
        returns: &[I64],
    },
    RuntimeFnDesc {
        name: "xulo_os_arch",
        addr: xulo_runtime::runtime::xulo_os_arch as *const u8,
        params: &[],
        returns: &[I64],
    },
];

/// 已 intern 的字符串常量：JIT 与 AOT 的落地方式不同，但对 lowering 透明
enum InternedString {
    /// JIT：进程内泄漏的 C 字符串，取地址做 iconst
    Ptr(*const u8),
    /// AOT：模块 .rodata 里的数据符号，用 global_value 取地址
    Data(DataId),
}

enum ModuleInner {
    Jit(JITModule),
    Aot(ObjectModule),
}

/// 后端模块管理：包住 JITModule / ObjectModule，给 codegen 一个统一入口
pub struct CompilerModule {
    mode: BackendMode,
    inner: ModuleInner,
    /// 字符串常量 intern 表（按内容去重，两后端共用）
    strings: HashMap<String, InternedString>,
}

impl CompilerModule {
    /// 按模式创建后端模块（JIT 会把运行时函数注册成进程内符号）
    pub fn new(mode: BackendMode) -> Result<Self, XuloError> {
        let inner = match mode {
            BackendMode::Jit => {
                let mut builder = JITBuilder::new(cranelift_module::default_libcall_names())
                    .map_err(|e| {
                        XuloError::new(
                            ErrorKind::Codegen,
                            format!("failed to create JITBuilder: {}", e),
                        )
                    })?;
                for desc in RUNTIME_FNS {
                    builder.symbol(desc.name, desc.addr);
                }
                ModuleInner::Jit(JITModule::new(builder))
            }
            BackendMode::Aot => {
                let isa_builder = cranelift_native::builder().map_err(|e| {
                    XuloError::new(
                        ErrorKind::Codegen,
                        format!("failed to get ISA builder: {}", e),
                    )
                })?;
                let flag_builder = settings::builder();
                let isa = isa_builder
                    .finish(settings::Flags::new(flag_builder))
                    .map_err(|e| {
                        XuloError::new(ErrorKind::Codegen, format!("failed to create ISA: {}", e))
                    })?;
                let obj_builder = ObjectBuilder::new(
                    isa,
                    "xulo_program".to_string(),
                    cranelift_module::default_libcall_names(),
                )
                .map_err(|e| {
                    XuloError::new(
                        ErrorKind::Codegen,
                        format!("failed to create ObjectBuilder: {}", e),
                    )
                })?;
                ModuleInner::Aot(ObjectModule::new(obj_builder))
            }
        };
        Ok(CompilerModule {
            mode,
            inner,
            strings: HashMap::new(),
        })
    }

    /// 当前后端模式
    pub fn mode(&self) -> BackendMode {
        self.mode
    }

    /// 声明一个函数（用户函数 Export，运行时函数 Import）
    pub fn declare_function(
        &mut self,
        name: &str,
        linkage: Linkage,
        sig: &Signature,
    ) -> Result<FuncId, XuloError> {
        let result = match &mut self.inner {
            ModuleInner::Jit(m) => m.declare_function(name, linkage, sig),
            ModuleInner::Aot(m) => m.declare_function(name, linkage, sig),
        };
        result.map_err(|e| {
            XuloError::new(
                ErrorKind::Codegen,
                format!("failed to declare function {}: {}", name, e),
            )
        })
    }

    /// 把已声明的函数带进某个正在构建的函数体（供 call 使用）
    pub fn declare_func_in_func(&mut self, id: FuncId, func: &mut Function) -> FuncRef {
        match &mut self.inner {
            ModuleInner::Jit(m) => m.declare_func_in_func(id, func),
            ModuleInner::Aot(m) => m.declare_func_in_func(id, func),
        }
    }

    /// 新建一个函数编译上下文
    pub fn make_context(&self) -> Context {
        match &self.inner {
            ModuleInner::Jit(m) => m.make_context(),
            ModuleInner::Aot(m) => m.make_context(),
        }
    }

    /// 目标 ISA（verifier 需要）
    pub fn isa(&self) -> &dyn TargetIsa {
        match &self.inner {
            ModuleInner::Jit(m) => m.isa(),
            ModuleInner::Aot(m) => m.isa(),
        }
    }

    /// 定义（编译）一个函数体
    pub fn define_function(&mut self, id: FuncId, ctx: &mut Context) -> Result<(), XuloError> {
        let result = match &mut self.inner {
            ModuleInner::Jit(m) => m.define_function(id, ctx),
            ModuleInner::Aot(m) => m.define_function(id, ctx),
        };
        result.map(|_| ()).map_err(|e| {
            XuloError::new(
                ErrorKind::Codegen,
                format!("failed to define function: {}", e),
            )
        })
    }

    /// intern 一个字符串常量（按内容去重；JIT 泄漏进程内指针，AOT 写入 .rodata）
    pub fn intern_string(&mut self, s: &str) -> Result<(), XuloError> {
        self.intern_string_inner(s)?;
        Ok(())
    }

    /// 取字符串常量地址对应的 cranelift 值（未 intern 过则按需 intern）
    pub fn string_value(
        &mut self,
        s: &str,
        builder: &mut FunctionBuilder,
    ) -> cranelift::codegen::ir::Value {
        if !self.strings.contains_key(s) {
            self.intern_string_inner(s)
                .expect("failed to intern string constant");
        }
        match &self.strings[s] {
            InternedString::Ptr(ptr) => builder.ins().iconst(I64, *ptr as i64),
            InternedString::Data(id) => {
                let data_id = *id;
                let gv = match &self.inner {
                    ModuleInner::Jit(_) => unreachable!("JIT 字符串常量不会是数据符号"),
                    ModuleInner::Aot(m) => m.declare_data_in_func(data_id, builder.func),
                };
                builder.ins().global_value(I64, gv)
            }
        }
    }

    /// JIT 收尾：把已定义的函数固化进可执行内存（AOT 无操作，收尾在 finish）
    pub fn finalize(&mut self) -> Result<(), XuloError> {
        match &mut self.inner {
            ModuleInner::Jit(m) => m.finalize_definitions().map_err(|e| {
                XuloError::new(ErrorKind::Codegen, format!("failed to finalize: {}", e))
            }),
            ModuleInner::Aot(_) => Ok(()),
        }
    }

    /// JIT 收尾后取入口函数指针（仅 JIT 模式有意义）
    pub fn finalized_entry(&self, id: FuncId) -> *const u8 {
        match &self.inner {
            ModuleInner::Jit(m) => m.get_finalized_function(id),
            ModuleInner::Aot(_) => panic!("finalized_entry 只支持 JIT 后端"),
        }
    }

    /// AOT 收尾：产出目标文件数据（仅 AOT 模式有意义）
    pub fn finish(self) -> ObjectProduct {
        match self.inner {
            ModuleInner::Jit(_) => panic!("finish 只支持 AOT 后端"),
            ModuleInner::Aot(m) => m.finish(),
        }
    }

    fn intern_string_inner(&mut self, s: &str) -> Result<InternedString, XuloError> {
        if let Some(existing) = self.strings.get(s) {
            return Ok(match existing {
                InternedString::Ptr(p) => InternedString::Ptr(*p),
                InternedString::Data(d) => InternedString::Data(*d),
            });
        }
        let interned = match &mut self.inner {
            ModuleInner::Jit(_) => {
                let c_string = CString::new(s).map_err(|e| {
                    XuloError::new(ErrorKind::Codegen, format!("invalid string: {}", e))
                })?;
                InternedString::Ptr(c_string.into_raw() as *const u8)
            }
            ModuleInner::Aot(m) => {
                let name = format!("str_{}", self.strings.len());
                let data_id = m
                    .declare_data(&name, Linkage::Local, false, false)
                    .map_err(|e| {
                        XuloError::new(
                            ErrorKind::Codegen,
                            format!("failed to declare string data: {}", e),
                        )
                    })?;
                let mut data_desc = DataDescription::new();
                let mut bytes = s.as_bytes().to_vec();
                bytes.push(0); // null terminator
                data_desc.define(bytes.into_boxed_slice());
                m.define_data(data_id, &data_desc).map_err(|e| {
                    XuloError::new(
                        ErrorKind::Codegen,
                        format!("failed to define string data: {}", e),
                    )
                })?;
                InternedString::Data(data_id)
            }
        };
        self.strings
            .insert(s.to_string(), interned.clone_interned());
        Ok(interned)
    }
}

impl InternedString {
    fn clone_interned(&self) -> Self {
        match self {
            InternedString::Ptr(p) => InternedString::Ptr(*p),
            InternedString::Data(d) => InternedString::Data(*d),
        }
    }
}

/// 引用 xulo-runtime 与 xulo-std，确保链接器包含运行时库（JIT 进程内执行时需要）
#[allow(dead_code)]
fn _ensure_runtime_linked() {
    let _ = xulo_runtime::runtime::xulo_print as *const u8;
    let _ = xulo_runtime::runtime::xulo_print_value as *const u8;
    let _ = xulo_runtime::runtime::xulo_string_concat as *const u8;
    let _ = xulo_runtime::runtime::xulo_stringify as *const u8;
    let _ = xulo_runtime::runtime::xulo_alloc_object as *const u8;
    let _ = xulo_runtime::runtime::xulo_alloc_array as *const u8;
    let _ = xulo_runtime::runtime::xulo_array_push as *const u8;
    let _ = xulo_runtime::runtime::xulo_array_len as *const u8;
    let _ = xulo_runtime::runtime::xulo_array_get as *const u8;
    let _ = xulo_runtime::runtime::xulo_array_set as *const u8;
    let _ = xulo_runtime::runtime::xulo_object_get as *const u8;
    let _ = xulo_runtime::runtime::xulo_object_set as *const u8;
    let _ = xulo_std::ffi::xulo_math_floor as *const u8;
    let _ = xulo_std::ffi::xulo_math_to_fixed as *const u8;
    let _ = xulo_std::ffi::xulo_time_monotonic as *const u8;
}
