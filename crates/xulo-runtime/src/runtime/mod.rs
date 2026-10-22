use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::value::format_number;

mod fs;
mod heap;
mod io;
mod regex;
mod scalar;
mod string;

// 再导出各子模块的 `xulo_*`：xulo-compiler/backend.rs 按 `xulo_runtime::runtime::xulo_*`
// 路径取函数地址，这里保持该路径不变（`no_mangle` 符号本身与模块布局无关）。
pub use fs::*;
pub use heap::*;
pub use io::*;
pub use regex::*;
pub use scalar::*;
pub use string::*;

/// xulo 对象
#[repr(C)]
pub struct XuloObject {
    pub fields: Vec<(String, i64)>,
    pub field_tags: Vec<i64>,
}

/// xulo 数组
#[repr(C)]
pub struct XuloArray {
    pub elements: Vec<i64>,
    pub tags: Vec<i64>,
}

/// xulo 枚举值：变体序号 + 供打印的名字 + payload 槽（值与类型标签）。
/// 名字指针指向编译产物里的静态字符串，生命周期与进程相同。
#[repr(C)]
pub struct XuloEnum {
    pub variant: i64,
    pub variant_name: *const c_char,
    pub type_name: *const c_char,
    pub elements: Vec<i64>,
    pub tags: Vec<i64>,
}

/// 闭包捕获单元：持有被装箱变量的原始位模式。
/// 标签不入 cell——读出时与未装箱路径一样按槽位/伴随标签推断。
#[repr(C)]
pub struct XuloCell {
    pub value: i64,
}

/// 闭包捕获环境：按自由变量的确定顺序容纳 cell 指针。
#[repr(C)]
pub struct XuloEnv {
    pub cells: Vec<i64>,
}

/// 闭包值：目标函数地址 + 捕获环境。函数地址是编译产物里闭包入口的
/// 实地址（JIT 下是 finalize 后的机器码指针，AOT 下经链接器解析）。
#[repr(C)]
pub struct XuloClosure {
    pub func: *const std::ffi::c_void,
    pub env: *mut XuloEnv,
}

thread_local! {
    static OBJECTS: RefCell<Vec<*mut XuloObject>> = const { RefCell::new(Vec::new()) };
    static ENUMS: RefCell<Vec<*mut XuloEnum>> = const { RefCell::new(Vec::new()) };
    static ARRAYS: RefCell<Vec<*mut XuloArray>> = const { RefCell::new(Vec::new()) };
    static STRINGS: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
    static FLOATS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    /// JIT 嵌入的字符串指针（仅记录地址，不拥有所有权）
    static JIT_STRINGS: RefCell<Vec<*const c_char>> = const { RefCell::new(Vec::new()) };
    /// 输出缓冲区（用于测试捕获 print 输出）
    static OUTPUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// 活跃的闭包捕获单元
    static CELLS: RefCell<Vec<*mut XuloCell>> = const { RefCell::new(Vec::new()) };
    /// 活跃的捕获环境
    static ENVS: RefCell<Vec<*mut XuloEnv>> = const { RefCell::new(Vec::new()) };
    /// 活跃的闭包值（清理所有者）
    static CLOSURES: RefCell<Vec<*mut XuloClosure>> = const { RefCell::new(Vec::new()) };
    /// 闭包指针集合：O(1) 判定一个 i64 是否是可调用闭包（detect/check）
    static CLOSURE_PTRS: RefCell<std::collections::HashSet<usize>> = RefCell::new(std::collections::HashSet::new());
    /// 活跃的编译正则（JIT/AOT 侧 RegExp 值的所有者）
    static REGEXPS: RefCell<Vec<*mut xulo_std::regex::RegExp>> = const { RefCell::new(Vec::new()) };
    /// 正则匹配对象的捕获组（match 对象本身是普通 XuloObject，
    /// `group(i)` 经此表取回组内容）
    static MATCH_GROUPS: RefCell<std::collections::HashMap<usize, Vec<Option<String>>>> =
        RefCell::new(std::collections::HashMap::new());
}

/// 取走输出缓冲区（测试用）
pub fn xulo_take_output() -> Vec<String> {
    OUTPUT.with(|out| out.borrow_mut().drain(..).collect())
}

