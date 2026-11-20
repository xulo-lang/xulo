# 异步

> 相关：[控制流](control-flow.md) · [变量与绑定](variables-and-state.md) · [应用结构](application.md)

## `async` / `await`（TypeScript 风格）

```xulo
async fn fetch_value(): int {
  let v = await compute()
  v
}
```

- 在 `fn` 前写 `async` 声明异步函数；调用它得到一个任务，结果用 `await` 取出
- `await` 只能出现在异步函数或异步闭包内；对非任务值使用 `await` 是语义错误

## 异步失败：`Result` 与 `?`

异步代码没有单独的异常机制。失败通过返回 `Result<T, E>` 表达，`?` 也可以出现在异步函数体内：

```xulo
async fn fetch_user(id: string): Result<string, Error> {
  let response = await fetch("/api/users/" + id)?
  response.json()
}

async fn load_name(id: string): Result<string, Error> {
  let user = await fetch_user(id)?
  Result::Ok(user)
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
