# 原生运行时（xulo-runtime）内存模型

> 面向想要理解 / 修改 `crates/xulo-runtime` 的读者。本文描述 Rust 解释器的
> 值表示、作用域、闭包与异步调度在内存上的组织方式，以及已知的内存关注点
> 与处置优先级。

---

## 1. 概述

原生运行时是一个**树遍历解释器**：纯 Rust、**无 GC、无 arena**。所有堆分配
通过所有权系统管理：

- **共享可变状态**用 `Rc<RefCell<T>>`（`List`、`Object`、`Promise`、`Env`）；
- **共享不可变状态**用 `Rc<T>`（`FunctionValue`）；
- **值类型**直接嵌入 `Value` 枚举（`Number`/`Boolean`/`Null`），或通过 `String`
  （字符数据在堆）与 `Box`（枚举 payload）持有。

因此整个运行时是**引用计数式内存**：谁持有 `Rc` 谁就有权访问，计数归零即释放。
CLI 场景下进程退出统一回收；**长驻场景（REPL / 库宿主）下的环境环已在
`Interpreter::drop` 时被清扫打断**（见 §4 与 §7 P1）；值级自环（`o.x = o`）仍
随 Rc 计数语义释放——那种自环由用户构造时持续存活，属引用计数模型的固有边界。

## 2. `Value` 的表示（`value.rs`）

`Value` 枚举的两种语义：

| 语义 | 变体 | 说明 |
|---|---|---|
| 按值 | `Number(f64)` / `Boolean(bool)` / `Null` | 直接嵌入枚举体 |
| 按值 | `String(String)` | 栈上仅字符串头，字符数据在堆 |
| 按值 | `Enum { enum_name, tag, payload: Option<Box<Value>> }` | 完整结构体内联；payload 走 `Box` 单分配 |
| 按值 | `Native(NativeFn)` | 内置函数指针（`print`/`str`） |
| 引用 | `List(Rc<RefCell<Vec<Value>>>)` | 共享可变句柄 |
| 引用 | `Object(Rc<RefCell<Vec<(String, Value)>>>)` | 共享可变句柄 |
| 引用 | `Function(Rc<FunctionValue>)` | 共享不可变函数体 + 捕获的闭包环境 |
| 引用 | `Promise(Rc<RefCell<Promise>>)` | 进行中的 async 调用，与每个 `await` 者共享 |

**克隆代价**：

- `Rc`/`RefCell` 变体克隆 = `Rc::clone`（引用计数 +1），**O(1)**；
- `String` 变体克隆是**深拷贝**，**O(n)** —— `Env::get` 查找返回 `value.clone()`
  时会复制字符串，字符串密集程序需留意。

`format_seen` 用「访问过的 `Rc::as_ptr` 指针集合」防止对**环**的无限递归
（如 `o.x = o`），并把环渲染为 `[<cycle>]` / `{ <cycle> }`；子树渲染完即把指针
从集合移除，因此共享但非环的引用（`[a, a]`）仍会各自打印一次。

## 3. 作用域链 `Env`（`env.rs`）

```rust
struct Env {
    parent: Option<Rc<RefCell<Env>>>,   // 词法外层（root 为 None）
    bindings: HashMap<String, Value>,   // 本层绑定
}
```

- 每个新作用域（函数调用、块、循环体、`try`/`catch`、`match` payload 绑定）
  都是 `Env::child(parent)` 新建的独立 `Env`。**`for`/`while` 每轮迭代都新建
  一个**（性能上较费，见 §7 P3）。
- 查找 `get(name)`：本层命中返回 `value.clone()`，否则沿 `parent` 上溯。
- 赋值 `assign(name, value)`：沿链找到**最近声明处**原地覆盖（模拟 JS `let`
  的重赋值语义），全链未声明则报错。

层次结构：

```text
root（Interpreter.global：print/str + 全部 impl 方法）
 └─ module（exec_module 为每个模块 Env::child(global)，imports 绑定于此）
     └─ 调用环境（函数调用 Env::child(closure)，默认参数在此求值）
         └─ 块 / 循环体 / try / match 绑定 子环境
```

`Interpreter` 本身持有：

- `out: RefCell<Vec<String>>` —— `print` 输出行；
- `global: Rc<RefCell<Env>>` —— 根作用域；
- 异步调度字段（见 §5）：`tasks`、`task_free`、`ready`、`current_task`、
  `task_yielder`、`call_depth`。

## 4. 闭包捕获与 Rc 环（已知问题 D11）

`FunctionValue`：

