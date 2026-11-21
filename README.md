# Xulo

Xulo 是一门静态类型系统编程语言：解析 `.xulo` 文件 → 语义检查 → 由 Rust 原生解释器执行。JS 代码生成已移除，工具链不依赖 Node.js。语言本身不定义 UI 层；UI 属于标准库/第三方库，用普通 `struct`、`enum`、`fn` 构建。

本仓库实现 Xulo 的 stage 0（Rust + [pest](https://pest.rs) 引导实现）：

```
.xulo 文件 → 词法分析 (Token) → 语法分析 (AST) → 语义检查 → 原生 Rust 解释器
```

## 安装（预编译）

Linux / macOS：

```bash
curl -fsSL https://github.com/xulo-lang/xulo/releases/latest/download/install.sh | sh
```

Windows（PowerShell）：

```powershell
irm https://github.com/xulo-lang/xulo/releases/latest/download/install.ps1 | iex
```

Unix 默认安装到 `$HOME/.local`，Windows 默认安装到
`$env:LOCALAPPDATA\Xulo`（`bin/` 与 `lib/xulo/`）；可通过 `XULO_PREFIX`
改前缀、`XULO_VERSION` 指定版本。确保安装目录的 `bin/` 在 `PATH` 中：

```bash
export PATH="$HOME/.local/bin:$PATH"
xulo --version
```

也可以从 [GitHub Releases](https://github.com/xulo-lang/xulo/releases) 手动下载对应
平台的 `xulo-<version>-<target>.tar.gz`（Windows 为 `.zip`），解压后把 `bin/` 加入
`PATH`。`xupm` 通过 `bin/../lib/xulo/` 下的 `libxulo_runtime.a`（Windows 为
`xulo_runtime.lib`）链接 AOT 产物，请保持目录结构完整。

支持的发布目标：`x86_64-unknown-linux-gnu`、`x86_64-apple-darwin`、
`aarch64-apple-darwin`、`x86_64-pc-windows-msvc`。
Linux 发布版使用静态链接的 LLVM 22；macOS / Windows 的 stage 0 预览版
`xupm` 使用 Cranelift 后端（无需安装 LLVM）。

## 构建

```bash
cargo build --release
# 二进制位于 target/release/xulo
```

原生解释器与 `xulo repl` 不依赖 Node.js。

### 原生 AOT 构建（xupm）

`xupm build` 通过内置 LLVM 后端生成原生可执行文件。源码构建需要 LLVM 22
开发库；未启用 `llvm` feature 时 `xupm` 使用 Cranelift（macOS/Windows 的
stage 0 预览版走这条路径，也可显式 `--features cranelift`）。`xupm build
--backend cranelift|llvm` 可显式选择后端；二进制未编译 `llvm` feature 时，显式
指定 `--backend llvm` 会直接报错，而不是静默回退到 Cranelift。Windows 上 AOT
链接需要 MSVC 的 `link.exe`，请在 Developer PowerShell 中运行。

```bash
# 动态链接 LLVM（开发用）
LLVM_SYS_221_PREFIX=/usr/lib/llvm-22 \
  cargo build --release -p xupm --features llvm-dynamic

# 静态链接 LLVM（分发用；需先构建运行时静态库）
cargo build -p xulo-runtime --release
LLVM_SYS_221_PREFIX=/usr/lib/llvm-22 \
  cargo build --release -p xupm --features distribution

target/release/xupm build examples/basics/hello.xulo --release
```

构建时可用 `XULO_RUNTIME_LIB=/path/to/libxulo_runtime.a` 指定运行时静态库；
未指定时按可执行文件旁的 `lib/xulo/`、`lib/` 与源码树 `target/release/`
依次查找。

## 编辑器支持（LSP）

`xulo-analyzer`（`crates/xulo-analyzer`）是基于 `xulo-ide`（编辑器分析库）的 LSP 语言服务器，提供跨文件 go-to-definition、hover、find-references、文档大纲、诊断（UTF-16 坐标、增量同步）、整文档格式化，以及语义高亮（`textDocument/semanticTokens/full` + TextMate 静态语法兜底，参考 rust-analyzer 双层做法）。直接运行二进制可用任意 LSP 客户端挂载：

```bash
# 服务器二进制（stdio JSON-RPC）
cargo build -p xulo-analyzer
target/debug/xulo-analyzer
```

VSCode：用 F5 或「Run Extension」调试启动 `editors/vscode/`（见 `.vscode/launch.json`，已带 `cargo build -p xulo-analyzer` 前置任务与 `XULO_LSP` 指向）；在设置中把 `xulo.server.path` 指向 `target/debug/xulo-analyzer`（支持 `${workspaceFolder}` 展开，留空时依次回退到 `XULO_LSP` 环境变量与 `PATH`）。扩展为无依赖手写 stdio LSP 客户端，提供定义/悬停/引用/大纲、`Format Document`、语义高亮与语法着色，`Xulo: Restart Language Server` 可重启服务器、`Xulo: Show Language Server Output` 查看服务器日志。注意：`xulo` 格式化会丢弃全部注释（`xulo-ide/src/format.rs`），因此**不要开启 format-on-save**，需要时手动执行「Format Document」。也可把 `editors/vscode/` 打包为 vsix 本地安装：`npm run package` 后 `code --install-extension xulo-analyzer-*.vsix`。

## 使用

```bash
# 编译并运行（Rust 原生解释器，不经过 Node.js）
xulo run examples/basics/hello.xulo

# 仅做词法/语法/语义检查
xulo check examples/functions/fibonacci.xulo

# 格式化（就地改写；注意：注释会被丢弃）
xulo fmt file.xulo

# 交互式 REPL（原生解释器；空行或 `run` 执行，`exit` 退出，Ctrl-D（Unix）/ Ctrl-C（Windows）退出，Tab 补全，历史持久化到 ~/.xulo_history 或 $XULO_HISTORY）
xulo repl

# 不带参数也默认进入 REPL
xulo
```

### 示例

`examples/` 按语法类别分目录，同时是 stage 0 的语法门禁与 stage 1 的自举语料：
每个程序都被 `cargo test --workspace` 中的 `examples` 门禁检查、运行，并与
`.expected` 侧车文件比对输出。索引、头部指令和覆盖表见
[`examples/README.md`](examples/README.md)。

| 目录 | 内容 |
|------|------|
| `basics/` | `hello`、词法（字面量/转义/模板/注释）、绑定与解构、运算符、模板、`if`/`for`/`while`/`match` 控制流 |
| `functions/` | 递归、闭包与高阶函数、泛型函数、参数（默认/具名/`mut`/`shared`）、`defer`、`move`/`copy` |
| `types/` | `struct`、`enum`、`trait`、可选类型与可选链、类型别名与联合/交叉/字面量类型、元组、定宽数值类型、泛型 `impl` 与 const 泛型 |
| `patterns/` | `match` 模式（字面量/区间/or/类型/guard）、`let` 解构 |
| `collections/` | 列表与对象、`Map`、`Set`、`Array<T, N>`、字符串与 `StringBuilder` |
| `modules/` | 跨文件模块、内联 `mod`、`use` 分组/别名/glob、`use type`、可见性 |
| `concurrency/` | `async`/`await`、`Task` 组合、`spawn` 四种目标 + service、`shared`/`lock` |
| `macros/` | 函数宏、派生宏、属性宏、`ast::quote!` |
| `testing/` | `#[test]` 选项组与 `xulo test` 报表 |
| `ffi/` | `extern "C"`、`Ptr`、`pub extern fn` |
| `errors/` | `Result` 与 `?` 传播、`panic`、除零、静态类型错误 |
| `std/` | `std` 各模块：`io` / `hint` / `math` / `time` / `fmt` / `json` / `regex` / `path` / `fs` / `process` / `os` / `net` / `http` |

运行任意程序：

```bash
xulo run examples/basics/hello.xulo
xulo run examples/modules/main.xulo      # 多文件程序单元
```
## 语言速览

```xulo
// 行注释；语句可以带分号，也可以省略
fn fib(n: number): number {
    if n <= 1 {
        return n
    } else {
        return fib(n - 1) + fib(n - 2)
    }
}

fn main() {
    let greeting = "Hello, world!"
    print(greeting)              // print -> console.log

    let xs = [1, 2, 3]           // 列表字面量
    for x in xs {
        print(x)
    }

    let person = { name: "lyy", age: 30 }   // 映射字面量
    print(person)
}
```

### 支持的特性

- 字面量：数字 `123` `3.14`、字符串 `"..."` / `'...'`、布尔 `true`/`false`、列表 `[...]`、映射 `{ key: value, ... }`
- 变量绑定：`let` / `const`，可选类型注解；赋值语句
- 函数：`fn name(p: number): number { ... }`，隐式/显式返回、递归、泛型（调用处推断）、可选/默认/命名参数、匿名函数（闭包）
- 类型：`string` `number` `int` `float` `bool` `List<T>` `Map<K, V>` `Set<T>` `Array<T, N>` `null` `T?` `T | U` `T & U`、字符串字面量联合、函数类型 `fn(...): T`、类型别名 `type`、枚举 `enum`（含关联数据、泛型）
- 控制流：`if` / `else if` / `else`（表达式与语句）、`for x in list`、`for i in 0..<n` / `for i in 0..=n`（区间：`..<` 排他上界、`..=` 闭合含上界）、`while`、`match`、`and`/`or`/`!`、三目、`defer`
- 表达式：成员访问、下标、`?.`、`??`、列表/映射展开 `...`
- 错误处理：内建 `Result<T, E>`、后缀 `?` 传播、`panic(...)`；没有 `try` / `catch` / `throw`
- 异步：`async fn`、异步闭包、`await`
- 模块系统：`mod` / `use` / `pub use`（路径、分组、通配、`use type`；`pub` 声明与再导出），按依赖拓扑序加载，循环检测、导出符号/类型跨模块校验
### 未实现 / 限制

- `fmt` 为基于 token 流的格式化器：会丢弃注释，且匹配/映射字面量的换行风格以源码为基准（不做智能重排）
- `repl` 为会话式 REPL：每轮用原生解释器重新编译并执行整个会话（无状态持久化到语言内部，靠重放保持跨条目变量），支持 `exit` / `clear`；行编辑走 rustyline（方向键历史、Tab 补全）
- 调用函数必须在调用点之前已声明（不支持前向引用）
- 原生运行时（`xulo run`）为 MVP：支持核心语言（字面量、变量、函数/闭包/递归、if/else、for/while、match、枚举、列表/映射、`Result`/`?`、`?.`/`??`/`...`、`print`/`println`/`str`）、`async`/`await`（协同调度）、本地 `mod`/`use`/`pub` 导出。`main` 必须返回 `unit`。原生 `print` 对列表/映射的输出格式为 `[1, 2]` / `{ k: v }`。
- `++`/`--` 自增运算符不在语言中

## 测试

```bash
cargo test
```

覆盖词法、语法、语义、原生解释器以及 CLI 端到端测试。另有 `tests/robustness.rs`：对抗语料 + 确定性 token-soup fuzz（3000 次）+ 深嵌套压力测试，断言编译器对任意输入都不 panic——解析嵌套深度超过 128 层返回 `"nesting is too deep"` 诊断而非崩溃。

CI（GitHub Actions，见 `.github/workflows/ci.yml`）执行 `cargo fmt --check` → `cargo clippy --all-targets -- -D warnings` → `cargo test` → 遍历运行全部 examples。

所有语义错误与警告均带源码定位（`--> file:line:col` + caret），诊断码见 `crates/xulo-core/src/error.rs`（`E0001`-`E0006`、警告 `W0001`）。

## 路线图：Stage 1（自举）准备度

stage 1 指用 Xulo 自身编写的编译器/工具链，stage 2 是 stage 1 编译自身。stage 0
已经支撑完整自举回路：Gate 1–6 全部达成（解释器原型、自解析、自检查、原生构建、
自举与规范一致性）。各项能力的证据、工作包与验收门禁见
[`docs/stage1-readiness.md`](docs/stage1-readiness.md)；可勾选的任务清单见
[`docs/tasks.md`](docs/tasks.md)：

- 已具备：核心语法/解释器、`mod`/`use` 模块、struct/enum/trait/泛型、
  `Result`/`?`、64 位 `int`、`unknown` 与 `is` 收窄、元组、List/Map/Set、
  std 的 fs/io/process/os/path/json/regex/fmt/hint、AOT 与 stage 1 自举。
- 仍属 P2 / 非门禁：LSP 与格式化器注释、async 任务的完整调度、
  `std::net`/`std::http` 之外的网络与包注册表、UI 库（语言不定义 UI 层）。

## 项目结构

```
├── Cargo.toml                  # 根 workspace + 共享依赖版本
├── crates/
│   ├── xulo-core/              # AST + 错误类型 + 诊断渲染（零依赖基础）
│   ├── xulo-lexer/             # Token 类型（词法规则在 grammar.pest）
│   ├── xulo-parser/            # 词法 + 语法分析（pest, grammar.pest）
│   ├── xulo-semantic/          # 语义检查 + 符号表 + 名称解析记录（供编辑器查询）
│   ├── xulo-compiler/          # 前端管线 compile() + 多文件模块加载/分析
│   ├── xulo-runtime/           # 原生树遍历解释器（xulo run 默认路径）
│   ├── xulo-std/               # 标准库原生实现：Math/Time 纯语义 + 编译路径 extern "C" 入口
│   ├── xulo-ide/               # 编辑器分析库：LineIndex、单文件查询、多文件 Workspace、诊断、格式化、语义 tokens（协议无关）
│   ├── xulo-analyzer/          # LSP 语言服务器（lsp-types/lsp-server，xulo-ide 上层，二进制 xulo-analyzer）
│   └── xulo-cli/               # CLI（run/check/fmt/repl，二进制名为 xulo；fmt 复用 xulo-ide）
├── editors/
│   └── vscode/                 # VSCode 扩展：无依赖手写 LSP 客户端 + TextMate 语法 + 语言配置
├── stdlib/                     # 未来标准库源码占位（.xulo）
├── docs/                       # 语言规范（EBNF / 语法）+ 原生运行时内存模型 + Stage 1 准备度矩阵与任务清单
├── examples/                   # 示例 .xulo 文件
└── tests/                      # （各 crate 的 tests/ 目录承载测试）
```

`xulo run` 用 `xulo-runtime` crate 的 Rust 解释器直接执行，不经 JS/Node。`xulo-ide` 与 LSP 协议解耦：`xulo-analyzer` 只负责把 `xulo-ide` 的 `Range`/`Location`/`Diagnostic`/符号条目映射为 `lsp-types` 并通过 stdio 收发；每次文档变更由 `xulo-analyzer` 重建内存 Workspace 后重新发布诊断（非增量，MVP 实现）。

## License

MIT，见 [LICENSE](LICENSE)。
