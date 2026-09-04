#![allow(unused)]
//! The virtual machine which executes roo code.

use std::sync::Arc;

use slotmap::{SlotMap, new_key_type};
use thiserror::Error;

use crate::instructions::Instr;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FnId(usize);

mod instructions;
mod value;

/// An error produced while executing a program.
#[derive(Debug, Error, PartialEq)]
pub enum VmError {
    /// A pop was attempted on an empty stack.
    #[error("stack underflow")]
    StackUnderflow,
    /// A pop or peek was attempted on an empty call stack.
    #[error("call stack underflow")]
    CallStackUnderflow,
    /// A heap reference did not point to a live object.
    #[error("invalid heap reference")]
    InvalidHeapRef,
    /// A function id did not point to a function proto in the unit.
    #[error("invalid function reference")]
    InvalidFnRef,
    /// A local variable index did not point to a valid stack slot for the current frame.
    #[error("invalid local index")]
    InvalidLocalIndex,
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

    call_stack: CallStack,

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
            call_stack: CallStack::new(),
            heap,
            program,
            strings,
        }
    }

    fn fetch(&mut self) -> Instr {
        let instr = self.program.fetch(self.ip);
        self.ip = self.ip.next();
        instr
    }

    fn new_call_frame(&mut self, offset: usize) -> CallFrame {
        CallFrame::new(self.stack.top() - offset, self.ip)
    }

    fn execute(&mut self) -> Result<(), VmError> {
        loop {
            let instr = self.fetch();
            match instr {
                Instr::Nop => {}
                Instr::Halt => break,
                Instr::Push(value) => self.stack.push(value),
                Instr::Pop => {
                    self.stack.pop()?;
                }
                Instr::Jump(target) => self.jump(target),
                Instr::JumpIfFalse(target) => {
                    if !self.stack.pop_bool()? {
                        self.jump(target);
                    }
                }
                Instr::LoadString(string_id) => {
                    let id = self.strings[string_id.0];
                    self.stack.push(Value::Ref(id));
                }
                Instr::Concat => {
                    let b = self.stack.pop_ref()?;
                    let a = self.stack.pop_ref()?;

                    let a = self.heap.get_string(a)?;
                    let b = self.heap.get_string(b)?;

                    let r = self
                        .heap
                        .insert(HeapObject::String(format!("{a}{b}").into()));

                    self.stack.push(Value::Ref(r));
                }
                Instr::Slice(start, end) => {
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
                Instr::Add => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a + b)?);
                }
                Instr::Sub => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a - b)?);
                }
                Instr::Mul => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a * b)?);
                }
                Instr::Div => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a / b)?);
                }
                Instr::Rem => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a % b)?);
                }
                Instr::BitXor => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a ^ b)?);
                }
                Instr::BitAnd => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a & b)?);
                }
                Instr::BitOr => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a | b)?);
                }
                Instr::Shl => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a << b)?);
                }
                Instr::Shr => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push((a >> b)?);
                }
                Instr::Not => {
                    let a = self.stack.pop()?;
                    self.stack.push((!a)?);
                }
                Instr::Neg => {
                    let a = self.stack.pop()?;
                    self.stack.push((-a)?);
                }
                Instr::Is => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(Value::Bool(a == b));
                }
                Instr::Eq => {
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
                Instr::Ne => {
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
                Instr::Lt => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(a.lt(b)?);
                }
                Instr::Le => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(a.le(b)?);
                }
                Instr::Gt => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(a.gt(b)?);
                }
                Instr::Ge => {
                    let b = self.stack.pop()?;
                    let a = self.stack.pop()?;
                    self.stack.push(a.ge(b)?);
                }
                Instr::Dup => {
                    let value = *self.stack.peek()?;
                    self.stack.push(value);
                }
                Instr::Array(count) => {
                    let mut elements = Vec::with_capacity(count);
                    for _ in 0..count {
                        elements.push(self.stack.pop()?);
                    }
                    elements.reverse();
                    let id = self.heap.insert(HeapObject::Array(elements));
                    self.stack.push(Value::Ref(id));
                }
                Instr::Index => {
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
                Instr::IndexMut => {
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
                Instr::Insert => {
                    let value = self.stack.pop()?;
                    let index = self.stack.pop_index()?;
                    let id = self.stack.pop_ref()?;
                    let elements = self.heap.get_array_mut(id)?;
                    if index > elements.len() {
                        return Err(VmError::IndexOutOfBounds);
                    }
                    elements.insert(index, value);
                }
                Instr::Remove => {
                    let index = self.stack.pop_index()?;
                    let id = self.stack.pop_ref()?;
                    let elements = self.heap.get_array_mut(id)?;
                    if index >= elements.len() {
                        return Err(VmError::IndexOutOfBounds);
                    }
                    elements.remove(index);
                }
                Instr::Len => {
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
                Instr::Call => {
                    let id = self.stack.pop_index()?;
                    // Assume that all arguments to the function are
                    // under the fn id which is popped above
                    let proto = self.program.get_fn(FnId(id))?;
                    let entry = proto.entry;
                    let offset = proto.arity;
                    let frame = self.new_call_frame(offset);
                    self.call_stack.push(frame);

                    self.jump(entry);
                }
                Instr::Ret => {
                    let frame = self.call_stack.pop()?;
                    let ret = self.stack.pop()?;

                    debug_assert!(self.stack.top() >= frame.top);
                    self.stack.truncate(frame.top);
                    self.stack.push(ret);

                    self.jump(frame.savedpc);
                }
                Instr::GetLocal(offset) => {
                    let frame = self.call_stack.peek()?;
                    let local = self.stack.get(frame.top + offset)?;

                    self.stack.push(*local);
                }
                Instr::SetLocal(offset) => {
                    let value = self.stack.pop()?;
                    let frame = self.call_stack.peek()?;

                    self.stack.set(frame.top + offset, value)?;
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
    instrs: Box<[Instr]>,
    strings: Box<[Box<str>]>,
    fns: Box<[FnProto]>,
}

impl Unit {
    fn new(instrs: Box<[Instr]>, strings: Box<[Box<str>]>, fns: Box<[FnProto]>) -> Self {
        Self {
            instrs,
            strings,
            fns,
        }
    }

    fn fetch(&self, addr: InstrAddr) -> Instr {
        self.instrs[addr.0]
    }

    fn strings(&self) -> &[Box<str>] {
        &self.strings
    }

    fn fns(&self) -> &[FnProto] {
        &self.fns
    }

    fn get_fn(&self, id: FnId) -> Result<&FnProto, VmError> {
        self.fns.get(id.0).ok_or(VmError::InvalidFnRef)
    }
}

impl From<Box<[Instr]>> for Unit {
    fn from(value: Box<[Instr]>) -> Self {
        Self {
            instrs: value,
            strings: Box::new([]),
            fns: Box::new([]),
        }
    }
}

pub struct FnProto {
    entry: InstrAddr,
    arity: usize,
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

    fn top(&self) -> usize {
        self.values.len()
    }

    fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
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

    fn get(&self, offset: usize) -> Result<&Value, VmError> {
        self.values.get(offset).ok_or(VmError::InvalidLocalIndex)
    }

    fn set(&mut self, offset: usize, value: Value) -> Result<(), VmError> {
        let slot = self
            .values
            .get_mut(offset)
            .ok_or(VmError::InvalidLocalIndex)?;
        *slot = value;
        Ok(())
    }
}

pub struct CallStack {
    frames: Vec<CallFrame>,
}

impl CallStack {
    fn new() -> Self {
        Self { frames: Vec::new() }
    }

    fn push(&mut self, frame: CallFrame) {
        self.frames.push(frame)
    }

    fn pop(&mut self) -> Result<CallFrame, VmError> {
        self.frames.pop().ok_or(VmError::CallStackUnderflow)
    }

    fn peek(&self) -> Result<&CallFrame, VmError> {
        self.frames.last().ok_or(VmError::CallStackUnderflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallFrame {
    top: usize,
    savedpc: InstrAddr,
}

impl CallFrame {
    fn new(top: usize, savedpc: InstrAddr) -> Self {
        Self { top, savedpc }
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
    /// A closure.
    Closure { fn_id: FnId, captures: Box<[Value]> },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm_with(instrs: Box<[Instr]>) -> Vm {
        Vm::new(Unit::from(instrs))
    }

    #[test]
    fn vm_new_allocates_the_units_strings_into_the_heap() {
        let unit = Unit::new(
            Box::new([Instr::Halt]),
            Box::new(["hello".into(), "world".into()]),
            Box::new([]),
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
            Box::new([Instr::LoadString(StringId(0)), Instr::Halt]),
            Box::new(["hello".into()]),
            Box::new([]),
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
                Instr::LoadString(StringId(0)),
                Instr::LoadString(StringId(0)),
                Instr::Halt,
            ]),
            Box::new(["hello".into()]),
            Box::new([]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), vm.stack.pop());
    }

    #[test]
    fn vm_new_leaves_the_units_own_strings_empty_after_moving_them() {
        let unit = Unit::new(
            Box::new([Instr::Halt]),
            Box::new(["hello".into()]),
            Box::new([]),
        );

        let vm = Vm::new(unit);

        assert_eq!(vm.program.strings(), &[] as &[Box<str>]);
    }

    #[test]
    fn vm_concat_joins_two_strings_and_pushes_a_new_ref() {
        let unit = Unit::new(
            Box::new([
                Instr::LoadString(StringId(0)),
                Instr::LoadString(StringId(1)),
                Instr::Concat,
                Instr::Halt,
            ]),
            Box::new(["foo".into(), "bar".into()]),
            Box::new([]),
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
        let mut vm = vm_with(Box::new([Instr::Concat, Instr::Halt]));
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(2));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_concat_on_a_non_string_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instr::Concat, Instr::Halt]));
        let array_id = vm.heap.insert(HeapObject::Array(vec![]));
        let string_id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(array_id));
        vm.stack.push(Value::Ref(string_id));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_slice_returns_a_substring() {
        let mut vm = vm_with(Box::new([Instr::Slice(0, 0), Instr::Halt]));
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
        let mut vm = vm_with(Box::new([Instr::Slice(0, 0), Instr::Halt]));
        let id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(5));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_slice_with_start_after_end_returns_an_error() {
        let mut vm = vm_with(Box::new([Instr::Slice(0, 0), Instr::Halt]));
        let id = vm.heap.insert(HeapObject::String("hello".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(3));
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_slice_off_a_char_boundary_returns_invalid_utf8_boundary() {
        let mut vm = vm_with(Box::new([Instr::Slice(0, 0), Instr::Halt]));
        let id = vm.heap.insert(HeapObject::String("é".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::InvalidUtf8Boundary));
    }

    #[test]
    fn vm_slice_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instr::Slice(0, 0), Instr::Halt]));
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
    fn call_stack_pop_on_empty_call_stack_returns_call_stack_underflow() {
        let mut call_stack = CallStack::new();

        assert_eq!(call_stack.pop(), Err(VmError::CallStackUnderflow));
    }

    #[test]
    fn call_stack_peek_on_empty_call_stack_returns_call_stack_underflow() {
        let call_stack = CallStack::new();

        assert_eq!(call_stack.peek(), Err(VmError::CallStackUnderflow));
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
        let mut vm = vm_with(Box::new([Instr::Nop, Instr::Halt]));

        assert_eq!(vm.ip, InstrAddr(0));
        vm.fetch();
        assert_eq!(vm.ip, InstrAddr(1));
    }

    #[test]
    fn vm_jump_sets_the_instruction_pointer_directly() {
        let mut vm = vm_with(Box::new([Instr::Nop, Instr::Halt]));
        vm.ip = InstrAddr(5);

        vm.jump(InstrAddr(2));

        assert_eq!(vm.ip, InstrAddr(2));
    }

    #[test]
    fn vm_jump_instruction_skips_over_the_instructions_in_between() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Jump(InstrAddr(3)),
            Instr::Push(Value::Int(99)),
            Instr::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_jump_instruction_can_jump_backward() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Jump(InstrAddr(4)),
            Instr::Push(Value::Int(2)),
            Instr::Halt,
            Instr::Jump(InstrAddr(2)),
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(2)));
        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
    }

    #[test]
    fn vm_jump_if_false_jumps_when_condition_is_false() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Bool(false)),
            Instr::JumpIfFalse(InstrAddr(4)),
            Instr::Push(Value::Int(99)),
            Instr::Halt,
            Instr::Push(Value::Int(1)),
            Instr::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_jump_if_false_does_not_jump_when_condition_is_true() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Bool(true)),
            Instr::JumpIfFalse(InstrAddr(4)),
            Instr::Push(Value::Int(1)),
            Instr::Halt,
            Instr::Push(Value::Int(99)),
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_jump_if_false_with_non_bool_condition_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(0)),
            Instr::JumpIfFalse(InstrAddr(3)),
            Instr::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_dup_pushes_a_copy_of_the_top_value() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Dup,
            Instr::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_dup_leaves_values_beneath_it_untouched() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Push(Value::Int(2)),
            Instr::Dup,
            Instr::Add,
            Instr::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(4)));
        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
    }

    #[test]
    fn vm_dup_on_an_empty_stack_returns_stack_underflow() {
        let mut vm = vm_with(Box::new([Instr::Dup, Instr::Halt]));

        assert_eq!(vm.execute(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_nop_leaves_the_stack_unchanged() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Nop,
            Instr::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_executes_add_and_leaves_the_result_on_the_stack() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Push(Value::Int(2)),
            Instr::Add,
            Instr::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(3)));
    }

    #[test]
    fn vm_lt_feeding_jump_if_false_implements_an_if_condition() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Push(Value::Int(2)),
            Instr::Lt,
            Instr::JumpIfFalse(InstrAddr(6)),
            Instr::Push(Value::Int(42)),
            Instr::Halt,
            Instr::Push(Value::Int(0)),
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(42)));
    }

    #[test]
    fn vm_is_returns_true_for_equal_scalars() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Push(Value::Int(1)),
            Instr::Is,
            Instr::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(true)));
    }

    #[test]
    fn vm_is_returns_false_for_different_scalars() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Push(Value::Int(2)),
            Instr::Is,
            Instr::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(false)));
    }

    #[test]
    fn vm_is_returns_true_for_the_same_heap_reference() {
        let mut vm = vm_with(Box::new([Instr::Is, Instr::Halt]));
        let id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Ref(id));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(true)));
    }

    #[test]
    fn vm_is_returns_false_for_different_refs_with_equal_string_content() {
        let mut vm = vm_with(Box::new([Instr::Is, Instr::Halt]));
        let a = vm.heap.insert(HeapObject::String("hi".into()));
        let b = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(false)));
    }

    #[test]
    fn vm_eq_returns_true_for_different_refs_with_equal_string_content() {
        let mut vm = vm_with(Box::new([Instr::Eq, Instr::Halt]));
        let a = vm.heap.insert(HeapObject::String("hi".into()));
        let b = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(true)));
    }

    #[test]
    fn vm_eq_returns_false_for_refs_with_different_string_content() {
        let mut vm = vm_with(Box::new([Instr::Eq, Instr::Halt]));
        let a = vm.heap.insert(HeapObject::String("hi".into()));
        let b = vm.heap.insert(HeapObject::String("bye".into()));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(false)));
    }

    #[test]
    fn vm_eq_on_a_non_string_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instr::Eq, Instr::Halt]));
        let a = vm.heap.insert(HeapObject::Array(vec![]));
        let b = vm.heap.insert(HeapObject::Array(vec![]));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_ne_returns_false_for_different_refs_with_equal_string_content() {
        let mut vm = vm_with(Box::new([Instr::Ne, Instr::Halt]));
        let a = vm.heap.insert(HeapObject::String("hi".into()));
        let b = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(a));
        vm.stack.push(Value::Ref(b));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Bool(false)));
    }

    #[test]
    fn vm_ne_returns_true_for_refs_with_different_string_content() {
        let mut vm = vm_with(Box::new([Instr::Ne, Instr::Halt]));
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
            Instr::Push(Value::Int(1)),
            Instr::Push(Value::Int(2)),
            Instr::Push(Value::Int(3)),
            Instr::Array(3),
            Instr::Halt,
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
        let mut vm = vm_with(Box::new([Instr::Array(0), Instr::Halt]));

        vm.execute().unwrap();

        let Value::Ref(id) = vm.stack.pop().unwrap() else {
            panic!("expected a Value::Ref");
        };
        assert_eq!(vm.heap.data.get(id), Some(&HeapObject::Array(vec![])));
    }

    #[test]
    fn vm_index_gets_an_array_element() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Push(Value::Int(2)),
            Instr::Push(Value::Int(3)),
            Instr::Array(3),
            Instr::Push(Value::Int(1)),
            Instr::Index,
            Instr::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(2)));
    }

    #[test]
    fn vm_index_gets_an_adt_field() {
        let mut vm = vm_with(Box::new([Instr::Index, Instr::Halt]));
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
            Instr::Push(Value::Int(1)),
            Instr::Array(1),
            Instr::Push(Value::Int(5)),
            Instr::Index,
            Instr::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_index_with_negative_index_returns_overflow() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Array(1),
            Instr::Push(Value::Int(-1)),
            Instr::Index,
            Instr::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::Overflow)));
    }

    #[test]
    fn vm_index_with_a_non_int_index_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Array(1),
            Instr::Push(Value::Bool(true)),
            Instr::Index,
            Instr::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_index_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Push(Value::Int(0)),
            Instr::Index,
            Instr::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_index_on_a_non_indexable_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instr::Index, Instr::Halt]));
        let id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_index_mut_sets_an_array_element() {
        let mut vm = vm_with(Box::new([Instr::IndexMut, Instr::Halt]));
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
        let mut vm = vm_with(Box::new([Instr::IndexMut, Instr::Halt]));
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
        let mut vm = vm_with(Box::new([Instr::IndexMut, Instr::Halt]));
        let id = vm.heap.insert(HeapObject::Array(vec![Value::Int(1)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(5));
        vm.stack.push(Value::Int(99));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_index_mut_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instr::IndexMut, Instr::Halt]));
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(99));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_insert_at_the_end_acts_like_append() {
        let mut vm = vm_with(Box::new([Instr::Insert, Instr::Halt]));
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
        let mut vm = vm_with(Box::new([Instr::Insert, Instr::Halt]));
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
        let mut vm = vm_with(Box::new([Instr::Insert, Instr::Halt]));
        let id = vm.heap.insert(HeapObject::Array(vec![Value::Int(1)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(5));
        vm.stack.push(Value::Int(99));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_insert_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instr::Insert, Instr::Halt]));
        vm.stack.push(Value::Int(1));
        vm.stack.push(Value::Int(0));
        vm.stack.push(Value::Int(2));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_insert_on_a_non_array_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instr::Insert, Instr::Halt]));
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
        let mut vm = vm_with(Box::new([Instr::Remove, Instr::Halt]));
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
        let mut vm = vm_with(Box::new([Instr::Remove, Instr::Halt]));
        let id = vm.heap.insert(HeapObject::Array(vec![Value::Int(1)]));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(5));

        assert_eq!(vm.execute(), Err(VmError::IndexOutOfBounds));
    }

    #[test]
    fn vm_remove_on_a_non_array_heap_object_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instr::Remove, Instr::Halt]));
        let id = vm.heap.insert(HeapObject::String("hi".into()));
        vm.stack.push(Value::Ref(id));
        vm.stack.push(Value::Int(0));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_len_returns_the_number_of_array_elements() {
        let mut vm = vm_with(Box::new([Instr::Len, Instr::Halt]));
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
        let mut vm = vm_with(Box::new([Instr::Len, Instr::Halt]));
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
        let mut vm = vm_with(Box::new([Instr::Len, Instr::Halt]));
        let id = vm.heap.insert(HeapObject::String("hello".into()));
        vm.stack.push(Value::Ref(id));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(5)));
    }

    #[test]
    fn vm_len_on_a_non_ref_value_returns_type_mismatch() {
        let mut vm = vm_with(Box::new([Instr::Len, Instr::Halt]));
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }

    #[test]
    fn vm_call_executes_a_zero_arity_function_and_returns_its_value() {
        let unit = Unit::new(
            Box::new([
                Instr::Push(Value::Int(0)),
                Instr::Call,
                Instr::Halt,
                Instr::Push(Value::Int(42)),
                Instr::Ret,
            ]),
            Box::new([]),
            Box::new([FnProto {
                entry: InstrAddr(3),
                arity: 0,
            }]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(42)));
    }

    #[test]
    fn vm_call_passes_arguments_to_the_function_via_the_stack() {
        let unit = Unit::new(
            Box::new([
                Instr::Push(Value::Int(5)),
                Instr::Push(Value::Int(0)),
                Instr::Call,
                Instr::Halt,
                Instr::Dup,
                Instr::Push(Value::Int(1)),
                Instr::Add,
                Instr::Ret,
            ]),
            Box::new([]),
            Box::new([FnProto {
                entry: InstrAddr(4),
                arity: 1,
            }]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(6)));
    }

    #[test]
    fn vm_call_discards_the_functions_locals_and_arguments_on_return() {
        let unit = Unit::new(
            Box::new([
                Instr::Push(Value::Int(1)),
                Instr::Push(Value::Int(2)),
                Instr::Push(Value::Int(0)),
                Instr::Call,
                Instr::Halt,
                Instr::Push(Value::Int(99)),
                Instr::Push(Value::Int(42)),
                Instr::Ret,
            ]),
            Box::new([]),
            Box::new([FnProto {
                entry: InstrAddr(5),
                arity: 2,
            }]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(42)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_call_with_an_invalid_fn_id_returns_invalid_fn_ref() {
        let mut vm = vm_with(Box::new([Instr::Call, Instr::Halt]));
        vm.stack.push(Value::Int(0));

        assert_eq!(vm.execute(), Err(VmError::InvalidFnRef));
    }

    #[test]
    fn vm_ret_on_an_empty_call_stack_returns_call_stack_underflow() {
        let mut vm = vm_with(Box::new([Instr::Ret, Instr::Halt]));

        assert_eq!(vm.execute(), Err(VmError::CallStackUnderflow));
    }

    #[test]
    fn vm_get_local_reads_a_value_that_is_not_on_top_of_the_stack() {
        // fn(x, y) { x + y }, called as f(3, 4)
        let unit = Unit::new(
            Box::new([
                Instr::Push(Value::Int(3)),
                Instr::Push(Value::Int(4)),
                Instr::Push(Value::Int(0)),
                Instr::Call,
                Instr::Halt,
                Instr::GetLocal(0),
                Instr::GetLocal(1),
                Instr::Add,
                Instr::Ret,
            ]),
            Box::new([]),
            Box::new([FnProto {
                entry: InstrAddr(5),
                arity: 2,
            }]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(7)));
    }

    #[test]
    fn vm_set_local_overwrites_a_value_that_a_later_get_local_observes() {
        // fn(x) { x = x + 100; x }, called as f(5)
        let unit = Unit::new(
            Box::new([
                Instr::Push(Value::Int(5)),
                Instr::Push(Value::Int(0)),
                Instr::Call,
                Instr::Halt,
                Instr::GetLocal(0),
                Instr::Push(Value::Int(100)),
                Instr::Add,
                Instr::SetLocal(0),
                Instr::GetLocal(0),
                Instr::Ret,
            ]),
            Box::new([]),
            Box::new([FnProto {
                entry: InstrAddr(4),
                arity: 1,
            }]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(105)));
    }

    #[test]
    fn vm_get_local_with_no_active_call_frame_returns_call_stack_underflow() {
        let mut vm = vm_with(Box::new([Instr::GetLocal(0), Instr::Halt]));

        assert_eq!(vm.execute(), Err(VmError::CallStackUnderflow));
    }

    #[test]
    fn vm_get_local_with_an_offset_past_the_stack_returns_invalid_local_index() {
        let mut vm = vm_with(Box::new([Instr::GetLocal(5), Instr::Halt]));
        vm.call_stack.push(CallFrame::new(0, InstrAddr(0)));

        assert_eq!(vm.execute(), Err(VmError::InvalidLocalIndex));
    }

    #[test]
    fn vm_set_local_with_no_active_call_frame_returns_call_stack_underflow() {
        let mut vm = vm_with(Box::new([Instr::SetLocal(0), Instr::Halt]));
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::CallStackUnderflow));
    }

    #[test]
    fn vm_set_local_with_an_offset_past_the_stack_returns_invalid_local_index() {
        let mut vm = vm_with(Box::new([Instr::SetLocal(5), Instr::Halt]));
        vm.call_stack.push(CallFrame::new(0, InstrAddr(0)));
        vm.stack.push(Value::Int(1));

        assert_eq!(vm.execute(), Err(VmError::InvalidLocalIndex));
    }

    #[test]
    fn vm_call_supports_a_function_calling_another_function() {
        // main: a() -> a: 10 + b() -> b: 5
        let unit = Unit::new(
            Box::new([
                Instr::Push(Value::Int(0)), // 0: call a
                Instr::Call,
                Instr::Halt,
                Instr::Push(Value::Int(10)), // 3: a's body
                Instr::Push(Value::Int(1)),  // call b
                Instr::Call,
                Instr::Add,
                Instr::Ret,
                Instr::Push(Value::Int(5)), // 8: b's body
                Instr::Ret,
            ]),
            Box::new([]),
            Box::new([
                FnProto {
                    entry: InstrAddr(3),
                    arity: 0,
                },
                FnProto {
                    entry: InstrAddr(8),
                    arity: 0,
                },
            ]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(15)));
    }

    #[test]
    fn vm_call_supports_mutually_recursive_functions() {
        // fn is_even(n) { if n == 0 { 1 } else { is_odd(n - 1) } }
        // fn is_odd(n)  { if n == 0 { 0 } else { is_even(n - 1) } }
        // main: is_even(4)
        let is_even_id = FnId(0);
        let is_odd_id = FnId(1);

        let is_even_entry = InstrAddr(4);
        let is_even_else = InstrAddr(11);
        let is_odd_entry = InstrAddr(16);
        let is_odd_else = InstrAddr(23);

        let unit = Unit::new(
            Box::new([
                Instr::Push(Value::Int(4)), // 0: main: is_even(4)
                Instr::Push(Value::Int(is_even_id.0 as i64)),
                Instr::Call,
                Instr::Halt,
                Instr::Dup, // 4: is_even(n)
                Instr::Push(Value::Int(0)),
                Instr::Eq,
                Instr::JumpIfFalse(is_even_else),
                Instr::Pop, // 8: base case: n == 0
                Instr::Push(Value::Int(1)),
                Instr::Ret,
                Instr::Push(Value::Int(1)), // 11: is_even_else: is_odd(n - 1)
                Instr::Sub,
                Instr::Push(Value::Int(is_odd_id.0 as i64)),
                Instr::Call,
                Instr::Ret,
                Instr::Dup, // 16: is_odd(n)
                Instr::Push(Value::Int(0)),
                Instr::Eq,
                Instr::JumpIfFalse(is_odd_else),
                Instr::Pop, // 20: base case: n == 0
                Instr::Push(Value::Int(0)),
                Instr::Ret,
                Instr::Push(Value::Int(1)), // 23: is_odd_else: is_even(n - 1)
                Instr::Sub,
                Instr::Push(Value::Int(is_even_id.0 as i64)),
                Instr::Call,
                Instr::Ret,
            ]),
            Box::new([]),
            Box::new([
                FnProto {
                    entry: is_even_entry,
                    arity: 1,
                },
                FnProto {
                    entry: is_odd_entry,
                    arity: 1,
                },
            ]),
        );

        let mut vm = Vm::new(unit);
        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
    }

    #[test]
    fn vm_pop_instruction_removes_the_top_value() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Pop,
            Instr::Halt,
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_halt_stops_execution_before_later_instructions_run() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Int(1)),
            Instr::Halt,
            Instr::Push(Value::Int(2)),
        ]));

        vm.execute().unwrap();

        assert_eq!(vm.stack.pop(), Ok(Value::Int(1)));
        assert_eq!(vm.stack.pop(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_add_on_an_empty_stack_returns_stack_underflow() {
        let mut vm = vm_with(Box::new([Instr::Add, Instr::Halt]));

        assert_eq!(vm.execute(), Err(VmError::StackUnderflow));
    }

    #[test]
    fn vm_add_with_mismatched_types_propagates_the_value_error() {
        let mut vm = vm_with(Box::new([
            Instr::Push(Value::Bool(true)),
            Instr::Push(Value::Int(1)),
            Instr::Add,
            Instr::Halt,
        ]));

        assert_eq!(vm.execute(), Err(VmError::Value(ValueError::TypeMismatch)));
    }
}
