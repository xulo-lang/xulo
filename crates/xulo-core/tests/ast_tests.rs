use xulo_core::ast::*;

#[test]
fn test_program_new() {
    let program = Program { statements: vec![] };
    assert!(program.statements.is_empty());
}

#[test]
fn test_program_with_statements() {
    let program = Program {
        statements: vec![Statement::Return(ReturnStmt {
            value: Some(Expression::Literal {
                value: Literal::Number(42.0),
                span: 0..2,
            }),
            span: 0..2,
        })],
    };
    assert_eq!(program.statements.len(), 1);
}

#[test]
fn test_statement_return() {
    let stmt = Statement::Return(ReturnStmt {
        value: None,
        span: 0..7,
    });
    assert!(matches!(stmt, Statement::Return(_)));
}

#[test]
fn test_statement_expr() {
    let stmt = Statement::Expr(ExprStmt {
        expr: Expression::Literal {
            value: Literal::Null,
            span: 0..4,
        },
        has_semicolon: true,
        span: 0..5,
    });
    assert!(matches!(stmt, Statement::Expr(_)));
}

#[test]
fn test_statement_for() {
    let stmt = Statement::For(ForStmt {
        iter_var: "i".to_string(),
        iter_var_span: 4..5,
        iterable: Expression::Identifier {
            name: "xs".to_string(),
            span: 9..11,
        },
        body: Block {
            statements: vec![],
        },
    });
    assert!(matches!(stmt, Statement::For(_)));
}

#[test]
fn test_statement_while() {
    let stmt = Statement::While(WhileStmt {
        condition: Expression::Literal {
            value: Literal::Boolean(true),
            span: 6..10,
        },
        body: Block {
            statements: vec![],
        },
    });
    assert!(matches!(stmt, Statement::While(_)));
}

#[test]
fn test_statement_try_catch() {
    let stmt = Statement::Try(TryStmt {
        try_block: Block {
            statements: vec![],
        },
        catch_var: "e".to_string(),
        catch_var_span: 15..16,
        catch_block: Block {
            statements: vec![],
        },
    });
    assert!(matches!(stmt, Statement::Try(_)));
}
