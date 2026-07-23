use crate::DataType;
use crate::aggregate::{Aggregate, Count, MaxF64, MaxI64, MinF64, MinI64, SumF64, SumI64};

#[derive(Debug, Clone, Copy)]
pub enum AggKind {
    Count,
    Sum,
    Min,
    Max,
}

pub fn make_aggregate(kind: AggKind, dt: DataType) -> Box<dyn Aggregate> {
    match (kind, dt) {
        (AggKind::Count, _) => Box::new(Count::default()),

        (AggKind::Sum, DataType::Int64) => Box::new(SumI64::default()),
        (AggKind::Sum, DataType::Float64) => Box::new(SumF64::default()),
        (AggKind::Sum, DataType::String) => panic!("sum over string column"),

        (AggKind::Min, DataType::Int64) => Box::new(MinI64::default()),
        (AggKind::Min, DataType::Float64) => Box::new(MinF64::default()),
        (AggKind::Min, DataType::String) => {
            panic!("min over string column: string ordering semantics not defined yet")
        }

        (AggKind::Max, DataType::Int64) => Box::new(MaxI64::default()),
        (AggKind::Max, DataType::Float64) => Box::new(MaxF64::default()),
        (AggKind::Max, DataType::String) => {
            panic!("max over string column: string ordering semantics not defined yet")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::value::Value;

    #[test]
    fn make_aggregate_count_int64_counts_rows() {
        let mut agg = make_aggregate(AggKind::Count, DataType::Int64);
        agg.update(&Column::Int64(vec![1, 2, 3]));
        assert_eq!(agg.result(), Some(Value::Int64(3)));
    }

    #[test]
    fn make_aggregate_count_float64_counts_rows() {
        let mut agg = make_aggregate(AggKind::Count, DataType::Float64);
        agg.update(&Column::Float64(vec![1.0, 2.0]));
        assert_eq!(agg.result(), Some(Value::Int64(2)));
    }

    #[test]
    fn make_aggregate_count_string_counts_rows() {
        let mut agg = make_aggregate(AggKind::Count, DataType::String);
        agg.update(&Column::String(vec!["a".into(), "b".into(), "c".into()]));
        assert_eq!(agg.result(), Some(Value::Int64(3)));
    }

    #[test]
    fn make_aggregate_sum_int64_sums_column() {
        let mut agg = make_aggregate(AggKind::Sum, DataType::Int64);
        agg.update(&Column::Int64(vec![1, 2, 3]));
        assert_eq!(agg.result(), Some(Value::Int64(6)));
    }

    #[test]
    fn make_aggregate_sum_float64_sums_column() {
        let mut agg = make_aggregate(AggKind::Sum, DataType::Float64);
        agg.update(&Column::Float64(vec![1.5, 2.5]));
        assert_eq!(agg.result(), Some(Value::Float64(4.0)));
    }

    #[test]
    fn make_aggregate_min_int64_computes_min() {
        let mut agg = make_aggregate(AggKind::Min, DataType::Int64);
        agg.update(&Column::Int64(vec![3, 1, 2]));
        assert_eq!(agg.result(), Some(Value::Int64(1)));
    }

    #[test]
    fn make_aggregate_min_float64_computes_min() {
        let mut agg = make_aggregate(AggKind::Min, DataType::Float64);
        agg.update(&Column::Float64(vec![3.0, 1.5, 2.0]));
        assert_eq!(agg.result(), Some(Value::Float64(1.5)));
    }

    #[test]
    fn make_aggregate_max_int64_computes_max() {
        let mut agg = make_aggregate(AggKind::Max, DataType::Int64);
        agg.update(&Column::Int64(vec![3, 1, 2]));
        assert_eq!(agg.result(), Some(Value::Int64(3)));
    }

    #[test]
    fn make_aggregate_max_float64_computes_max() {
        let mut agg = make_aggregate(AggKind::Max, DataType::Float64);
        agg.update(&Column::Float64(vec![3.0, 1.5, 2.0]));
        assert_eq!(agg.result(), Some(Value::Float64(3.0)));
    }

    #[test]
    #[should_panic(expected = "sum over string column")]
    fn make_aggregate_sum_string_panics() {
        make_aggregate(AggKind::Sum, DataType::String);
    }

    #[test]
    #[should_panic(expected = "string ordering semantics not defined yet")]
    fn make_aggregate_min_string_panics() {
        make_aggregate(AggKind::Min, DataType::String);
    }

    #[test]
    #[should_panic(expected = "string ordering semantics not defined yet")]
    fn make_aggregate_max_string_panics() {
        make_aggregate(AggKind::Max, DataType::String);
    }
}
