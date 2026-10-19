# 快速上手

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

# 生成 JS 文件（有外部 ESM import 时自动用 .mjs 后缀）
xulo build examples/hello.xulo -o hello.js

# 仅做词法/语法/语义检查
xulo check examples/fibonacci.xulo

# 格式化（基于 token 流重排空格；注意：注释会被丢弃）
xulo fmt file.xulo

# 交互式 REPL（空行执行当前输入，无分号表达式自动回显值，`exit` 退出）
xulo repl
```

## Hello, World

```xulo
fn main() {
    print("Hello, world!")     // print -> console.log
}
```

运行：

```bash
xulo run hello.xulo
# Hello, world!
```

## 一个稍完整的例子

```xulo
fn fib(n: number): number {
    if n <= 1 {
        return n
    } else {
        return fib(n - 1) + fib(n - 2)
    }
}

fn main() {
    let greeting = "Hello, world!"
    print(greeting)

    let xs = [1, 2, 3]          // 列表字面量
    for x in xs {
        print(x)
    }

    let person = { name: "lyy", age: 30 }   // 对象字面量
    print(person)

    print(fib(10))              // 55
}
```

## 测试

```bash
cargo test
```

覆盖词法、语法、语义、代码生成以及端到端（真实调用 `node`）测试。
