# 控制流

> 相关：[表达式](expressions.md) · [函数](functions.md) · [异步](async.md)

## `if` 表达式（Rust/Swift 风格）

```xulo
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

## `for` 循环（Swift 风格）

```xulo
for item in items {
  print(item)
}

for i in 0..<10 {     // 范围（左闭右开）
  print(i)
}
```

## `while` 循环

```xulo
let count = 0
while count < 10 {
  count = count + 1
}
```

## `match`（Rust 风格）

```xulo
match value {
  0 => "zero"
  1 => "one"
  _ => "other"
}
```

- 各分支的尾部表达式必须相互兼容（与 `if` 两分支的规则一致）：类型互不兼容时静态报错
- 分支间分隔可选逗号或换行，两者均可
- 泛型枚举的 payload（`Result<T> { Success(T) ... }`）在 arm 内被擦除为 `any`，可安全地与其他分支的类型合并
- 支持枚举成员与 payload 绑定：

```xulo
let result = Result::Success(42)
match result {
  Result::Success(value) => print("Got: " + value)
  Result::Error(msg) => print("Error: " + msg)
}
```

## 异常：`throw` / `try` / `catch`

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

- `throw <expr>` 抛出异常
- `try { ... } catch (e) { ... }` 捕获异常
