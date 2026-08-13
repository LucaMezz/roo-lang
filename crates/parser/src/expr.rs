//! Expressions (`Expr`/`ExprKind`) — the Pratt-parsed operator precedence
//! setup and every `ExprKind` variant's parser live here.

use crate::*;
use ast::*;
use chumsky::pratt::*;
use lexer::Token;

/// `#[annotations]* ident (":" expr)?` — the no-`: expr` shorthand
/// (`Point { x }`, short for `Point { x: x }`) refers to a binding with
/// the same name as the field, so it desugars to a single-segment
/// `ExprKind::Path` here rather than needing its own `ExprField`
/// representation.
fn expr_field<'src>(expr: impl FigParser<'src, Expr> + 'src) -> impl FigParser<'src, ExprField> {
    annotations()
        .then(ident())
        .then(just(Token::Colon).ignore_then(expr).map(Box::new).or_not())
        .map_with(|((annotations, ident), value), e| {
            let value = value.unwrap_or_else(|| {
                Box::new(Expr {
                    kind: ExprKind::Path(
                        None,
                        Path {
                            segments: vec![PathSegment {
                                ident: ident.clone(),
                                args: None,
                            }],
                            span: ident.span,
                        },
                    ),
                    span: ident.span,
                    annotations: Vec::new(),
                })
            });
            ExprField {
                annotations,
                ident,
                expr: value,
                span: span(e),
            }
        })
}

fn struct_expr<'src>(expr: impl FigParser<'src, Expr> + 'src) -> impl FigParser<'src, StructExpr> {
    q_self()
        .map(Box::new)
        .or_not()
        .then(path(ty()))
        .then(
            expr_field(expr.clone())
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .then_ignore(just(Token::Comma).or_not())
                .then(just(Token::DotDot).ignore_then(expr).map(Box::new).or_not())
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map(|((qself, path), (fields, rest))| StructExpr {
            qself,
            path,
            fields,
            rest,
        })
}

pub fn method_call<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
) -> impl FigParser<'src, MethodCall> {
    expr.clone()
        .map(Box::new)
        .then_ignore(just(Token::Dot))
        .then(path_segment_turbofish(ty()))
        .then(
            expr.map(Box::new)
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .map_with(|((receiver, seg), args), e| MethodCall {
            receiver,
            seg,
            args,
            span: span(e),
        })
}

fn param<'src>(expr: impl FigParser<'src, Expr> + 'src) -> impl FigParser<'src, Param> {
    annotations()
        .then(pat_no_top_alt(expr).map(Box::new))
        .then(just(Token::Colon).ignore_then(ty()).map(Box::new).or_not())
        .map_with(|((annotations, pat), ty), e| Param {
            annotations,
            pat,
            ty,
            span: span(e),
        })
}

fn closure<'src>(expr: impl FigParser<'src, Expr> + 'src) -> impl FigParser<'src, Closure> {
    param(expr.clone())
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Pipe), just(Token::Pipe))
        .then(fn_ret_ty(ty()))
        .then(expr.map(Box::new))
        .map(|((inputs, output), body)| Closure {
            fn_decl: Box::new(FnDecl { inputs, output }),
            body,
        })
}

pub fn expr_array<'src>(expr: impl FigParser<'src, Expr> + 'src) -> impl FigParser<'src, ExprKind> {
    expr.map(Box::new)
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBracket), just(Token::RBracket))
        .map(ExprKind::Array)
}

fn expr_tuple_or_paren<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
) -> impl FigParser<'src, ExprKind> {
    expr.clone()
        .map(Box::new)
        .then_ignore(just(Token::Comma))
        .repeated()
        .collect::<Vec<_>>()
        .then(expr.map(Box::new).or_not())
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .map(|(mut elems, last)| match (elems.is_empty(), last) {
            (true, Some(only)) => ExprKind::Paren(only),
            (_, last) => {
                elems.extend(last);
                ExprKind::Tup(elems)
            }
        })
}

fn expr_path<'src>() -> impl FigParser<'src, ExprKind> {
    q_self()
        .map(Box::new)
        .or_not()
        .then(path_turbofish(ty()))
        .map(|(qself, path)| ExprKind::Path(qself, path))
}

fn expr_underscore<'src>() -> impl FigParser<'src, ExprKind> {
    just(Token::Identifier("_")).map(|_| ExprKind::Underscore)
}

fn range_limits<'src>() -> impl FigParser<'src, RangeLimits> {
    choice((
        just(Token::DotDotEq).map(|_| RangeLimits::Closed),
        just(Token::DotDot).map(|_| RangeLimits::HalfOpen),
    ))
}

fn expr_range_no_start<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
) -> impl FigParser<'src, ExprKind> {
    range_limits()
        .then(expr.or_not())
        .map(|(limits, end)| ExprKind::Range(None, end.map(Box::new), limits))
}

