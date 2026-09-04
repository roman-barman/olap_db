use crate::core::{DataType, StringColumn};
use crate::{Block, Column, Schema};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

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

/// A temp root plus the part directory inside it. The root must stay alive
/// for the whole test — bind it, don't discard it.
pub(crate) fn part_dir() -> (TempDir, PathBuf) {
    let root = TempDir::new().unwrap();
    let dir = root.path().join("part_0");
    (root, dir)
}

/// The staging directory `PartWriter` writes into before `finish`.
pub(crate) fn staging_of(dir: &Path) -> PathBuf {
    let mut name = dir.file_name().unwrap().to_os_string();
    name.push(".tmp");
    dir.with_file_name(name)
}

/// The block's columns in the order it holds them — the projection tests are
/// all a single assertion on this.
pub(crate) fn column_names(block: &Block) -> Vec<&str> {
    block.columns().iter().map(|(n, _)| n.as_str()).collect()
}

/// Sorted directory listing, so tests can pin exactly which files exist.
pub(crate) fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
