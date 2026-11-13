//! AOT 产物输出：把 IR 编成 .o，再用系统链接器生成可执行文件。
//!
//! 函数体 lowering 与 JIT 共用 `codegen`（见 `backend` 的后端分层说明）；
//! 本文件只关心“输出成文件”：目标文件落盘、运行时库定位、cc 链接参数。
//!
//! IR -> 目标文件这一步是可替换的：默认走 Cranelift（语言开发/测试），
//! 启用 `llvm` feature 后 `xupm build` 走 LLVM。两条路径共用同一套
//! load -> analyze -> IR -> link 流程，避免出现两套会漂移的流水线。

use std::path::{Path, PathBuf};

use cranelift_object::ObjectProduct;

use xulo_core::error::{ErrorKind, XuloError};
use xulo_core::ir::IrModule;

use crate::backend::BackendMode;
use crate::codegen::CodeGen;

/// IR -> 目标文件使用的后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AotBackend {
    /// Cranelift（默认；语言开发与测试路径）
    Cranelift,
    /// LLVM（`llvm` feature；`xupm build` 的产品路径）
    Llvm,
}

impl AotBackend {
    pub fn name(self) -> &'static str {
        match self {
            AotBackend::Cranelift => "cranelift",
            AotBackend::Llvm => "llvm",
        }
    }
}

/// 优化档位（只影响 IR -> 目标文件的优化级别，不切换后端）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BuildProfile {
    /// `xupm build`：低优化
    #[default]
    Debug,
    /// `xupm build --release`：高优化
    Release,
}

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
/// - release 用 `--strip-all` 去掉符号表（约 -24%）；debug 只 `--strip-debug`，
///   保留符号便于诊断
pub fn link_executable(obj_path: &Path, exe_path: &Path, strip_all: bool) -> Result<(), XuloError> {
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
        .arg(if strip_all {
            "-Wl,--strip-all"
        } else {
            "-Wl,--strip-debug"
        })
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

/// The result of building one entry module to a native executable.
pub struct BuildOutput {
    pub executable: PathBuf,
    pub warnings: Vec<XuloError>,
    /// 实际使用的后端（用于构建日志/排障）。
    pub backend: AotBackend,
}

/// Load, analyze, lower, and link `entry` into a native executable at
/// `output`. Compatibility entry point: Cranelift + debug profile, used by
/// the existing tests.
pub fn build_executable(entry: &Path, output: &Path) -> Result<BuildOutput, XuloError> {
    build_executable_with_profile(entry, output, AotBackend::Cranelift, BuildProfile::Debug)
}

/// 指定后端（debug profile）构建。
pub fn build_executable_with(
    entry: &Path,
    output: &Path,
    backend: AotBackend,
) -> Result<BuildOutput, XuloError> {
    build_executable_with_profile(entry, output, backend, BuildProfile::Debug)
}

/// 共享构建流水线：load -> analyze -> dispatch/builtin rewriting -> IR ->
/// `emit_object(backend)` -> link。唯一的分支点是 IR -> 目标文件。
pub fn build_executable_with_profile(
    entry: &Path,
    output: &Path,
    backend: AotBackend,
    profile: BuildProfile,
) -> Result<BuildOutput, XuloError> {
    let mut loaded = crate::module::load(entry)?;
    let mut warnings = crate::module::analyze(&mut loaded, true)?;
    crate::module::apply_trait_dispatch(&mut loaded);
    // IR 后端只认限定的 `std::math::…` 键（解释器从 use 绑定解析裸名）
    crate::module::apply_std_calls(&mut loaded);
    let ir = crate::compile_modules_to_ir(&loaded)?;

    if let Some(dir) = output.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| {
            XuloError::new(
                ErrorKind::Io,
                format!("cannot create output directory {}: {e}", dir.display()),
            )
        })?;
    }

    let backend = resolve_backend(backend, &mut warnings);
    let obj_path = output.with_extension("o");
    emit_object(backend, &ir, &obj_path, profile)?;
    let link_result = link_executable(&obj_path, output, profile == BuildProfile::Release);
    let _ = std::fs::remove_file(&obj_path);
    link_result?;
    Ok(BuildOutput {
        executable: output.to_path_buf(),
        warnings,
        backend,
    })
}

/// 未启用 `llvm` feature 时回退到 Cranelift 并记录警告（仅语言开发用途；
/// 发行构建必须启用 `llvm`，不应发布回退后的二进制）。
fn resolve_backend(backend: AotBackend, _warnings: &mut Vec<XuloError>) -> AotBackend {
    match backend {
        AotBackend::Cranelift => AotBackend::Cranelift,
        #[cfg(feature = "llvm")]
        AotBackend::Llvm => AotBackend::Llvm,
        #[cfg(not(feature = "llvm"))]
        AotBackend::Llvm => {
            _warnings.push(XuloError::new(
                ErrorKind::Warning,
                "LLVM not enabled; falling back to Cranelift (development only)",
            ));
            AotBackend::Cranelift
        }
    }
}

/// IR -> 目标文件（后端唯一分支点）。
fn emit_object(
    backend: AotBackend,
    ir: &IrModule,
    obj_path: &Path,
    _profile: BuildProfile,
) -> Result<(), XuloError> {
    match backend {
        AotBackend::Cranelift => {
            let codegen = AotCodeGen::new()?;
            let product = codegen.compile(ir)?;
            write_object_file(&product, obj_path)
        }
        #[cfg(feature = "llvm")]
        AotBackend::Llvm => {
            crate::llvm::emit_object(ir, obj_path, matches!(_profile, BuildProfile::Release))
        }
        #[cfg(not(feature = "llvm"))]
        AotBackend::Llvm => unreachable!("LLVM backend resolved to Cranelift without the feature"),
    }
}

fn runtime_library_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "xulo_runtime.lib"
    } else {
        "libxulo_runtime.a"
    }
}

fn existing(path: PathBuf) -> Option<PathBuf> {
    path.exists().then_some(path)
}

/// 定位 AOT 链接所需的运行时静态库。
///
/// 查找顺序（7.5）：`XULO_RUNTIME_LIB` -> 可执行文件旁的 `lib/xulo/` 或
/// 安装前缀 `<exe>/../lib/xulo/` -> 源码树 `target/release`（仅开发）。
fn runtime_library_path() -> Result<PathBuf, XuloError> {
    let name = runtime_library_name();
    if let Some(path) = std::env::var_os("XULO_RUNTIME_LIB").map(PathBuf::from) {
        return existing(path).ok_or_else(|| {
            XuloError::new(
                ErrorKind::Codegen,
                format!(
                    "XULO_RUNTIME_LIB points to a missing runtime library: {}",
                    std::env::var("XULO_RUNTIME_LIB").unwrap_or_default()
                ),
            )
        });
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidates = [
            dir.join("lib").join("xulo").join(name),
            dir.join(name),
            dir.join("../lib").join("xulo").join(name),
            dir.join("../lib").join(name),
        ];
        for candidate in candidates {
            if let Some(path) = existing(candidate) {
                return Ok(path);
            }
        }
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("."));
    let runtime_lib = workspace_root.join("target/release").join(name);
    existing(runtime_lib.clone()).ok_or_else(|| {
        XuloError::new(
            ErrorKind::Codegen,
            format!(
                "runtime library not found at {}（请先运行 cargo build --release -p xulo-runtime，\
                 或设置 XULO_RUNTIME_LIB）",
                runtime_lib.display()
            ),
        )
    })
}
