# 附录：编译器实现要点

> 本节面向想要修改 / 理解编译器的读者，与语言规范正文相对独立。

## UI Block vs 代码块的区分

**问题：** `VStack { Text("Hello") }` 中的 `{ }` 是 UI Children 列表，而 `fn main() { let x = 5 }` 中的 `{ }` 是普通代码块。

**解决方案：** 在 AST 层面区分两种节点类型。

```text
// UI Block — 只能包含 UI 元素
UIBlock {
  children: Vec<UIElement>  // Component, Text, Expr, if, for, Group
}

// UIElement::Expr 承载组件块内的「表达式子元素」，语义阶段限定其类型为
// string / View / Any / list<View|string|Any> 之一；list 渲染为
// 嵌套数组，由运行时渲染器展平（与 if/for 产物同一约定）。

// 代码块 — 只能包含普通语句
Block {
  statements: Vec<Statement>  // let, fn, if, for, return, expr
}
```

**解析策略：**

- `ComponentName { ... }` → 解析为 UI Block（大写标识符 + `(`/`{` 触发）
- `fn` 函数体 → 解析为普通 Block
- `if` / `for` / `while` 的 `{ }` → 根据上下文决定（组件块内为 UI 元素，其余为代码块）

## 隐式返回与分号

**问题：** `a + b` 是返回值，`a + b;` 是语句。

**解决方案：** 检查函数体最后一个语句是否是无分号表达式。

```xulo
// 函数体末尾无分号 → 隐式返回
fn add(a: number, b: number): number {
  a + b  // ✅ 隐式返回
}

// 函数体末尾有分号 → 语句，警告
fn add(a: number, b: number): number {
  a + b;  // ⚠️ 警告：忽略返回值
}
```

## `T?` 与 `a ? b : c` 的区分

**问题：** 同一个 `?` 在类型中是可选标记，在表达式中是三目运算符。

**解决方案：** 上下文感知解析。

```text
解析类型时：T? → Optional(T)
解析表达式时：a ? b : c → Ternary
```

## 解析器实现建议

| 阶段 | 方法 | 说明 |
|------|------|------|
| 词法分析 | 独立 Lexer | 生成 Token 流，不区分上下文 |
| 类型解析 | 上下文感知 | 解析类型时识别 `T?`、`T \| U`、`list<T>` |
| 表达式解析 | Pratt Parser | 处理优先级、三目、函数调用 |
| UI Block 解析 | 专用解析器 | 只允许 UI 元素，禁止普通语句 |
| 语义分析 | 访问者模式 | 类型检查、作用域、生命周期 |
| 模块打包 | 图加载 + IIFE | 依赖拓扑序、循环检测、导出符号/类型跨模块校验 |

## 模块打包（实现要点）

- **加载**：从入口文件开始 DFS，本地导入（相对路径或本地存在的裸名）先加载依赖（后序 → 拓扑序），循环导入报错。
- **语义**：按依赖顺序逐个 `analyze_with`，把依赖导出的符号（真实签名）与类型种子化给导入方；导入不存在的导出名报错。
- **代码生成**：每个模块编译为一个 `const __modN = (() => { ...; return { 导出 }; })();`，导入方用 `const { a } = __modN;` 解构；入口模块 IIFE 内最后调用 `main()`。
- **外部依赖**：非本地说明符原样生成顶层 ESM `import`（要求输出为 `.mjs` 或在 `package.json` 中 `"type": "module"`）；`xulo run` 会把临时 JS 写到源文件同目录，借用 Node 的 `node_modules` 向上查找来解析外部包。
- **跨模块调用**：导入的函数保留参数名，支持具名实参调用。

## 诊断与定位（span）

从词法到语义的所有节点都在 AST 上携带源码 `span`，因此错误与警告都能给出精确位置：

```text
error[E0003]: return type mismatch: expected `number`, found `string`
  --> bad.xulo:3:17
     |
  3  |   fn f(): number { "hi" }
     |                  ^^^^^^^^
```

- **诊断码**：`E0001` 词法 / `E0002` 语法 / `E0003` 语义 / `E0004` IO / `E0005` 代码生成 / `W0001` 警告。
- **语义检查**维护 `current_span`，所有 `self.err(...)` 自动附加当前位置；警告（例如"忽略返回值"）也是带 span 的 `XuloError`（`Warning` kind），与错误走同一渲染路径。
- **REPL 值回显**：无分号、非声明/定义/控制流的表达式会自动包一层 `print(...)` 回显其值；失败时自动回退为原样编译。

## 健壮性（fuzz 与深度守卫）

- **嵌套深度守卫**：解析器用 `thread_local` 计数 + RAII `enter_nest()`，超过 `MAX_NEST_DEPTH`（128）即返回 `Cut` 错误 `"nesting is too deep"`，拒绝超深嵌套而不再栈溢出。
- **语义线性检查**：块尾部表达式不再被重复检查——`check_block_tail` / `check_block_implicit` 对每条语句只检查一次（尾部表达式若为块值仍走 `check_expression` 保持 `await` 的取值位置语义）。修复前嵌套 `if` 语句是 O(2^n)，深度 20 需 446ms；修复后约 38µs。
- **词法 EOF 守卫**：未闭合字符串/输入中断时返回带定位的诊断，而不是对空切片 panic。
- **测试**：`tests/robustness.rs` 包含 40+ 对抗语料、3000 次确定性 token-soup fuzz（xorshift64）、深嵌套压力（64MiB 栈线程）与超深嵌套拒绝断言，全部要求不 panic。CI（GitHub Actions）执行 `fmt --check` → `clippy -D warnings` → `cargo test` → 遍历运行全部 examples。
