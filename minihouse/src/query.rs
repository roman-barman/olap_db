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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_predicate_int64_eq_returns_matching_mask() {
        let col = Column::Int64(vec![1, 2, 3, 2]);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::Int64(2));
        assert_eq!(mask, vec![false, true, false, true]);
    }

    #[test]
    fn eval_predicate_int64_gt_returns_matching_mask() {
        let col = Column::Int64(vec![1, 2, 3, 2]);
        let mask = eval_predicate(&col, CmpOp::Gt, &Value::Int64(2));
        assert_eq!(mask, vec![false, false, true, false]);
    }

    #[test]
    fn eval_predicate_int64_lt_returns_matching_mask() {
        let col = Column::Int64(vec![1, 2, 3, 2]);
        let mask = eval_predicate(&col, CmpOp::Lt, &Value::Int64(2));
        assert_eq!(mask, vec![true, false, false, false]);
    }

    #[test]
    fn eval_predicate_float64_eq_returns_matching_mask() {
        let col = Column::Float64(vec![1.0, 2.5, 3.0, 2.5]);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::Float64(2.5));
        assert_eq!(mask, vec![false, true, false, true]);
    }

    #[test]
    fn eval_predicate_float64_gt_returns_matching_mask() {
        let col = Column::Float64(vec![1.0, 2.5, 3.0, 2.5]);
        let mask = eval_predicate(&col, CmpOp::Gt, &Value::Float64(2.5));
        assert_eq!(mask, vec![false, false, true, false]);
    }

    #[test]
    fn eval_predicate_float64_lt_returns_matching_mask() {
        let col = Column::Float64(vec![1.0, 2.5, 3.0, 2.5]);
        let mask = eval_predicate(&col, CmpOp::Lt, &Value::Float64(2.5));
        assert_eq!(mask, vec![true, false, false, false]);
    }

    #[test]
    fn eval_predicate_string_eq_returns_matching_mask() {
        let col = Column::String(vec!["a".into(), "b".into(), "a".into()]);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::String("a".into()));
        assert_eq!(mask, vec![true, false, true]);
    }

    #[test]
    #[should_panic(expected = "Unsupported comparison")]
    fn eval_predicate_string_gt_panics() {
        let col = Column::String(vec!["a".into()]);
        eval_predicate(&col, CmpOp::Gt, &Value::String("a".into()));
    }

    #[test]
    #[should_panic(expected = "Unsupported comparison")]
    fn eval_predicate_string_lt_panics() {
        let col = Column::String(vec!["a".into()]);
        eval_predicate(&col, CmpOp::Lt, &Value::String("a".into()));
    }

    #[test]
    #[should_panic(expected = "do not match")]
    fn eval_predicate_int64_column_float64_value_panics() {
        let col = Column::Int64(vec![1, 2, 3]);
        eval_predicate(&col, CmpOp::Eq, &Value::Float64(1.0));
    }

    #[test]
    #[should_panic(expected = "do not match")]
    fn eval_predicate_int64_column_string_value_panics() {
        let col = Column::Int64(vec![1, 2, 3]);
        eval_predicate(&col, CmpOp::Eq, &Value::String("1".into()));
    }

    #[test]
    #[should_panic(expected = "do not match")]
    fn eval_predicate_string_column_int64_value_panics() {
        let col = Column::String(vec!["a".into()]);
        eval_predicate(&col, CmpOp::Eq, &Value::Int64(1));
    }

    #[test]
    fn eval_predicate_on_empty_int64_column_returns_empty_mask() {
        let col = Column::new(DataType::Int64);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::Int64(1));
        assert_eq!(mask, Vec::<bool>::new());
    }

    #[test]
    fn eval_predicate_on_empty_float64_column_returns_empty_mask() {
        let col = Column::new(DataType::Float64);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::Float64(1.0));
        assert_eq!(mask, Vec::<bool>::new());
    }

    #[test]
    fn eval_predicate_on_empty_string_column_returns_empty_mask() {
        let col = Column::new(DataType::String);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::String("a".into()));
        assert_eq!(mask, Vec::<bool>::new());
    }

    #[test]
    fn value_data_type_matches_variant() {
        assert_eq!(Value::Int64(1).data_type(), DataType::Int64);
        assert_eq!(Value::Float64(1.0).data_type(), DataType::Float64);
        assert_eq!(Value::String("a".into()).data_type(), DataType::String);
    }
}
