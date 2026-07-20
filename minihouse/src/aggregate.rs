use crate::column::Column;
use crate::types::DataType;
use crate::value::Value;

pub trait Aggregate {
    fn update(&mut self, col: &Column);
    fn result(&self) -> Option<Value>;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Count {
    count: usize,
}

impl Aggregate for Count {
    fn update(&mut self, col: &Column) {
        self.count += col.len()
    }

    fn result(&self) -> Option<Value> {
        Some(Value::Int64(
            i64::try_from(self.count).expect("count overflows i64"),
        ))
    }
}

#[derive(Debug, Clone, Default)]
pub struct SumI64 {
    sum: i64,
}

impl Aggregate for SumI64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Int64(v) => {
                self.sum = v
                    .iter()
                    .try_fold(self.sum, |a, &x| a.checked_add(x))
                    .expect("sum overflows i64");
            }
            other => panic!("SumI64: expected Int64 column, got {:?}", other.data_type()),
        }
    }

    fn result(&self) -> Option<Value> {
        Some(Value::Int64(self.sum))
    }
}

#[derive(Debug, Clone, Default)]
pub struct SumF64 {
    sum: f64,
}

impl Aggregate for SumF64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Float64(v) => {
                self.sum += v.iter().sum::<f64>();
            }
            other => panic!(
                "SumF64: expected Float64 column, got {:?}",
                other.data_type()
            ),
        }
    }

    fn result(&self) -> Option<Value> {
        Some(Value::Float64(self.sum))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DataType;

    #[test]
    fn count_default_result_is_zero() {
        let count = Count::default();
        assert_eq!(count.result(), Some(Value::Int64(0)));
    }

    #[test]
    fn count_update_int64_column_adds_len() {
        let mut count = Count::default();
        count.update(&Column::Int64(vec![1, 2, 3]));
        assert_eq!(count.result(), Some(Value::Int64(3)));
    }

    #[test]
    fn count_update_float64_column_adds_len() {
        let mut count = Count::default();
        count.update(&Column::Float64(vec![1.0, 2.0]));
        assert_eq!(count.result(), Some(Value::Int64(2)));
    }

    #[test]
    fn count_update_string_column_adds_len() {
        let mut count = Count::default();
        count.update(&Column::String(vec![
            "a".into(),
            "b".into(),
            "c".into(),
            "d".into(),
        ]));
        assert_eq!(count.result(), Some(Value::Int64(4)));
    }

    #[test]
    fn count_update_accumulates_across_multiple_calls() {
        let mut count = Count::default();
        count.update(&Column::Int64(vec![1, 2, 3]));
        count.update(&Column::Int64(vec![4, 5]));
        assert_eq!(count.result(), Some(Value::Int64(5)));
    }

    #[test]
    fn count_update_with_empty_column_is_noop() {
        let mut count = Count::default();
        count.update(&Column::Int64(vec![1, 2]));
        count.update(&Column::new(DataType::Int64));
        assert_eq!(count.result(), Some(Value::Int64(2)));
    }

    #[test]
    fn count_result_is_idempotent_without_update() {
        let mut count = Count::default();
        count.update(&Column::Int64(vec![1, 2, 3]));
        assert_eq!(count.result(), Some(Value::Int64(3)));
        assert_eq!(count.result(), Some(Value::Int64(3)));
    }

    #[test]
    #[should_panic(expected = "count overflows i64")]
    fn count_result_panics_on_usize_overflow() {
        let count = Count { count: usize::MAX };
        count.result();
    }

    #[test]
    fn count_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(Count::default());
        agg.update(&Column::Int64(vec![1, 2, 3]));
        agg.update(&Column::Float64(vec![1.0]));
        assert_eq!(agg.result(), Some(Value::Int64(4)));
    }

    #[test]
    fn sum_i64_default_result_is_zero() {
        let sum = SumI64::default();
        assert_eq!(sum.result(), Some(Value::Int64(0)));
    }

    #[test]
    fn sum_i64_update_int64_column_computes_sum() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![1, 2, 3]));
        assert_eq!(sum.result(), Some(Value::Int64(6)));
    }

    #[test]
    fn sum_i64_update_accumulates_across_multiple_calls() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![1, 2, 3]));
        sum.update(&Column::Int64(vec![4, 5]));
        assert_eq!(sum.result(), Some(Value::Int64(15)));
    }

    #[test]
    fn sum_i64_update_with_negative_numbers() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![10, -3, -2]));
        assert_eq!(sum.result(), Some(Value::Int64(5)));
    }

    #[test]
    fn sum_i64_update_with_empty_column_is_noop() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![1, 2]));
        sum.update(&Column::new(DataType::Int64));
        assert_eq!(sum.result(), Some(Value::Int64(3)));
    }

    #[test]
    #[should_panic(expected = "SumI64: expected Int64 column")]
    fn sum_i64_update_panics_on_wrong_column_type() {
        let mut sum = SumI64::default();
        sum.update(&Column::Float64(vec![1.0]));
    }

    #[test]
    #[should_panic(expected = "sum overflows i64")]
    fn sum_i64_update_panics_on_i64_overflow() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![i64::MAX, 1]));
    }

    #[test]
    fn sum_i64_result_is_idempotent_without_update() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![1, 2, 3]));
        assert_eq!(sum.result(), Some(Value::Int64(6)));
        assert_eq!(sum.result(), Some(Value::Int64(6)));
    }

    #[test]
    fn sum_i64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(SumI64::default());
        agg.update(&Column::Int64(vec![1, 2, 3]));
        agg.update(&Column::Int64(vec![4]));
        assert_eq!(agg.result(), Some(Value::Int64(10)));
    }

    #[test]
    fn sum_f64_default_result_is_zero() {
        let sum = SumF64::default();
        assert_eq!(sum.result(), Some(Value::Float64(0.0)));
    }

    #[test]
    fn sum_f64_update_float64_column_computes_sum() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![1.5, 2.5, 3.0]));
        assert_eq!(sum.result(), Some(Value::Float64(7.0)));
    }

    #[test]
    fn sum_f64_update_accumulates_across_multiple_calls() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![1.5, 2.5]));
        sum.update(&Column::Float64(vec![1.0]));
        assert_eq!(sum.result(), Some(Value::Float64(5.0)));
    }

    #[test]
    fn sum_f64_update_with_negative_numbers() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![10.0, -3.5, -2.5]));
        assert_eq!(sum.result(), Some(Value::Float64(4.0)));
    }

    #[test]
    fn sum_f64_update_with_empty_column_is_noop() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![1.5, 2.5]));
        sum.update(&Column::new(DataType::Float64));
        assert_eq!(sum.result(), Some(Value::Float64(4.0)));
    }

    #[test]
    #[should_panic(expected = "SumF64: expected Float64 column")]
    fn sum_f64_update_panics_on_wrong_column_type() {
        let mut sum = SumF64::default();
        sum.update(&Column::Int64(vec![1]));
    }

    #[test]
    fn sum_f64_update_does_not_panic_on_overflow_saturates_to_infinity() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![f64::MAX, f64::MAX]));
        match sum.result() {
            Some(Value::Float64(v)) => assert!(v.is_infinite()),
            other => panic!("expected Some(Value::Float64(_)), got {:?}", other),
        }
    }

    #[test]
    fn sum_f64_result_is_idempotent_without_update() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![1.5, 2.5]));
        assert_eq!(sum.result(), Some(Value::Float64(4.0)));
        assert_eq!(sum.result(), Some(Value::Float64(4.0)));
    }

    #[test]
    fn sum_f64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(SumF64::default());
        agg.update(&Column::Float64(vec![1.5, 2.5]));
        agg.update(&Column::Float64(vec![1.0]));
        assert_eq!(agg.result(), Some(Value::Float64(5.0)));
    }
}
