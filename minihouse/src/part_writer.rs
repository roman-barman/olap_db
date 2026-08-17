use crate::codec::{Codec, CodecError};
use crate::column_io::{write_f64_chunk, write_i64_chunk, write_str_chunk};
use crate::{Block, Column, DataType};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

pub(crate) struct PartWriter {
    dir: PathBuf,
    tmp_dir: PathBuf,
    schema: Vec<(String, DataType)>,
    writers: Vec<ColumnFiles>,
    num_rows: usize,
    codec: Codec,
}

impl PartWriter {
    pub(crate) fn new(
        dir: PathBuf,
        schema: &[(String, DataType)],
        codec: Codec,
    ) -> std::io::Result<Self> {
        let tmp_dir = {
            let mut name = dir
                .file_name()
                .expect("part dir must have a file name")
                .to_os_string();
            name.push(".tmp");
            dir.with_file_name(name)
        };

        if tmp_dir.exists() {
            std::fs::remove_dir_all(&tmp_dir)?;
        }

        std::fs::create_dir_all(&tmp_dir)?;

        let writers = schema
            .iter()
            .map(|(name, dt)| {
                Ok(match dt {
                    DataType::Int64 | DataType::Float64 => ColumnFiles::Single(BufWriter::new(
                        File::create(tmp_dir.join(format!("{name}.bin")))?,
                    )),
                    DataType::String => ColumnFiles::Pair {
                        data: BufWriter::new(File::create(
                            tmp_dir.join(format!("{name}.data.bin")),
                        )?),
                        offsets: BufWriter::new(File::create(
                            tmp_dir.join(format!("{name}.offsets.bin")),
                        )?),
                    },
                })
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        Ok(Self {
            dir,
            tmp_dir,
            schema: schema.to_vec(),
            writers,
            num_rows: 0,
            codec,
        })
    }

    pub(crate) fn write_block(&mut self, block: &Block) -> Result<(), CodecError> {
        assert_eq!(
            block.columns().len(),
            self.schema.len(),
            "PartWriter::write_block: block has {} columns, schema has {}",
            block.columns().len(),
            self.schema.len()
        );

        for (i, (((s_name, s_type), (b_name, column)), files)) in self
            .schema
            .iter()
            .zip(block.columns())
            .zip(self.writers.iter_mut())
            .enumerate()
        {
            assert!(
                s_name == b_name && column.data_type() == *s_type,
                "PartWriter::write_block: column {i} expected ('{s_name}', {s_type:?}), \
             got ('{b_name}', {:?})",
                column.data_type()
            );

            match (files, column) {
                (ColumnFiles::Single(w), Column::Int64(v)) => write_i64_chunk(w, v, self.codec)?,
                (ColumnFiles::Single(w), Column::Float64(v)) => write_f64_chunk(w, v, self.codec)?,
                (ColumnFiles::Pair { data, offsets }, Column::String(sc)) => {
                    write_str_chunk(data, offsets, sc, self.codec)?
                }
                _ => unreachable!(
                    "column {i} ('{s_name}'): files/column shape mismatch — broken constructor"
                ),
            }
        }

        self.num_rows += block.num_rows();
        Ok(())
    }
    pub(crate) fn finish(mut self) -> std::io::Result<()> {
        let mut schema_writer = BufWriter::new(File::create(self.tmp_dir.join("schema.txt"))?);
        schema_writer.write_all(b"version=1\n")?;
        write!(schema_writer, "num_rows={}\n", self.num_rows)?;
        for (name, data_type) in self.schema.iter() {
            write!(schema_writer, "column={}:{}\n", name, data_type.as_str())?;
        }
        schema_writer.flush()?;

        for writer in self.writers.iter_mut() {
            match writer {
                ColumnFiles::Single(w) => w.flush()?,
                ColumnFiles::Pair { data, offsets } => {
                    data.flush()?;
                    offsets.flush()?;
                }
            }
        }

        std::fs::rename(self.tmp_dir, self.dir)?;
        Ok(())
    }
}

enum ColumnFiles {
    Single(BufWriter<File>), // Int64, Float64
    Pair {
        data: BufWriter<File>,
        offsets: BufWriter<File>,
    }, // String
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column_io::{read_f64_chunk, read_i64_chunk, read_str_chunk};
    use crate::string_column::StringColumn;
    use std::fs;
    use std::io::BufReader;
    use std::path::Path;
    use tempfile::TempDir;

    /// A temp root plus the part directory inside it that the writer targets.
    /// The root must stay alive for the whole test — bind it, don't discard it.
    fn part_dir() -> (TempDir, PathBuf) {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("part_0");
        (root, dir)
    }

    /// The staging directory `PartWriter` writes into before `finish`.
    fn staging_of(dir: &Path) -> PathBuf {
        let mut name = dir.file_name().unwrap().to_os_string();
        name.push(".tmp");
        dir.with_file_name(name)
    }

    fn string_column(values: &[&str]) -> StringColumn {
        let mut col = StringColumn::new();
        for v in values {
            col.push(v);
        }
        col
    }

    fn sample_schema() -> Vec<(String, DataType)> {
        vec![
            ("id".to_string(), DataType::Int64),
            ("name".to_string(), DataType::String),
            ("score".to_string(), DataType::Float64),
        ]
    }

    fn sample_block(ids: &[i64], names: &[&str], scores: &[f64]) -> Block {
        assert_eq!(ids.len(), names.len());
        assert_eq!(ids.len(), scores.len());
        Block::new(
            vec![
                ("id".to_string(), Column::Int64(ids.to_vec())),
                ("name".to_string(), Column::String(string_column(names))),
                ("score".to_string(), Column::Float64(scores.to_vec())),
            ],
            ids.len(),
        )
    }

    fn open(dir: &Path, file: &str) -> BufReader<File> {
        BufReader::new(File::open(dir.join(file)).unwrap())
    }

    /// Every chunk in a numeric column file, in write order.
    fn i64_chunks(dir: &Path, name: &str) -> Vec<Vec<i64>> {
        let mut r = open(dir, &format!("{name}.bin"));
        let mut chunks = Vec::new();
        while let Some(c) = read_i64_chunk(&mut r).unwrap() {
            chunks.push(c);
        }
        chunks
    }

    fn f64_chunks(dir: &Path, name: &str) -> Vec<Vec<f64>> {
        let mut r = open(dir, &format!("{name}.bin"));
        let mut chunks = Vec::new();
        while let Some(c) = read_f64_chunk(&mut r).unwrap() {
            chunks.push(c);
        }
        chunks
    }

    fn str_chunks(dir: &Path, name: &str) -> Vec<StringColumn> {
        let mut data = open(dir, &format!("{name}.data.bin"));
        let mut offsets = open(dir, &format!("{name}.offsets.bin"));
        let mut chunks = Vec::new();
        while let Some(c) = read_str_chunk(&mut data, &mut offsets).unwrap() {
            chunks.push(c);
        }
        chunks
    }

    fn schema_text(dir: &Path) -> String {
        fs::read_to_string(dir.join("schema.txt")).unwrap()
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    // ---- round trip ----------------------------------------------------

    #[test]
    fn single_block_round_trips_all_three_column_types() {
        let (_root, dir) = part_dir();

        let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
        writer
            .write_block(&sample_block(
                &[1, -2, i64::MAX],
                &["a", "", "日本語"],
                &[1.5, -2.5, 0.0],
            ))
            .unwrap();
        writer.finish().unwrap();

        assert_eq!(i64_chunks(&dir, "id"), vec![vec![1, -2, i64::MAX]]);
        assert_eq!(f64_chunks(&dir, "score"), vec![vec![1.5, -2.5, 0.0]]);
        assert_eq!(
            str_chunks(&dir, "name"),
            vec![string_column(&["a", "", "日本語"])]
        );
    }

    #[test]
    fn multiple_blocks_are_read_back_in_write_order() {
        let (_root, dir) = part_dir();

        let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
        writer
            .write_block(&sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0]))
            .unwrap();
        writer
            .write_block(&sample_block(&[3], &["c"], &[3.0]))
            .unwrap();
        writer
            .write_block(&sample_block(&[4, 5], &["d", "e"], &[4.0, 5.0]))
            .unwrap();
        writer.finish().unwrap();

        assert_eq!(
            i64_chunks(&dir, "id"),
            vec![vec![1, 2], vec![3], vec![4, 5]]
        );
        assert_eq!(
            f64_chunks(&dir, "score"),
            vec![vec![1.0, 2.0], vec![3.0], vec![4.0, 5.0]]
        );
        assert_eq!(
            str_chunks(&dir, "name"),
            vec![
                string_column(&["a", "b"]),
                string_column(&["c"]),
                string_column(&["d", "e"]),
            ]
        );
    }

    #[test]
    fn zero_blocks_produces_empty_column_files() {
        let (_root, dir) = part_dir();

        PartWriter::new(dir.clone(), &sample_schema())
            .unwrap()
            .finish()
            .unwrap();

        assert_eq!(fs::metadata(dir.join("id.bin")).unwrap().len(), 0);
        assert_eq!(fs::metadata(dir.join("name.data.bin")).unwrap().len(), 0);
        assert!(i64_chunks(&dir, "id").is_empty());
        assert!(str_chunks(&dir, "name").is_empty());
        assert!(schema_text(&dir).contains("num_rows=0\n"));
    }

    /// A zero-row block still emits a framed, zero-length chunk — it is not the
    /// same as end-of-file. A reader that treats an empty chunk as EOF would
    /// silently truncate every part written after one.
    #[test]
    fn empty_block_writes_a_zero_length_chunk() {
        let (_root, dir) = part_dir();

        let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
        writer.write_block(&sample_block(&[], &[], &[])).unwrap();
        writer
            .write_block(&sample_block(&[7], &["x"], &[7.5]))
            .unwrap();
        writer.finish().unwrap();

        assert!(fs::metadata(dir.join("id.bin")).unwrap().len() > 0);
        assert_eq!(i64_chunks(&dir, "id"), vec![vec![], vec![7]]);
        assert_eq!(
            str_chunks(&dir, "name"),
            vec![StringColumn::new(), string_column(&["x"])]
        );
    }

    /// One row is far below `BufWriter`'s capacity, so it reaches disk only if
    /// `finish` flushes. Larger payloads would spill on their own and hide the bug.
    #[test]
    fn finish_flushes_buffered_writes() {
        let (_root, dir) = part_dir();

        let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
        writer
            .write_block(&sample_block(&[42], &["tiny"], &[0.5]))
            .unwrap();
        writer.finish().unwrap();

        assert_eq!(i64_chunks(&dir, "id"), vec![vec![42]]);
        assert_eq!(str_chunks(&dir, "name"), vec![string_column(&["tiny"])]);
        assert_eq!(f64_chunks(&dir, "score"), vec![vec![0.5]]);
    }

    // ---- schema.txt ----------------------------------------------------

    #[test]
    fn schema_file_has_exact_expected_contents() {
        let (_root, dir) = part_dir();

        let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
        writer
            .write_block(&sample_block(
                &[1, 2, 3],
                &["a", "b", "c"],
                &[1.0, 2.0, 3.0],
            ))
            .unwrap();
        writer.finish().unwrap();

        assert_eq!(
            schema_text(&dir),
            "version=1\nnum_rows=3\ncolumn=id:Int64\ncolumn=name:String\ncolumn=score:Float64\n"
        );
    }

    #[test]
    fn schema_file_preserves_declaration_order() {
        let (_root, dir) = part_dir();
        let schema = vec![
            ("zulu".to_string(), DataType::Float64),
            ("alpha".to_string(), DataType::String),
            ("mike".to_string(), DataType::Int64),
        ];

        PartWriter::new(dir.clone(), &schema)
            .unwrap()
            .finish()
            .unwrap();

        assert_eq!(
            schema_text(&dir),
            "version=1\nnum_rows=0\ncolumn=zulu:Float64\ncolumn=alpha:String\ncolumn=mike:Int64\n"
        );
    }

    #[test]
    fn num_rows_accumulates_across_blocks() {
        let (_root, dir) = part_dir();

        let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
        writer
            .write_block(&sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0]))
            .unwrap();
        writer.write_block(&sample_block(&[], &[], &[])).unwrap();
        writer
            .write_block(&sample_block(
                &[3, 4, 5],
                &["c", "d", "e"],
                &[3.0, 4.0, 5.0],
            ))
            .unwrap();
        writer.finish().unwrap();

