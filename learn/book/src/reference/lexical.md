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

| 关键字 | 用途 |
|--------|------|
| `fn` `return` | 函数定义 / 返回 |
| `let` `const` `mut` | 变量绑定 / 可变绑定 |
| `if` `else` | 条件 |
| `for` `in` | 循环 |
| `while` | 循环 |
| `match` `and` `or` | 模式匹配 / 逻辑与或 |
| `async` `await` | 异步 |
| `try` `catch` `throw` | 异常 |
| `import` `pub` `use` `from` `as` | 模块（导入与公开导出） |
| `type` `enum` `struct` | 类型声明 |
| `trait` `impl` `where` | 特征声明 / 实现 / 泛型约束 |
| `null` `true` `false` | 字面量关键字 |
| `bool` `string` `number` `int` `float` | 原始类型名关键字 |
| `i8` `i16` `i32` `i64` `u8` `u16` `u32` `u64` `f32` `f64` | 定宽数值类型名关键字 |
| `unit` `unknown` | 单例类型名关键字 |
| `print` | 内置输出函数 |
| `str` | 内置转换 `str(x)`，返回字符串 |

`_` 是保留通配符：在 `match` 模式与 `enum` 载荷占位中匹配任意值，不作为普通标识符使用。

### 已使用的符号

| 符号 | 现状 |
|------|------|
| `?` `!` `..` `::` `\|` `&` | 运算符（已使用） |
| `#` | 为未来的宏 / 元数据语法预留 |

`$` 只出现在模板字符串插值 `${...}` 中，作为插值的起始标记；它不是独立运算符。

### 预留（未来可能的）关键字

以下单词现在既不是关键字、也没有语法含义，但**已被保留**：任何把它们用作标识符（变量 / 函数 / 类型 / 成员名）的代码都会在解析时报 `unexpected reserved keyword X`。预留为未来语言特性留出演进空间：

`abstract` `actor` `associatedtype` `bench` `break` `case` `cfg` `channel` `class` `continue` `default` `defer` `deinit` `derive` `doc` `do` `export` `extension` `fallthrough` `final` `generic` `generator` `global` `guard` `init` `interface` `isolated` `iterator` `lazy` `library` `local` `macro` `meta` `module` `move` `new` `open` `override` `package` `priv` `protocol` `receiver` `ref` `rethrows` `select` `sender` `spawn` `static` `super` `switch` `task` `this` `typealias` `unowned` `unsafe` `virtual` `weak` `yield`

> `self` 是保留字，但在 `impl` 块的方法参数列表与方法体内可作为接收者使用（见「特征」一章）。
> `priv` 已预留但**当前无私有语义**：现有模块系统中，未导出的声明本身即为模块私有。预留仅在词法层禁止其作标识符。

### 建议避免的名称（非强制）

以下内建类型名不是关键字，但建议不要作为变量名使用（当前不报错，仅约定）：

`List<T>` `Task<T>`

## 字面量

```xulo
123            // number（整数）
3.14           // number（浮点）
"hello"        // string（双引号）
'world'        // string（单引号）
true           // bool
false          // bool
null           // null
[1, 2, 3]      // 列表字面量
{ name: "lyy" } // 对象字面量
```

字符串支持转义：`\"`、`\'`、`\\`、`\n`、`\t`、`\r`。

## 运算符与符号

| 符号 | 含义 |
|------|------|
| `+` `-` `*` `/` | 算术 |
| `==` `!=` `<` `>` `<=` `>=` | 比较 |
| `and` `or` `!` | 逻辑 |
| `??` | 空合并 |
| `?:`（`?` `:`） | 三目 |
| `..<` | 范围（左闭右开） |
| `..=` | 范围（闭区间，含右端） |
| `.` `?.` | 成员访问 / 可选成员访问 |
| `[ ]` | 下标 |
| `...` | 展开（列表/对象字面量内） |
| `::` | 枚举成员 / 命名空间 |
| `=` | 赋值 |
| `=>` | `match` 分支 |
| `,` `;` `:` `(` `)` `{` `}` | 分隔 / 分组 |

语句末尾的分号可省略。