/// 将一行文本推入输出缓冲区
fn push_output(line: String) {
    OUTPUT.with(|out| out.borrow_mut().push(line));
}

fn format_value(value: i64, tag: i64) -> String {
    match tag {
        0 => {
            let ptr = value as *const c_char;
            unsafe {
                if ptr.is_null() {
                    "null".to_string()
                } else {
                    match CStr::from_ptr(ptr).to_str() {
                        Ok(s) => s.to_string(),
                        Err(_) => "null".to_string(),
                    }
                }
            }
        }
        1 => format!("{}", value),
        2 => format_number(f64::from_bits(value as u64)),
        3 => {
            let arr = value as *const XuloArray;
            unsafe {
                if arr.is_null() {
                    "[]".to_string()
                } else {
                    let elements = &(*arr).elements;
                    let tags = &(*arr).tags;
                    let strs: Vec<String> = elements
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let t = if i < tags.len() { tags[i] } else { 1 };
                            format_value(*v, t)
                        })
                        .collect();
                    format!("[{}]", strs.join(", "))
                }
            }
        }
        4 => {
            let obj = value as *const XuloObject;
            unsafe {
                if obj.is_null() {
                    "{}".to_string()
                } else {
                    let fields = &(*obj).fields;
                    let field_tags = &(*obj).field_tags;
                    let strs: Vec<String> = fields
                        .iter()
                        .enumerate()
                        .map(|(i, (k, v))| {
                            let t = if i < field_tags.len() {
                                field_tags[i]
                            } else {
                                0
                            };
                            format!("{}: {}", k, format_value(*v, t))
                        })
                        .collect();
                    format!("{{ {} }}", strs.join(", "))
                }
            }
        }
        5 => format!("{}", value != 0),
        6 => "null".to_string(),
        8 => "<function>".to_string(),
        9 => "<regex>".to_string(),
        7 => {
            let e = value as *const XuloEnum;
            unsafe {
                if e.is_null() {
                    "null".to_string()
                } else {
                    let cstr = |p: *const c_char| {
                        if p.is_null() {
                            String::new()
                        } else {
                            CStr::from_ptr(p).to_str().unwrap_or("").to_string()
                        }
                    };
                    let ty = cstr((*e).type_name);
                    let vn = cstr((*e).variant_name);
                    let elements = &(*e).elements;
                    let tags = &(*e).tags;
                    if elements.is_empty() {
                        format!("{}.{}", ty, vn)
                    } else if elements.len() == 1 {
                        let t = if tags.is_empty() { 1 } else { tags[0] };
                        format!("{}.{}({})", ty, vn, format_value(elements[0], t))
                    } else {
                        let strs: Vec<String> = elements
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                let t = if i < tags.len() { tags[i] } else { 1 };
                                format_value(*v, t)
                            })
                            .collect();
                        format!("{}.{}([{}])", ty, vn, strs.join(", "))
                    }
                }
            }
        }
        _ => format!("{}", value),
    }
}

/// 把一个 Rust 字符串驻留为运行时字符串（返回可交给 JIT 的 C 指针）
fn intern_string(s: impl Into<String>) -> *const c_char {
    let c_str = CString::new(s.into()).unwrap();
    let ptr = c_str.as_ptr();
    STRINGS.with(|strings| {
        strings.borrow_mut().push(c_str);
    });
    ptr
}

/// 清理所有分配的内存
#[unsafe(no_mangle)]
pub extern "C" fn xulo_cleanup() {
    OBJECTS.with(|objects| {
        for ptr in objects.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });
    ARRAYS.with(|arrays| {
        for ptr in arrays.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });

    ENUMS.with(|enums| {
        for ptr in enums.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });

    CLOSURES.with(|closures| {
        for ptr in closures.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });
    CLOSURE_PTRS.with(|ps| ps.borrow_mut().clear());

    REGEXPS.with(|rs| {
        for ptr in rs.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });
    MATCH_GROUPS.with(|groups| groups.borrow_mut().clear());

    ENVS.with(|envs| {
        for ptr in envs.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });

    CELLS.with(|cells| {
        for ptr in cells.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });

    STRINGS.with(|strings| {
        strings.borrow_mut().clear();
    });

    // std::math::toFixed 交给编译产物的字符串由 xulo-std 持有，一并释放。
    xulo_std::ffi::release_strings();
}
