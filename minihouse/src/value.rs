use crate::types::DataType;

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

#[cfg(test)]
mod tests {
    use crate::types::DataType;
    use crate::value::Value;

    #[test]
    fn value_data_type_matches_variant() {
        assert_eq!(Value::Int64(1).data_type(), DataType::Int64);
        assert_eq!(Value::Float64(1.0).data_type(), DataType::Float64);
        assert_eq!(Value::String("a".into()).data_type(), DataType::String);
    }
}
