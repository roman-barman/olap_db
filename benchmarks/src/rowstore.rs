pub struct Row {
    pub id: i64,
    pub ts: i64,
    pub url: String,
    pub dur: i64,
}

pub struct RowTable {
    pub rows: Vec<Row>,
}

pub fn sum_dur_where_ts_gt(t: &RowTable, x: i64) -> i64 {
    t.rows.iter().filter(|r| r.ts > x).map(|r| r.dur).sum()
}

pub fn count_where_ts_gt(t: &RowTable, x: i64) -> i64 {
    t.rows.iter().filter(|r| r.ts > x).count() as i64
}
