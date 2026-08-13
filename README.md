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

# 格式化（尚未实现）
xulo fmt file.xulo

# 交互式 REPL（尚未实现）
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
- 变量绑定：`let x = ...`，可选类型注解 `let x: number = ...`
- 函数：`fn name(p: number): number { ... }`，支持递归
- 控制流：`if` / `else` / `else if`，`for x in list { ... }`（编译为 `for (const x of ...)`）
- 运算符：`+ - * / == != < > <= >=`
- 类型：`string` `number` `boolean` `list` `object`；`print(...)` 为内置函数
- 语义检查：未声明变量、重复声明、操作数类型兼容、if 条件须为布尔、返回值类型匹配、for 迭代须为列表

### 未实现 / 限制

- `fmt` 与 `repl` 子命令为占位实现
- 调用函数必须在调用点之前已声明（不支持前向引用）
- 没有 `while` 循环、字符串拼接运算符 `+` 仅限数字、对象字段不支持成员访问表达式

## 测试

```bash
cargo test
```

覆盖词法、语法、语义、代码生成以及端到端（真实调用 `node`）测试。

## 项目结构

```
src/
├── main.rs             # 入口，调用 cli
├── lib.rs              # 导出模块 + compile() 管线
├── cli.rs              # run/build/check/fmt/repl 子命令
├── ast.rs              # 抽象语法树
├── diagnostics.rs      # 美化错误报告
├── error.rs            # 错误类型 (E0001-E0006)
├── lexer/              # 词法分析
│   ├── token.rs
│   └── mod.rs
├── parser/             # 语法分析（winnow）
│   ├── mod.rs
│   ├── statement.rs
│   └── expression.rs
├── semantic/           # 语义检查
│   ├── mod.rs
│   └── symbol_table.rs
└── codegen/            # 生成 JavaScript
    ├── mod.rs
    └── javascript.rs
tests/                  # 集成测试
examples/               # 示例 .xulo 文件
```