fn expr_break<'src>(expr: impl FigParser<'src, Expr> + 'src) -> impl FigParser<'src, ExprKind> {
    just(Token::Break)
        .ignore_then(ident().map(|ident| Label { ident }).or_not())
        .then(expr.map(Box::new).or_not())
        .map(|(label, value)| ExprKind::Break(label, value))
}

fn expr_continue<'src>() -> impl FigParser<'src, ExprKind> {
    just(Token::Continue)
        .ignore_then(ident().map(|ident| Label { ident }).or_not())
        .map(ExprKind::Continue)
}

fn expr_ret<'src>(expr: impl FigParser<'src, Expr> + 'src) -> impl FigParser<'src, ExprKind> {
    just(Token::Return)
        .ignore_then(expr.map(Box::new).or_not())
        .map(ExprKind::Ret)
}

fn condition<'src>(
    pat: impl FigParser<'src, Pat> + 'src,
    cond_expr: impl FigParser<'src, Expr> + 'src,
) -> impl FigParser<'src, Expr> {
    let let_condition = just(Token::Let)
        .ignore_then(pat)
        .then_ignore(just(Token::Eq))
        .then(cond_expr.clone())
        .map_with(|(pat, value), e| Expr {
            kind: ExprKind::Let(Box::new(pat), Box::new(value), span(e)),
            span: span(e),
            annotations: Vec::new(),
        });

    choice((let_condition, cond_expr))
}

fn expr_if<'src>(
    pat: impl FigParser<'src, Pat> + 'src,
    cond_expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, ExprKind> {
    recursive(move |if_expr| {
        just(Token::If)
            .ignore_then(condition(pat.clone(), cond_expr.clone()))
            .then(block.clone())
            .then(
                just(Token::Else)
                    .ignore_then(choice((
                        if_expr.map_with(|kind, e| Expr {
                            kind,
                            span: span(e),
                            annotations: Vec::new(),
                        }),
                        block.clone().map_with(|blk, e| Expr {
                            kind: ExprKind::Block(Box::new(blk), None),
                            span: span(e),
                            annotations: Vec::new(),
                        }),
                    )))
                    .map(Box::new)
                    .or_not(),
            )
            .map(|((cond, then_block), else_branch)| {
                ExprKind::If(Box::new(cond), Box::new(then_block), else_branch)
            })
    })
}

fn expr_while<'src>(
    pat: impl FigParser<'src, Pat> + 'src,
    cond_expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, ExprKind> {
    label()
        .or_not()
        .then_ignore(just(Token::While))
        .then(condition(pat, cond_expr))
        .then(block)
        .map(|((label, cond), body)| ExprKind::While(Box::new(cond), Box::new(body), label))
}

fn expr_for<'src>(
    pat: impl FigParser<'src, Pat> + 'src,
    cond_expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, ExprKind> {
    label()
        .or_not()
        .then_ignore(just(Token::For))
        .then(pat.map(Box::new))
        .then_ignore(just(Token::In))
        .then(cond_expr.map(Box::new))
        .then(block.map(Box::new))
        .map(|(((label, pat), iter), body)| ExprKind::ForLoop {
            pat,
            iter,
            body,
            label,
        })
}

fn expr_loop<'src>(block: impl FigParser<'src, Block> + 'src) -> impl FigParser<'src, ExprKind> {
    label()
        .or_not()
        .then_ignore(just(Token::Loop))
        .then(block.map(Box::new))
        .map_with(|(label, body), e| ExprKind::Loop(body, label, span(e)))
}

fn expr_block<'src>(block: impl FigParser<'src, Block> + 'src) -> impl FigParser<'src, ExprKind> {
    label()
        .or_not()
        .then(block.map(Box::new))
        .map(|(label, blk)| ExprKind::Block(blk, label))
}

fn expr_match<'src>(
    pat: impl FigParser<'src, Pat> + 'src,
    cond_expr: impl FigParser<'src, Expr> + 'src,
    expr: impl FigParser<'src, Expr> + 'src,
) -> impl FigParser<'src, ExprKind> {
    just(Token::Match)
        .ignore_then(cond_expr.map(Box::new))
        .then(
            arm(pat, expr)
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map(|(scrutinee, arms)| ExprKind::Match(scrutinee, arms))
}

