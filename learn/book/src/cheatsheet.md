# 速查表

> 一张总表。详细规则见对应参考章节。

## 关键字

| 关键字 | 用途 | 章节 |
|--------|------|------|
| `fn` `return` | 函数定义 / 返回 | [函数](reference/functions.md) |
| `let` `let mut` `const` | 变量绑定 / 可变绑定 / 常量 | [变量与绑定](reference/variables-and-state.md) |
| `if` `else` | 条件 | [控制流](reference/control-flow.md) |
| `for` `in` | 循环 | [控制流](reference/control-flow.md) |
| `while` `break` `continue` `defer` | 循环与延迟执行 | [控制流](reference/control-flow.md) |
| `match` | 模式匹配 | [控制流](reference/control-flow.md) |
| `and` `or` `is` `not` | 逻辑与类型判断 | [表达式](reference/expressions.md) |
| `async` `await` `spawn` `lock` `shared` | 异步与并发 | [异步](reference/async.md) |
| `mod` `use` `pub` `as` `super` | 模块 | [模块系统](reference/modules.md) |
| `struct` `type` `enum` `trait` `impl` `where` | 类型声明 | [数据与类型](reference/types.md) |
| `extern` `copy` `move` `self` `mut` | FFI、所有权与接收者 | [数据与类型](reference/types.md) |
| `null` `true` `false` | 字面量 | [词法结构](reference/lexical.md) |
| `bool` `string` `number` `int` `float` | 原始类型名 | [数据与类型](reference/types.md) |
| `i8` `i16` `i32` `i64` `u8` `u16` `u32` `u64` `f32` `f64` `isize` `usize` `unit` `unknown` | 定宽数值 / 单例类型名 | [数据与类型](reference/types.md) |

`print`、`println`、`str` 是 prelude 内建名，不是关键字。内建类型名：`string` `number` `int` `float` `bool` `unit` `List<T>` `Map<K, V>` `Set<T>` `Array<T, N>` `Task<T>` `Range<T>` `Result<T, E>` `Error`。

## 运算符优先级（高 → 低）

| 优先级 | 运算符 | 结合性 |
|--------|--------|--------|
| 后缀 | `f(x)` `x[i]` `x.y` `x.0` `x?.y` `x?.0` `x?` | 左 |
| 一元 | `!` `-` `~` `await` `move` `copy` | 前缀 |
| 幂 | `**` | 右 |
| 乘除 | `*` `/` `%` | 左 |
| 加减 | `+` `-` | 左 |
| 移位 | `<<` `>>` | 左 |
| 位与 / 异或 / 或 | `&` `^` `\|` | 左 |
| 区间 | `..<` `..=` | 非结合 |
| 比较 | `<` `>` `<=` `>=` | 非结合 |
| 类型判断 | `is` `is not` | 非结合 |
| 相等 | `==` `!=` | 左 |
| 空合并 | `??` | 左 |
| 逻辑与 | `and` | 左 |
| 逻辑或 | `or` | 左 |
| 三目 | `?` `:` | 右 |
| 赋值 | `=` | 右 |

完整规则见 `spec/expressions/operators.md`。

## 常见写法速查

```xulo
// 变量
let x = 1
let mut n = 0
const NAME = "Xulo"
let s: string? = null

// 类型
struct User { name: string, age: number }
type Status = "active" | "inactive"
enum Theme { Light Dark System }

// 函数
fn add(a: number, b: number): number { a + b }
fn greet(name: string = "stranger"): string { "Hello, " + name }
let double = fn(x: number): number { x * 2 }

// 控制流
let m = if a > b { a } else { b }
for i in 0..<10 { print(i) }
while n < 10 { n = n + 1 }
match x { 0 => "zero" _ => "other" }

// 结构体与普通函数
struct Counter { count: number }
fn bump(counter: Counter): Counter { Counter(count: counter.count + 1) }
let mut c = Counter(count: 0)
c = bump(c)

// 错误处理：Result 是数据，用 match 或 ? 处理
fn half(n: int): Result<int, string> {
  if n % 2 == 0 { Result::Ok(n / 2) } else { Result::Err("odd input") }
}
fn quarter(n: int): Result<int, string> {
  let h = half(n)?      // 失败时把 Result::Err 返回给调用者
  Result::Ok(h / 2)
}

// 异步
async fn load(): string { let v = await fetch_value() str(v) }

// 模块
mod math;
use math::{add, PI};
pub fn f(): int { 1 }
```
