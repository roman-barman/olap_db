use crate::column_vs_row::row_table::{Row, RowTable};
use minihouse::Column;
use minihouse::DataType;
use minihouse::{Block, Codec};
use rand::prelude::StdRng;
use rand::{RngExt, SeedableRng};

pub(super) fn generate(n: usize, block_size: usize) -> (Vec<Block>, RowTable) {
    let mut rng = StdRng::seed_from_u64(42);

    let mut row_table = RowTable {
        rows: Vec::with_capacity(n),
    };

    let mut id_column = Column::with_capacity(DataType::Int64, block_size);
    let mut ts_column = Column::with_capacity(DataType::Int64, block_size);
    let mut url_column = Column::with_capacity(DataType::String, block_size);
    let mut dur_column = Column::with_capacity(DataType::Int64, block_size);
    let mut blocks = Vec::with_capacity(n / block_size + 1);

    for i in 0..n {
        let id = i as i64;
        let ts = rng.random_range(0..1_000_000);
        let url = format!("/page/{}", rng.random_range(0..1000));
        let dur = rng.random_range(1..10_000);

        row_table.rows.push(Row {
            id,
            ts,
            url: url.clone(),
            dur,
        });

        id_column.push_i64(id);
        ts_column.push_i64(ts);
        url_column.push_str(url.as_str());
        dur_column.push_i64(dur);

        if id_column.len() == block_size {
            blocks.push(make_block(id_column, ts_column, url_column, dur_column));

            id_column = Column::with_capacity(DataType::Int64, block_size);
            ts_column = Column::with_capacity(DataType::Int64, block_size);
            url_column = Column::with_capacity(DataType::String, block_size);
            dur_column = Column::with_capacity(DataType::Int64, block_size);
        }
    }

    blocks.push(make_block(id_column, ts_column, url_column, dur_column));

    (blocks, row_table)
}

fn make_block(id: Column, ts: Column, url: Column, dur: Column) -> Block {
    let n = id.len();
    Block::new(
        vec![
            ("id".into(), id),
            ("ts".into(), ts),
            ("url".into(), url),
            ("dur".into(), dur),
        ],
        n,
    )
}

pub(super) fn schema() -> Vec<(String, DataType)> {
    vec![
        ("id".into(), DataType::Int64),
        ("ts".into(), DataType::Int64),
        ("url".into(), DataType::String),
        ("dur".into(), DataType::Int64),
    ]
}