fn primary_expr<'src>(
    allow_struct_lit: bool,
    pat: impl FigParser<'src, Pat> + 'src,
    expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, ExprKind> {
    let cond_expr = if allow_struct_lit {
        expr_no_struct_lit().boxed()
    } else {
        expr.clone().boxed()
    };

    choice((
        literal().map(ExprKind::Lit),
        expr_array(expr.clone()),
        expr_tuple_or_paren(expr.clone()),
        struct_expr(expr.clone())
            .map(Box::new)
            .map(ExprKind::Struct)
            .filter(move |_| allow_struct_lit),
        closure(expr.clone()).map(Box::new).map(ExprKind::Closure),
        expr_if(pat.clone(), cond_expr.clone(), block.clone()),
        expr_while(pat.clone(), cond_expr.clone(), block.clone()),
        expr_for(pat.clone(), cond_expr.clone(), block.clone()),
        expr_loop(block.clone()),
        expr_match(pat.clone(), cond_expr, expr.clone()),
        expr_block(block),
        expr_break(expr.clone()),
        expr_continue(),
        expr_ret(expr.clone()),
        expr_range_no_start(expr.clone()),
        expr_underscore(),
        expr_path(),
    ))
}

fn bin_op_at<'src>(prec: ExprPrecedence) -> impl FigParser<'src, BinOp> {
    bin_op().filter(move |op| op.kind.precedence() == prec)
}

fn fixity_assoc(fixity: Fixity, prec: ExprPrecedence) -> Associativity {
    match fixity {
        Fixity::Left => left(prec as u16),
        Fixity::Right => right(prec as u16),
        Fixity::None => none(prec as u16),
    }
}

fn bin_op_fold<'src>(lhs: Expr, op: BinOp, rhs: Expr, e: &mut Extra<'src, '_>) -> Expr {
    Expr {
        kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)),
        span: span(e),
        annotations: Vec::new(),
    }
}

fn expr_pratt<'src>(
    atom: impl FigParser<'src, Expr> + 'src,
    expr: impl FigParser<'src, Expr> + 'src,
) -> impl FigParser<'src, Expr> {
    atom.pratt((
        prefix(
            ExprPrecedence::Prefix as u16,
            un_op(),
            |op, rhs: Expr, e| Expr {
                kind: ExprKind::Unary(op, Box::new(rhs)),
                span: span(e),
                annotations: Vec::new(),
            },
        ),
        infix(
            fixity_assoc(Fixity::Right, ExprPrecedence::Assign),
            just(Token::Eq),
            |lhs: Expr, _, rhs: Expr, e| Expr {
                kind: ExprKind::Assign(Box::new(lhs), Box::new(rhs), span(e)),
                span: span(e),
                annotations: Vec::new(),
            },
        ),
        infix(
            fixity_assoc(Fixity::Right, ExprPrecedence::Assign),
            assign_op(),
            |lhs: Expr, op, rhs: Expr, e| Expr {
                kind: ExprKind::AssignOp(op, Box::new(lhs), Box::new(rhs)),
                span: span(e),
                annotations: Vec::new(),
            },
        ),
        infix(
            fixity_assoc(Fixity::Left, ExprPrecedence::Product),
            bin_op_at(ExprPrecedence::Product),
            bin_op_fold,
        ),
        infix(
            fixity_assoc(Fixity::Left, ExprPrecedence::Sum),
            bin_op_at(ExprPrecedence::Sum),
            bin_op_fold,
        ),
        infix(
            fixity_assoc(Fixity::Left, ExprPrecedence::Shift),
            bin_op_at(ExprPrecedence::Shift),
            bin_op_fold,
        ),
        infix(
            fixity_assoc(Fixity::Left, ExprPrecedence::BitAnd),
            bin_op_at(ExprPrecedence::BitAnd),
            bin_op_fold,
        ),
        infix(
            fixity_assoc(Fixity::Left, ExprPrecedence::BitXor),
            bin_op_at(ExprPrecedence::BitXor),
            bin_op_fold,
        ),
        infix(
            fixity_assoc(Fixity::Left, ExprPrecedence::BitOr),
            bin_op_at(ExprPrecedence::BitOr),
            bin_op_fold,
        ),
        infix(
            fixity_assoc(Fixity::None, ExprPrecedence::Compare),
            bin_op_at(ExprPrecedence::Compare),
            bin_op_fold,
        ),
        infix(
            fixity_assoc(Fixity::Left, ExprPrecedence::LAnd),
            bin_op_at(ExprPrecedence::LAnd),
            bin_op_fold,
        ),
        infix(
            fixity_assoc(Fixity::Left, ExprPrecedence::LOr),
            bin_op_at(ExprPrecedence::LOr),
            bin_op_fold,
        ),
        postfix(
            ExprPrecedence::Unambiguous as u16 + 1,
            just(Token::Dot)
                .ignore_then(path_segment_turbofish(ty()))
                .then(
                    expr.clone()
                        .map(Box::new)
                        .separated_by(just(Token::Comma))
                        .collect::<Vec<_>>()
                        .delimited_by(just(Token::LParen), just(Token::RParen)),
                ),
            |receiver: Expr, (seg, args), e| Expr {
                kind: ExprKind::MethodCall(Box::new(MethodCall {
                    receiver: Box::new(receiver),
                    seg,
                    args,
                    span: span(e),
                })),
                span: span(e),
                annotations: Vec::new(),
            },
        ),
        postfix(
            ExprPrecedence::Unambiguous as u16 + 1,
            expr.clone()
                .map(Box::new)
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LParen), just(Token::RParen)),
            |callee: Expr, args, e| Expr {
                kind: ExprKind::Call(Box::new(callee), args),
                span: span(e),
                annotations: Vec::new(),
            },
        ),
        postfix(
            ExprPrecedence::Unambiguous as u16 + 1,
            just(Token::Dot).ignore_then(ident()),
            |receiver: Expr, field, e| Expr {
                kind: ExprKind::Field(Box::new(receiver), field),
                span: span(e),
                annotations: Vec::new(),
            },
        ),
        postfix(
            ExprPrecedence::Unambiguous as u16 + 1,
            expr.clone()
                .map(Box::new)
                .delimited_by(just(Token::LBracket), just(Token::RBracket)),
            |receiver: Expr, index, e| Expr {
                kind: ExprKind::Index(Box::new(receiver), index, span(e)),
                span: span(e),
                annotations: Vec::new(),
            },
        ),
        postfix(
            ExprPrecedence::Unambiguous as u16,
            just(Token::Question),
            |receiver: Expr, _, e| Expr {
                kind: ExprKind::Try(Box::new(receiver)),
                span: span(e),
                annotations: Vec::new(),
            },
        ),
        postfix(
            ExprPrecedence::Range as u16,
            range_limits().then(expr.clone().or_not()),
            |lhs: Expr, (limits, end): (RangeLimits, Option<Expr>), e| Expr {
                kind: ExprKind::Range(Some(Box::new(lhs)), end.map(Box::new), limits),
                span: span(e),
                annotations: Vec::new(),
            },
        ),
    ))
}

