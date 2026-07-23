#![warn(clippy::all)]

pub mod aggregate;
mod block;
mod column;
mod helpers;
pub mod query;
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
