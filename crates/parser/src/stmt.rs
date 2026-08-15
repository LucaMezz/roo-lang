//! Statements, blocks, and match-arm scaffolding (`Local`/`Stmt`/`Block`/
//! `Guard`/`Arm`).

use crate::*;
use ast::*;
use lexer::Token;

/// `#[annotations]* "let" pattern (":" type)? ("=" expr ("else" block)?)? ";"`.
///
/// No `pub` here even though `grammar.md`'s `let_stmt` shows one — `Local`
/// has no `vis` field to put it in. Flagging, not fixing: that's a real
/// AST/grammar mismatch, same shape as the `GenericParam`/`PatField`
/// audit gaps earlier, out of scope for "finish `ExprKind`".
fn local<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, Local> {
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

/// `item | local | expr ";"? ` — parses `expr` once regardless of
/// whether a trailing `;` follows, rather than trying `expr ";"` and
/// `expr` alone as two separate `choice` branches (which would re-parse
/// the same expression twice on every semicolon-less statement) — same
/// shared-prefix shape as `meta_item`/`generic_param`.
///
/// Uses `item_with(expr, block)`, not the top-level `item()` — `item()`
/// builds its own fresh `expr`/`block` internally, which here would
/// recurse forever (`expr -> block -> stmt -> item -> expr -> ...`).
/// `item_with` instead reuses the `expr`/`block` already being tied by
/// `expr()`/`block()`'s own recursion, same reason `block` itself takes
/// `expr` as a parameter instead of calling `expr()` directly.
fn stmt<'src>(
    expr: impl FigParser<'src, Expr> + 'src,
    block: impl FigParser<'src, Block> + 'src,
) -> impl FigParser<'src, Stmt> {
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

/// `"{" statement* "}"` — a block's "trailing expression" isn't a
/// separate AST field (unlike the tree-sitter grammar's `block` rule,
/// which does have one); it's just whatever the *last* `Stmt` happens to
/// be (`StmtKind::Expr`, no semicolon) — `stmt` above already produces
/// that naturally, nothing extra needed here.
///
/// Self-recursive (a block can contain another `{ ... }`), so ties its
/// own knot via `recursive()`, same as `ty`/`pat`/`expr` — takes `expr`
/// as a parameter since it's used *from inside* `expr()`'s own
/// recursive tie (`ExprKind::Block`/`If`/`While`/`ForLoop`/`Loop` all
/// need a `Block`), so it can't call `expr()` directly without
/// recreating the exact E0720 cycle `fn_ret_ty` hit earlier.
pub fn block<'src>(expr: impl FigParser<'src, Expr> + 'src) -> impl FigParser<'src, Block> {
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

pub fn guard<'src>(expr: impl FigParser<'src, Expr> + 'src) -> impl FigParser<'src, Guard> {
    just(Token::If).ignore_then(expr).map(|cond| Guard { cond })
}

pub fn arm<'src>(
    pat: impl FigParser<'src, Pat> + 'src,
    expr: impl FigParser<'src, Expr> + 'src,
) -> impl FigParser<'src, Arm> {
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
