# 数据与类型

> 相关：[变量与状态](variables-and-state.md) · [函数](functions.md) · [速查表](../cheatsheet.md)

## 基础类型

```xulo
string         // 字符串
number         // 数字（整数和浮点统一）
boolean        // 布尔值
list<T>        // 列表（泛型）
object         // 对象（结构体）
null           // 空值
T?             // 可选类型（Swift 风格，等价于 T | null）
T | U          // 联合类型（TS 风格）
T & U          // 交叉类型（TS 风格）
```

`number` 统一表示整数与浮点；`print(...)` 与 `list<T>` 是内建能力。`Component` 是内建**标记类型**：编译器用它识别「返回该类型的函数即 UI 组件」，本身无运行时实现，具体组件由标准库 `@xulo/ui` 提供（见 [UI 组件](ui.md)）。

## 类型别名（统一使用 `type`，无 `interface`）

```xulo
// 对象类型
type User = {
  name: string
  age: number
  email: string?
}

// 联合类型
type Status = "active" | "inactive" | "pending"

// 函数类型
type Handler = fn(request: Request): Response

// 组合类型（交叉类型）
type UserWithRole = User & {
  role: string
}

// 泛型
type Result<T> = {
  data: T?
  error: string?
}
```

## 枚举类型（enum）

枚举用于定义一组固定的值。

### 简单枚举

```xulo
enum Theme {
  Light
  Dark
  System
}
```

### 带关联数据的枚举（Swift/Rust 风格）

```xulo
enum Result<T> {
  Success(T)
  Error(string)
}

enum Action {
  Click
  Submit(data: object)   // 具名关联数据（字段名）
  Cancel
}

enum Person {
  Nobody
  Named(string, number)  // 多参数 payload
}
```

payload 可以是位置形式 `Success(T)`、具名形式 `Submit(data: object)`，也支持多参数 `Named(string, number)`；构造按位置传参。匹配时每个参数对应一个绑定，用 `_` 丢弃不需要的槽位：

```xulo
Action::Submit({ message: "hi" })
match a {
  Action::Submit(data) => data
}

let p = Person::Named("Ada", 36)
match p {
  Person::Named(name, _) => name   // 丢弃 age
  Person::Nobody => "anon"
}
```

### 使用

```xulo
let theme = Theme::Dark

match theme {
  Theme::Light => "☀️"
  Theme::Dark => "🌙"
  Theme::System => "💻"
}

let result = Result::Success(42)
match result {
  Result::Success(value) => print("Got: " + value)
  Result::Error(msg) => print("Error: " + msg)
}
```

### 规则

- 枚举名称首字母大写
- 成员首字母大写
- 关联数据类型放在 `(type)` 中
- 支持泛型（类型参数在类型检查时擦除）
