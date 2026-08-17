use crate::*;
use ast::*;
use lexer::Token;

pub fn bin_op<'src>() -> impl RooParser<'src, BinOp> {
    select! {
        Token::Plus = e => BinOpKind::Add,
        Token::Minus = e => BinOpKind::Sub,
        Token::Star = e => BinOpKind::Mul,
        Token::Slash = e => BinOpKind::Div,
        Token::Percent = e => BinOpKind::Rem,
        Token::AndAnd = e => BinOpKind::And,
        Token::OrOr = e => BinOpKind::Or,
        Token::Caret = e => BinOpKind::BitXor,
        Token::Amp = e => BinOpKind::BitAnd,
        Token::Pipe = e => BinOpKind::BitOr,
        Token::Shl = e => BinOpKind::Shl,
        Token::Shr = e => BinOpKind::Shr,
        Token::EqEq = e => BinOpKind::Eq,
        Token::Lt = e => BinOpKind::Lt,
        Token::LtEq = e => BinOpKind::Le,
        Token::NotEq = e => BinOpKind::Ne,
        Token::GtEq = e => BinOpKind::Ge,
        Token::Gt = e => BinOpKind::Gt,
    }
    .map_with(|kind, e| BinOp {
        kind,
        span: span(e),
    })
}

pub fn assign_op<'src>() -> impl RooParser<'src, AssignOp> {
    select! {
        Token::PlusEq = e => AssignOpKind::AddAssign,
        Token::MinusEq = e => AssignOpKind::SubAssign,
        Token::StarEq = e => AssignOpKind::MulAssign,
        Token::SlashEq = e => AssignOpKind::DivAssign,
        Token::PercentEq = e => AssignOpKind::RemAssign,
        Token::AmpEq = e => AssignOpKind::BitAndAssign,
        Token::PipeEq = e => AssignOpKind::BitOrAssign,
        Token::CaretEq = e => AssignOpKind::BitXorAssign,
        Token::ShlEq = e => AssignOpKind::ShlAssign,
        Token::ShrEq = e => AssignOpKind::ShrAssign,
    }
    .map_with(|kind, e| AssignOp {
        kind,
        span: span(e),
    })
}

pub fn un_op<'src>() -> impl RooParser<'src, UnOp> {
    select! {
        Token::Bang => UnOp::Not,
        Token::Minus => UnOp::Neg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tokens;

    #[test]
    fn parses_every_binary_operator() {
        let cases = [
            ("+", BinOpKind::Add),
            ("-", BinOpKind::Sub),
            ("*", BinOpKind::Mul),
            ("/", BinOpKind::Div),
            ("%", BinOpKind::Rem),
            ("&&", BinOpKind::And),
            ("||", BinOpKind::Or),
            ("^", BinOpKind::BitXor),
            ("&", BinOpKind::BitAnd),
            ("|", BinOpKind::BitOr),
            ("<<", BinOpKind::Shl),
            (">>", BinOpKind::Shr),
            ("==", BinOpKind::Eq),
            ("<", BinOpKind::Lt),
            ("<=", BinOpKind::Le),
            ("!=", BinOpKind::Ne),
            (">=", BinOpKind::Ge),
            (">", BinOpKind::Gt),
        ];
        for (src, expected) in cases {
            let tokens = tokens(src);
            let parsed = bin_op().parse(tokens).into_result().expect("should parse");
            assert_eq!(parsed.kind, expected, "input: {src}");
            assert_eq!(
                parsed.span,
                ast::Span {
                    start: 0,
                    end: src.len()
                },
                "input: {src}"
            );
        }
    }

    #[test]
    fn parses_every_compound_assignment_operator() {
        let cases = [
            ("+=", AssignOpKind::AddAssign),
            ("-=", AssignOpKind::SubAssign),
            ("*=", AssignOpKind::MulAssign),
            ("/=", AssignOpKind::DivAssign),
            ("%=", AssignOpKind::RemAssign),
            ("&=", AssignOpKind::BitAndAssign),
            ("|=", AssignOpKind::BitOrAssign),
            ("^=", AssignOpKind::BitXorAssign),
            ("<<=", AssignOpKind::ShlAssign),
            (">>=", AssignOpKind::ShrAssign),
        ];
        for (src, expected) in cases {
            let tokens = tokens(src);
            let parsed = assign_op()
                .parse(tokens)
                .into_result()
                .expect("should parse");
            assert_eq!(parsed.kind, expected, "input: {src}");
            assert_eq!(
                parsed.span,
                ast::Span {
                    start: 0,
                    end: src.len()
                },
                "input: {src}"
            );
        }
    }

    #[test]
    fn rejects_plain_eq_as_an_assign_op() {
        let tokens = tokens("=");
        assert!(assign_op().parse(tokens).into_result().is_err());
    }

    #[test]
    fn parses_both_unary_operators() {
        let cases = [("!", UnOp::Not), ("-", UnOp::Neg)];
        for (src, expected) in cases {
            let tokens = tokens(src);
            let parsed = un_op().parse(tokens).into_result().expect("should parse");
            assert_eq!(parsed, expected, "input: {src}");
        }
    }
}
