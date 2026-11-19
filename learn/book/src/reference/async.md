# 异步

> 相关：[控制流](control-flow.md) · [变量与绑定](variables-and-state.md) · [应用结构](application.md)

## `async` / `await`（TypeScript 风格）

```xulo
async fn fetch_user(id: string) {
  let response = await fetch("/api/users/" + id)
  let data = await response.json()
  return data
}
```

- 在 `fn` 前写 `async` 声明异步函数；调用它得到一个任务，结果用 `await` 取出
- `await` 只能出现在异步函数或异步闭包内；对非任务值使用 `await` 是语义错误
- `throw <expr>` 抛出异常

## `try` / `catch`

```xulo
async fn fetch_user(id: string) {
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
async fn main() {
  let work = async fn(): number { 21 * 2 }
  print(await work())   // 42
}
```

异步闭包以 `async` 前缀声明，写作 `async fn(参数): 返回类型 { ... }`；调用返回 `Task<T>`，可用 `await`。

## 异步入口

`main` 也可以声明为 `async fn main()`；运行时执行入口并等待其任务完成。详见 [应用结构](application.md)。
