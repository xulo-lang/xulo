use xulo_core::ast::*;

#[test]
fn test_binary_operator_symbols() {
    assert_eq!(BinaryOperator::Add.symbol(), "+");
    assert_eq!(BinaryOperator::Sub.symbol(), "-");
    assert_eq!(BinaryOperator::Mul.symbol(), "*");
    assert_eq!(BinaryOperator::Div.symbol(), "/");
    assert_eq!(BinaryOperator::Eq.symbol(), "==");
    assert_eq!(BinaryOperator::Neq.symbol(), "!=");
    assert_eq!(BinaryOperator::Lt.symbol(), "<");
    assert_eq!(BinaryOperator::Gt.symbol(), ">");
    assert_eq!(BinaryOperator::Lte.symbol(), "<=");
    assert_eq!(BinaryOperator::Gte.symbol(), ">=");
    assert_eq!(BinaryOperator::And.symbol(), "and");
    assert_eq!(BinaryOperator::Or.symbol(), "or");
}

#[test]
fn test_binary_operator_equality() {
    assert_eq!(BinaryOperator::Add, BinaryOperator::Add);
    assert_ne!(BinaryOperator::Add, BinaryOperator::Sub);
    assert_ne!(BinaryOperator::Mul, BinaryOperator::Div);
    assert_ne!(BinaryOperator::Eq, BinaryOperator::Neq);
    assert_ne!(BinaryOperator::Lt, BinaryOperator::Gt);
}

#[test]
fn test_unary_operator_symbols() {
    assert_eq!(UnaryOperator::Not.symbol(), "!");
    assert_eq!(UnaryOperator::Neg.symbol(), "-");
}

#[test]
fn test_unary_operator_equality() {
    assert_eq!(UnaryOperator::Not, UnaryOperator::Not);
    assert_eq!(UnaryOperator::Neg, UnaryOperator::Neg);
    assert_ne!(UnaryOperator::Not, UnaryOperator::Neg);
}
