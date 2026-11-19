//! 聚合常量的惰性槽。
//!
//! AOT 把列表/对象/struct/enum 等聚合 `const` 的初始化表达式物化到按程序
//! 编号的槽里：首次引用时求值一次，之后所有引用共享同一份堆值。入口函数
//! 先调用 [`xulo_const_reset`] 清空槽表，避免同一进程里重复执行入口
//! （JIT 测试）读到上一次程序的值。

use std::cell::RefCell;

thread_local! {
    static CONST_SLOTS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
}

/// 按 `count` 槽位重置槽表（入口第一条指令）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_const_reset(count: i64) {
    let len = usize::try_from(count).unwrap_or(0);
    CONST_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        slots.clear();
        slots.resize(len, 0);
    });
}

/// 读槽；未初始化、越界或未重置时返回 0
#[unsafe(no_mangle)]
pub extern "C" fn xulo_const_get(slot: i64) -> i64 {
    let Ok(index) = usize::try_from(slot) else {
        return 0;
    };
    CONST_SLOTS.with(|slots| slots.borrow().get(index).copied().unwrap_or(0))
}

/// 写入已物化的槽值
#[unsafe(no_mangle)]
pub extern "C" fn xulo_const_set(slot: i64, value: i64) {
    let Ok(index) = usize::try_from(slot) else {
        return;
    };
    CONST_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        if index >= slots.len() {
            slots.resize(index + 1, 0);
        }
        slots[index] = value;
    });
}
