# Xulo

Xulo 是一门 UI First 编程语言：解析 `.xulo` 文件 → 语义检查 → 由 Rust 原生解释器执行。JS 代码生成已移除，工具链不依赖 Node.js。

本仓库实现 Xulo 的 MVP（Rust + [winnow](https://github.com/winnow-rs/winnow)）：

```
.xulo 文件 → 词法分析 (Token) → 语法分析 (AST) → 语义检查 → 原生 Rust 解释器
```

## 构建

```bash
cargo build --release
# 二进制位于 target/release/xulo
```

原生解释器与 `xulo repl` 不依赖 Node.js。

## 编辑器支持（LSP）

`xulo-analyzer`（`crates/xulo-analyzer`）是基于 `xulo-ide`（编辑器分析库）的 LSP 语言服务器，提供跨文件 go-to-definition、hover、find-references、文档大纲、诊断（UTF-16 坐标、增量同步）、整文档格式化，以及语义高亮（`textDocument/semanticTokens/full` + TextMate 静态语法兜底，参考 rust-analyzer 双层做法）。直接运行二进制可用任意 LSP 客户端挂载：

```bash
# 服务器二进制（stdio JSON-RPC）
cargo build -p xulo-analyzer
target/debug/xulo-analyzer
```

VSCode：用 F5 或「Run Extension」调试启动 `editors/vscode/`（见 `.vscode/launch.json`）；在设置中把 `xulo.server.path` 指向 `target/debug/xulo-analyzer`（或加入 PATH）。扩展为无依赖手写 stdio LSP 客户端，提供定义/悬停/引用/大纲、`Format Document`、语义高亮与语法着色，`Xulo: Restart Language Server` 命令可重启服务器。也可把 `editors/vscode/` 打包为 vsix 本地安装：`code --install-extension xulo-analyzer-*.vsix`。

## 使用

```bash
# 编译并运行（Rust 原生解释器，不经过 Node.js）
xulo run examples/hello.xulo

# 渲染运行：把 main(): View 布局并绘制到 ANSI 终端（示例见 examples/ui.xulo）
xulo run --render terminal examples/ui.xulo

# 渲染运行：原生 webview 窗口（需要 webview feature；Linux 需 webkit2gtk-4.1 + X11/XWayland）
# 布局/绘制引擎（xulo-ui）编译为 WASM 并在页面内运行，如同 egui/eframe 的 web 后端
cargo run --bin xulo --features webview -- run --render webview examples/ui.xulo

# 仅做词法/语法/语义检查
xulo check examples/fibonacci.xulo

# 格式化（就地改写；注意：注释会被丢弃）
xulo fmt file.xulo

# 交互式 REPL（原生解释器；空行或 `run` 执行，`exit` 退出，Ctrl-D（Unix）/ Ctrl-C（Windows）退出，Tab 补全，历史持久化到 ~/.xulo_history 或 $XULO_HISTORY）
xulo repl

# 不带参数也默认进入 REPL
xulo
```

### 示例

`examples/` 下每个 `.xulo` 都可通过 `xulo run examples/<文件>` 直接运行：

| 文件 | 展示的语法 |
|------|-----------|
| `enums_match.xulo` | `enum`（含关联数据、泛型）+ `match` 表达式 |
| `closures_hof.xulo` | 匿名函数 / 闭包（捕获、可变捕获）/ 高阶函数 / 函数值调用 |
| `generics.xulo` | 泛型函数（调用处推断类型实参） |
| `templates.xulo` | 反引号模板字面量 `${...}`、转义、多行、嵌套、Unicode |
| `collections.xulo` | 列表 / 对象：下标、`+` 拼接、`...` 展开、成员访问 |
| `optional_chaining.xulo` | `T?` 可选类型、`?.`、`??`、三目、`and` / `or` / `!` |
| `control_flow.xulo` | 区间 `..<` / `...`、`for` / `while`、`if` 表达式、`match` |
| `for.xulo` | 闭合区间 `1...5` + 模板插值 |

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

    let person = { name: "lyy", age: 30 }   // 对象字面量
    print(person)
}
```

### 支持的特性

- 字面量：数字 `123` `3.14`、字符串 `"..."` / `'...'`、布尔 `true`/`false`、列表 `[...]`、对象 `{ key: value, ... }`
- 变量绑定：`let` / `const`，可选类型注解；赋值语句
- 函数：`fn name(p: number): number { ... }`，隐式/显式返回、递归、泛型（调用处推断）、可选/默认/命名参数、匿名函数（闭包）
- 类型：`string` `number` `boolean` `list<T>` `object` `null` `T?` `T | U` `T & U`、字符串字面量联合、函数类型 `fn(...): T`、类型别名 `type`、枚举 `enum`（含关联数据、泛型）
- 控制流：`if` / `else if` / `else`（表达式与语句）、`for x in list`、`for i in 0..<n` / `for i in 0...n`（区间：`..<` 排他上界、`...` 闭合含上界，Swift 语义）、`while`、`match`、`and`/`or`/`!`、三目、`throw`/`try`/`catch`
- 表达式：成员访问、下标、`?.`、`??`、列表/对象展开 `...`、`$name` 双向绑定
- 异步：`: async` 返回标注、`await`
- 模块系统：`import` / `pub`（named/namespace/type-only/bare；`pub` 声明、`pub use { a, b }` 两种导出形态），按依赖拓扑序加载，循环检测、导出符号/类型跨模块校验
- UI：`View` 返回类型、组件块语法（`VStack { Text(...) }`）、`@State` / `@Store` / `@Effect` / `@Environment`、UI 条件/循环渲染

### UI 运行时约定

- `@State` / `@Store` / `@Effect` / `@Environment` 只能在返回类型为 `View` 的函数顶层使用（嵌套块/普通函数内报语义错误）。术语：**组件**（component）指构造 UI 的语法/函数形态，**`View`** 是组件函数返回的类型（渲染树值）。
- 组件块内允许「表达式子元素」（`string` / `View` / `list<View>`，列表渲染为嵌套数组）。外部组件调用（`@xulo/ui` 包，原生无实现）按位置实参构造成 `Name({ key: value, children: [...] })` 形状的 props 对象。
- 渲染：`xulo run --render terminal examples/ui.xulo` 把入口 `main` 返回的 `View` 布局并绘制到 ANSI 终端（字符单元、边框按钮、true-color 背景）；`--render webview`（`--features webview`）用 wry 打开原生窗口——布局/绘制引擎 `xulo-ui` 编译为 **WASM**（`xulo-ui-wasm`，原始 ABI：`xulo_layout`/`xulo_hit_test`），页面实例化后在浏览器内完成布局，JS 只做最后的画布栅格化，如同 egui/eframe 的 web 后端。Linux 下窗口走 GTK 原生主循环（`build_gtk`，wry 官方推荐路径），并把 `Screen.backgroundColor` 同时设为窗口与页面的即时背景——打开即有主题色、无黑屏过渡。渲染管线按 crate 分层：`xulo-ui`（widget 树 + 布局 + 绘制命令，零依赖）、`xulo-renderer-terminal`（字符网格后端，零外部依赖）、`xulo-ui-wasm`（同引擎的 wasm32 产物，框架构建时经 `build.rs` 编译内嵌）、`xulo-renderer-webview`（wry 窗口后端，feature 化）、`xulo-framework`（统一 `run()`：编译 → 执行 → 取渲染树 → 转换 → 布局 → 绘制 → 后端输出）。
- 交互：`--render` 为交互式会话。终端后端每帧渲染后输入 `1..N` 点击对应按钮、`r`/回车刷新、`q` 退出；webview 后端在页面上真实响应鼠标点击（wasm 内命中测试）。点击执行 `Button.onClick`（闭包在解释器内运行、改写 `@State` 信号），随后 `main` 被重新调用以重渲染——解释器按绑定位置复用 `@State` 单元，因此状态跨帧存活。`@Effect` 每次重渲染都会重跑。已知限制：同一组件多次实例化会共享 `@State`（尚未跟踪实例身份）。

### 未实现 / 限制

- `fmt` 为基于 token 流的格式化器：会丢弃注释，且匹配/对象字面量的换行风格以源码为基准（不做智能重排）
- `repl` 为会话式 REPL：每轮用原生解释器重新编译并执行整个会话（无状态持久化到语言内部，靠重放保持跨条目变量），支持 `exit` / `clear`；行编辑走 rustyline（方向键历史、Tab 补全）
- 调用函数必须在调用点之前已声明（不支持前向引用）
- `$` 绑定（`{ value, onChange }`）：对 `@State` 信号，`onChange` 写回信号单元；对普通绑定为空操作
- 原生运行时（`xulo run`）为 MVP：支持核心语言（字面量、变量、函数/闭包/递归、if/else、for/while、match、枚举、列表/对象、try/catch、`?.`/`??`/`...`、`print`/`println`/`str`）、`async`/`await`（协同调度）、本地 `import`/`pub` 导出（named/namespace）以及 UI。返回 `View` 的组件函数、`@State`/`@Store`/`@Effect`、组件块与 `$` 绑定均可用；无 `--render` 时组件树被构建并丢弃（无头运行），`--render terminal` 时布局绘制到终端。`print` 副作用（如 `@Effect` 中）可观察；外部包（`@xulo/ui`）的导入名绑定为 `null` 占位符，`@Environment` 仍报「不支持」（无注入机制）。原生 `print` 对列表/对象的输出格式为 `[1, 2]` / `{ k: v }`。
- `++`/`--` 自增运算符不在语言中

## 测试

```bash
cargo test
```

覆盖词法、语法、语义、原生解释器以及 CLI 端到端测试。另有 `tests/robustness.rs`：对抗语料 + 确定性 token-soup fuzz（3000 次）+ 深嵌套压力测试，断言编译器对任意输入都不 panic——解析嵌套深度超过 128 层返回 `"nesting is too deep"` 诊断而非崩溃。

CI（GitHub Actions，见 `.github/workflows/ci.yml`）执行 `cargo fmt --check` → `cargo clippy --all-targets -- -D warnings` → `cargo test` → 遍历运行全部 examples。

所有语义错误与警告均带源码定位（`--> file:line:col` + caret），诊断码见 `src/error.rs`（`E0001`-`E0005`、警告 `W0001`）。

## 项目结构

```
├── Cargo.toml                  # 根 workspace + 共享依赖版本
├── crates/
│   ├── xulo-core/              # AST + 错误类型 + 诊断渲染（零依赖基础）
│   ├── xulo-lexer/             # 词法分析
│   ├── xulo-parser/            # 语法分析（winnow）
│   ├── xulo-semantic/          # 语义检查 + 符号表 + 名称解析记录（供编辑器查询）
│   ├── xulo-codegen/           # 生成 JavaScript（已废弃，保留但不再参与执行路径）
│   ├── xulo-compiler/          # 前端管线 compile() + 多文件模块加载/分析
│   ├── xulo-runtime/           # 原生树遍历解释器（xulo run 默认路径）
│   ├── xulo-ui/                # 纯 UI 逻辑：widget 树 + 布局 + 绘制命令（零依赖，对应 egui）
│   ├── xulo-ui-wasm/           # 同一布局引擎的 wasm32 产物（原始 ABI，框架 build.rs 编译内嵌）
│   ├── xulo-renderer-terminal/ # 终端渲染后端：PaintOp → 字符网格 + ANSI（零外部依赖）
│   ├── xulo-renderer-webview/  # webview 渲染后端：页面内实例化 wasm 布局引擎（wry + winit，feature 化）
│   ├── xulo-framework/         # 统一入口：编译 → 执行 → 取渲染树 → 布局 → 后端输出（对应 eframe）
│   ├── xulo-ide/               # 编辑器分析库：LineIndex、单文件查询、多文件 Workspace、诊断、格式化、语义 tokens（协议无关）
│   ├── xulo-analyzer/          # LSP 语言服务器（lsp-types/lsp-server，xulo-ide 上层，二进制 xulo-analyzer）
│   └── xulo-cli/               # CLI（run/check/fmt/repl，二进制名为 xulo；fmt 复用 xulo-ide）
├── editors/
│   └── vscode/                 # VSCode 扩展：无依赖手写 LSP 客户端 + TextMate 语法 + 语言配置
├── stdlib/                     # 未来标准库源码占位（.xulo）
├── docs/                       # 语言规范（EBNF / 语法）+ 原生运行时内存模型
├── examples/                   # 示例 .xulo 文件
└── tests/                      # （各 crate 的 tests/ 目录承载测试）
```

`xulo run` 用 `xulo-runtime` crate 的 Rust 解释器直接执行，不经 JS/Node。`xulo-ide` 与 LSP 协议解耦：`xulo-analyzer` 只负责把 `xulo-ide` 的 `Range`/`Location`/`Diagnostic`/符号条目映射为 `lsp-types` 并通过 stdio 收发；每次文档变更由 `xulo-analyzer` 重建内存 Workspace 后重新发布诊断（非增量，MVP 实现）。
