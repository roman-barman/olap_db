use crate::string_column::StringColumn;
use crate::{Block, Column, DataType, Schema};

pub(crate) fn sample_schema() -> Schema {
    Schema::new(vec![
        ("id".to_string(), DataType::Int64),
        ("name".to_string(), DataType::String),
        ("score".to_string(), DataType::Float64),
    ])
    .unwrap()
}

pub(crate) fn sample_block(ids: &[i64], names: &[&str], scores: &[f64]) -> Block {
    assert_eq!(ids.len(), names.len());
    assert_eq!(ids.len(), scores.len());
    Block::new(
        vec![
            ("id".to_string(), Column::Int64(ids.to_vec())),
            (
                "name".to_string(),
                Column::String(StringColumn::new_with_values(names)),
            ),
            ("score".to_string(), Column::Float64(scores.to_vec())),
        ],
        ids.len(),
    )
}
