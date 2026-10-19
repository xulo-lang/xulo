# 异步

> 相关：[控制流](control-flow.md) · [变量与状态](variables-and-state.md)

## `async` / `await`（TypeScript 风格）

```xulo
fn fetchUser(id: string): async {
  let response = await fetch("/api/users/" + id)
  let data = await response.json()
  return data
}
```

- `: async` 返回标注声明异步函数（等价 JS `async function`）
- `await` 只能出现在异步函数内；对非 Promise（非 `async` 返回）值使用 `await` 是语义错误
- `throw <expr>` 抛出异常

## `try` / `catch`

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

## 异步闭包

```xulo
fn main(): async {
  let work = fn(): async { 21 * 2 }
  print(await work())   // 42
}
```

异步闭包写作 `fn(): async [类型]`，调用返回 Promise，可用 `await`。

## 异步函数中的状态限制

异步函数**不能**使用 `@State`、`@Store`、`@Effect`、`@Environment`。异步操作应通过**直接调用 Store API**：

```xulo
// ✅ 正确：直接调用 useAppStore()
pub fn fetchUser(id: string): async {
  const store = useAppStore()
  store.actions.setLoading(true)
  let data = await fetch("/api/users/" + id).json()
  store.actions.setUser(data)
}

// ❌ 错误：在异步函数里使用 @Store 装饰器
pub fn fetchUser(id: string): async {
  @Store const { setUser } = useAppStore()  // ❌ 禁止
}
```
