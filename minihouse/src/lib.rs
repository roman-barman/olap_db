#![warn(clippy::all)]
#![allow(clippy::manual_slice_size_calculation)]

pub mod aggregate;
mod codec;
mod column_io;
mod core;
mod part_reader;
mod part_writer;
pub mod query;
mod storage_error;
mod table;
#[cfg(test)]
mod test_fixture;

pub use codec::Codec;
pub use core::{Block, Column, Schema, Value};
pub use table::Table;
