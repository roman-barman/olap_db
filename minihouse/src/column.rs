use crate::types::DataType;

#[derive(Debug, Clone, PartialEq)]
pub enum Column {
    Int64(Vec<i64>),
    Float64(Vec<f64>),
    String(Vec<String>),
}

impl Column {
    pub fn new(dt: DataType) -> Self {
        Self::with_capacity(dt, 0)
    }

    pub fn with_capacity(dt: DataType, cap: usize) -> Self {
        match dt {
            DataType::Int64 => Column::Int64(Vec::with_capacity(cap)),
            DataType::Float64 => Column::Float64(Vec::with_capacity(cap)),
            DataType::String => Column::String(Vec::with_capacity(cap)),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Column::Int64(v) => v.len(),
            Column::Float64(v) => v.len(),
            Column::String(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn data_type(&self) -> DataType {
        match self {
            Column::Int64(_) => DataType::Int64,
            Column::Float64(_) => DataType::Float64,
            Column::String(_) => DataType::String,
        }
    }

    pub fn push_i64(&mut self, value: i64) {
        match self {
            Column::Int64(v) => v.push(value),
            _ => panic!("push_i64 on {:?} column", self.data_type()),
        }
    }
    pub fn push_f64(&mut self, value: f64) {
        match self {
            Column::Float64(v) => v.push(value),
            _ => panic!("push_f64 on {:?} column", self.data_type()),
        }
    }
    pub fn push_str(&mut self, value: &str) {
        match self {
            Column::String(v) => v.push(value.to_string()),
            _ => panic!("push_str on {:?} column", self.data_type()),
        }
    }

    pub fn filter(&self, mask: &[bool]) -> Column {
        assert_eq!(
            mask.len(),
            self.len(),
            "filter: mask length != column length"
        );
        let cap = mask.iter().filter(|&&m| m).count();

        if cap == 0 {
            return Column::new(self.data_type());
        }
        if cap == mask.len() {
            return self.clone();
        }

        match self {
            Column::Int64(v) => Column::Int64(filter_slice(v, mask, cap)),
            Column::Float64(v) => Column::Float64(filter_slice(v, mask, cap)),
            Column::String(v) => Column::String(filter_slice(v, mask, cap)),
        }
    }
}

fn filter_slice<T: Clone>(v: &[T], mask: &[bool], cap: usize) -> Vec<T> {
    let mut out = Vec::with_capacity(cap);
    out.extend(
        v.iter()
            .zip(mask)
            .filter_map(|(x, &m)| m.then(|| x.clone())),
    );
    out
}
