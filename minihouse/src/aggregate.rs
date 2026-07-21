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

#[derive(Debug, Clone, Default)]
pub struct MinI64 {
    min: Option<i64>,
}

impl Aggregate for MinI64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Int64(v) => {
                let block_min = v.iter().copied().min();
                self.min = match (self.min, block_min) {
                    (None, m) => m,
                    (m, None) => m,
                    (Some(a), Some(b)) => Some(a.min(b)),
                };
            }
            other => panic!("MinI64: expected Int64 column, got {:?}", other.data_type()),
        }
    }

    fn result(&self) -> Option<Value> {
        self.min.map(Value::Int64)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MaxI64 {
    max: Option<i64>,
}

impl Aggregate for MaxI64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Int64(v) => {
                let block_max = v.iter().copied().max();
                self.max = match (self.max, block_max) {
                    (None, m) => m,
                    (m, None) => m,
                    (Some(a), Some(b)) => Some(a.max(b)),
                };
            }
            other => panic!("MaxI64: expected Int64 column, got {:?}", other.data_type()),
        }
    }

    fn result(&self) -> Option<Value> {
        self.max.map(Value::Int64)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MinF64 {
    min: Option<f64>,
}

impl Aggregate for MinF64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Float64(v) => {
                let block_min = v.iter().copied().min_by(|a, b| a.total_cmp(b));
                self.min = match (self.min, block_min) {
                    (None, m) => m,
                    (m, None) => m,
                    (Some(a), Some(b)) => Some(if a.total_cmp(&b).is_le() { a } else { b }),
                };
            }
            other => panic!(
                "MinF64: expected Float64 column, got {:?}",
                other.data_type()
            ),
        }
    }

    fn result(&self) -> Option<Value> {
        self.min.map(Value::Float64)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MaxF64 {
    max: Option<f64>,
}

impl Aggregate for MaxF64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Float64(v) => {
                let block_max = v.iter().copied().max_by(|a, b| a.total_cmp(b));
                self.max = match (self.max, block_max) {
                    (None, m) => m,
                    (m, None) => m,
                    (Some(a), Some(b)) => Some(if a.total_cmp(&b).is_ge() { a } else { b }),
                };
            }
            other => panic!(
                "MaxF64: expected Float64 column, got {:?}",
                other.data_type()
            ),
        }
    }

    fn result(&self) -> Option<Value> {
        self.max.map(Value::Float64)
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

    #[test]
    fn min_i64_default_result_is_none() {
        let min = MinI64::default();
        assert_eq!(min.result(), None);
    }

    #[test]
    fn min_i64_update_single_column_computes_min() {
        let mut min = MinI64::default();
        min.update(&Column::Int64(vec![3, 1, 2]));
        assert_eq!(min.result(), Some(Value::Int64(1)));
    }

    #[test]
    fn min_i64_update_accumulates_across_multiple_calls() {
        let mut min = MinI64::default();
        min.update(&Column::Int64(vec![5, 3]));
        min.update(&Column::Int64(vec![4, 1, 9]));
        assert_eq!(min.result(), Some(Value::Int64(1)));
    }

    #[test]
    fn min_i64_update_with_negative_numbers() {
        let mut min = MinI64::default();
        min.update(&Column::Int64(vec![10, -3, -2]));
        assert_eq!(min.result(), Some(Value::Int64(-3)));
    }

    #[test]
    fn min_i64_update_with_empty_column_is_noop() {
        let mut min = MinI64::default();
        min.update(&Column::Int64(vec![5, 2]));
        min.update(&Column::new(DataType::Int64));
        assert_eq!(min.result(), Some(Value::Int64(2)));
    }

    #[test]
    fn min_i64_update_empty_column_before_data_is_none_then_updates() {
        let mut min = MinI64::default();
        min.update(&Column::new(DataType::Int64));
        assert_eq!(min.result(), None);
        min.update(&Column::Int64(vec![7, 2, 9]));
        assert_eq!(min.result(), Some(Value::Int64(2)));
    }

    #[test]
    #[should_panic(expected = "MinI64: expected Int64 column")]
    fn min_i64_update_panics_on_wrong_column_type() {
        let mut min = MinI64::default();
        min.update(&Column::Float64(vec![1.0]));
    }

    #[test]
    fn min_i64_result_is_idempotent_without_update() {
        let mut min = MinI64::default();
        min.update(&Column::Int64(vec![3, 1, 2]));
        assert_eq!(min.result(), Some(Value::Int64(1)));
        assert_eq!(min.result(), Some(Value::Int64(1)));
    }

    #[test]
    fn min_i64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(MinI64::default());
        agg.update(&Column::Int64(vec![5, 3]));
        agg.update(&Column::Int64(vec![1]));
        assert_eq!(agg.result(), Some(Value::Int64(1)));
    }

    #[test]
    fn max_i64_default_result_is_none() {
        let max = MaxI64::default();
        assert_eq!(max.result(), None);
    }

    #[test]
    fn max_i64_update_single_column_computes_max() {
        let mut max = MaxI64::default();
        max.update(&Column::Int64(vec![3, 1, 2]));
        assert_eq!(max.result(), Some(Value::Int64(3)));
    }

    #[test]
    fn max_i64_update_accumulates_across_multiple_calls() {
        let mut max = MaxI64::default();
        max.update(&Column::Int64(vec![5, 3]));
        max.update(&Column::Int64(vec![4, 9, 1]));
        assert_eq!(max.result(), Some(Value::Int64(9)));
    }

    #[test]
    fn max_i64_update_with_negative_numbers() {
        let mut max = MaxI64::default();
        max.update(&Column::Int64(vec![-10, -3, -2]));
        assert_eq!(max.result(), Some(Value::Int64(-2)));
    }

    #[test]
    fn max_i64_update_with_empty_column_is_noop() {
        let mut max = MaxI64::default();
        max.update(&Column::Int64(vec![5, 2]));
        max.update(&Column::new(DataType::Int64));
        assert_eq!(max.result(), Some(Value::Int64(5)));
    }

    #[test]
    fn max_i64_update_empty_column_before_data_is_none_then_updates() {
        let mut max = MaxI64::default();
        max.update(&Column::new(DataType::Int64));
        assert_eq!(max.result(), None);
        max.update(&Column::Int64(vec![7, 2, 9]));
        assert_eq!(max.result(), Some(Value::Int64(9)));
    }

    #[test]
    #[should_panic(expected = "MaxI64: expected Int64 column")]
    fn max_i64_update_panics_on_wrong_column_type() {
        let mut max = MaxI64::default();
        max.update(&Column::Float64(vec![1.0]));
    }

    #[test]
    fn max_i64_result_is_idempotent_without_update() {
        let mut max = MaxI64::default();
        max.update(&Column::Int64(vec![3, 1, 2]));
        assert_eq!(max.result(), Some(Value::Int64(3)));
        assert_eq!(max.result(), Some(Value::Int64(3)));
    }

    #[test]
    fn max_i64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(MaxI64::default());
        agg.update(&Column::Int64(vec![5, 3]));
        agg.update(&Column::Int64(vec![9]));
        assert_eq!(agg.result(), Some(Value::Int64(9)));
    }

    #[test]
    fn min_f64_default_result_is_none() {
        let min = MinF64::default();
        assert_eq!(min.result(), None);
    }

    #[test]
    fn min_f64_update_single_column_computes_min() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![3.0, 1.5, 2.0]));
        assert_eq!(min.result(), Some(Value::Float64(1.5)));
    }

    #[test]
    fn min_f64_update_accumulates_across_multiple_calls() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![5.0, 3.0]));
        min.update(&Column::Float64(vec![4.0, 1.5, 9.0]));
        assert_eq!(min.result(), Some(Value::Float64(1.5)));
    }

    #[test]
    fn min_f64_update_with_negative_numbers() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![10.0, -3.5, -2.0]));
        assert_eq!(min.result(), Some(Value::Float64(-3.5)));
    }

    #[test]
    fn min_f64_update_with_empty_column_is_noop() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![5.0, 2.0]));
        min.update(&Column::new(DataType::Float64));
        assert_eq!(min.result(), Some(Value::Float64(2.0)));
    }

    #[test]
    fn min_f64_update_empty_column_before_data_is_none_then_updates() {
        let mut min = MinF64::default();
        min.update(&Column::new(DataType::Float64));
        assert_eq!(min.result(), None);
        min.update(&Column::Float64(vec![7.0, 2.0, 9.0]));
        assert_eq!(min.result(), Some(Value::Float64(2.0)));
    }

    #[test]
    #[should_panic(expected = "MinF64: expected Float64 column")]
    fn min_f64_update_panics_on_wrong_column_type() {
        let mut min = MinF64::default();
        min.update(&Column::Int64(vec![1]));
    }

    #[test]
    fn min_f64_result_is_idempotent_without_update() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![3.0, 1.5, 2.0]));
        assert_eq!(min.result(), Some(Value::Float64(1.5)));
        assert_eq!(min.result(), Some(Value::Float64(1.5)));
    }

    #[test]
    fn min_f64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(MinF64::default());
        agg.update(&Column::Float64(vec![5.0, 3.0]));
        agg.update(&Column::Float64(vec![1.5]));
        assert_eq!(agg.result(), Some(Value::Float64(1.5)));
    }

    #[test]
    fn min_f64_update_with_nan_is_ignored_by_total_cmp() {
        // `total_cmp` orders NaN as greater than all other values (positive NaN
        // sorts above +inf), so a NaN in the input never becomes the min as
        // long as at least one non-NaN value is present.
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![f64::NAN, 5.0, 1.0]));
        assert_eq!(min.result(), Some(Value::Float64(1.0)));
    }

    #[test]
    fn min_f64_update_with_infinities() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
        ]));
        assert_eq!(min.result(), Some(Value::Float64(f64::NEG_INFINITY)));
    }

    #[test]
    fn min_f64_update_distinguishes_negative_zero() {
        // total_cmp orders -0.0 < 0.0, unlike `==`, so assert on sign bit
        // rather than PartialEq (under which -0.0 == 0.0).
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![0.0, -0.0]));
        match min.result() {
            Some(Value::Float64(v)) => assert!(v.is_sign_negative()),
            other => panic!("expected Some(Value::Float64(_)), got {:?}", other),
        }
    }

    #[test]
    fn max_f64_default_result_is_none() {
        let max = MaxF64::default();
        assert_eq!(max.result(), None);
    }

    #[test]
    fn max_f64_update_single_column_computes_max() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![3.0, 1.5, 2.0]));
        assert_eq!(max.result(), Some(Value::Float64(3.0)));
    }

    #[test]
    fn max_f64_update_accumulates_across_multiple_calls() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![5.0, 3.0]));
        max.update(&Column::Float64(vec![4.0, 9.5, 1.0]));
        assert_eq!(max.result(), Some(Value::Float64(9.5)));
    }

    #[test]
    fn max_f64_update_with_negative_numbers() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![-10.0, -3.5, -2.0]));
        assert_eq!(max.result(), Some(Value::Float64(-2.0)));
    }

    #[test]
    fn max_f64_update_with_empty_column_is_noop() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![5.0, 2.0]));
        max.update(&Column::new(DataType::Float64));
        assert_eq!(max.result(), Some(Value::Float64(5.0)));
    }

    #[test]
    fn max_f64_update_empty_column_before_data_is_none_then_updates() {
        let mut max = MaxF64::default();
        max.update(&Column::new(DataType::Float64));
        assert_eq!(max.result(), None);
        max.update(&Column::Float64(vec![7.0, 2.0, 9.0]));
        assert_eq!(max.result(), Some(Value::Float64(9.0)));
    }

    #[test]
    #[should_panic(expected = "MaxF64: expected Float64 column")]
    fn max_f64_update_panics_on_wrong_column_type() {
        let mut max = MaxF64::default();
        max.update(&Column::Int64(vec![1]));
    }

    #[test]
    fn max_f64_result_is_idempotent_without_update() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![3.0, 1.5, 2.0]));
        assert_eq!(max.result(), Some(Value::Float64(3.0)));
        assert_eq!(max.result(), Some(Value::Float64(3.0)));
    }

    #[test]
    fn max_f64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(MaxF64::default());
        agg.update(&Column::Float64(vec![5.0, 3.0]));
        agg.update(&Column::Float64(vec![9.5]));
        assert_eq!(agg.result(), Some(Value::Float64(9.5)));
    }

    #[test]
    fn max_f64_update_with_nan_total_cmp_ranks_it_highest() {
        // Unlike MinF64, `total_cmp` ranks (positive, quiet) NaN as greater
        // than +inf and every finite value, so a NaN in the input becomes
        // the max — the opposite of IEEE `<`/`>` comparisons, where every
        // NaN comparison is false.
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![f64::NAN, 5.0, 1.0]));
        match max.result() {
            Some(Value::Float64(v)) => assert!(v.is_nan()),
            other => panic!("expected Some(Value::Float64(_)), got {:?}", other),
        }
    }

    #[test]
    fn max_f64_update_with_infinities() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
        ]));
        assert_eq!(max.result(), Some(Value::Float64(f64::INFINITY)));
    }

    #[test]
    fn max_f64_update_distinguishes_negative_zero() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![0.0, -0.0]));
        match max.result() {
            Some(Value::Float64(v)) => assert!(v.is_sign_positive()),
            other => panic!("expected Some(Value::Float64(_)), got {:?}", other),
        }
    }
}
