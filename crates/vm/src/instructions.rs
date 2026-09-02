use crate::value::Value;

#[derive(Debug, Clone, Copy)]
pub enum Instruction {
    Nop,
    Halt,
    Push(Value),
    Pop,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitXor,
    BitAnd,
    BitOr,
    Shl,
    Shr,
    Not,
    Neg,
}
