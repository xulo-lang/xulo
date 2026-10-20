//! AOT 产物输出：把 IR 编成 .o，再用系统链接器生成可执行文件。
//!
//! 函数体 lowering 与 JIT 共用 `codegen`（见 `backend` 的后端分层说明）；
//! 本文件只关心“输出成文件”：目标文件落盘、运行时库定位、cc 链接参数。

use std::path::{Path, PathBuf};

use cranelift_object::ObjectProduct;

use xulo_core::error::{ErrorKind, XuloError};
use xulo_core::ir::IrModule;

use crate::backend::BackendMode;
use crate::codegen::CodeGen;

/// AOT codegen 入口：内部复用 codegen 的统一 lowering，收尾走 .o 产物
pub struct AotCodeGen {
    codegen: CodeGen,
}

impl AotCodeGen {
    pub fn new() -> Result<Self, XuloError> {
        Ok(AotCodeGen {
            codegen: CodeGen::with_mode(BackendMode::Aot)?,
        })
    }

    /// IR -> 目标文件数据（.o 字节由 `write_object_file` 落盘）
    pub fn compile(self, ir_module: &IrModule) -> Result<ObjectProduct, XuloError> {
        self.codegen.compile_aot(ir_module)
    }
}

/// 把 AOT 产物写成 .o 文件
pub fn write_object_file(product: &ObjectProduct, obj_path: &Path) -> Result<(), XuloError> {
    let file = std::fs::File::create(obj_path).map_err(|e| {
        XuloError::new(
            ErrorKind::Codegen,
            format!("failed to create {}: {}", obj_path.display(), e),
        )
    })?;
    product.object.write_stream(file).map_err(|e| {
        XuloError::new(
            ErrorKind::Codegen,
            format!("failed to write object file: {}", e),
        )
    })
}

/// 用系统 cc 把 .o 与 libxulo_runtime.a 链接成可执行文件。
///
/// - `--gc-sections`：归档成员整块进链接，不 gc 会把 addr2line/gimli 回溯和用不到的
///   std 全拉进来（.text 1.54MB -> 0.33MB）
/// - `--strip-debug`：staticlib 内含工具链预编译 std 的 DWARF，实测占产物 66%
pub fn link_executable(obj_path: &Path, exe_path: &Path) -> Result<(), XuloError> {
    let runtime_lib = runtime_library_path()?;
    let status = std::process::Command::new("cc")
        .arg("-no-pie")
        .arg("-o")
        .arg(exe_path)
        .arg(obj_path)
        .arg(&runtime_lib)
        .arg("-lpthread")
        .arg("-lm")
        .arg("-ldl")
        .arg("-Wl,--gc-sections")
        .arg("-Wl,--strip-debug")
        .status()
        .map_err(|e| {
            XuloError::new(
                ErrorKind::Codegen,
                format!("failed to run linker: {}（请确认已安装 cc/gcc/clang）", e),
            )
        })?;
    if !status.success() {
        return Err(XuloError::new(
            ErrorKind::Codegen,
            format!("linker failed with status: {}", status),
        ));
    }
    Ok(())
}

/// 定位工作区里的 libxulo_runtime.a（AOT 链接需要）
fn runtime_library_path() -> Result<PathBuf, XuloError> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("."));
    let runtime_lib = workspace_root.join("target/release/libxulo_runtime.a");
    if !runtime_lib.exists() {
        return Err(XuloError::new(
            ErrorKind::Codegen,
            format!(
                "runtime library not found at {}（请先运行 cargo build --release -p xulo-runtime）",
                runtime_lib.display()
            ),
        ));
    }
    Ok(runtime_lib)
}
