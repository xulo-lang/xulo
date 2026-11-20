# 词法结构

> 相关：[数据与类型](types.md) · [表达式](expressions.md) · [速查表](../cheatsheet.md)

## 注释

```xulo
// 行注释

/*
  块注释
*/
```

词法分析器跳过两种注释：

- 行注释：`// ... \n`
- 块注释：`/* ... */`（未闭合的块注释是词法错误）

## 标识符与关键字

```text
identifier     = (letter | "_") { letter | digit | "_" } ;
```

- 普通标识符以字母或 `_` 开头，可含字母、数字、`_`。
- 类型名约定以大写字母开头（`User`、`Result`），但语法上仍是普通标识符。
- 枚举成员与命名空间用 `::` 访问：`Theme::Dark`。
- **关键字与保留字（本清单之外的均已确定，不再变更）** 不可用作标识符。

### 已使用的关键字

规范定义的封闭关键字集共 61 个，按用途分组如下（完整清单见 `spec/lexical-structure.md`）：

| 类别 | 关键字 | 用途 |
|------|--------|------|
| 声明 | `const` `enum` `extern` `fn` `impl` `let` `mod` `mut` `self` `struct` `trait` `type` `where` | 常量、类型、函数、模块、方法与泛型约束 |
| 控制流 | `break` `continue` `defer` `else` `for` `if` `in` `match` `panic` `return` `while` | 循环、分支、延迟执行与返回 |
| 模块 | `as` `pub` `super` `use` | 路径别名、可见性、父模块与路径绑定 |
| 异步与并发 | `async` `await` `lock` `shared` `spawn` | 异步函数、任务、临界区与共享状态 |
| 字面量与运算符 | `and` `false` `is` `not` `or` `true` | 布尔字面量与词运算符 |
| 所有权 | `copy` `move` | 深拷贝与所有权转移 |
| 原始类型 | `bool` `string` `number` `int` `float` `i8` `i16` `i32` `i64` `u8` `u16` `u32` `u64` `f32` `f64` `isize` `usize` `null` `unit` `unknown` | 内建值类型与字面量 |

`print`、`println`、`str` 是 prelude 中的普通名字（内建函数），不是关键字，可以被同名绑定遮蔽。

`_` 是通配符名字：在 `match` 模式与 `enum` 载荷占位中匹配任意值，不作为普通标识符使用。

### 已使用的符号

| 符号 | 现状 |
|------|------|
| `?` `!` `??` `?.` `::` `\|` `&` `^` `~` `<<` `>>` `**` `:=` | 运算符 |
| `..<` `..=` | 半开 / 闭合区间 |
| `...` | 仅用于列表 / 映射字面量内的展开 |
| `#` | 属性开符，只在 `#[...]` 中有意义 |

`$` 只出现在模板字符串插值 `${...}` 与 `ast::quote!` 的 `${...}` 洞中，作为插值的起始标记；它不是独立运算符。

### 预留（未来可能的）关键字

以下单词现在既不是关键字、也没有语法含义，但**已被保留**：任何把它们用作标识符（变量 / 函数 / 类型 / 成员名）的代码都会在解析时报 `unexpected reserved keyword X`。预留为未来语言特性留出演进空间：

`abstract` `actor` `arguments` `associatedtype` `bench` `case` `cfg` `channel` `class` `debugger` `default` `deinit` `delete` `derive` `do` `doc` `eval` `export` `extension` `fallthrough` `final` `function` `generator` `generic` `global` `guard` `init` `instanceof` `interface` `isolated` `iterator` `lazy` `library` `local` `macro` `meta` `module` `new` `open` `override` `package` `priv` `protocol` `receiver` `ref` `rethrows` `select` `sender` `static` `switch` `task` `this` `typealias` `unowned` `unsafe` `var` `virtual` `void` `weak` `with` `yield`

> 保留表不含关键字：`mod`、`move`、`self`、`spawn`、`super` 都是关键字，不是保留字。
> `priv` 已预留但**当前无私有语义**：未导出的声明本身即为模块私有；预留只在词法层禁止其作标识符。

### 建议避免的名称（非强制）

以下内建类型名不是关键字，但建议不要作为变量名使用（当前不报错，仅约定）：

`List<T>` `Map<K, V>` `Set<T>` `Array<T, N>` `Task<T>` `Range<T>` `Result<T, E>` `Error`

## 字面量

```xulo
123            // int（整数）
3.14           // float（浮点）
"hello"        // string（双引号）
'world'        // string（单引号）
true           // bool
false          // bool
null           // null
[1, 2, 3]      // List<int> 字面量
{ name: "lyy" } // Map<string, string> 字面量
```

字符串支持转义：`\"`、`\'`、`\\`、`\n`、`\t`、`\r`。

## 运算符与符号

| 符号 | 含义 |
|------|------|
| `+` `-` `*` `/` `%` `**` | 算术、取模、幂 |
| `&` `\|` `^` `~` `<<` `>>` | 位运算与移位 |
| `==` `!=` `<` `>` `<=` `>=` | 比较 |
| `and` `or` `!` | 逻辑 |
| `??` | 空合并 |
| `?` `:` | 三目 |
| `..<` | 范围（左闭右开） |
| `..=` | 范围（闭区间，含右端） |
| `.` `?.` | 成员访问 / 可选成员访问 |
| `[ ]` | 下标 |
| `...` | 展开（列表 / 映射字面量内） |
| `::` | 枚举成员 / 命名空间 |
| `=` `:=` | 赋值 / 可变绑定简写 |
| `=>` | `match` 分支 / 闭包体 |
| `,` `;` `:` `(` `)` `{` `}` | 分隔 / 分组 |

语句末尾的分号可省略。
