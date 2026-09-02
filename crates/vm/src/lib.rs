#![allow(unused)]
//! The virtual machine which executes roo code.

use std::sync::Arc;

use slotmap::{SlotMap, new_key_type};
use thiserror::Error;

use crate::instructions::Instruction;
use crate::value::{Value, ValueError};

new_key_type! {
    /// A pointer to a heap-allocated object.
    pub struct HeapId;
}

mod instructions;
mod value;

/// An error produced while executing a program.
#[derive(Debug, Error, PartialEq)]
pub enum VmError {
    /// A pop was attempted on an empty stack.
    #[error("stack underflow")]
    StackUnderflow,
    /// A heap reference did not point to a live object.
    #[error("invalid heap reference")]
    InvalidHeapRef,
    /// An index was out of the bounds of the container being indexed.
    #[error("index out of bounds")]
    IndexOutOfBounds,
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
                Instruction::Jump(target) => self.jump(target),
                Instruction::JumpIfFalse(target) => {
                    if !self.stack.pop_bool()? {
                        self.jump(target);
                    }
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
                Instruction::Eq => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(a.eq(b)?);
                }
                Instruction::Ne => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(a.ne(b)?);
                }
                Instruction::Lt => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(a.lt(b)?);
                }
                Instruction::Le => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(a.le(b)?);
                }
                Instruction::Gt => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(a.gt(b)?);
                }
                Instruction::Ge => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(a.ge(b)?);
                }
                Instruction::Dup => {
                    let value = *self.stack.peek()?;
                    self.stack.push(value);
                }
                Instruction::Array(count) => {
                    let mut elements = Vec::with_capacity(count);
                    for _ in 0..count {
                        elements.push(self.stack.pop()?);
                    }
                    elements.reverse();
                    let id = self.heap.insert(HeapObject::Array(elements));
                    self.stack.push(Value::Ref(id));
                }
                Instruction::Index => {
                    let index = self.stack.pop_index()?;
                    let id = self.stack.pop_ref()?;
                    let elements = match self.heap.get(id)? {
                        HeapObject::Array(elements) => elements.as_slice(),
                        HeapObject::Adt { fields, .. } => fields.iter().as_slice(),
                        _ => return Err(ValueError::TypeMismatch.into()),
                    };
                    let value = *elements.get(index).ok_or(VmError::IndexOutOfBounds)?;
                    self.stack.push(value);
                }
                Instruction::IndexMut => {
                    let value = self.stack.pop()?;
                    let index = self.stack.pop_index()?;
                    let id = self.stack.pop_ref()?;
                    let elements = match self.heap.get_mut(id)? {
                        HeapObject::Array(elements) => elements.as_mut_slice(),
                        HeapObject::Adt { fields, .. } => fields,
                        _ => return Err(ValueError::TypeMismatch.into()),
                    };
                    let val = elements.get_mut(index).ok_or(VmError::IndexOutOfBounds)?;
                    std::mem::replace(val, value);
                }
                Instruction::Insert => {
                    let value = self.stack.pop()?;
                    let index = self.stack.pop_index()?;
                    let id = self.stack.pop_ref()?;
                    let elements = self.heap.get_array_mut(id)?;
                    if index > elements.len() {
                        return Err(VmError::IndexOutOfBounds);
                    }
                    elements.insert(index, value);
                }
                Instruction::Remove => {
                    let index = self.stack.pop_index()?;
                    let id = self.stack.pop_ref()?;
                    let elements = self.heap.get_array_mut(id)?;
                    if index >= elements.len() {
                        return Err(VmError::IndexOutOfBounds);
                    }
                    elements.remove(index);
                }
                Instruction::Len => {
                    let id = self.stack.pop_ref()?;
                    let len = match self.heap.get(id)? {
                        HeapObject::Array(elements) => elements.len(),
                        HeapObject::Adt { fields, .. } => fields.len(),
                        HeapObject::String(string) => string.len(),
                        _ => return Err(ValueError::TypeMismatch.into()),
                    };
                    self.stack.push(Value::Int(
                        i64::try_from(len).map_err(|_| ValueError::Overflow)?,
                    ));
                }
                _ => unimplemented!(),
            };
        }
        Ok(())
    }

    fn jump(&mut self, instr: usize) {
        self.ip = instr;
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

    fn pop_bool(&mut self) -> Result<bool, VmError> {
        match self.pop()? {
            Value::Bool(value) => Ok(value),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn pop_int(&mut self) -> Result<i64, VmError> {
        match self.pop()? {
            Value::Int(value) => Ok(value),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn pop_ref(&mut self) -> Result<HeapId, VmError> {
        match self.pop()? {
            Value::Ref(id) => Ok(id),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn pop_index(&mut self) -> Result<usize, VmError> {
        let index = self.pop_int()?;
        usize::try_from(index).map_err(|_| ValueError::Overflow.into())
    }

    fn peek(&self) -> Result<&Value, VmError> {
        self.values.last().ok_or(VmError::StackUnderflow)
    }
}

/// A heap
pub struct Heap {
    /// The data stored in the heap.
    data: SlotMap<HeapId, HeapObject>,
}

impl Heap {
    fn new() -> Self {
        Self {
            data: SlotMap::with_key(),
        }
    }

    fn get(&self, id: HeapId) -> Result<&HeapObject, VmError> {
        self.data.get(id).ok_or(VmError::InvalidHeapRef)
    }

    fn get_mut(&mut self, id: HeapId) -> Result<&mut HeapObject, VmError> {
        self.data.get_mut(id).ok_or(VmError::InvalidHeapRef)
    }

    fn get_value(&self, id: HeapId) -> Result<&Value, VmError> {
        match self.get(id)? {
            HeapObject::Value(value) => Ok(value),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn get_value_mut(&mut self, id: HeapId) -> Result<&mut Value, VmError> {
        match self.get_mut(id)? {
            HeapObject::Value(value) => Ok(value),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn get_adt(&self, id: HeapId) -> Result<(u32, &[Value]), VmError> {
        match self.get(id)? {
            HeapObject::Adt { tag, fields } => Ok((*tag, fields)),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn get_adt_mut(&mut self, id: HeapId) -> Result<(u32, &mut [Value]), VmError> {
        match self.get_mut(id)? {
            HeapObject::Adt { tag, fields } => Ok((*tag, fields)),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn get_array(&self, id: HeapId) -> Result<&Vec<Value>, VmError> {
        match self.get(id)? {
            HeapObject::Array(elements) => Ok(elements),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn get_array_mut(&mut self, id: HeapId) -> Result<&mut Vec<Value>, VmError> {
        match self.get_mut(id)? {
            HeapObject::Array(elements) => Ok(elements),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn get_string(&self, id: HeapId) -> Result<&str, VmError> {
        match self.get(id)? {
            HeapObject::String(string) => Ok(string),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn get_string_mut(&mut self, id: HeapId) -> Result<&mut str, VmError> {
        match self.get_mut(id)? {
            HeapObject::String(string) => Ok(string),
            _ => Err(ValueError::TypeMismatch.into()),
        }
    }

    fn insert(&mut self, object: HeapObject) -> HeapId {
        self.data.insert(object)
    }
}

/// An object which is allocated on the heap.
#[derive(Debug, PartialEq)]
pub enum HeapObject {
    /// A stack value with fixed size, which has instead been
    /// allocated on the heap.
    Value(Value),
    /// A heap-allocated algebraic data type.
    Adt {
        /// Which variant is active.
        tag: u32,
        /// The fields of the variant.
        fields: Box<[Value]>,
    },
    /// A heap-allocated array of stack values.
    Array(Vec<Value>),
    /// A heap-allocated string.
    String(Box<str>),
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
    fn heap_insert_returns_distinct_ids_for_each_object() {
        let mut heap = Heap::new();

        let a = heap.insert(HeapObject::String("a".into()));
        let b = heap.insert(HeapObject::String("b".into()));

        assert_ne!(a, b);
        assert_eq!(heap.data.get(a), Some(&HeapObject::String("a".into())));
        assert_eq!(heap.data.get(b), Some(&HeapObject::String("b".into())));
    }

    #[test]
    fn heap_get_returns_the_inserted_object() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::String("hi".into()));

        assert_eq!(heap.get(id), Ok(&HeapObject::String("hi".into())));
    }

    #[test]
    fn heap_get_with_invalid_id_returns_invalid_heap_ref() {
        let heap = Heap::new();

        assert_eq!(heap.get(HeapId::default()), Err(VmError::InvalidHeapRef));
    }

    #[test]
    fn heap_get_value_returns_the_value() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::Value(Value::Int(42)));

        assert_eq!(heap.get_value(id), Ok(&Value::Int(42)));
    }

    #[test]
    fn heap_get_value_mut_allows_mutation() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::Value(Value::Int(1)));

        *heap.get_value_mut(id).unwrap() = Value::Int(2);

        assert_eq!(heap.get_value(id), Ok(&Value::Int(2)));
    }

    #[test]
    fn heap_get_value_on_wrong_variant_returns_type_mismatch() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::String("hi".into()));

        assert_eq!(
            heap.get_value(id),
            Err(VmError::Value(ValueError::TypeMismatch))
        );
    }

    #[test]
    fn heap_get_adt_returns_the_tag_and_fields() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::Adt {
            tag: 3,
            fields: Box::new([Value::Int(1), Value::Int(2)]),
        });

        assert_eq!(
            heap.get_adt(id),
            Ok((3, [Value::Int(1), Value::Int(2)].as_slice()))
        );
    }

    #[test]
    fn heap_get_adt_mut_allows_mutating_a_field() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::Adt {
            tag: 0,
            fields: Box::new([Value::Int(1)]),
        });

        heap.get_adt_mut(id).unwrap().1[0] = Value::Int(99);

        assert_eq!(
            heap.get(id),
            Ok(&HeapObject::Adt {
                tag: 0,
                fields: Box::new([Value::Int(99)]),
            })
        );
    }

    #[test]
    fn heap_get_array_returns_the_array() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::Array(vec![Value::Int(1), Value::Int(2)]));

        assert_eq!(heap.get_array(id), Ok(&vec![Value::Int(1), Value::Int(2)]));
    }

    #[test]
    fn heap_get_array_mut_allows_mutation() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::Array(vec![Value::Int(1)]));

        heap.get_array_mut(id).unwrap().push(Value::Int(2));

        assert_eq!(
            heap.get(id),
            Ok(&HeapObject::Array(vec![Value::Int(1), Value::Int(2)]))
        );
    }

    #[test]
    fn heap_get_array_on_wrong_variant_returns_type_mismatch() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::String("hi".into()));

        assert_eq!(
            heap.get_array(id),
            Err(VmError::Value(ValueError::TypeMismatch))
        );
    }

    #[test]
    fn heap_get_array_mut_on_wrong_variant_returns_type_mismatch() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::Adt {
            tag: 0,
            fields: Box::new([]),
        });

        assert_eq!(
            heap.get_array_mut(id),
            Err(VmError::Value(ValueError::TypeMismatch))
        );
    }

    #[test]
    fn heap_get_string_returns_the_string() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::String("hi".into()));

        assert_eq!(heap.get_string(id), Ok("hi"));
    }

    #[test]
    fn heap_get_string_mut_allows_mutation() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::String("hi".into()));

        heap.get_string_mut(id).unwrap().make_ascii_uppercase();

        assert_eq!(heap.get_string(id), Ok("HI"));
    }

    #[test]
    fn heap_get_string_on_wrong_variant_returns_type_mismatch() {
        let mut heap = Heap::new();
        let id = heap.insert(HeapObject::Array(vec![]));

        assert_eq!(
            heap.get_string(id),
            Err(VmError::Value(ValueError::TypeMismatch))
        );
    }

    #[test]
    fn vm_fetch_advances_the_instruction_pointer() {
        let mut vm = vm_with(vec![Instruction::Nop, Instruction::Halt]);

        assert_eq!(vm.ip, 0);
        vm.fetch();
        assert_eq!(vm.ip, 1);
    }

    #[test]
    fn vm_jump_sets_the_instruction_pointer_directly() {
        let mut vm = vm_with(vec![Instruction::Nop, Instruction::Halt]);
        vm.ip = 5;

        vm.jump(2);

        assert_eq!(vm.ip, 2);
    }

    #[test]
    fn vm_jump_instruction_skips_over_the_instructions_in_between() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Jump(3),
            Instruction::Push(Value::Int(99)),
            Instruction::Halt,
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_jump_instruction_can_jump_backward() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Jump(4),
            Instruction::Push(Value::Int(2)),
            Instruction::Halt,
            Instruction::Jump(2),
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(2)));
        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
    }

    #[test]
    fn vm_jump_if_false_jumps_when_condition_is_false() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Bool(false)),
            Instruction::JumpIfFalse(4),
            Instruction::Push(Value::Int(99)),
            Instruction::Halt,
            Instruction::Push(Value::Int(1)),
            Instruction::Halt,
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_jump_if_false_does_not_jump_when_condition_is_true() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Bool(true)),
            Instruction::JumpIfFalse(4),
            Instruction::Push(Value::Int(1)),
            Instruction::Halt,
            Instruction::Push(Value::Int(99)),
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_jump_if_false_with_non_bool_condition_returns_type_mismatch() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(0)),
            Instruction::JumpIfFalse(3),
            Instruction::Halt,
        ]);

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_dup_pushes_a_copy_of_the_top_value() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Dup,
            Instruction::Halt,
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_dup_leaves_values_beneath_it_untouched() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Dup,
            Instruction::Add,
            Instruction::Halt,
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(4)));
        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
    }

    #[test]
    fn vm_dup_on_an_empty_stack_returns_stack_underflow() {
        let mut vm = vm_with(vec![Instruction::Dup, Instruction::Halt]);

        assert_eq!(vm.execute(), Err(VmError::StackUnderflow));
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
    fn vm_lt_feeding_jump_if_false_implements_an_if_condition() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Lt,
            Instruction::JumpIfFalse(6),
            Instruction::Push(Value::Int(42)),
            Instruction::Halt,
            Instruction::Push(Value::Int(0)),
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(42)));
    }

    #[test]
    fn vm_array_instruction_allocates_an_array_and_pushes_a_ref() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Push(Value::Int(3)),
            Instruction::Array(3),
            Instruction::Halt,
        ]);

        vm.execute().unwrap();

        let Value::Ref(id) = vm.stack.pop().unwrap() else {
            panic!("expected a Value::Ref");
        };
        assert_eq!(
            vm.heap.data.get(id),
            Some(&HeapObject::Array(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3)
            ]))
        );
    }

    #[test]
    fn vm_array_instruction_with_zero_elements_allocates_an_empty_array() {
        let mut vm = vm_with(vec![Instruction::Array(0), Instruction::Halt]);

        vm.execute().unwrap();

        let Value::Ref(id) = vm.stack.pop().unwrap() else {
            panic!("expected a Value::Ref");
        };
        assert_eq!(vm.heap.data.get(id), Some(&HeapObject::Array(vec![])));
    }

    #[test]
    fn vm_index_gets_an_array_element() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Push(Value::Int(3)),
            Instruction::Array(3),
            Instruction::Push(Value::Int(1)),
            Instruction::Index,
            Instruction::Halt,
        ]);

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(2)));
    }

    #[test]
    fn vm_index_gets_an_adt_field() {
        let mut vm = vm_with(vec![Instruction::Index, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::Adt {
            tag: 0,
            fields: Box::new([Value::Int(10), Value::Int(20)]),
        });
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(1));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(20)));
    }

    #[test]
    fn vm_index_out_of_bounds_returns_an_error() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Array(1),
            Instruction::Push(Value::Int(5)),
            Instruction::Index,
            Instruction::Halt,
        ]);

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_index_with_negative_index_returns_overflow() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Array(1),
            Instruction::Push(Value::Int(-1)),
            Instruction::Index,
            Instruction::Halt,
        ]);

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::Overflow)));
    }

    #[test]
    fn vm_index_with_a_non_int_index_returns_type_mismatch() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Array(1),
            Instruction::Push(Value::Bool(true)),
            Instruction::Index,
            Instruction::Halt,
        ]);

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_index_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(vec![
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(0)),
            Instruction::Index,
            Instruction::Halt,
        ]);

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_index_on_a_non_indexable_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(vec![Instruction::Index, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_index_mut_sets_an_array_element() {
        let mut vm = vm_with(vec![Instruction::IndexMut, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
        ]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(99));

        vm.execute().unwrap();

        assert_eq!(
            vm.heap.get(id),
            Ok(&HeapObject::Array(vec![
                Value::Int(1),
                Value::Int(99),
                Value::Int(3)
            ]))
        );
    }

    #[test]
    fn vm_index_mut_sets_an_adt_field() {
        let mut vm = vm_with(vec![Instruction::IndexMut, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::Adt {
            tag: 0,
            fields: Box::new([Value::Int(10), Value::Int(20)]),
        });
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(99));

        vm.execute().unwrap();

        assert_eq!(
            vm.heap.get(id),
            Ok(&HeapObject::Adt {
                tag: 0,
                fields: Box::new([Value::Int(99), Value::Int(20)]),
            })
        );
    }

    #[test]
    fn vm_index_mut_out_of_bounds_returns_an_error() {
        let mut vm = vm_with(vec![Instruction::IndexMut, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::Array(vec![Value::Int(1)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(5));
        vm.stack.push(Value::Int(99));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_index_mut_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(vec![Instruction::IndexMut, Instruction::Halt]);
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(99));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_insert_at_the_end_acts_like_append() {
        let mut vm = vm_with(vec![Instruction::Insert, Instruction::Halt]);
        let id = vm
            .heap
            .insert(HeapObject::Array(vec![Value::Int(1), Value::Int(2)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(2));
        vm.stack.push(Value::Int(3));

        vm.execute().unwrap();

        assert_eq!(
            vm.heap.get(id),
            Ok(&HeapObject::Array(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3)
            ]))
        );
    }

    #[test]
    fn vm_insert_in_the_middle_shifts_later_elements() {
        let mut vm = vm_with(vec![Instruction::Insert, Instruction::Halt]);
        let id = vm
            .heap
            .insert(HeapObject::Array(vec![Value::Int(1), Value::Int(3)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(2));

        vm.execute().unwrap();

        assert_eq!(
            vm.heap.get(id),
            Ok(&HeapObject::Array(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3)
            ]))
        );
    }

    #[test]
    fn vm_insert_out_of_bounds_returns_an_error_instead_of_panicking() {
        let mut vm = vm_with(vec![Instruction::Insert, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::Array(vec![Value::Int(1)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(5));
        vm.stack.push(Value::Int(99));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_insert_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(vec![Instruction::Insert, Instruction::Halt]);
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(2));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_insert_on_a_non_array_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(vec![Instruction::Insert, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::Adt {
            tag: 0,
            fields: Box::new([]),
        });
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_remove_removes_the_element_at_the_index() {
        let mut vm = vm_with(vec![Instruction::Remove, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
        ]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(1));

        vm.execute().unwrap();

        assert_eq!(
            vm.heap.get(id),
            Ok(&HeapObject::Array(vec![Value::Int(1), Value::Int(3)]))
        );
    }

    #[test]
    fn vm_remove_out_of_bounds_returns_an_error_instead_of_panicking() {
        let mut vm = vm_with(vec![Instruction::Remove, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::Array(vec![Value::Int(1)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(5));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_remove_on_a_non_array_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(vec![Instruction::Remove, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_len_returns_the_number_of_array_elements() {
        let mut vm = vm_with(vec![Instruction::Len, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
        ]));
        vm.stack.push(Value::Ref(id));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(3)));
    }

    #[test]
    fn vm_len_returns_the_number_of_adt_fields() {
        let mut vm = vm_with(vec![Instruction::Len, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::Adt {
            tag: 0,
            fields: Box::new([Value::Int(1), Value::Int(2)]),
        });
        vm.stack.push(Value::Ref(id));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(2)));
    }

    #[test]
    fn vm_len_returns_the_byte_length_of_a_string() {
        let mut vm = vm_with(vec![Instruction::Len, Instruction::Halt]);
        let id = vm.heap.insert(HeapObject::String("hello".into()));
        vm.stack.push(Value::Ref(id));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(5)));
    }

    #[test]
    fn vm_len_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(vec![Instruction::Len, Instruction::Halt]);
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
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
