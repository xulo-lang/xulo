# 表达式

> 相关：[控制流](control-flow.md) · [函数](functions.md) · [词法结构](lexical.md)

## 运算符优先级（高 → 低）

```text
后缀            x.y  x[i]  f(x)  x?.y
一元            !  -  await
乘除            *  /
加减            +  -
比较/相等        <  >  <=  >=  ==  !=  ..<
空合并          ??
逻辑与          and
逻辑或          or
三目            ?:
赋值            =
```

> 说明：实现把相等（`==`/`!=`）与比较（`<`/`>`/`<=`/`>=`）合并为同一优先级、左结合；形式语法（EBNF）中是两层，见 [形式语法](grammar.md)。

## 逻辑与三目

```xulo
let ok = a > 1 and b < 2       // `and` / `or`（不含 `&&` `||`）
let n = a > 1 ? "big" : "small" // 三目
print(!flag)                    // 逻辑非
```

## 字符串拼接

```xulo
let who = "Xulo"
print("Hello, " + who + "!")    // `+` 拼接字符串，可混入 number/boolean/null
```

## 成员访问 / 下标 / 可选链 / 空合并

```xulo
user.name           // 成员访问
xs[0]               // 下标
user?.name          // 可选成员访问（对象为 null 时得到 null）
let name = user?.name ?? "anonymous"   // ?? 空合并
```

## 列表展开

```xulo
let head = [1, 2]
let tail = [3, 4]
let all = [...head, ...tail]   // 展开合并
let withExtra = [...all, 9]
```

`...` 只能出现在列表/对象字面量内，展开操作数必须是 `list<T>`（列表）或对象。

## 对象展开

```xulo
let base = { a: 1 }
let copy = { ...base, b: 2 }   // 展开合并
```

## `match` 表达式

`match` 是表达式（详见 [控制流 · match](control-flow.md)）：

```xulo
let label = match status {
  0 => "zero"
  1 => "one"
  _ => "other"
}
```

## 函数值调用

函数值可从任意表达式调用：

```xulo
xs[0](10)        // 调用列表中的函数
getFn()(x)       // 调用返回的函数
(f)(5)           // 调用括号包裹的函数值
```
