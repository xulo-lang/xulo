# Xulo

Xulo 是一种玩具编程语言：解析 `.xulo` 文件 → 生成 JavaScript → 通过 Node.js 运行。

本仓库实现 Xulo 的 MVP（Rust + [winnow](https://github.com/winnow-rs/winnow)）：

```
.xulo 文件 → 词法分析 (Token) → 语法分析 (AST) → 语义检查 → 代码生成 (JS) → node 运行
```

## 构建

```bash
cargo build --release
# 二进制位于 target/release/xulo
```

需要本机安装 Node.js（`xulo run` 通过 `node` 执行生成的 JS）。

## 使用

```bash
# 编译并运行
xulo run examples/hello.xulo

# 生成 JS 文件
xulo build examples/hello.xulo -o hello.js

# 仅做词法/语法/语义检查
xulo check examples/fibonacci.xulo

# 格式化（就地改写；注意：注释会被丢弃）
xulo fmt file.xulo

# 交互式 REPL（空行执行当前输入，`exit` 退出）
xulo repl
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

    let person = { name: "lyy", age: 30 }   // 对象字面量
    print(person)
}
```

### 支持的特性

- 字面量：数字 `123` `3.14`、字符串 `"..."` / `'...'`、布尔 `true`/`false`、列表 `[...]`、对象 `{ key: value, ... }`
- 变量绑定：`let` / `const`，可选类型注解；赋值语句
- 函数：`fn name(p: number): number { ... }`，隐式/显式返回、递归、泛型（调用处推断）、可选/默认/命名参数、匿名函数（闭包）
- 类型：`string` `number` `boolean` `list<T>` `object` `null` `T?` `T | U` `T & U`、字符串字面量联合、函数类型 `fn(...): T`、类型别名 `type`、枚举 `enum`（含关联数据、泛型）
- 控制流：`if` / `else if` / `else`（表达式与语句）、`for x in list`、`for i in 0..<n`、`while`、`match`、`and`/`or`/`!`、三目、`throw`/`try`/`catch`
- 表达式：成员访问、下标、`?.`、`??`、列表/对象展开 `...`、`$name` 双向绑定
- 异步：`: async` 返回标注、`await`
- 模块系统：`import` / `export`（named/default/namespace/type-only/bare），本地打包为 IIFE，外部包原样 ESM
- UI：`Component` 返回类型、组件块语法（`VStack { Text(...) }`）、`@State` / `@Store` / `@Effect` / `@Environment`、UI 条件/循环渲染

### UI 运行时约定

- UI 组件来自外部包（`@xulo/ui`），原样保留为 ESM `import`；组件调用降级为 `Name({ key: value, children: [...] })`（props 对象 + `children` 数组，位置实参放入 `"0"`/`"1"` 键）。
- `@State` 编译为响应式信号（`__signal`），读写分别变为 `.get()` / `.set()`；`@Effect` → `__effect`，`@Environment` → `__env`，组件函数体包裹在 `__component(function(){...})` 中。
- 编译器按需内联一个最小响应式运行时；`fn main(): Component` 会生成 `if (typeof __xulo_mount === "function") __xulo_mount(main());` 挂载钩子，由外部运行时负责真正的渲染/更新。
- `@State` / `@Store` / `@Effect` / `@Environment` 只能在返回类型为 `Component` 的函数顶层使用（嵌套块/普通函数内报语义错误）。

### 未实现 / 限制

- `fmt` 为基于 token 流的格式化器：会丢弃注释，且匹配/对象字面量的换行风格以源码为基准（不做智能重排）
- `repl` 为会话式 REPL：每轮重新编译并执行整个会话（无状态持久化到语言内部），支持 `exit` / `clear`
- 调用函数必须在调用点之前已声明（不支持前向引用）
- `@Store` 的 `$` 绑定写方向为空操作；跨模块/`@xulo/store` 的订阅重渲染由外部运行时接管
- 组件函数体在重渲染时会重新执行（`@State` 信号已提升到函数级；`@Effect` 仅在依赖数组（若有）变化或挂载时重新执行）
- `++`/`--` 自增运算符不在语言中

## 测试

```bash
cargo test
```

覆盖词法、语法、语义、代码生成以及端到端（真实调用 `node`）测试。另有 `tests/robustness.rs`：对抗语料 + 确定性 token-soup fuzz（3000 次）+ 深嵌套压力测试，断言编译器对任意输入都不 panic——解析嵌套深度超过 128 层返回 `"nesting is too deep"` 诊断而非崩溃。

CI（GitHub Actions，见 `.github/workflows/ci.yml`）执行 `cargo fmt --check` → `cargo clippy --all-targets -- -D warnings` → `cargo test` → 遍历运行全部 examples。

所有语义错误与警告均带源码定位（`--> file:line:col` + caret），诊断码见 `src/error.rs`（`E0001`-`E0005`、警告 `W0001`）。

## 项目结构

```
src/
├── main.rs             # 入口，调用 cli
├── lib.rs              # 导出模块 + compile() 管线
├── cli.rs              # run/build/check/fmt/repl 子命令
├── ast.rs              # 抽象语法树
├── diagnostics.rs      # 美化错误报告
├── error.rs            # 错误类型 (E0001-E0005, W0001)
├── formatter.rs        # xulo fmt 格式化器
├── module.rs           # 多文件加载 + 打包（IIFE）
├── lexer/              # 词法分析
│   ├── token.rs
│   └── mod.rs
├── parser/             # 语法分析（winnow）
│   ├── mod.rs
│   ├── statement.rs
│   ├── expression.rs
│   └── types.rs
├── semantic/           # 语义检查
│   ├── mod.rs
│   └── symbol_table.rs
└── codegen/            # 生成 JavaScript
    ├── mod.rs
    └── javascript.rs
tests/                  # 集成测试
examples/               # 示例 .xulo 文件
```
