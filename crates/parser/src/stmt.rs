use crate::*;
use ast::*;
use lexer::Token;

fn local<'src>(
    expr: impl RooParser<'src, Expr> + 'src,
    block: impl RooParser<'src, Block> + 'src,
) -> impl RooParser<'src, Local> {
    annotations()
        .then_ignore(just(Token::Let))
        .then(pat(expr.clone()).map(Box::new))
        .then(just(Token::Colon).ignore_then(ty()).map(Box::new).or_not())
        .then(
            just(Token::Eq)
                .ignore_then(expr)
                .then(just(Token::Else).ignore_then(block).map(Box::new).or_not())
                .or_not(),
        )
        .then_ignore(just(Token::Semi))
        .map_with(|(((annotations, pat), ty), init), e| {
            let kind = match init {
                None => LocalKind::Decl,
                Some((value, None)) => LocalKind::Init(Box::new(value)),
                Some((value, Some(else_block))) => LocalKind::InitElse(Box::new(value), else_block),
            };
            Local {
                pat,
                ty,
                kind,
                annotations,
                span: span(e),
            }
        })
}

fn stmt<'src>(
    expr: impl RooParser<'src, Expr> + 'src,
    block: impl RooParser<'src, Block> + 'src,
) -> impl RooParser<'src, Stmt> {
    let item_stmt = item_with(expr.clone(), block.clone())
        .map(Box::new)
        .map(StmtKind::Item);

    let local_stmt = local(expr.clone(), block).map(Box::new).map(StmtKind::Let);

    let expr_stmt = expr
        .map(Box::new)
        .then(just(Token::Semi).or_not())
        .map(|(e, semi)| {
            if semi.is_some() {
                StmtKind::Semi(e)
            } else {
                StmtKind::Expr(e)
            }
        });

    choice((item_stmt, local_stmt, expr_stmt)).map_with(|kind, e| Stmt {
        kind,
        span: span(e),
    })
}

pub fn block<'src>(expr: impl RooParser<'src, Expr> + 'src) -> impl RooParser<'src, Block> {
    recursive(|block| {
        stmt(expr.clone(), block)
            .repeated()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map_with(|stmts, e| Block {
                stmts,
                span: span(e),
            })
    })
}

pub fn guard<'src>(expr: impl RooParser<'src, Expr> + 'src) -> impl RooParser<'src, Guard> {
    just(Token::If).ignore_then(expr).map(|cond| Guard { cond })
}

pub fn arm<'src>(
    pat: impl RooParser<'src, Pat> + 'src,
    expr: impl RooParser<'src, Expr> + 'src,
) -> impl RooParser<'src, Arm> {
    annotations()
        .then(pat.map(Box::new))
        .then(guard(expr.clone()).map(Box::new).or_not())
        .then_ignore(just(Token::FatArrow))
        .then(expr.map(Box::new).or_not())
        .then_ignore(just(Token::Comma).or_not())
        .map_with(|(((annotations, pat), guard), body), e| Arm {
            annotations,
            pat,
            guard,
            body,
            span: span(e),
        })
}
