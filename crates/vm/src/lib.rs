#![allow(unused)]
//! The virtual machine which executes roo code.

use std::sync::Arc;

use thiserror::Error;

use crate::instructions::Instruction;
use crate::value::{Value, ValueError};

mod instructions;
mod value;

/// An error produced while executing a program.
#[derive(Debug, Error, PartialEq)]
pub enum VmError {
    /// A pop was attempted on an empty stack.
    #[error("stack underflow")]
    StackUnderflow,
    /// A value operation failed.
    #[error(transparent)]
    Value(#[from] ValueError),
}

/// The virtual machine
pub struct Vm {
    /// Address of the next instruction to be executed.
    ip: usize,

    /// The stack of the VM
    stack: Stack,

    /// The heap of the VM
    heap: Heap,

    /// The program to be executed by the VM
    program: Arc<Program>,
}

impl Vm {
    fn new(program: Arc<Program>) -> Self {
        Self {
            ip: 0,
            stack: Stack::new(),
            heap: Heap::new(),
            program,
        }
    }

    fn fetch(&mut self) -> Instruction {
        let instr = self.program.fetch(self.ip);
        self.ip += 1;
        instr
    }

    fn execute(&mut self) -> Result<(), VmError> {
        loop {
            let instr = self.fetch();
            match instr {
                Instruction::Nop => {}
                Instruction::Halt => break,
                Instruction::Push(value) => self.stack.push(value),
                Instruction::Pop => {
                    self.stack.pop()?;
                }
                Instruction::Add => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a + b)?);
                }
                Instruction::Sub => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a - b)?);
                }
                Instruction::Mul => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a * b)?);
                }
                Instruction::Div => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a / b)?);
                }
                Instruction::Rem => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a % b)?);
                }
                Instruction::BitXor => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a ^ b)?);
                }
                Instruction::BitAnd => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a & b)?);
                }
                Instruction::BitOr => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a | b)?);
                }
                Instruction::Shl => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a << b)?);
                }
                Instruction::Shr => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a >> b)?);
                }
                Instruction::Not => {
                    let a = self.stack.pop()?;
                    self.stack.push((!a)?);
                }
                Instruction::Neg => {
                    let a = self.stack.pop()?;
                    self.stack.push((-a)?);
                }
                _ => unimplemented!(),
            };
        }
        Ok(())
    }
}

/// A program made up of instructions.
#[derive(Default)]
pub struct Program {
    instrs: Vec<Instruction>,
}

impl Program {
    fn fetch(&self, instr: usize) -> Instruction {
        self.instrs[instr]
    }
}

impl From<Vec<Instruction>> for Program {
    fn from(value: Vec<Instruction>) -> Self {
        Self { instrs: value }
    }
}

/// A stack
pub struct Stack {
    /// The values on the stack.
    values: Vec<Value>,
}

impl Stack {
    fn new() -> Self {
        Self { values: Vec::new() }
    }

    fn push(&mut self, value: Value) {
        self.values.push(value);
    }

    fn pop(&mut self) -> Result<Value, VmError> {
        self.values.pop().ok_or(VmError::StackUnderflow)
    }

    fn peek(&mut self) -> Result<&Value, VmError> {
        self.values.last().ok_or(VmError::StackUnderflow)
    }
}

/// A heap
pub struct Heap {
    /// The data stored in the heap.
    data: Vec<u8>,
}

impl Heap {
    fn new() -> Self {
        Self { data: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm_with(instrs: Vec<Instruction>) -> Vm {
        Vm::new(Arc::new(Program::from(instrs)))
    }

    #[test]
    fn stack_pop_returns_the_last_pushed_value() {
        let mut stack = Stack::new();
        stack.push(Value::Int(1));
        stack.push(Value::Int(2));

        assert_eq!(stack.pop(), Ok(Value::Int(2)));
        assert_eq!(stack.pop(), Ok(Value::Int(1)));
    }

    #[test]
    fn stack_pop_on_empty_stack_returns_stack_underflow() {
        let mut stack = Stack::new();

        assert_eq!(stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn stack_peek_returns_the_top_value_without_removing_it() {
        let mut stack = Stack::new();
        stack.push(Value::Int(42));

        assert_eq!(stack.peek(), Ok(&Value::Int(42)));
        assert_eq!(stack.pop(), Ok(Value::Int(42)));
    }

    #[test]
    fn stack_peek_on_empty_stack_returns_stack_underflow() {
        let mut stack = Stack::new();

        assert_eq!(stack.peek(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_fetch_advances_the_instruction_pointer() {
        let mut vm = vm_with(vec![Instruction::Nop, Instruction::Halt]);

        assert_eq!(vm.ip, 0);
        vm.fetch();
        assert_eq!(vm.ip, 1);
    }

    #[test]
    fn vm_nop_leaves_the_stack_unchanged() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Nop,
            Instruction::Halt,
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_executes_add_and_leaves_the_result_on_the_stack() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Add,
            Instruction::Halt,
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(3)));
    }

    #[test]
    fn vm_pop_instruction_removes_the_top_value() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Pop,
            Instruction::Halt,
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_halt_stops_execution_before_later_instructions_run() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Halt,
            Instruction::Push(Value::Int(2)),
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_add_on_an_empty_stack_returns_stack_underflow() {
        let mut vm = vm_with(vec![Instruction::Add, Instruction::Halt]);

        assert_eq!(vm.execute(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_add_with_mismatched_types_propagates_the_value_error() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Bool(true)),
            Instruction::Push(Value::Int(1)),
            Instruction::Add,
            Instruction::Halt,
        ]);

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }
}
