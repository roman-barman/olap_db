#![warn(clippy::all)]
#![allow(clippy::manual_slice_size_calculation)]
#![deny(unreachable_pub)]

pub mod aggregate;
mod core;
pub mod query;
mod storage;
mod table;
#[cfg(test)]
mod test_fixture;

pub use core::{Block, Column, DataType, Schema, Value};
pub use storage::Codec;
pub use table::Table;
