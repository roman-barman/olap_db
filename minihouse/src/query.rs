use crate::column::Column;
use crate::types::DataType;

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum CmpOp {
    Gt,
    Lt,
    Eq,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int64(i64),
    Float64(f64),
    String(String),
}

impl Value {
    pub fn data_type(&self) -> DataType {
        match self {
            Value::Int64(_) => DataType::Int64,
            Value::Float64(_) => DataType::Float64,
            Value::String(_) => DataType::String,
        }
    }
}

pub fn eval_predicate(col: &Column, op: CmpOp, value: &Value) -> Vec<bool> {
    assert_eq!(
        col.data_type(),
        value.data_type(),
        "Data types of column '{:?}' and value '{:?}' do not match",
        col.data_type(),
        value.data_type()
    );

    if value.data_type() == DataType::String && op != CmpOp::Eq {
        panic!("Unsupported comparison {op:?} for string values");
    }

    match (col, value) {
        (Column::Int64(v), Value::Int64(x)) => match op {
            CmpOp::Gt => cmp_loop(v, |a| a > x),
            CmpOp::Lt => cmp_loop(v, |a| a < x),
            CmpOp::Eq => cmp_loop(v, |a| a == x),
        },
        (Column::Float64(v), Value::Float64(x)) => match op {
            CmpOp::Gt => cmp_loop(v, |a| a > x),
            CmpOp::Lt => cmp_loop(v, |a| a < x),
            CmpOp::Eq => cmp_loop(v, |a| a == x),
        },
        (Column::String(v), Value::String(x)) => match op {
            CmpOp::Eq => cmp_loop(v, |a| a == x),
            _ => unreachable!("Unsupported comparison {op:?} for string values"),
        },
        _ => unreachable!(
            "Type mismatch survived assert: {:?} vs {:?}",
            value.data_type(),
            col.data_type()
        ),
    }
}

fn cmp_loop<T, F>(v: &[T], f: F) -> Vec<bool>
where
    F: Fn(&T) -> bool,
{
    v.iter().map(f).collect()
}