fn expr_no_struct_lit<'src>() -> impl FigParser<'src, Expr> {
    recursive(|expr| {
        let block = block(expr.clone());
        let pat = pat(expr.clone());

        let atom = annotations()
            .then(primary_expr(false, pat, expr.clone(), block))
            .map_with(|(annotations, kind), e| Expr {
                annotations,
                kind,
                span: span(e),
            });

        expr_pratt(atom, expr)
    })
}

pub fn expr<'src>() -> impl FigParser<'src, Expr> {
    recursive(|expr| {
        let block = block(expr.clone());
        let pat = pat(expr.clone());

        let atom = annotations()
            .then(primary_expr(true, pat, expr.clone(), block))
            .map_with(|(annotations, kind), e| Expr {
                annotations,
                kind,
                span: span(e),
            });

        expr_pratt(atom, expr)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

    #[test]
    fn parses_a_bare_literal_expr() {
        let tokens = tokens("5");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        assert!(matches!(parsed.kind, ExprKind::Lit(_)));
    }

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        // 1 + 2 * 3 must be 1 + (2 * 3), not (1 + 2) * 3.
        let tokens = tokens("1 + 2 * 3");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Binary(op, lhs, rhs) = parsed.kind else {
            panic!("expected ExprKind::Binary, got {:?}", parsed.kind);
        };
        assert_eq!(op.kind, BinOpKind::Add);
        assert!(matches!(lhs.kind, ExprKind::Lit(_)));
        let ExprKind::Binary(inner_op, ..) = rhs.kind else {
            panic!(
                "expected the right side to be a nested Binary, got {:?}",
                rhs.kind
            );
        };
        assert_eq!(inner_op.kind, BinOpKind::Mul);
    }

    #[test]
    fn multiplication_still_binds_tighter_when_written_first() {
        // 1 * 2 + 3 must be (1 * 2) + 3, not 1 * (2 + 3).
        let tokens = tokens("1 * 2 + 3");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Binary(op, lhs, rhs) = parsed.kind else {
            panic!("expected ExprKind::Binary, got {:?}", parsed.kind);
        };
        assert_eq!(op.kind, BinOpKind::Add);
        assert!(matches!(rhs.kind, ExprKind::Lit(_)));
        let ExprKind::Binary(inner_op, ..) = lhs.kind else {
            panic!(
                "expected the left side to be a nested Binary, got {:?}",
                lhs.kind
            );
        };
        assert_eq!(inner_op.kind, BinOpKind::Mul);
    }

    #[test]
    fn subtraction_is_left_associative() {
        // 1 - 2 - 3 must be (1 - 2) - 3, not 1 - (2 - 3).
        let tokens = tokens("1 - 2 - 3");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Binary(op, lhs, rhs) = parsed.kind else {
            panic!("expected ExprKind::Binary, got {:?}", parsed.kind);
        };
        assert_eq!(op.kind, BinOpKind::Sub);
        assert!(matches!(rhs.kind, ExprKind::Lit(_)));
        assert!(matches!(lhs.kind, ExprKind::Binary(..)));
    }

    #[test]
    fn parses_a_unary_expression_combined_with_binary() {
        let tokens = tokens("-1 + 2");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Binary(op, lhs, rhs) = parsed.kind else {
            panic!("expected ExprKind::Binary, got {:?}", parsed.kind);
        };
        assert_eq!(op.kind, BinOpKind::Add);
        assert!(matches!(rhs.kind, ExprKind::Lit(_)));
        let ExprKind::Unary(un, _) = lhs.kind else {
            panic!("expected the left side to be Unary, got {:?}", lhs.kind);
        };
        assert_eq!(un, UnOp::Neg);
    }

    #[test]
    fn rejects_chained_comparisons() {
        // Comparisons are Fixity::None (non-associative) — `1 < 2 < 3`
        // isn't a single valid expression.
        let tokens = tokens("1 < 2 < 3");
        assert!(expr().parse(&tokens).into_result().is_err());
    }

    #[test]
    fn parses_an_array_of_expressions_with_operators() {
        let tokens = tokens("[1 + 2, 3]");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Array(elems) = parsed.kind else {
            panic!("expected ExprKind::Array, got {:?}", parsed.kind);
        };
        assert_eq!(elems.len(), 2);
        assert!(matches!(elems[0].kind, ExprKind::Binary(..)));
        assert!(matches!(elems[1].kind, ExprKind::Lit(_)));
    }

    #[test]
    fn parses_a_method_call_postfix_on_a_literal_atom() {
        let tokens = tokens("5.foo()");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::MethodCall(call) = parsed.kind else {
            panic!("expected ExprKind::MethodCall, got {:?}", parsed.kind);
        };
        assert_eq!(call.seg.ident.name, "foo");
        assert!(call.args.is_empty());
        assert!(matches!(call.receiver.kind, ExprKind::Lit(_)));
    }

    #[test]
    fn parses_chained_method_calls() {
        // Proves the postfix Pratt operator actually loops: each `.foo()`
        // becomes the receiver for the next one, left to right.
        let tokens = tokens("5.foo().bar()");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::MethodCall(outer) = parsed.kind else {
            panic!("expected ExprKind::MethodCall, got {:?}", parsed.kind);
        };
        assert_eq!(outer.seg.ident.name, "bar");
        let ExprKind::MethodCall(inner) = &outer.receiver.kind else {
            panic!(
                "expected the receiver to be a MethodCall, got {:?}",
                outer.receiver.kind
            );
        };
        assert_eq!(inner.seg.ident.name, "foo");
        assert!(matches!(inner.receiver.kind, ExprKind::Lit(_)));
    }

    #[test]
    fn method_calls_bind_tighter_than_binary_operators() {
        // 1 + 2.foo() must be 1 + (2.foo()), not (1 + 2).foo().
        let tokens = tokens("1 + 2.foo()");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Binary(op, lhs, rhs) = parsed.kind else {
            panic!("expected ExprKind::Binary, got {:?}", parsed.kind);
        };
        assert_eq!(op.kind, BinOpKind::Add);
        assert!(matches!(lhs.kind, ExprKind::Lit(_)));
        assert!(matches!(rhs.kind, ExprKind::MethodCall(_)));
    }

    #[test]
    fn parses_a_turbofish_method_call() {
        let tokens = tokens("5.foo::<int>()");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::MethodCall(call) = parsed.kind else {
            panic!("expected ExprKind::MethodCall, got {:?}", parsed.kind);
        };
        assert!(call.seg.args.is_some());
    }

    #[test]
    fn parses_a_parenthesized_expr_as_paren_not_tuple() {
        let tokens = tokens("(1 + 2)");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Paren(inner) = parsed.kind else {
            panic!("expected ExprKind::Paren, got {:?}", parsed.kind);
        };
        assert!(matches!(inner.kind, ExprKind::Binary(..)));
    }

    #[test]
    fn parses_a_single_trailing_comma_expr_as_a_one_tuple() {
        let tokens = tokens("(1,)");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Tup(elems) = parsed.kind else {
            panic!("expected ExprKind::Tup, got {:?}", parsed.kind);
        };
        assert_eq!(elems.len(), 1);
    }

    #[test]
    fn parses_unit_as_an_empty_tuple() {
        let tokens = tokens("()");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Tup(elems) = parsed.kind else {
            panic!("expected ExprKind::Tup, got {:?}", parsed.kind);
        };
        assert!(elems.is_empty());
    }

    #[test]
    fn parses_a_multi_element_tuple() {
        let tokens = tokens("(1, 2, 3)");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Tup(elems) = parsed.kind else {
            panic!("expected ExprKind::Tup, got {:?}", parsed.kind);
        };
        assert_eq!(elems.len(), 3);
    }

    #[test]
    fn parses_a_multi_segment_path_expr() {
        let tokens = tokens("Foo::Bar");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Path(qself, path) = parsed.kind else {
            panic!("expected ExprKind::Path, got {:?}", parsed.kind);
        };
        assert!(qself.is_none());
        assert_eq!(path.segments.len(), 2);
    }

    #[test]
    fn parses_a_bare_ident_as_a_single_segment_path() {
        let tokens = tokens("foo");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Path(_, path) = parsed.kind else {
            panic!("expected ExprKind::Path, got {:?}", parsed.kind);
        };
        assert_eq!(path.segments.len(), 1);
        assert_eq!(path.segments[0].ident.name, "foo");
    }

    #[test]
    fn parses_an_underscore_expr() {
        let tokens = tokens("_");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        assert!(matches!(parsed.kind, ExprKind::Underscore));
    }

    #[test]
    fn parses_a_no_start_half_open_range() {
        let tokens = tokens("..5");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Range(start, end, limits) = parsed.kind else {
            panic!("expected ExprKind::Range, got {:?}", parsed.kind);
        };
        assert!(start.is_none());
        assert!(end.is_some());
        assert!(matches!(limits, RangeLimits::HalfOpen));
    }

    #[test]
    fn parses_a_bare_no_start_no_end_range() {
        let tokens = tokens("..");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Range(start, end, _) = parsed.kind else {
            panic!("expected ExprKind::Range, got {:?}", parsed.kind);
        };
        assert!(start.is_none());
        assert!(end.is_none());
    }

    #[test]
    fn parses_a_with_start_half_open_range() {
        let tokens = tokens("0..5");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Range(start, end, limits) = parsed.kind else {
            panic!("expected ExprKind::Range, got {:?}", parsed.kind);
        };
        assert!(start.is_some());
        assert!(end.is_some());
        assert!(matches!(limits, RangeLimits::HalfOpen));
    }

    #[test]
    fn parses_a_with_start_closed_range() {
        let tokens = tokens("0..=5");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Range(start, end, limits) = parsed.kind else {
            panic!("expected ExprKind::Range, got {:?}", parsed.kind);
        };
        assert!(start.is_some());
        assert!(end.is_some());
        assert!(matches!(limits, RangeLimits::Closed));
    }

    #[test]
    fn parses_a_with_start_range_with_no_end() {
        let tokens = tokens("0..");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Range(start, end, _) = parsed.kind else {
            panic!("expected ExprKind::Range, got {:?}", parsed.kind);
        };
        assert!(start.is_some());
        assert!(end.is_none());
    }

    #[test]
    fn parses_a_bare_break() {
        let tokens = tokens("break");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Break(label, value) = parsed.kind else {
            panic!("expected ExprKind::Break, got {:?}", parsed.kind);
        };
        assert!(label.is_none());
        assert!(value.is_none());
    }

    #[test]
    fn parses_a_break_with_a_labeled_target_and_a_value() {
        let tokens = tokens("break outer 5");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Break(label, value) = parsed.kind else {
            panic!("expected ExprKind::Break, got {:?}", parsed.kind);
        };
        assert_eq!(label.expect("should have a label").ident.name, "outer");
        assert!(matches!(
            value.expect("should have a value").kind,
            ExprKind::Lit(_)
        ));
    }

    #[test]
    fn parses_a_continue_with_a_label() {
        let tokens = tokens("continue outer");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Continue(label) = parsed.kind else {
            panic!("expected ExprKind::Continue, got {:?}", parsed.kind);
        };
        assert_eq!(label.expect("should have a label").ident.name, "outer");
    }

    #[test]
    fn parses_a_bare_return() {
        let tokens = tokens("return");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Ret(value) = parsed.kind else {
            panic!("expected ExprKind::Ret, got {:?}", parsed.kind);
        };
        assert!(value.is_none());
    }

    #[test]
    fn parses_a_return_with_a_value() {
        let tokens = tokens("return 5");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Ret(value) = parsed.kind else {
            panic!("expected ExprKind::Ret, got {:?}", parsed.kind);
        };
        assert!(value.is_some());
    }

    #[test]
    fn parses_an_if_else_chain() {
        let tokens = tokens("if a { 1 } else if b { 2 } else { 3 }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::If(cond, then_block, else_branch) = parsed.kind else {
            panic!("expected ExprKind::If, got {:?}", parsed.kind);
        };
        assert!(matches!(cond.kind, ExprKind::Path(..)));
        assert_eq!(then_block.stmts.len(), 1);
        let else_branch = else_branch.expect("should have an else branch");
        let ExprKind::If(_, _, inner_else) = &else_branch.kind else {
            panic!(
                "expected the else branch to itself be an If, got {:?}",
                else_branch.kind
            );
        };
        assert!(inner_else.is_some());
    }

    #[test]
    fn parses_an_if_with_no_else() {
        let tokens = tokens("if a { 1 }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::If(_, _, else_branch) = parsed.kind else {
            panic!("expected ExprKind::If, got {:?}", parsed.kind);
        };
        assert!(else_branch.is_none());
    }

    #[test]
    fn parses_an_if_let_expression() {
        let tokens = tokens("if let Some(x) = maybe { x } else { 0 }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::If(cond, ..) = parsed.kind else {
            panic!("expected ExprKind::If, got {:?}", parsed.kind);
        };
        assert!(matches!(cond.kind, ExprKind::Let(..)));
    }

    #[test]
    fn parses_a_while_loop() {
        let tokens = tokens("while running { go(); }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::While(cond, body, label) = parsed.kind else {
            panic!("expected ExprKind::While, got {:?}", parsed.kind);
        };
        assert!(matches!(cond.kind, ExprKind::Path(..)));
        assert_eq!(body.stmts.len(), 1);
        assert!(label.is_none());
    }

    #[test]
    fn parses_a_labeled_while_let_loop() {
        let tokens = tokens("outer: while let Some(x) = it { }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::While(cond, _, label) = parsed.kind else {
            panic!("expected ExprKind::While, got {:?}", parsed.kind);
        };
        assert!(matches!(cond.kind, ExprKind::Let(..)));
        assert_eq!(label.expect("should have a label").ident.name, "outer");
    }

    #[test]
    fn parses_a_for_loop() {
        let tokens = tokens("for item in items { }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::ForLoop {
            pat,
            iter,
            body,
            label,
        } = parsed.kind
        else {
            panic!("expected ExprKind::ForLoop, got {:?}", parsed.kind);
        };
        assert!(matches!(pat.kind, PatKind::Ident(..)));
        assert!(matches!(iter.kind, ExprKind::Path(..)));
        assert!(body.stmts.is_empty());
        assert!(label.is_none());
    }

    #[test]
    fn parses_a_labeled_loop() {
        let tokens = tokens("outer: loop { break; }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Loop(body, label, _) = parsed.kind else {
            panic!("expected ExprKind::Loop, got {:?}", parsed.kind);
        };
        assert_eq!(body.stmts.len(), 1);
        assert_eq!(label.expect("should have a label").ident.name, "outer");
    }

    #[test]
    fn parses_a_bare_block_as_an_expression() {
        let tokens = tokens("{ 5 }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Block(block, label) = parsed.kind else {
            panic!("expected ExprKind::Block, got {:?}", parsed.kind);
        };
        assert_eq!(block.stmts.len(), 1);
        assert!(label.is_none());
    }

    #[test]
    fn parses_a_match_with_multiple_arms_and_a_guard() {
        let tokens = tokens("match n { x if x < 0 => 1, _ => 2, }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Match(scrutinee, arms) = parsed.kind else {
            panic!("expected ExprKind::Match, got {:?}", parsed.kind);
        };
        assert!(matches!(scrutinee.kind, ExprKind::Path(..)));
        assert_eq!(arms.len(), 2);
        assert!(arms[0].guard.is_some());
        assert!(arms[1].guard.is_none());
        assert!(matches!(arms[1].pat.kind, PatKind::Wild));
    }

    #[test]
    fn parses_a_struct_literal_expr_with_shorthand_and_explicit_fields() {
        let tokens = tokens("Point { x: 1, y }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Struct(s) = parsed.kind else {
            panic!("expected ExprKind::Struct, got {:?}", parsed.kind);
        };
        assert_eq!(s.path.segments[0].ident.name, "Point");
        assert_eq!(s.fields.len(), 2);
        assert_eq!(s.fields[0].ident.name, "x");
        assert_eq!(s.fields[1].ident.name, "y");
        assert!(matches!(s.fields[1].expr.kind, ExprKind::Path(..)));
        assert!(s.rest.is_none());
    }

    #[test]
    fn parses_a_struct_literal_with_a_base_initializer() {
        let tokens = tokens("Point { x: 1, ..other }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Struct(s) = parsed.kind else {
            panic!("expected ExprKind::Struct, got {:?}", parsed.kind);
        };
        assert_eq!(s.fields.len(), 1);
        assert!(s.rest.is_some());
    }

    #[test]
    fn parses_an_untyped_closure() {
        let tokens = tokens("|a, b| a + b");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Closure(closure) = parsed.kind else {
            panic!("expected ExprKind::Closure, got {:?}", parsed.kind);
        };
        assert_eq!(closure.fn_decl.inputs.len(), 2);
        assert!(closure.fn_decl.inputs[0].ty.is_none());
        assert!(matches!(closure.body.kind, ExprKind::Binary(..)));
    }

    #[test]
    fn parses_a_typed_closure_with_a_return_type_and_block_body() {
        let tokens = tokens("|a: int, b: int| -> int { a + b }");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Closure(closure) = parsed.kind else {
            panic!("expected ExprKind::Closure, got {:?}", parsed.kind);
        };
        assert!(closure.fn_decl.inputs[0].ty.is_some());
        assert!(matches!(closure.fn_decl.output, FnRetTy::Ty(_)));
        assert!(matches!(closure.body.kind, ExprKind::Block(..)));
    }

    #[test]
    fn parses_a_call_expr() {
        let tokens = tokens("foo(1, 2)");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Call(callee, args) = parsed.kind else {
            panic!("expected ExprKind::Call, got {:?}", parsed.kind);
        };
        assert!(matches!(callee.kind, ExprKind::Path(..)));
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn parses_a_field_access() {
        let tokens = tokens("foo.bar");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Field(receiver, field) = parsed.kind else {
            panic!("expected ExprKind::Field, got {:?}", parsed.kind);
        };
        assert!(matches!(receiver.kind, ExprKind::Path(..)));
        assert_eq!(field.name, "bar");
    }

    #[test]
    fn a_call_immediately_after_a_field_access_is_still_field_then_call() {
        // `(x.foo)(a)` isn't valid fig syntax for calling a field's value
        // directly (no parens around `x.foo` here) — but a field access
        // NOT immediately followed by an ident-then-paren pair (i.e. the
        // callee itself, not a further `.method()`) should still compose
        // as Field then Call rather than erroring.
        let tokens = tokens("foo.bar(1)");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        // `.bar(1)` right after a field/path must be read as a method
        // call, never as Field(foo, bar) followed by a bare Call.
        assert!(matches!(parsed.kind, ExprKind::MethodCall(_)));
    }

    #[test]
    fn parses_an_index_expr() {
        let tokens = tokens("foo[0]");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Index(receiver, index, _) = parsed.kind else {
            panic!("expected ExprKind::Index, got {:?}", parsed.kind);
        };
        assert!(matches!(receiver.kind, ExprKind::Path(..)));
        assert!(matches!(index.kind, ExprKind::Lit(_)));
    }

    #[test]
    fn parses_a_try_expr() {
        let tokens = tokens("foo()?");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Try(inner) = parsed.kind else {
            panic!("expected ExprKind::Try, got {:?}", parsed.kind);
        };
        assert!(matches!(inner.kind, ExprKind::Call(..)));
    }

    #[test]
    fn parses_chained_call_field_index_and_try() {
        let tokens = tokens("world.entities[0].get_component(kind)?.value");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Field(receiver, field) = parsed.kind else {
            panic!(
                "expected the outermost node to be ExprKind::Field, got {:?}",
                parsed.kind
            );
        };
        assert_eq!(field.name, "value");
        assert!(matches!(receiver.kind, ExprKind::Try(_)));
    }

    #[test]
    fn parses_a_plain_assignment() {
        let tokens = tokens("x = 5");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Assign(lhs, rhs, _) = parsed.kind else {
            panic!("expected ExprKind::Assign, got {:?}", parsed.kind);
        };
        assert!(matches!(lhs.kind, ExprKind::Path(..)));
        assert!(matches!(rhs.kind, ExprKind::Lit(_)));
    }

    #[test]
    fn parses_a_compound_assignment() {
        let tokens = tokens("x += 1");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::AssignOp(op, ..) = parsed.kind else {
            panic!("expected ExprKind::AssignOp, got {:?}", parsed.kind);
        };
        assert_eq!(op.kind, AssignOpKind::AddAssign);
    }

    #[test]
    fn assignment_is_right_associative() {
        // x = y = 5 must be x = (y = 5), not (x = y) = 5.
        let tokens = tokens("x = y = 5");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Assign(lhs, rhs, _) = parsed.kind else {
            panic!("expected ExprKind::Assign, got {:?}", parsed.kind);
        };
        assert!(matches!(lhs.kind, ExprKind::Path(..)));
        assert!(matches!(rhs.kind, ExprKind::Assign(..)));
    }

    #[test]
    fn assignment_binds_looser_than_binary_operators() {
        // x = 1 + 2 must be x = (1 + 2), not (x = 1) + 2.
        let tokens = tokens("x = 1 + 2");
        let parsed = expr().parse(&tokens).into_result().expect("should parse");
        let ExprKind::Assign(_, rhs, _) = parsed.kind else {
            panic!("expected ExprKind::Assign, got {:?}", parsed.kind);
        };
        assert!(matches!(rhs.kind, ExprKind::Binary(..)));
    }
}
