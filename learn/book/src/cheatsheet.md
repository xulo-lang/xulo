# 速查表

> 一张总表。详细规则见对应参考章节。

## 关键字

| 关键字 | 用途 | 章节 |
|--------|------|------|
| `fn` `return` | 函数定义 / 返回 | [函数](reference/functions.md) |
| `let` `const` | 变量绑定 | [变量与状态](reference/variables-and-state.md) |
| `if` `else` | 条件 | [控制流](reference/control-flow.md) |
| `for` `in` | 循环 | [控制流](reference/control-flow.md) |
| `while` | 循环 | [控制流](reference/control-flow.md) |
| `match` | 模式匹配 | [控制流](reference/control-flow.md) |
| `and` `or` | 逻辑与 / 或 | [表达式](reference/expressions.md) |
| `async` `await` | 异步 | [异步](reference/async.md) |
| `try` `catch` `throw` | 异常 | [控制流](reference/control-flow.md) |
| `import` `export` `from` `as` `default` | 模块 | [模块系统](reference/modules.md) |
| `type` `enum` | 类型声明 | [数据与类型](reference/types.md) |
| `null` `true` `false` | 字面量 | [词法结构](reference/lexical.md) |
| `print` | 内置输出 | [词法结构](reference/lexical.md) |

装饰器（`@` 后，仅 `Component` 函数顶层）：`@State`、`@Store`、`@Effect`、`@Environment`。

内建类型名：`string` `number` `boolean` `list` `object` `Component`。

## 运算符优先级（高 → 低）

| 优先级 | 运算符 | 结合性 |
|--------|--------|--------|
| 后缀 | `x.y` `x?.y` `x[i]` `f(x)` | 左 |
| 一元 | `!` `-` `await` | 右 |
| 乘除 | `*` `/` | 左 |
| 加减 | `+` `-` | 左 |
| 比较/相等 | `<` `>` `<=` `>=` `==` `!=` `..<` | 左 |
| 空合并 | `??` | 左 |
| 逻辑与 | `and` | 左 |
| 逻辑或 | `or` | 左 |
| 三目 | `?` `:` | 右 |
| 赋值 | `=` | 右 |

> 实现把相等（`==`/`!=`）与比较（`<`/`>`/`<=`/`>=`）合并为同一层；形式语法中是两层（见 [形式语法](reference/grammar.md)）。

## 常见写法速查

```xulo
// 变量
let x = 1
const NAME = "Xulo"
let s: string? = null

// 类型
type User = { name: string, age: number }
type Status = "active" | "inactive"
enum Theme { Light Dark System }

// 函数
fn add(a: number, b: number): number { a + b }
fn greet(name: string = "stranger"): string { "Hello, " + name }
let double = fn(x: number): number { x * 2 }

// 控制流
let m = if a > b { a } else { b }
for i in 0..<10 { print(i) }
while c < 10 { c = c + 1 }
match x { 0 => "zero" _ => "other" }

// 组件与状态
fn Counter(): Component {
  @State let n: number = 0
  VStack {
    Text("Count: " + str(n))
    Button(onClick: fn() { n = n + 1 }) { Text("+") }
  }
}

// 异步
fn load(): async { let v = await fetch() return v }

// 模块
import { add } from "./math"
export fn f(): number { 1 }
```
