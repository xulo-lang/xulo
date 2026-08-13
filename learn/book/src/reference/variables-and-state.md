# 变量与状态

> 相关：[数据与类型](types.md) · [UI 组件](ui.md) · [应用结构](application.md)

Xulo 的绑定分两类：普通变量（`let` / `const`）与响应式状态（`@State` / `@Store` / `@Environment` / `@Effect`）。后三者是语言关键字（不是装饰器），只能在返回类型为 `Component` 的函数顶层使用。

## 变量绑定（TypeScript 风格）

```xulo
const APP_NAME = "Xulo"       // 常量（不可变）
let count = 0                 // 变量（可变）
let name: string = "Alice"    // 带类型标注
let maybe: string? = null     // 可选类型
```

**规则：**

- `const` = 不可变绑定；`let` = 可变绑定
- 类型在变量名后，用 `:` 分隔（TS 风格）
- 类型可推断，标注可省略
- 赋值语句 `count = count + 1` 修改可变绑定；给 `const` 赋值是语义错误

## 局部状态 `@State`（SwiftUI 风格）

```xulo
@State let count: number = 0
@State let name: string = ""
@State let isActive: boolean = true
```

- `@State` 只能在 UI 组件（返回 `Component` 的函数）顶层声明
- 触发 UI 重新渲染
- 编译为响应式信号：读取 `.get()`，写入 `.set()`

## 全局状态 `@Store`（Zustand 风格）

### 定义 Store

```xulo
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

// 注意：当前实现暂不支持显式泛型调用（`createStore<AppState>`），故省略类型实参。
export const useAppStore = createStore(
  {
    user: null,
    theme: Theme::Light,
    notifications: [],
    loading: false
  },
  {
    setUser: setUser,
    setTheme: setTheme
  }
)
```

### 使用 Store

```xulo
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

- `@Store` 解构取值（`{ user }`）和取函数（`{ setUser }`）
- 依赖追踪与重渲染由外部 store 运行时接管

## 环境注入 `@Environment`

```xulo
@Environment let router: Router
router.push("/about")
```

- 从外部运行时注入一个值（编译为 `__env("Router")`）
- 同样仅在 `Component` 函数顶层可用

## 副作用 `@Effect`（SwiftUI 风格）

```xulo
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

- `@Effect` 在组件挂载时执行
- 支持依赖数组 `[id]`
- 支持返回清理函数

## 双向绑定 `$`（SwiftUI 风格）

```xulo
// 双向绑定（用 $ 前缀）
Input(value: $name)
Checkbox(checked: $isActive)

// 等价于
Input(value: name, onInput: fn(e) {
  name = e.value
})
```

- `$` 前缀表示双向绑定
- 只能用于 `@State` 或 `@Store` 变量
- 编译为 `{ value, onChange }`（`@State` 的写方向通过信号 `.set()` 生效）

## 状态 / 副作用使用限制

> **`@State`、`@Store`、`@Effect`、`@Environment` 只能在返回类型为 `Component` 的函数顶层使用。**

### ✅ 正确用法（UI 组件中）

```xulo
fn UserProfile(): Component {
  @Store const { user, loading } = useAppStore()
  @State let editing: boolean = false
  @Effect fn() { fetchUser(id) }

  VStack { ... }
}
```

### ❌ 错误用法（普通函数 / 异步函数 / 嵌套块）

```xulo
fn helperFunction() {
  @Store const { user } = useAppStore()  // ❌ 禁止
  @State let count = 0                   // ❌ 禁止
}

fn main(): Component {
  if true {
    @State let x = 0                     // ❌ 禁止（嵌套块）
  }
}
```

### 规则总结

| 语法 | 允许的位置 | 禁止的位置 |
|------|-----------|-----------|
| `@State` | 仅 `fn ...(): Component` 顶层 | 普通函数、异步函数、嵌套块 |
| `@Store` | 仅 `fn ...(): Component` 顶层 | 普通函数、异步函数、嵌套块 |
| `@Effect` | 仅 `fn ...(): Component` 顶层 | 普通函数、异步函数、嵌套块 |
| `@Environment` | 仅 `fn ...(): Component` 顶层 | 普通函数、异步函数、嵌套块 |
