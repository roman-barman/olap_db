use crate::rowstore::{Row, RowTable};
use minihouse::block::Block;
use minihouse::column::Column;
use minihouse::table::Table;
use minihouse::types::DataType;
use rand::prelude::StdRng;
use rand::{RngExt, SeedableRng};

pub fn generate(n: usize, block_size: usize) -> (Table, RowTable) {
    let mut rng = StdRng::seed_from_u64(42);

    let mut row_table = RowTable {
        rows: Vec::with_capacity(n),
    };
    let mut col_table = Table::new(vec![
        ("id".into(), DataType::Int64),
        ("ts".into(), DataType::Int64),
        ("url".into(), DataType::String),
        ("dur".into(), DataType::Int64),
    ]);

    let mut id_column = Column::with_capacity(DataType::Int64, block_size);
    let mut ts_column = Column::with_capacity(DataType::Int64, block_size);
    let mut url_column = Column::with_capacity(DataType::String, block_size);
    let mut dur_column = Column::with_capacity(DataType::Int64, block_size);

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
            col_table.insert(make_block(id_column, ts_column, url_column, dur_column));

            id_column = Column::with_capacity(DataType::Int64, block_size);
            ts_column = Column::with_capacity(DataType::Int64, block_size);
            url_column = Column::with_capacity(DataType::String, block_size);
            dur_column = Column::with_capacity(DataType::Int64, block_size);
        }
    }

    col_table.insert(make_block(id_column, ts_column, url_column, dur_column));

    (col_table, row_table)
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
