#![warn(clippy::all)]

pub mod aggregate;
pub mod aggregate_factory;
pub mod block;
pub mod column;
pub mod execute;
pub mod helpers;
pub mod query;
pub mod table;
pub mod value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Int64,
    Float64,
    String,
}