        assert!(schema_text(&dir).contains("num_rows=5\n"));
    }

    #[test]
    fn empty_schema_writes_header_only() {
        let (_root, dir) = part_dir();

        PartWriter::new(dir.clone(), &[]).unwrap().finish().unwrap();

        assert_eq!(entries(&dir), vec!["schema.txt"]);
        assert_eq!(schema_text(&dir), "version=1\nnum_rows=0\n");
    }

    /// `num_rows` comes from `Block::num_rows`, not from any column, so a
    /// column-less block still moves the counter.
    #[test]
    fn column_less_blocks_still_count_rows() {
        let (_root, dir) = part_dir();

        let mut writer = PartWriter::new(dir.clone(), &[]).unwrap();
        writer.write_block(&Block::new(vec![], 5)).unwrap();
        writer.finish().unwrap();

        assert_eq!(schema_text(&dir), "version=1\nnum_rows=5\n");
    }

    // ---- file layout ---------------------------------------------------

    #[test]
    fn part_directory_contains_one_file_per_numeric_and_two_per_string_column() {
        let (_root, dir) = part_dir();

        PartWriter::new(dir.clone(), &sample_schema())
            .unwrap()
            .finish()
            .unwrap();

        assert_eq!(
            entries(&dir),
            vec![
                "id.bin",
                "name.data.bin",
                "name.offsets.bin",
                "schema.txt",
                "score.bin",
            ]
        );
    }

    #[test]
    fn new_creates_missing_parent_directories() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("nested/deeper/part_0");

        PartWriter::new(dir.clone(), &sample_schema())
            .unwrap()
            .finish()
            .unwrap();

        assert!(dir.join("schema.txt").is_file());
    }

    // ---- atomicity -----------------------------------------------------

    #[test]
    fn writes_go_to_staging_dir_until_finish() {
        let (_root, dir) = part_dir();
        let staging = staging_of(&dir);

        let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
        writer
            .write_block(&sample_block(&[1], &["a"], &[1.0]))
            .unwrap();

        assert!(!dir.exists(), "part must not be visible before finish");
        assert!(staging.is_dir());

        writer.finish().unwrap();

        assert!(dir.is_dir());
        assert!(!staging.exists());
    }

    /// The point of the staging directory: an aborted write never appears as a
    /// readable part.
    #[test]
    fn dropping_without_finish_leaves_no_part_dir() {
        let (_root, dir) = part_dir();

        {
            let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
            writer
                .write_block(&sample_block(&[1], &["a"], &[1.0]))
                .unwrap();
        }

        assert!(!dir.exists());
    }

    /// There is no `Drop` impl, so an abandoned staging directory outlives the
    /// writer. It is not leaked forever: the next `PartWriter::new` for the same
    /// part reclaims it — see `new_clears_stale_staging_contents`.
    #[test]
    fn dropping_without_finish_leaves_staging_dir_behind() {
        let (_root, dir) = part_dir();

        drop(PartWriter::new(dir.clone(), &sample_schema()).unwrap());

        assert!(staging_of(&dir).is_dir());
    }

    #[test]
    fn finish_fails_when_target_dir_exists_and_is_non_empty() {
        let (_root, dir) = part_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("existing.bin"), b"old").unwrap();

        let writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
        let err = writer.finish().unwrap_err();

        assert!(err.raw_os_error().is_some(), "expected an OS rename error");
        assert_eq!(fs::read(dir.join("existing.bin")).unwrap(), b"old");
    }

    /// `rename` succeeds over an empty directory, so an existing part directory
    /// is replaced without warning. Documents current behavior.
    #[test]
    fn finish_replaces_an_existing_empty_target_dir() {
        let (_root, dir) = part_dir();
        fs::create_dir_all(&dir).unwrap();

        PartWriter::new(dir.clone(), &sample_schema())
            .unwrap()
            .finish()
            .unwrap();

        assert!(dir.join("schema.txt").is_file());
    }

    /// `new` discards a staging directory left behind by a crashed run, so its
    /// debris never reaches the finished part.
    #[test]
    fn new_clears_stale_staging_contents() {
        let (_root, dir) = part_dir();
        let staging = staging_of(&dir);
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("orphan.bin"), b"stale").unwrap();

        let writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();

        assert_eq!(
            entries(&staging),
            vec!["id.bin", "name.data.bin", "name.offsets.bin", "score.bin",]
        );

        writer.finish().unwrap();

        assert_eq!(
            entries(&dir),
            vec![
                "id.bin",
                "name.data.bin",
                "name.offsets.bin",
                "schema.txt",
                "score.bin",
            ]
        );
    }

    /// Debris that shares a name with a real column file is handled by
    /// `File::create` truncating rather than appending — a second line of
    /// defence that holds even without the staging wipe above. Without it, a
    /// half-written part's offsets would be spliced onto fresh data.
    #[test]
    fn new_truncates_stale_files_that_collide_with_column_names() {
        let (_root, dir) = part_dir();
        let staging = staging_of(&dir);
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("id.bin"), b"garbage-id-bytes").unwrap();
        fs::write(staging.join("name.data.bin"), b"garbage-data").unwrap();
        fs::write(staging.join("name.offsets.bin"), b"garbage-offsets").unwrap();
        fs::write(staging.join("score.bin"), b"garbage-score").unwrap();
        fs::write(staging.join("schema.txt"), b"version=0\nnum_rows=99\n").unwrap();

        let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
        writer
            .write_block(&sample_block(&[7], &["x"], &[7.5]))
            .unwrap();
        writer.finish().unwrap();

        assert_eq!(i64_chunks(&dir, "id"), vec![vec![7]]);
        assert_eq!(str_chunks(&dir, "name"), vec![string_column(&["x"])]);
        assert_eq!(f64_chunks(&dir, "score"), vec![vec![7.5]]);
        assert_eq!(
            schema_text(&dir),
            "version=1\nnum_rows=1\ncolumn=id:Int64\ncolumn=name:String\ncolumn=score:Float64\n"
        );
    }

    // ---- contract violations -------------------------------------------

    #[test]
    #[should_panic(expected = "block has 2 columns, schema has 3")]
    fn write_block_panics_on_column_count_mismatch() {
        let (_root, dir) = part_dir();
        let mut writer = PartWriter::new(dir, &sample_schema()).unwrap();

        let block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![1])),
                ("name".to_string(), Column::String(string_column(&["a"]))),
            ],
            1,
        );
        let _ = writer.write_block(&block);
    }

    #[test]
    #[should_panic(expected = "column 1 expected ('name', String)")]
    fn write_block_panics_on_column_name_mismatch() {
        let (_root, dir) = part_dir();
        let mut writer = PartWriter::new(dir, &sample_schema()).unwrap();

        let block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![1])),
                ("label".to_string(), Column::String(string_column(&["a"]))),
                ("score".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );
        let _ = writer.write_block(&block);
    }

    #[test]
    #[should_panic(expected = "column 0 expected ('id', Int64), got ('id', Float64)")]
    fn write_block_panics_on_type_mismatch() {
        let (_root, dir) = part_dir();
        let mut writer = PartWriter::new(dir, &sample_schema()).unwrap();

        let block = Block::new(
            vec![
                ("id".to_string(), Column::Float64(vec![1.0])),
                ("name".to_string(), Column::String(string_column(&["a"]))),
                ("score".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );
        let _ = writer.write_block(&block);
    }

    /// Columns are matched positionally, so the right set in the wrong order
    /// must be rejected rather than silently written to the wrong files.
    #[test]
    #[should_panic(expected = "column 1 expected ('name', String)")]
    fn write_block_panics_on_reordered_columns() {
        let (_root, dir) = part_dir();
        let mut writer = PartWriter::new(dir, &sample_schema()).unwrap();

        let block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![1])),
                ("score".to_string(), Column::Float64(vec![1.0])),
                ("name".to_string(), Column::String(string_column(&["a"]))),
            ],
            1,
        );
        let _ = writer.write_block(&block);
    }

    #[test]
    #[should_panic(expected = "part dir must have a file name")]
    fn new_panics_when_dir_has_no_file_name() {
        // Panics before any filesystem call, so nothing touches the root.
        let _ = PartWriter::new(PathBuf::from("/"), &sample_schema());
    }
}