```rust
struct FunctionValue {
    params: Vec<Param>,
    body: Block,
    return_type: Option<Type>,
    is_async: bool,
    closure: Rc<RefCell<Env>>,   // 定义时的词法环境
}
```

函数值捕获**定义环境**。顶层 `fn a` 注册时被 `define` 进 global，同时它的
`closure` 也指向 global：

```text
global
  └─ "a" ──> FunctionValue { closure: global }   （Rc 环）
```

任何顶层函数都会形成这个环。结果：`drop(Interpreter)` 后 global 仍至少存活一个
引用计数，**环内的内存不会回收**。CLI 进程退出即回收，无实际影响；长驻宿主若
反复创建/丢弃 Interpreter 会逐次泄漏。

**处置（已修复，§7 P1）**：`Interpreter::drop` 实现里清空 root 环境的绑定
（`Env::clear`），`global ──> "a"` 这条边断开后函数值被释放，closure 不再持有
global，整个环境图随之回收。子环境（模块 / 调用 / 块）通过强 `parent` 链挂靠
root，随持有它们的值一起释放。未改用 `Weak<RefCell<Env>>`：那会改变逃逸闭包
语义——`makeAdder` 这类「函数内 `return fn` 捕获调用环境」的闭包在定义环境 drop
后 `upgrade` 会失败，与 JS 路径分叉（双路径 parity 测试锁定），故不采用。

## 5. 异步运行时内存

基于 **corosensei 栈式协程**：每个 `async` 调用 `spawn_async` 时：

- 分配一个 **1 MiB 协程栈**（`DefaultStack`），suspend 时完整保留调用帧；
- 创建一个共享 `Promise`（`Rc<RefCell<Promise>>`）与一个 `Task`；
- `Task` 存入 `tasks: Vec<Option<Task>>`，`task_yielder` 与之索引对齐
  （spawn 时同步 push，复用槽时置 `None`）。

调度：

- `tasks`：任务槽。完成（协程 `Return`）后置 `None`，id 回收进 `task_free`
  空闲列表，下一次 `spawn_async` 优先复用；最外层 resume 的 Return 还会
  `truncate` 尾部的连续 `None` 槽，因此长驻场景下槽位**有界**（P2 已修复，
  见 §7）；
- `ready: VecDeque<(usize, Control)>`：微任务 FIFO，`drive()` 循环弹头执行；
- `Promise { state, awaiters: VecDeque<usize> }`：`await` 未 settle 时把任务 id
  入队；settle 后取出所有 awaiter，按 FIFO 推入 `ready`；
- `await` 一个**已 settle** 的 Promise 不会立即继续，而是重新排队（对齐 JS
  微任务语义）；
- `CURRENT_INTERP: thread_local Cell<*const Interpreter>`：`'static` 协程闭包
  无法捕获 `&Interpreter`，借由该裸指针在 `resume` 期间访问（单线程，suspend
  期间指针始终有效；`resume_task` 内用 `debug_assert` 固化这一不变式，见 §7）；
- `call_depth: Cell<usize>` + `MAX_CALL_DEPTH = 128`：同步 / 异步递归统一计数，
  超限返回干净错误而非宿主栈溢出（每层 async 递归会新建一个 1 MiB 栈，需要上限
  兜底）。

```text
spawn_async(fn) ──> Promise (共享)  +  Task { coro: 1MiB 栈, promise }
                       │  awaiters: VecDeque<task_id>
tasks: Vec<Option<Task>>   task_free: Vec<usize>（可复用槽位）
ready: VecDeque<(id, Control)>
```

## 6. 相等性与别名语义

- `equal()`（`==`/`!=`）：`List`/`Object`/`Promise` 用 `Rc::ptr_eq` 判**身份**
  （对应 JS 引用 `===`）；`Enum` 按结构（enum_name + tag + payload 递归）比较；
  数字/字符串/布尔/`null` 按值。
- 列表/对象是共享句柄，赋值 `b = a; b[0] = 9` 会同时改变 `a`（JS 引用语义），
  与 JS 路径一致。

## 7. 已知内存关注点与处置计划

> 2026-08-18 对运行时内存模型逐条对照实现验证，优先级 P0–P4 如下。
> 「已修复」项均有回归测试锁定；「未修复」项多为受语言语义约束的优化，
> 见各项说明。

