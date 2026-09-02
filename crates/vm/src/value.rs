use std::ops::{Add, BitAnd, BitOr, BitXor, Div, Mul, Neg, Not, Rem, Shl, Shr, Sub};

use thiserror::Error;

use crate::HeapId;

/// A value
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Ref(HeapId),
}

/// An error produced by a fallible operation on a `Value`.
#[derive(Debug, Error, PartialEq)]
pub enum ValueError {
    #[error("type mismatch")]
    TypeMismatch,
    #[error("division by zero")]
    DivisionByZero,
    #[error("shift amount out of range")]
    ShiftOverflow,
    #[error("arithmetic overflow")]
    Overflow,
}

impl Add for Value {
    type Output = Result<Value, ValueError>;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => {
                a.checked_add(b).map(Value::Int).ok_or(ValueError::Overflow)
            }
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl Sub for Value {
    type Output = Result<Value, ValueError>;

    fn sub(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => {
                a.checked_sub(b).map(Value::Int).ok_or(ValueError::Overflow)
            }
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl Mul for Value {
    type Output = Result<Value, ValueError>;

    fn mul(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => {
                a.checked_mul(b).map(Value::Int).ok_or(ValueError::Overflow)
            }
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl Div for Value {
    type Output = Result<Value, ValueError>;

    fn div(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Value::Int(_), Value::Int(0)) => Err(ValueError::DivisionByZero),
            (Value::Int(a), Value::Int(b)) => {
                a.checked_div(b).map(Value::Int).ok_or(ValueError::Overflow)
            }
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl Rem for Value {
    type Output = Result<Value, ValueError>;

    fn rem(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Value::Int(_), Value::Int(0)) => Err(ValueError::DivisionByZero),
            (Value::Int(a), Value::Int(b)) => {
                a.checked_rem(b).map(Value::Int).ok_or(ValueError::Overflow)
            }
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a % b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl BitXor for Value {
    type Output = Result<Value, ValueError>;

    fn bitxor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a ^ b)),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a ^ b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl BitAnd for Value {
    type Output = Result<Value, ValueError>;

    fn bitand(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a & b)),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a & b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl BitOr for Value {
    type Output = Result<Value, ValueError>;

    fn bitor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a | b)),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a | b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl Shl for Value {
    type Output = Result<Value, ValueError>;

    fn shl(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => {
                let shift = u32::try_from(b).map_err(|_| ValueError::ShiftOverflow)?;
                a.checked_shl(shift)
                    .map(Value::Int)
                    .ok_or(ValueError::ShiftOverflow)
            }
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl Shr for Value {
    type Output = Result<Value, ValueError>;

    fn shr(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => {
                let shift = u32::try_from(b).map_err(|_| ValueError::ShiftOverflow)?;
                a.checked_shr(shift)
                    .map(Value::Int)
                    .ok_or(ValueError::ShiftOverflow)
            }
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl Not for Value {
    type Output = Result<Value, ValueError>;

    fn not(self) -> Self::Output {
        match self {
            Value::Bool(a) => Ok(Value::Bool(!a)),
            Value::Int(a) => Ok(Value::Int(!a)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl Neg for Value {
    type Output = Result<Value, ValueError>;

    fn neg(self) -> Self::Output {
        match self {
            Value::Int(a) => a.checked_neg().map(Value::Int).ok_or(ValueError::Overflow),
            Value::Float(a) => Ok(Value::Float(-a)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl Value {
    pub fn eq(self, rhs: Self) -> Result<Value, ValueError> {
        match (self, rhs) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a == b)),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a == b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a == b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    pub fn ne(self, rhs: Self) -> Result<Value, ValueError> {
        match (self, rhs) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a != b)),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a != b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a != b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    pub fn lt(self, rhs: Self) -> Result<Value, ValueError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    pub fn le(self, rhs: Self) -> Result<Value, ValueError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    pub fn gt(self, rhs: Self) -> Result<Value, ValueError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    pub fn ge(self, rhs: Self) -> Result<Value, ValueError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_ints_returns_their_sum() {
        assert_eq!(Value::Int(1) + Value::Int(2), Ok(Value::Int(3)));
    }

    #[test]
    fn adding_floats_returns_their_sum() {
        assert_eq!(Value::Float(1.5) + Value::Float(2.5), Ok(Value::Float(4.0)));
    }

    #[test]
    fn adding_mismatched_types_returns_type_mismatch() {
        assert_eq!(
            Value::Int(1) + Value::Bool(true),
            Err(ValueError::TypeMismatch)
        );
    }

    #[test]
    fn adding_ints_that_overflow_returns_overflow() {
        assert_eq!(
            Value::Int(i64::MAX) + Value::Int(1),
            Err(ValueError::Overflow)
        );
    }

    #[test]
    fn subtracting_ints_returns_their_difference() {
        assert_eq!(Value::Int(5) - Value::Int(3), Ok(Value::Int(2)));
    }

    #[test]
    fn subtracting_ints_that_overflow_returns_overflow() {
        assert_eq!(
            Value::Int(i64::MIN) - Value::Int(1),
            Err(ValueError::Overflow)
        );
    }

    #[test]
    fn multiplying_ints_returns_their_product() {
        assert_eq!(Value::Int(4) * Value::Int(3), Ok(Value::Int(12)));
    }

    #[test]
    fn multiplying_ints_that_overflow_returns_overflow() {
        assert_eq!(
            Value::Int(i64::MAX) * Value::Int(2),
            Err(ValueError::Overflow)
        );
    }

    #[test]
    fn dividing_ints_returns_their_quotient() {
        assert_eq!(Value::Int(6) / Value::Int(3), Ok(Value::Int(2)));
    }

    #[test]
    fn dividing_int_by_zero_returns_division_by_zero() {
        assert_eq!(Value::Int(1) / Value::Int(0), Err(ValueError::DivisionByZero));
    }

    #[test]
    fn dividing_int_min_by_negative_one_returns_overflow() {
        assert_eq!(
            Value::Int(i64::MIN) / Value::Int(-1),
            Err(ValueError::Overflow)
        );
    }

    #[test]
    fn dividing_float_by_zero_returns_infinity_instead_of_erroring() {
        assert_eq!(
            Value::Float(1.0) / Value::Float(0.0),
            Ok(Value::Float(f64::INFINITY))
        );
    }

    #[test]
    fn remainder_of_ints_returns_the_remainder() {
        assert_eq!(Value::Int(7) % Value::Int(2), Ok(Value::Int(1)));
    }

    #[test]
    fn remainder_of_int_by_zero_returns_division_by_zero() {
        assert_eq!(Value::Int(1) % Value::Int(0), Err(ValueError::DivisionByZero));
    }

    #[test]
    fn remainder_of_int_min_by_negative_one_returns_overflow() {
        assert_eq!(
            Value::Int(i64::MIN) % Value::Int(-1),
            Err(ValueError::Overflow)
        );
    }

    #[test]
    fn bitxor_of_bools_returns_their_logical_xor() {
        assert_eq!(
            Value::Bool(true) ^ Value::Bool(false),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn bitxor_of_ints_returns_their_bitwise_xor() {
        assert_eq!(Value::Int(0b110) ^ Value::Int(0b011), Ok(Value::Int(0b101)));
    }

    #[test]
    fn bitand_of_ints_returns_their_bitwise_and() {
        assert_eq!(Value::Int(0b110) & Value::Int(0b011), Ok(Value::Int(0b010)));
    }

    #[test]
    fn bitor_of_ints_returns_their_bitwise_or() {
        assert_eq!(Value::Int(0b100) | Value::Int(0b001), Ok(Value::Int(0b101)));
    }

    #[test]
    fn shl_shifts_bits_left() {
        assert_eq!(Value::Int(1) << Value::Int(4), Ok(Value::Int(16)));
    }

    #[test]
    fn shl_with_negative_shift_returns_shift_overflow() {
        assert_eq!(
            Value::Int(1) << Value::Int(-1),
            Err(ValueError::ShiftOverflow)
        );
    }

    #[test]
    fn shl_with_shift_amount_too_large_returns_shift_overflow() {
        assert_eq!(
            Value::Int(1) << Value::Int(64),
            Err(ValueError::ShiftOverflow)
        );
    }

    #[test]
    fn shr_shifts_bits_right() {
        assert_eq!(Value::Int(16) >> Value::Int(4), Ok(Value::Int(1)));
    }

    #[test]
    fn not_of_bool_negates_it() {
        assert_eq!(!Value::Bool(true), Ok(Value::Bool(false)));
    }

    #[test]
    fn not_of_int_returns_its_bitwise_complement() {
        assert_eq!(!Value::Int(0), Ok(Value::Int(-1)));
    }

    #[test]
    fn not_of_float_returns_type_mismatch() {
        assert_eq!(!Value::Float(1.0), Err(ValueError::TypeMismatch));
    }

    #[test]
    fn neg_of_int_negates_it() {
        assert_eq!(-Value::Int(5), Ok(Value::Int(-5)));
    }

    #[test]
    fn neg_of_int_min_returns_overflow() {
        assert_eq!(-Value::Int(i64::MIN), Err(ValueError::Overflow));
    }

    #[test]
    fn neg_of_float_negates_it() {
        assert_eq!(-Value::Float(1.5), Ok(Value::Float(-1.5)));
    }

    #[test]
    fn neg_of_bool_returns_type_mismatch() {
        assert_eq!(-Value::Bool(true), Err(ValueError::TypeMismatch));
    }

    #[test]
    fn eq_of_equal_ints_returns_true() {
        assert_eq!(Value::Int(1).eq(Value::Int(1)), Ok(Value::Bool(true)));
    }

    #[test]
    fn eq_of_different_ints_returns_false() {
        assert_eq!(Value::Int(1).eq(Value::Int(2)), Ok(Value::Bool(false)));
    }

    #[test]
    fn eq_of_equal_floats_returns_true() {
        assert_eq!(Value::Float(1.5).eq(Value::Float(1.5)), Ok(Value::Bool(true)));
    }

    #[test]
    fn eq_of_equal_bools_returns_true() {
        assert_eq!(
            Value::Bool(true).eq(Value::Bool(true)),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn eq_of_mismatched_types_returns_type_mismatch() {
        assert_eq!(
            Value::Int(1).eq(Value::Bool(true)),
            Err(ValueError::TypeMismatch)
        );
    }

    #[test]
    fn ne_of_different_ints_returns_true() {
        assert_eq!(Value::Int(1).ne(Value::Int(2)), Ok(Value::Bool(true)));
    }

    #[test]
    fn ne_of_equal_ints_returns_false() {
        assert_eq!(Value::Int(1).ne(Value::Int(1)), Ok(Value::Bool(false)));
    }

    #[test]
    fn ne_of_mismatched_types_returns_type_mismatch() {
        assert_eq!(
            Value::Int(1).ne(Value::Bool(true)),
            Err(ValueError::TypeMismatch)
        );
    }

    #[test]
    fn lt_of_smaller_int_returns_true() {
        assert_eq!(Value::Int(1).lt(Value::Int(2)), Ok(Value::Bool(true)));
    }

    #[test]
    fn lt_of_larger_int_returns_false() {
        assert_eq!(Value::Int(2).lt(Value::Int(1)), Ok(Value::Bool(false)));
    }

    #[test]
    fn lt_of_bools_returns_type_mismatch() {
        assert_eq!(
            Value::Bool(true).lt(Value::Bool(false)),
            Err(ValueError::TypeMismatch)
        );
    }

    #[test]
    fn le_of_equal_ints_returns_true() {
        assert_eq!(Value::Int(1).le(Value::Int(1)), Ok(Value::Bool(true)));
    }

    #[test]
    fn le_of_larger_int_returns_false() {
        assert_eq!(Value::Int(2).le(Value::Int(1)), Ok(Value::Bool(false)));
    }

    #[test]
    fn gt_of_larger_int_returns_true() {
        assert_eq!(Value::Int(2).gt(Value::Int(1)), Ok(Value::Bool(true)));
    }

    #[test]
    fn gt_of_smaller_int_returns_false() {
        assert_eq!(Value::Int(1).gt(Value::Int(2)), Ok(Value::Bool(false)));
    }

    #[test]
    fn ge_of_equal_ints_returns_true() {
        assert_eq!(Value::Int(1).ge(Value::Int(1)), Ok(Value::Bool(true)));
    }

    #[test]
    fn ge_of_smaller_int_returns_false() {
        assert_eq!(Value::Int(1).ge(Value::Int(2)), Ok(Value::Bool(false)));
    }
}
