//! `xulo_detect_tag` must recognize every runtime registration path. The
//! implementation keeps O(1) membership sets next to the registration vectors,
//! so each kind is checked after being created through its public entry point.

use std::ffi::CString;

use xulo_runtime::runtime::{
    xulo_alloc_array, xulo_alloc_enum, xulo_alloc_object, xulo_cleanup, xulo_detect_tag,
    xulo_register_float, xulo_register_string, xulo_string_concat,
};

#[test]
fn detect_tag_recognizes_registered_values() {
    let left = CString::new("left").unwrap();
    let right = CString::new("right").unwrap();
    let literal = CString::new("literal").unwrap();

    let string = xulo_string_concat(left.as_ptr(), right.as_ptr());
    assert_eq!(xulo_detect_tag(string as i64), 0);

    xulo_register_string(literal.as_ptr());
    assert_eq!(xulo_detect_tag(literal.as_ptr() as i64), 0);

    let array = xulo_alloc_array(0);
    assert_eq!(xulo_detect_tag(array as i64), 3);

    let object = xulo_alloc_object(0);
    assert_eq!(xulo_detect_tag(object as i64), 4);

    let enum_value = xulo_alloc_enum(c"T".as_ptr(), c"V".as_ptr(), 0, 0);
    assert_eq!(xulo_detect_tag(enum_value as i64), 7);

    let float_bits = 1.5f64.to_bits() as i64;
    xulo_register_float(float_bits);
    assert_eq!(xulo_detect_tag(float_bits), 2);

    assert_eq!(xulo_detect_tag(42), 1);
}

#[test]
fn cleanup_clears_string_membership() {
    let left = CString::new("a").unwrap();
    let right = CString::new("b").unwrap();

    let before = xulo_string_concat(left.as_ptr(), right.as_ptr());
    assert_eq!(xulo_detect_tag(before as i64), 0);

    xulo_cleanup();

    let after = xulo_string_concat(left.as_ptr(), right.as_ptr());
    assert_eq!(xulo_detect_tag(after as i64), 0);
}
