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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstrAddr(usize);

impl InstrAddr {
    fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StringId(usize);

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
    #[error("string slice does not fall on a char boundary")]
    InvalidUtf8Boundary,
    /// A value operation failed.
    #[error(transparent)]
    Value(#[from] ValueError),
}

/// The virtual machine
pub struct Vm {
    /// Address of the next instruction to be executed.
    ip: InstrAddr,

    /// The stack of the VM
    stack: Stack,

    /// The heap of the VM
    heap: Heap,

    /// The program to be executed by the VM
    program: Unit,

    strings: Box<[HeapId]>,
}

impl Vm {
    fn new(mut program: Unit) -> Self {
        let mut heap = Heap::new();
        let strings = Vec::from(std::mem::take(&mut program.strings))
            .into_iter()
            .map(|s| heap.insert(HeapObject::String(s)))
            .collect();

        Self {
            ip: InstrAddr(0),
            stack: Stack::new(),
            heap,
            program,
            strings,
        }
    }

    fn fetch(&mut self) -> Instruction {
        let instr = self.program.fetch(self.ip);
        self.ip = self.ip.next();
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
                Instruction::LoadString(string_id) => {
                    let id = self.strings[string_id.0];
                    self.stack.push(Value::Ref(id));
                }
                Instruction::Concat => {
                    let b = self.stack.pop_ref()?;
                    let a = self.stack.pop_ref()?;

                    let a = self.heap.get_string(a)?;
                    let b = self.heap.get_string(b)?;

                    let r = self
                        .heap
                        .insert(HeapObject::String(format!("{a}{b}").into()));

                    self.stack.push(Value::Ref(r));
                }
                Instruction::Slice(start, end) => {
                    let end = self.stack.pop_index()?;
                    let start = self.stack.pop_index()?;
                    let id = self.stack.pop_ref()?;
                    let s = self.heap.get_string(id)?;
                    if start > end || end > s.len() {
                        return Err(VmError::IndexOutOfBounds);
                    }
                    let slice = s.get(start..end).ok_or(VmError::InvalidUtf8Boundary)?;
                    let new_id = self.heap.insert(HeapObject::String(slice.into()));
                    self.stack.push(Value::Ref(new_id));
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
                Instruction::Is => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(Value::Bool(a == b));
                }
                Instruction::Eq => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    let value = match (a, b) {
                        (Value::Ref(a), Value::Ref(b)) => {
                            Value::Bool(self.heap.get_string(a)? == self.heap.get_string(b)?)
                        }
                        _ => a.eq(b)?,
                    };
                    self.stack.push(value);
                }
                Instruction::Ne => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    let value = match (a, b) {
                        (Value::Ref(a), Value::Ref(b)) => {
                            Value::Bool(self.heap.get_string(a)? != self.heap.get_string(b)?)
                        }
                        _ => a.ne(b)?,
                    };
                    self.stack.push(value);
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

    fn jump(&mut self, addr: InstrAddr) {
        self.ip = addr;
    }
}

/// A program made up of instructions.
#[derive(Default)]
pub struct Unit {
    instrs: Box<[Instruction]>,
    strings: Box<[Box<str>]>,
}

impl Unit {
    fn new(instrs: Box<[Instruction]>, strings: Box<[Box<str>]>) -> Self {
        Self { instrs, strings }
    }

    fn fetch(&self, addr: InstrAddr) -> Instruction {
        self.instrs[addr.0]
    }

