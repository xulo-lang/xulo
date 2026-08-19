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

`number` 统一表示整数与浮点；`print(...)` 与 `list<T>` 是内建能力。`View` 是内建**标记类型**：编译器用它识别「返回该类型的函数即 UI 组件」，本身无运行时实现，具体组件由标准库 `@xulo/ui` 提供（见 [UI 组件](ui.md)）。

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

## 特征（trait）

`trait` 声明一组方法签名，作为「能力」契约；`impl Trait for Type` 为某个具名类型提供这些方法的实现。`self`（保留字）作为接收者，标记该方法为「实例方法」。

```xulo
trait Area {
    fn area(self): number
    fn perimeter(self): number
}

type Rectangle = { w: number, h: number }

impl Area for Rectangle {
    fn area(self): number { self.w * self.h }
    fn perimeter(self): number { 2 * (self.w + self.h) }
}
```

调用通过**显式派发** `Trait::method(接收者, ...)` 进行——编译器把它解析成对 `impl` 方法（mangled 为 `impl_{Trait}_{Type}_{method}`）的静态调用，零运行时开销：

```xulo
fn main() {
    let r: Rectangle = { w: 3, h: 4 }
    print(str(Area::area(r)))        // 12
    print(str(Area::perimeter(r)))   // 14
}
```

### 泛型约束

类型参数可以用 `T: Trait`（行内）或 `where T: Trait` 约束。有界类型参数在类型检查时被精化为特征的结构形状，可对其成员访问；多个特征用 `&` 连接：

```xulo
fn describe<T: Area>(t: T): number {
    Area::area(t)
}
```

### 规则

- 派发接收者必须是**静态具名类型**：`Trait::method(recv)` 的 `recv` 需要在调用点能解析出具体的具名类型，且该类型已注册 `impl`，否则报「does not implement trait」。
- `impl` 方法必须实现 trait 声明的全部方法，且签名（含 `self`）与声明的参数/返回类型双向可赋值。
- 泛型参数的值在运行时被擦除，因此 `T: Trait` 约束下无法做派发调用——只有具名类型的实例可以。
- `trait` 是可导出的模块成员（`pub trait`），可被其他模块 `import type` 引入后再本地实现。
