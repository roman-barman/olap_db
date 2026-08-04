#![warn(clippy::all)]

pub mod aggregate;
mod block;
mod codec;
mod column;
mod column_io;
mod helpers;
mod part_writer;
pub mod query;
mod string_column;
mod table;
mod value;

pub use block::Block;
pub use column::Column;
pub use table::Table;
pub use value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Int64,
    Float64,
    String,
}

impl DataType {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            DataType::Int64 => "Int64",
            DataType::Float64 => "Float64",
            DataType::String => "String",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn as_str_matches_on_disk_schema_tokens() {
        assert_eq!(DataType::Int64.as_str(), "Int64");
        assert_eq!(DataType::Float64.as_str(), "Float64");
        assert_eq!(DataType::String.as_str(), "String");
    }
}