    fn strings(&self) -> &[Box<str>] {
        &self.strings
    }
}

impl From<Box<[Instruction]>> for Unit {
    fn from(value: Box<[Instruction]>) -> Self {
        Self {
            instrs: value,
            strings: Box::new([]),
        }
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

    fn vm_with(instrs: Box<[Instruction]>) -> Vm {
        Vm::new(Unit::from(instrs))
    }

    #[test]
    fn vm_new_allocates_the_units_strings_into_the_heap() {
        let unit = Unit::new(
            Box::new([Instruction::Halt]),
            Box::new(["hello".into(), "world".into()]),
        );

        let vm = Vm::new(unit);

        assert_eq!(vm.strings.len(), 2);
        assert_eq!(
            vm.heap.get(vm.strings[0]),
            Ok(&HeapObject::String("hello".into()))
        );
        assert_eq!(
            vm.heap.get(vm.strings[1]),
            Ok(&HeapObject::String("world".into()))
        );
    }

    #[test]
    fn vm_load_string_pushes_a_ref_to_the_heap_allocated_string() {
        let unit = Unit::new(
            Box::new([Instruction::LoadString(StringId(0)), Instruction::Halt]),
            Box::new(["hello".into()]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        let Value::Ref(id) = vm.stack.pop().unwrap() else {
            panic!("expected a Value::Ref");
        };
        assert_eq!(vm.heap.get(id), Ok(&HeapObject::String("hello".into())));
    }

    #[test]
    fn vm_load_string_can_load_the_same_string_multiple_times() {
        let unit = Unit::new(
            Box::new([
                Instruction::LoadString(StringId(0)),
                Instruction::LoadString(StringId(0)),
                Instruction::Halt,
            ]),
            Box::new(["hello".into()]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), vm.stack.pop());
    }

    #[test]
    fn vm_new_leaves_the_units_own_strings_empty_after_moving_them() {
        let unit = Unit::new(Box::new([Instruction::Halt]), Box::new(["hello".into()]));

        let vm = Vm::new(unit);

        assert_eq!(vm.program.strings(), &[] as &[Box<str>]);
    }

    #[test]
    fn vm_concat_joins_two_strings_and_pushes_a_new_ref() {
        let unit = Unit::new(
            Box::new([
                Instruction::LoadString(StringId(0)),
                Instruction::LoadString(StringId(1)),
                Instruction::Concat,
                Instruction::Halt,
            ]),
            Box::new(["foo".into(), "bar".into()]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        let Value::Ref(id) = vm.stack.pop().unwrap() else {
            panic!("expected a Value::Ref");
        };
        assert_eq!(vm.heap.get(id), Ok(&HeapObject::String("foobar".into())));
    }

    #[test]
    fn vm_concat_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instruction::Concat, Instruction::Halt]));
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(2));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_concat_on_a_non_string_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instruction::Concat, Instruction::Halt]));
        let array_id = vm.heap.insert(HeapObject::Array(vec![]));
        let string_id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(array_id));
        vm.stack.push(Value::Ref(string_id));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_slice_returns_a_substring() {
        let mut vm = vm_with(Box::new([Instruction::Slice(0, 0), Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::String("hello".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(3));

        vm.execute().unwrap();

        let Value::Ref(new_id) = vm.stack.pop().unwrap() else {
            panic!("expected a Value::Ref");
        };
        assert_eq!(vm.heap.get(new_id), Ok(&HeapObject::String("el".into())));
    }

    #[test]
    fn vm_slice_out_of_bounds_returns_an_error() {
        let mut vm = vm_with(Box::new([Instruction::Slice(0, 0), Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(5));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_slice_with_start_after_end_returns_an_error() {
        let mut vm = vm_with(Box::new([Instruction::Slice(0, 0), Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::String("hello".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(3));
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_slice_off_a_char_boundary_returns_invalid_utf8_boundary() {
        let mut vm = vm_with(Box::new([Instruction::Slice(0, 0), Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::String("é".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::InvalidUtf8Boundary));
    }

    #[test]
    fn vm_slice_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instruction::Slice(0, 0), Instruction::Halt]));
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
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
        let mut vm = vm_with(Box::new([Instruction::Nop, Instruction::Halt]));

        assert_eq!(vm.ip, InstrAddr(0));
        vm.fetch();
        assert_eq!(vm.ip, InstrAddr(1));
    }

    #[test]
    fn vm_jump_sets_the_instruction_pointer_directly() {
        let mut vm = vm_with(Box::new([Instruction::Nop, Instruction::Halt]));
        vm.ip = InstrAddr(5);

        vm.jump(InstrAddr(2));

        assert_eq!(vm.ip, InstrAddr(2));
    }

    #[test]
    fn vm_jump_instruction_skips_over_the_instructions_in_between() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Jump(InstrAddr(3)),
            Instruction::Push(Value::Int(99)),
            Instruction::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_jump_instruction_can_jump_backward() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Jump(InstrAddr(4)),
            Instruction::Push(Value::Int(2)),
            Instruction::Halt,
            Instruction::Jump(InstrAddr(2)),
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(2)));
        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
    }

    #[test]
    fn vm_jump_if_false_jumps_when_condition_is_false() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Bool(false)),
            Instruction::JumpIfFalse(InstrAddr(4)),
            Instruction::Push(Value::Int(99)),
            Instruction::Halt,
            Instruction::Push(Value::Int(1)),
            Instruction::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_jump_if_false_does_not_jump_when_condition_is_true() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Bool(true)),
            Instruction::JumpIfFalse(InstrAddr(4)),
            Instruction::Push(Value::Int(1)),
            Instruction::Halt,
            Instruction::Push(Value::Int(99)),
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_jump_if_false_with_non_bool_condition_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(0)),
            Instruction::JumpIfFalse(InstrAddr(3)),
            Instruction::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_dup_pushes_a_copy_of_the_top_value() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Dup,
            Instruction::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_dup_leaves_values_beneath_it_untouched() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Dup,
            Instruction::Add,
            Instruction::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(4)));
        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
    }

    #[test]
    fn vm_dup_on_an_empty_stack_returns_stack_underflow() {
        let mut vm = vm_with(Box::new([Instruction::Dup, Instruction::Halt]));

        assert_eq!(vm.execute(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_nop_leaves_the_stack_unchanged() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Nop,
            Instruction::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_executes_add_and_leaves_the_result_on_the_stack() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Add,
            Instruction::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(3)));
    }

    #[test]
    fn vm_lt_feeding_jump_if_false_implements_an_if_condition() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Lt,
            Instruction::JumpIfFalse(InstrAddr(6)),
            Instruction::Push(Value::Int(42)),
            Instruction::Halt,
            Instruction::Push(Value::Int(0)),
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(42)));
    }

    #[test]
    fn vm_is_returns_true_for_equal_scalars() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(1)),
            Instruction::Is,
            Instruction::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(true)));
    }

    #[test]
    fn vm_is_returns_false_for_different_scalars() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Is,
            Instruction::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(false)));
    }

    #[test]
    fn vm_is_returns_true_for_the_same_heap_reference() {
        let mut vm = vm_with(Box::new([Instruction::Is, Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Ref(id));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(true)));
    }

    #[test]
    fn vm_is_returns_false_for_different_refs_with_equal_string_content() {
        let mut vm = vm_with(Box::new([Instruction::Is, Instruction::Halt]));
        let a = vm.heap.insert(HeapObject::String("hi".into()));
        let b = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(false)));
    }

    #[test]
    fn vm_eq_returns_true_for_different_refs_with_equal_string_content() {
        let mut vm = vm_with(Box::new([Instruction::Eq, Instruction::Halt]));
        let a = vm.heap.insert(HeapObject::String("hi".into()));
        let b = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(true)));
    }

    #[test]
    fn vm_eq_returns_false_for_refs_with_different_string_content() {
        let mut vm = vm_with(Box::new([Instruction::Eq, Instruction::Halt]));
        let a = vm.heap.insert(HeapObject::String("hi".into()));
        let b = vm.heap.insert(HeapObject::String("bye".into()));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(false)));
    }

    #[test]
    fn vm_eq_on_a_non_string_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instruction::Eq, Instruction::Halt]));
        let a = vm.heap.insert(HeapObject::Array(vec![]));
        let b = vm.heap.insert(HeapObject::Array(vec![]));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_ne_returns_false_for_different_refs_with_equal_string_content() {
        let mut vm = vm_with(Box::new([Instruction::Ne, Instruction::Halt]));
        let a = vm.heap.insert(HeapObject::String("hi".into()));
        let b = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(false)));
    }

    #[test]
    fn vm_ne_returns_true_for_refs_with_different_string_content() {
        let mut vm = vm_with(Box::new([Instruction::Ne, Instruction::Halt]));
        let a = vm.heap.insert(HeapObject::String("hi".into()));
        let b = vm.heap.insert(HeapObject::String("bye".into()));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(true)));
    }

    #[test]
    fn vm_array_instruction_allocates_an_array_and_pushes_a_ref() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Push(Value::Int(3)),
            Instruction::Array(3),
            Instruction::Halt,
        ]));

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
        let mut vm = vm_with(Box::new([Instruction::Array(0), Instruction::Halt]));

        vm.execute().unwrap();

        let Value::Ref(id) = vm.stack.pop().unwrap() else {
            panic!("expected a Value::Ref");
        };
        assert_eq!(vm.heap.data.get(id), Some(&HeapObject::Array(vec![])));
    }

    #[test]
    fn vm_index_gets_an_array_element() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(2)),
            Instruction::Push(Value::Int(3)),
            Instruction::Array(3),
            Instruction::Push(Value::Int(1)),
            Instruction::Index,
            Instruction::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(2)));
    }

    #[test]
    fn vm_index_gets_an_adt_field() {
        let mut vm = vm_with(Box::new([Instruction::Index, Instruction::Halt]));
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
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Array(1),
            Instruction::Push(Value::Int(5)),
            Instruction::Index,
            Instruction::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_index_with_negative_index_returns_overflow() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Array(1),
            Instruction::Push(Value::Int(-1)),
            Instruction::Index,
            Instruction::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::Overflow)));
    }

    #[test]
    fn vm_index_with_a_non_int_index_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Array(1),
            Instruction::Push(Value::Bool(true)),
            Instruction::Index,
            Instruction::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_index_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Push(Value::Int(0)),
            Instruction::Index,
            Instruction::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_index_on_a_non_indexable_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instruction::Index, Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_index_mut_sets_an_array_element() {
        let mut vm = vm_with(Box::new([Instruction::IndexMut, Instruction::Halt]));
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
        let mut vm = vm_with(Box::new([Instruction::IndexMut, Instruction::Halt]));
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
        let mut vm = vm_with(Box::new([Instruction::IndexMut, Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::Array(vec![Value::Int(1)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(5));
        vm.stack.push(Value::Int(99));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_index_mut_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instruction::IndexMut, Instruction::Halt]));
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(99));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_insert_at_the_end_acts_like_append() {
        let mut vm = vm_with(Box::new([Instruction::Insert, Instruction::Halt]));
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
        let mut vm = vm_with(Box::new([Instruction::Insert, Instruction::Halt]));
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
        let mut vm = vm_with(Box::new([Instruction::Insert, Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::Array(vec![Value::Int(1)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(5));
        vm.stack.push(Value::Int(99));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_insert_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instruction::Insert, Instruction::Halt]));
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(2));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_insert_on_a_non_array_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instruction::Insert, Instruction::Halt]));
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
        let mut vm = vm_with(Box::new([Instruction::Remove, Instruction::Halt]));
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
        let mut vm = vm_with(Box::new([Instruction::Remove, Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::Array(vec![Value::Int(1)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(5));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_remove_on_a_non_array_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instruction::Remove, Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_len_returns_the_number_of_array_elements() {
        let mut vm = vm_with(Box::new([Instruction::Len, Instruction::Halt]));
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
        let mut vm = vm_with(Box::new([Instruction::Len, Instruction::Halt]));
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
        let mut vm = vm_with(Box::new([Instruction::Len, Instruction::Halt]));
        let id = vm.heap.insert(HeapObject::String("hello".into()));
        vm.stack.push(Value::Ref(id));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(5)));
    }

    #[test]
    fn vm_len_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instruction::Len, Instruction::Halt]));
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_pop_instruction_removes_the_top_value() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Pop,
            Instruction::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_halt_stops_execution_before_later_instructions_run() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Int(1)),
            Instruction::Halt,
            Instruction::Push(Value::Int(2)),
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_add_on_an_empty_stack_returns_stack_underflow() {
        let mut vm = vm_with(Box::new([Instruction::Add, Instruction::Halt]));

        assert_eq!(vm.execute(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_add_with_mismatched_types_propagates_the_value_error() {
        let mut vm = vm_with(Box::new([
            Instruction::Push(Value::Bool(true)),
            Instruction::Push(Value::Int(1)),
            Instruction::Add,
            Instruction::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }
}
