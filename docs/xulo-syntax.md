# Xulo 语言语法规范 v1.0

Xulo 是一门 **UI 优先**的编程语言，语法吸收 Rust、Swift、TypeScript 的设计精华。

---

## 目录

1. [注释](#1-注释)
2. [变量绑定](#2-变量绑定)
3. [基础类型](#3-基础类型)
4. [类型别名](#4-类型别名)
5. [枚举类型](#5-枚举类型)
6. [函数](#6-函数)
7. [控制流](#7-控制流)
8. [模块系统](#8-模块系统)
9. [局部状态（@State）](#9-局部状态state)
10. [全局状态（@Store）](#10-全局状态store)
11. [副作用（@Effect）](#11-副作用effect)
12. [状态/副作用使用限制](#12-状态副作用使用限制)
13. [异步操作](#13-异步操作)
14. [UI 组件](#14-ui-组件)
15. [Props（组件参数）](#15-props组件参数)
16. [事件处理](#16-事件处理)
17. [绑定（$）](#17-绑定)
18. [主题系统](#18-主题系统)
19. [路由（标准库）](#19-路由标准库)
20. [应用入口：main](#20-应用入口main)
21. [编译器实现要点](#21-编译器实现要点)
22. [完整示例](#22-完整示例)

---

## 1. 注释

```
// 行注释

/*
  块注释
*/
```

---

## 2. 变量绑定（TypeScript 风格）

```
const APP_NAME = "Xulo"       // 常量（不可变）
let count = 0                 // 变量（可变）
let name: string = "Alice"    // 带类型标注
let maybe: string? = null     // 可选类型
```

**规则：**
- `const` = 不可变绑定
- `let` = 可变绑定
- 类型在变量名后，用 `:` 分隔（TS 风格）
- 类型可推断，省略标注

---

## 3. 基础类型（TypeScript 风格）

```
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

---

## 4. 类型别名（统一使用 `type`，无 `interface`）

```
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

---

## 5. 枚举类型（enum）

枚举用于定义一组固定的值。

### 简单枚举

```
enum Theme {
  Light
  Dark
  System
}

enum Status {
  Active
  Inactive
  Pending
}
```

### 带关联数据的枚举（Swift/Rust 风格）

```
enum Result<T> {
  Success(T)
  Error(string)
}

enum Action {
  Click
  Submit(data: object)   // 具名关联数据（字段名）
  Cancel
}
```

payload 可以是位置形式 `Success(T)`，也可以是具名形式 `Submit(data: object)`；两种形式构造与匹配都是位置传参：`Action::Submit({...})`、`match a { Action::Submit(d) => d }`。

### 使用

```
let theme = Theme::Dark

match theme {
  Theme::Light => "☀️"
  Theme::Dark => "🌙"
  Theme::System => "💻"
}

// 带数据的枚举
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
- 支持泛型

---

## 6. 函数（Rust + TS 混合风格）

```
// 无返回值
fn log(message: string) {
  print(message)
}

// 有返回值（隐式返回，Rust 风格）
fn add(a: number, b: number): number {
  a + b
}

// 显式 return
fn subtract(a: number, b: number): number {
  return a - b
}

// 泛型
fn first<T>(list: list<T>): T {
  list[0]
}

// 泛型调用处推断类型实参：`first([1, 2, 3])` 把 `T` 绑定为 `number`
let n: number = first([1, 2, 3])
let s: string = first(["a"])      // ✅ T = string
let bad: string = first([1, 2])   // ❌ 推断 T = number，与 string 不兼容

// 可选参数
fn greet(name: string?): string {
  if name != null {
    "Hello, " + name
  } else {
    "Hello, stranger"
  }
}

// 可选参数可省略（调用时可不传）
greet()     // ✅ name = null
greet("A")  // ✅

// 默认参数值
fn greet(name: string = "stranger"): string {
  "Hello, " + name
}

// 命名参数（参数可乱序传入）
fn Button(label: string, variant: string = "primary"): Component

Button(variant: "outline", label: "Submit")  // ✅ 命名参数
```

**规则：**
- 用 `fn` 关键字（Rust 风格）
- 参数类型用 `:`（TS/Swift 风格）
- 返回值用 `:`（TS/Swift 风格）
- 最后表达式无分号 = 隐式返回（Rust 风格）；**声明了返回类型时，尾部表达式须与返回类型匹配**（否则语义错误）
- 也支持 `return` 显式返回
- 无返回值时省略返回类型
- 参数可带默认值 `name: string = "stranger"`
- 可选参数 `name: string?` 与带默认值的参数一样，调用时可省略（返回 `null`/默认值）
- 调用时可用命名参数 `greet(name: "X")`；一旦使用命名参数，所有实参都须命名，且可乱序
- 字符串字面量联合类型（`type Status = "active" | "inactive"`）在实参处接受对应字面量

### 匿名函数 / 闭包（Function Values）

`fn` 也可在表达式位置出现，作为值传递（闭包捕获外层作用域，等价 JS 闭包）：

```
fn apply(f: fn(number): number, x: number): number {
  f(x)
}

fn makeAdder(n: number): fn(number): number {
  fn(v: number): number { v + n }   // ✅ 捕获 n
}

fn main() {
  let double = fn(x: number): number {
    x * 2
  }

  let add5 = makeAdder(5)
  print(apply(double, 3))   // 6
  print(add5(10))           // 15

  // 异步闭包：`fn(): async`
  let work = fn(): async { 42 }
  let v = await work()      // 42
}
```

**规则：**
- 匿名函数类型为 `fn(参数类型): 返回类型`（`Type::FnSig`），可赋值给带该类型的参数/变量。
- 通过捕获自动访问外层局部变量，可变捕获可直接修改外层 `let` 绑定。
- 调用函数值时只用位置实参，数量必须精确匹配；具名实参不支持。
- 函数值可从任意表达式中调用：`xs[0](10)`、`getFn()(x)`、`(f)(5)`。
- `fn(...)` 出现在语句位置（如块末用于隐式返回）时按匿名函数表达式解析。
- 异步闭包写作 `fn(): async [类型]`，调用返回 Promise，可用 `await`。
- 常用于回调/高阶函数（如 UI 的事件回调 `Button(onClick: fn() { ... })`）。

---

## 7. 控制流

### if 表达式（Rust/Swift 风格）

```
let max = if a > b { a } else { b }

if condition {
  // ...
} else if other {
  // ...
} else {
  // ...
}
```

`if` 用作表达式时，两分支的尾部表达式类型须兼容（否则「incompatible types」错误）。

### for 循环（Swift 风格）

```
for item in items {
  print(item)
}

for i in 0..<10 {
  print(i)
}
```

### match（Rust 风格）

```
match value {
  0 => "zero"
  1 => "one"
  _ => "other"
}
```

- 各分支的尾部表达式必须相互兼容（与 `if` 两分支的规则一致）：类型互不兼容时静态报错。
- 分支间分隔可选逗号或换行，两者均可。
- 泛型枚举的 payload（`Result<T> { Success(T) ... }`）在 arm 内被擦除为 `any`，可安全地与其他分支的类型合并。

### 逻辑与三目

```
let ok = a > 1 and b < 2      // `and` / `or`（不含 `&&` `||`）
let n = a > 1 ? "big" : "small"  // 三目
print(!flag)                     // 逻辑非
```

### 字符串拼接

```
let who = "Xulo"
print("Hello, " + who + "!")     // `+` 拼接字符串，可混入 number/boolean/null
```

### while 循环

```
let count = 0
while count < 10 {
  count = count + 1
}
```

### 成员访问 / 下标 / 可选链 / 空合并

```
user.name          // 成员访问
xs[0]              // 下标
user?.name         // 可选成员访问（对象为 null 时得到 null）
let name = user?.name ?? "anonymous"   // ?? 空合并
```

### 列表展开

```
let head = [1, 2]
let tail = [3, 4]
let all = [...head, ...tail]   // 展开合并
let withExtra = [...all, 9]
```

`...` 只能出现在列表/对象字面量内，展开操作数必须是 `list<T>`（列表）或对象（对象字面量）。

### 对象展开

```
let base = { a: 1 }
let copy = { ...base, b: 2 }   // 展开合并
```

### 运算符优先级（高 → 低）

```
* / + - < > <= >= == != ?? and or ?:
小组件语法用于成员/下标/调用：x.y x[i] f(x) x?.y
```

---

## 8. 模块系统（TypeScript 风格）

```
// 导出
export fn add(a: number, b: number): number {
  a + b
}

export const PI = 3.14

export type User = {
  name: string
}

export enum Status {
  Active
  Inactive
}

// 默认导出
export default fn main() {
  print("Hello")
}

// 导入
import { add, PI } from "math"
import type { User } from "types"
import * as math from "math"
import "core"
```

**导入解析规则（打包器）：**
- 相对路径或本地存在的模块名（如 `./math`、`math`）→ 解析为本地 `.xulo` 文件，参与打包。
- 其余说明符（如 `@xulo/ui`）→ 视为外部包，原样生成 ESM `import`。
- `import type` 只供类型检查；运行时仅当导入名同时是运行时值（如 `enum`）时保留其值绑定，以便 `Kind::Admin` 可用，否则完全擦除；`export enum` 同时导出运行时值与类型。
- **无运行时模块加载器**：每个文件编译为一个返回导出对象的 IIFE，依赖按拓扑序内联进同一个 JS 文件；入口文件加载时执行其 `main()`。
- 循环导入（A → B → A）会被拒绝。

---

## 9. 局部状态（@State）（SwiftUI 风格）

```
@State let count: number = 0
@State let name: string = ""
@State let isActive: boolean = true
```

**规则：**
- `@State` 是语言关键字（不是装饰器）
- 只能在 UI 组件顶层声明
- 触发 UI 重新渲染
- 用于组件内部状态

---

## 10. 全局状态（@Store）（Zustand 风格）

### 定义 Store

```
// stores/app.xulo
import { createStore } from "@xulo/store"

type AppState = {
  user: User?
  theme: Theme
  notifications: list<string>
  loading: boolean
}

// ✅ 对象字面量用括号包裹，避免歧义
fn setUser(state: AppState, user: User?): AppState {
  ({ ...state, user: user })
}

fn setTheme(state: AppState, theme: Theme): AppState {
  ({ ...state, theme: theme })
}

export const useAppStore = createStore<AppState>(
  {
    user: null,
    theme: Theme::Light,
    notifications: [],
    loading: false
  },
  {
    setUser,
    setTheme
  }
)
```

### 使用 Store

```
import { useAppStore } from "../stores/app"

fn Home(): Component {
  @Store const { user, theme, loading } = useAppStore()
  @Store const { setTheme } = useAppStore()
  
  VStack {
    Text("User: " + user?.name)
    Button(onClick: fn() { setTheme(Theme::Dark) }) {
      Text("Dark mode")
    }
  }
}
```

**规则：**
- `@Store` 是语言关键字
- 自动追踪依赖，变化时触发重渲染
- 解构取值（`{ user }`）和取函数（`{ setUser }`）

---

## 11. 副作用（@Effect）（SwiftUI 风格）

```
@Effect fn() {
  // 组件挂载时执行
  fetchUser(id)
}

@Effect fn() {
  // 依赖变化时重新执行
  fetchUser(id)
}, [id]

@Effect fn() {
  // 清理函数
  setupSubscription()
  return fn() {
    cleanupSubscription()
  }
}
```

**规则：**
- `@Effect` 是语言关键字
- 在组件挂载时执行
- 支持依赖数组 `[id]`，变化时重新执行
- 支持返回清理函数
- 清理函数在组件卸载或依赖变化前执行

---

## 12. 状态/副作用使用限制

### 核心原则

> **`@State`、`@Store`、`@Effect` 只能在返回类型为 `Component` 的函数顶层使用。**

### ✅ 正确用法（UI 组件中）

```
fn UserProfile(): Component {
  @Store const { user, loading } = useAppStore()
  @State let editing: boolean = false
  @Effect fn() { fetchUser(id) }
  
  VStack { ... }
}
```

### ❌ 错误用法（普通函数中）

```
fn helperFunction() {
  @Store const { user } = useAppStore()  // ❌ 禁止
  @State let count = 0                   // ❌ 禁止
}
```

### ✅ 正确的异步操作写法

异步函数通过**直接调用 Store API**，不能使用 `@Store` 装饰器：

```
// ✅ 正确：直接调用 useAppStore()
export fn fetchUser(id: string): async {
  const store = useAppStore()
  store.actions.setLoading(true)
  let data = await fetch("/api/users/" + id).json()
  store.actions.setUser(data)
}

// ❌ 错误：使用 @Store 装饰器
export fn fetchUser(id: string): async {
  @Store const { setUser } = useAppStore()  // ❌ 禁止
}
```

### 规则总结

| 语法 | 允许的位置 | 禁止的位置 |
|------|-----------|-----------|
| `@State` | 仅 `fn ...(): Component` 顶层 | 普通函数、异步函数、嵌套块 |
| `@Store` | 仅 `fn ...(): Component` 顶层 | 普通函数、异步函数、嵌套块 |
| `@Effect` | 仅 `fn ...(): Component` 顶层 | 普通函数、异步函数、嵌套块 |

---

## 13. 异步操作

### async / await（TypeScript 风格）

```
fn fetchUser(id: string): async {
  let response = await fetch("/api/users/" + id)
  let data = await response.json()
  return data
}
```

- `: async` 返回标注声明异步函数（等价 JS `async function`）。
- `await` 只能出现在异步函数内；对非 Promise（非 `async` 返回）值使用 `await` 是语义错误。
- `throw <expr>` 抛出异常。

### try / catch

```
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

### useAsync（标准库工具）

```
import { useAsync } from "@xulo/async"

fn UserProfile(id: string): Component {
  const { data: user, loading, error, refetch } = useAsync(
    fn() { fetchUser(id) },
    [id]
  )
  
  if loading { Text("Loading...") }
  else if error { Text("Error") }
  else { Text(user.name) }
}
```

**规则：**
- `async` 关键字标记异步函数
- `await` 等待 Promise 完成
- `try/catch` 捕获异步错误
- `useAsync` 是标准库提供的工具函数（可选）
- 异步函数中不能使用 `@State`、`@Store`、`@Effect`

---

## 14. UI 组件

### 基础组件（来自 @xulo/ui 标准库）

```
VStack, HStack, ZStack  // 布局容器
Center                   // 居中容器
Text                     // 文本
Button                   // 按钮
Input                    // 输入框
Checkbox                 // 复选框
Link                     // 链接
Divider                  // 分割线
Card, CardHeader, ...   // 卡片组件
Screen                   // 根容器
```

### 组件使用

```
// 带子元素的块语法
VStack(spacing: 16) {
  Text("Hello")
  Button(onClick: submit) {
    Text("Submit")
  }
}

// 无子元素直接用括号
Text("Hello", color: "blue")

// 嵌套
Screen {
  Center {
    Card(width: 400, radius: "lg") {
      Text("Welcome")
    }
  }
}
```

### 自定义组件

```
// 组件 = 返回 Component 的函数
fn MyCard(title: string, children: list<Component>): Component {
  Card(radius: "md", shadow: "sm") {
    Text(title, weight: "bold")
    children
  }
}

// 使用自定义组件
MyCard(title: "Hello") {
  Text("Content goes here")
}
```

### 条件渲染

```
if isLoggedIn {
  Text("Welcome back!")
} else {
  Button(onClick: login) {
    Text("Sign in")
  }
}
```

### 循环渲染

```
for item in items {
  Text(item)
}
```

---

## 15. Props（组件参数）

### 必传参数

```
fn Button(label: string): Component

// 调用时必须传入
Button(label: "Submit")  // ✅
Button()                 // ❌ 错误
```

### 可选参数（?）

```
fn Button(label: string, icon: string?): Component

// 调用时可传可不传
Button(label: "Submit")              // ✅ icon = null
Button(label: "Submit", icon: "save") // ✅
```

### 默认值（=）

```
fn Button(label: string, variant: string = "primary"): Component

// 调用时可传可不传
Button(label: "Submit")                 // ✅ variant = "primary"
Button(label: "Submit", variant: "outline") // ✅
```

### 组合

```
fn Button(
  label: string,                     // 必传
  variant: string = "primary",       // 默认值
  icon: string? = null,              // 可选，默认 null
  disabled: boolean? = false         // 可选，默认 false
): Component
```

---

## 16. 事件处理

```
// 函数引用
fn handleClick() {
  count = count + 1
}
Button(onClick: handleClick) {
  Text("Click me")
}

// 内联函数
Button(onClick: fn() {
  count = count + 1
}) {
  Text("Click me")
}

// 带参数的事件处理
fn handleInput(value: string) {
  name = value
}
Input(onInput: fn(e) { handleInput(e.value) })
```

---

## 17. 绑定（$）（SwiftUI 风格）

```
// 双向绑定（用 $ 前缀）
Input(value: $name)
Checkbox(checked: $isActive)

// 等价于
Input(value: name, onInput: fn(e) {
  name = e.value
})
```

**规则：**
- `$` 前缀表示双向绑定
- 只能用于 `@State` 或 `@Store` 变量
- 自动生成读/写两个方向

---

## 18. 主题系统

```
// 主题 token（来自 @xulo/theme）
Text("Hello", color: "$theme.text.primary")
Text("Note", color: "$theme.text.secondary")
Button(variant: "$theme.button.primary")

// 色值
Text("Red", color: "#ff0000")
Text("Named", color: "blue")
```

**规则：**
- `$theme.*` 引用主题系统的 token
- 支持自定义 token
- 各平台映射到对应主题系统

---

## 19. 路由（标准库）

```
import { Router, Route, Link } from "@xulo/router"

// 定义页面
fn Home(): Component { ... }
fn About(): Component { ... }
fn Profile(id: string): Component { ... }

// 路由配置
fn main(): Component {
  Router {
    Route(path: "/", component: Home)
    Route(path: "/about", component: About)
    Route(path: "/profile/:id", component: Profile)
    Route(path: "*", component: NotFound)
  }
}

// 声明式导航
Link(to: "/about") {
  Text("About")
}

// 编程式导航
@Environment let router: Router
router.push("/about")
```

---

## 20. 应用入口：main

`fn main()` 是 Xulo 应用的入口点，根据返回值类型决定身份：

### UI 应用（返回 Component）

```
fn main(): Component {
  Screen {
    VStack {
      Text("Hello, World!")
    }
  }
}
```

### CLI 脚本（无返回值）

```
fn main() {
  print("Hello from Xulo")
}
```

### 规则

- `fn main(): Component` → 根组件，渲染 UI
- `fn main()` (void) → 逻辑入口，执行脚本
- 一个文件只能有一个 `main` 函数
- 有 `main` 的文件是可执行应用，无 `main` 的文件是库模块

### 运行行为

| 文件类型 | `xulo run` 行为 |
|---------|----------------|
| 有 `fn main(): Component` | 渲染到目标平台（Web/Godot/SwiftUI） |
| 有 `fn main()` | 执行脚本，输出到终端 |
| 无 `main` | 视为库模块，仅导出供其他文件使用 |

---

## 21. 编译器实现要点

### 21.1 UI Block vs 代码块的区分

**问题：** `VStack { Text("Hello") }` 中的 `{ }` 是 UI Children 列表，而 `fn main() { let x = 5 }` 中的 `{ }` 是普通代码块。

**解决方案：** 在 AST 层面区分两种节点类型。

```
// UI Block — 只能包含 UI 元素
UIBlock {
  children: Vec<UIElement>  // Component, Text, if, for
}

// 代码块 — 只能包含普通语句
Block {
  statements: Vec<Statement>  // let, fn, if, for, return, expr
}
```

**解析策略：**
- `ComponentName { ... }` → 解析为 UI Block
- `fn` 函数体 → 解析为普通 Block
- `if` / `for` / `while` 的 `{ }` → 根据上下文决定

### 21.2 隐式返回与分号

**问题：** `a + b` 是返回值，`a + b;` 是语句。

**解决方案：** 检查函数体最后一个语句是否是无分号表达式。

```
// 函数体末尾无分号 → 隐式返回
fn add(a: number, b: number): number {
  a + b  // ✅ 隐式返回
}

// 函数体末尾有分号 → 语句，警告
fn add(a: number, b: number): number {
  a + b;  // ⚠️ 警告：忽略返回值
}
```

### 21.3 `T?` 与 `a ? b : c` 的区分

**问题：** 同一个 `?` 在类型中是可选标记，在表达式中是三目运算符。

**解决方案：** 上下文感知解析。

```
解析类型时：T? → Optional(T)
解析表达式时：a ? b : c → Ternary
```

### 21.4 解析器实现建议

| 阶段 | 方法 | 说明 |
|------|------|------|
| 词法分析 | 独立 Lexer | 生成 Token 流，不区分上下文 |
| 类型解析 | 上下文感知 | 解析类型时识别 `T?`、`T \| U`、`list<T>` |
| 表达式解析 | Pratt Parser | 处理优先级、三目、函数调用 |
| UI Block 解析 | 专用解析器 | 只允许 UI 元素，禁止普通语句 |
| 语义分析 | 访问者模式 | 类型检查、作用域、生命周期 |
| 模块打包 | 图加载 + IIFE | 依赖拓扑序、循环检测、导出符号/类型跨模块校验 |

### 21.5 模块打包（实现要点）

- **加载**：从入口文件开始 DFS，本地导入（相对路径或本地存在的裸名）先加载依赖（后序 → 拓扑序），循环导入报错。
- **语义**：按依赖顺序逐个 `analyze_with`，把依赖导出的符号（真实签名）与类型种子化给导入方；导入不存在的导出名报错。
- **代码生成**：每个模块编译为一个 `const __modN = (() => { ...; return { 导出 }; })();`，导入方用 `const { a } = __modN;` 解构；入口模块 IIFE 内最后调用 `main()`。
- **外部依赖**：非本地说明符原样生成顶层 ESM `import`（要求输出为 `.mjs` 或在 `package.json` 中 `"type": "module"`）。
- **跨模块调用**：导入的函数保留参数名，支持具名实参调用。

---

## 22. 完整示例

### 文件结构

```
project/
├── main.xulo          # 应用入口
├── stores/
│   └── app.xulo       # 全局状态
├── components/
│   └── button.xulo    # 自定义组件
└── pages/
    └── profile.xulo   # 页面组件
```

### `types.xulo`（共享类型）

```
// 枚举
export enum Theme {
  Light
  Dark
  System
}

export enum Status {
  Active
  Inactive
  Pending
}

// 类型别名
export type User = {
  id: string
  name: string
  email: string
  status: Status
}

export type Result<T> = {
  data: T?
  error: string?
}
```

### `stores/app.xulo`

```
import { createStore } from "@xulo/store"
import { type User, type Theme } from "../types"

type AppState = {
  user: User?
  theme: Theme
  notifications: list<string>
  loading: boolean
  error: string?
}

// ✅ 对象字面量用括号包裹
fn setUser(state: AppState, user: User?): AppState {
  ({ ...state, user: user, loading: false })
}

fn setTheme(state: AppState, theme: Theme): AppState {
  ({ ...state, theme: theme })
}

fn setLoading(state: AppState, loading: boolean): AppState {
  ({ ...state, loading: loading })
}

fn setError(state: AppState, error: string): AppState {
  ({ ...state, error: error, loading: false })
}

fn addNotification(state: AppState, message: string): AppState {
  ({ ...state, notifications: state.notifications + [message] })
}

export const useAppStore = createStore<AppState>(
  {
    user: null,
    theme: Theme::Light,
    notifications: [],
    loading: false,
    error: null
  },
  {
    setUser,
    setTheme,
    setLoading,
    setError,
    addNotification
  }
)

// ✅ 异步函数直接调用 useAppStore()，不使用 @Store
export fn fetchUser(id: string): async {
  const store = useAppStore()
  
  store.actions.setLoading(true)
  try {
    let response = await fetch("/api/users/" + id)
    let data = await response.json()
    store.actions.setUser(data)
    store.actions.addNotification("User loaded: " + data.name)
  } catch (e) {
    store.actions.setError(e.message)
  }
}
```

### `components/button.xulo`

```
import { type Theme } from "../types"

export enum ButtonVariant {
  Primary
  Secondary
  Outline
  Ghost
}

export fn PrimaryButton(
  label: string,
  onClick: fn()? = null,
  disabled: boolean? = false
): Component {
  Button(
    variant: ButtonVariant::Primary,
    onClick: onClick,
    disabled: disabled,
    width: "100%"
  ) {
    Text(label, weight: "bold")
  }
}

export fn OutlineButton(
  label: string,
  onClick: fn()? = null
): Component {
  Button(variant: ButtonVariant::Outline, onClick: onClick) {
    Text(label)
  }
}
```

### `pages/profile.xulo`

```
import { useAppStore, fetchUser } from "../stores/app"
import { PrimaryButton, OutlineButton } from "../components/button"
import { type User, type Theme, type Status } from "../types"

type Props = {
  id: string
}

fn UserProfile(props: Props): Component {
  // ✅ @State/@Store/@Effect 只能在 Component 函数顶层使用
  @State let editing: boolean = false
  @State let editName: string = ""
  @Store const { user, theme, loading, error } = useAppStore()
  @Store const { setTheme, addNotification } = useAppStore()
  
  @Effect fn() {
    fetchUser(props.id)
  }
  
  // 当 user 变化时更新编辑字段
  @Effect fn() {
    if user != null {
      editName = user.name
    }
  }, [user]
  
  Card(radius: "lg", shadow: "sm") {
    VStack(spacing: 16) {
      HStack {
        Text("User Profile", weight: "bold", size: 24)
        Spacer()
        Button(onClick: fn() {
          let newTheme = match theme {
            Theme::Light => Theme::Dark
            Theme::Dark => Theme::Light
            Theme::System => Theme::Light
          }
          setTheme(newTheme)
        }) {
          Text(match theme {
            Theme::Light => "🌙"
            Theme::Dark => "☀️"
            Theme::System => "💻"
          })
        }
      }
      
      if loading {
        Text("Loading...", color: "$theme.text.secondary")
      } else if error != null {
        Text("Error: " + error, color: "$theme.danger")
      } else if user != null {
        VStack(spacing: 8) {
          if editing {
            Input(value: $editName)
            HStack(spacing: 12) {
              PrimaryButton(label: "Save", onClick: fn() {
                // 保存逻辑
                editing = false
                addNotification("Name updated to: " + editName)
              })
              OutlineButton(label: "Cancel", onClick: fn() {
                editing = false
                editName = user?.name ?? ""
              })
            }
          } else {
            Text(user.name, weight: "bold", size: 18)
            Text(user.email, color: "$theme.text.secondary")
            Text("Status: " + match user.status {
              Status::Active => "🟢 Active"
              Status::Inactive => "🔴 Inactive"
              Status::Pending => "🟡 Pending"
            })
            PrimaryButton(label: "Edit Profile", onClick: fn() {
              editing = true
            })
          }
        }
      } else {
        Text("User not found", color: "$theme.warning")
      }
    }
  }
}
```

### `main.xulo`

```
import { Screen, VStack, Center } from "@xulo/ui"
import { UserProfile } from "./pages/profile"
import { useAppStore } from "./stores/app"
import { type Theme } from "./types"

fn main(): Component {
  @Store const { theme } = useAppStore()
  
  Screen(
    background: match theme {
      Theme::Light => "#ffffff"
      Theme::Dark => "#1a1a2e"
      Theme::System => "#f5f5f5"
    }
  ) {
    Center {
      VStack(spacing: 24) {
        Text("Xulo App", weight: "bold", size: 28)
        UserProfile(id: "123")
      }
    }
  }
}
```

---

## 语法总结

| 特性 | 风格来源 |
|------|---------|
| 变量绑定 `const`/`let` | TypeScript |
| 函数 `fn` | Rust |
| 类型标注 `: Type` | TypeScript / Swift |
| 隐式返回 | Rust |
| 可选类型 `T?` | Swift |
| 泛型 `<T>` | TypeScript / Rust |
| 联合类型 `T \| U` | TypeScript |
| 交叉类型 `T & U` | TypeScript |
| 枚举 `enum` | Swift / Rust |
| 枚举关联数据 | Swift / Rust |
| 模块 `import`/`export` | TypeScript |
| if 表达式 | Rust / Swift |
| match | Rust |
| `@State` | SwiftUI |
| `@Store` | 新设计（Zustand 风格） |
| `@Effect` | SwiftUI |
| `$` 绑定 | SwiftUI |
| `async`/`await` | TypeScript |
| 块语法 `{ ... }` | SwiftUI |
| 应用入口 `main` | Rust/Go |
| 对象字面量 `({ ... })` | 自定义（消除歧义） |
| `@State/@Store/@Effect` 使用限制 | 自定义 |
