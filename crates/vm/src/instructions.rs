use crate::value::Value;

#[derive(Debug, Clone, Copy)]
pub enum Instruction {
    Nop,
    Halt,
    Push(Value),
    Pop,
    Jump(usize),
    JumpIfFalse(usize),
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
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
