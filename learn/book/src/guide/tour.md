# 语言之旅

这一节用一段完整代码带你走一遍 Xulo 的核心语法。每个特性在 [参考](../reference/lexical.md) 部分都有更详细的说明。

## 数据与类型

```xulo
type User = {
  name: string
  age: number
  email: string?          // 可选类型
}

type Status = "active" | "inactive"   // 字符串字面量联合

enum Theme {
  Light
  Dark
  System
}

enum Result<T> {           // 带关联数据的泛型枚举
  Success(T)
  Error(string)
}
```

## 函数

```xulo
// 隐式返回（Rust 风格）
fn add(a: number, b: number): number {
  a + b
}

// 可选 / 默认参数，命名实参
fn Button(label: string, variant: string = "primary"): Component {
  // ...
}
Button(variant: "outline", label: "Submit")
```

## 控制流

```xulo
let max = if a > b { a } else { b }     // if 表达式

for i in 0..<10 { print(i) }            // 范围

let label = match theme {
  Theme::Light => "☀️"
  Theme::Dark => "🌙"
  _ => "💻"
}
```

## 响应式状态

```xulo
fn Counter(): Component {
  @State let count: number = 0

  VStack {
    Text("Count: " + str(count))
    Button(onClick: fn() { count = count + 1 }) {
      Text("+")
    }
  }
}
```

`@State` 是响应式信号，赋值触发重渲染；`@Store` / `@Effect` / `@Environment` 提供全局状态、副作用与环境注入。详见 [变量与状态](../reference/variables-and-state.md)。

## 异步

```xulo
fn fetchUser(id: string): async {
  try {
    let response = await fetch("/api/users/" + id)
    return await response.json()
  } catch (e) {
    print("Error: " + e.message)
    return null
  }
}
```

## 模块

```xulo
// math.xulo
pub fn add(a: number, b: number): number { a + b }
pub const PI = 3.14

// main.xulo
import { add, PI } from "./math"
fn main() { print(add(1, PI)) }
```

## 下一步

- 完整的语法参考：见 [第二部分 · 参考](../reference/lexical.md)。
- 一张总表：见 [速查表](../cheatsheet.md)。
- 一个多文件完整应用：见 [完整示例](../appendix/example.md)。