| 优先级 | 关注点 | 现状 |
|---|---|---|
| P0 | 无阻塞级问题 | — |
| P1 | Rc 环（D11）：顶层函数捕获自身定义环境 | **已修复**：`Interpreter::drop` 清扫 root 绑定破环；逃逸闭包语义保持强引用（未改用 Weak，见下） |
| P2 | `tasks` 槽位从不缩容 | **已修复**：空闲列表复用 + 尾部 `truncate`，槽位有界 |
| P3 | `for`/`while` 每轮新建 `Env` | 未修复：按轮捕获语义（JS `let` 对齐）已锁定，复用需先探测循环体无闭包 |
| P4 | 异步协程栈 1 MiB/个 | 未修复：eval 递归跑在协程栈内，需实测深递归后定值 |
| — | `String` 深拷贝 | 未计划：`Env::get` 与格式化输出可能复制字符串 |
| — | `CURRENT_INTERP` 裸指针不变式 | 结构上安全，已加 `debug_assert` 固化（未改架构） |

各项展开：

**P1 — 打破闭包环（已修复）**：`Interpreter::drop` 先 `Env::clear` 清空 global
绑定，切断 `env -> fn value -> closure -> env` 自环，环境图随 Interpreter 一并
回收（`tests/memory.rs` 取消 `#[ignore]` 锁定）。刻意没走
`FunctionValue.closure: Weak<RefCell<Env>>` 的全量弱化：逃逸闭包（如
`makeAdder` 返回捕获调用环境的匿名 `fn`，双路径 parity 锁定）需要强引用保住
定义环境，Weak 会让其 `upgrade` 失败而语义分叉。drop 清扫把「破环」限定在
Interpreter 生命周期边界，语义零变化，代价仅是 drop 时的一次根绑定清空。

**P2 — `tasks` 槽位复用（已修复）**：原 `tasks: Vec<Option<Task>>` 只 push，
完成的任务留 `None` 永久空洞，`task_yielder` 同步累积陈旧指针。修复为：
`spawn_async` 优先从 `task_free` 弹出复用槽位（同步把该槽 yielder 重置为
`None`），Return 时回收 id；最外层 resume 的 Return 修剪尾部连续 `None`。一个
易错点：**只能在最外层 resume 时修剪**——嵌套 resume 中所有正在运行的协程槽位
都是 `None`（被 `take`），此时修剪会截断活跃槽位。

**P3 — 逐轮 Env 复用（暂缓）**：`for`/`while` 每轮迭代分配一个 `Env`
（`exec_block:600` 也会再建一层）。**注意这是语义而不是缺陷**：循环体内定义的
`fn` 捕获**当轮**迭代环境（`interpreter.rs` 的 `exec_for` 每轮新建 + `register_fn`
闭包捕获），与 JS 路径的 `for (let …)` 一致——即「按轮捕获」，`funcs[0]/[1]/[2]()`
分别读到 0/1/2（`tests/loop_captures.rs` + CLI 双路径 parity 测试已锁定）。若复用
循环轮次 Env，所有闭包会共享同一个迭代环境（经典 `var i` 闭包陷阱）。可选的
安全优化：**仅当循环体不含任何 `fn`/`FnExpr` 时**复用 loop 子环境（块环境仍需
每轮新建），语义等价且省一半分配，收益有限，暂缓。

**P4 — 协程栈大小（暂缓）**：corosensei `DefaultStack::default()` 即
`1024 * 1024`（mmap + guard page），每次 `Coroutine::new` 分配全新栈；且栈内
跑着解释器 eval 递归，`MAX_CALL_DEPTH = 128` 时每层 Rust 栈消耗可观。调小到
512 KiB 等需先实测深 async 递归（现测试断言了 100 层 async 递归仍正确）。

**`String` 深拷贝**：`Env::get` 返回借用或 Cow 可减少复制，暂无计划。

**`CURRENT_INTERP` 不变式**：协程只在 `resume_task`（`self` 方法）内运行且由
`self.tasks` 持有，`drop(Interpreter)` 会连带销毁全部协程，悬挂不会发生——是
「脆弱而非 bug」，已用 `debug_assert` 固化。

生命周期上，`Interpreter::new` 创建 root 环境，模块/调用/块环境都是 root 的
后代；正常情况下随 `Interpreter` 一起被释放（除 D11 环）。

### 回归测试

- `crates/xulo-runtime/tests/memory.rs`：D11 破环后的预期（`Interpreter::drop`
  清空 root 绑定 → 弱引用 upgrade 为 `None`）+ 无环对照组（裸 `print` 本就
  无环）。
- `crates/xulo-runtime/tests/task_slots.rs`：P2 —— 大量顺序 / fire-and-forget
  async 调用后槽位数始终保持有界，且输出顺序正确。
- `crates/xulo-runtime/tests/loop_captures.rs` + CLI 双路径 parity：P3 语义锁定
  —— 循环体内 `fn` 按轮捕获（`funcs[0]/[1]/[2]()` = 0/1/2），JS 与原生（默认）
  两路径一致。