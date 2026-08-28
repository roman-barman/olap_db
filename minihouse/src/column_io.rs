use crate::codec::{Codec, CodecError, MAX_BLOCK_SIZE, read_block, write_block};
use crate::string_column::StringColumn;
use std::io::{Read, Write};

pub(crate) fn write_i64_chunk(
    w: &mut impl Write,
    values: &[i64],
    codec: Codec,
) -> Result<(), CodecError> {
    let block_size = values.len() * size_of::<i64>();
    let mut bytes = Vec::with_capacity(block_size);
    for v in values {
        bytes.extend_from_slice(&v.to_le_bytes());
    }

    write_block(w, &bytes, codec)
}

pub(crate) fn read_i64_chunk(r: &mut impl Read) -> Result<Option<Vec<i64>>, CodecError> {
    let Some(bytes) = read_block(r)? else {
        return Ok(None);
    };

    let data_size = size_of::<i64>();

    if bytes.len() % data_size != 0 {
        return Err(CodecError::Corrupt(format!(
            "i64 chunk of {} bytes is not a multiple of {}",
            bytes.len(),
            data_size
        )));
    }

    let values = bytes
        .chunks_exact(data_size)
        .map(|c| i64::from_le_bytes(c.try_into().unwrap()))
        .collect();

    Ok(Some(values))
}

pub(crate) fn write_f64_chunk(
    w: &mut impl Write,
    values: &[f64],
    codec: Codec,
) -> Result<(), CodecError> {
    let block_size = values.len() * size_of::<f64>();
    let mut bytes = Vec::with_capacity(block_size);
    for v in values {
        bytes.extend_from_slice(&v.to_le_bytes());
    }

    write_block(w, &bytes, codec)
}

pub(crate) fn read_f64_chunk(r: &mut impl Read) -> Result<Option<Vec<f64>>, CodecError> {
    let Some(bytes) = read_block(r)? else {
        return Ok(None);
    };

    let data_size = size_of::<f64>();

    if bytes.len() % data_size != 0 {
        return Err(CodecError::Corrupt(format!(
            "f64 chunk of {} bytes is not a multiple of {}",
            bytes.len(),
            data_size
        )));
    }

    let values = bytes
        .chunks_exact(data_size)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
        .collect();

    Ok(Some(values))
}

pub(crate) fn write_str_chunk(
    data_w: &mut impl Write,
    offsets_w: &mut impl Write,
    chunk: &StringColumn,
    codec: Codec,
) -> Result<(), CodecError> {
    assert!(
        chunk.data_len() <= MAX_BLOCK_SIZE,
        "string chunk too big: {} bytes — long strings not supported yet",
        chunk.data_len()
    );

    write_block(data_w, chunk.data(), codec)?;

    let offset = chunk.offsets();
    let block_size = offset.len() * size_of::<u32>();
    let mut bytes = Vec::with_capacity(block_size);
    for v in offset {
        bytes.extend_from_slice(&v.to_le_bytes());
    }

    write_block(offsets_w, &bytes, codec)
}
pub(crate) fn read_str_chunk(
    data_r: &mut impl Read,
    offsets_r: &mut impl Read,
) -> Result<Option<StringColumn>, CodecError> {
    let data = read_block(data_r)?;
    let offset_b = read_block(offsets_r)?;

    match (data, offset_b) {
        (None, None) => Ok(None),
        (Some(data), Some(offset_b)) => {
            let offset_data_size = size_of::<u32>();
            if offset_b.len() % offset_data_size != 0 {
                return Err(CodecError::Corrupt(format!(
                    "string offset chunk of {} bytes is not a multiple of {}",
                    offset_b.len(),
                    offset_data_size
                )));
            }
            let offset = offset_b
                .chunks_exact(offset_data_size)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            let column = StringColumn::try_from_parts(data, offset).map_err(|e| {
                CodecError::Corrupt(format!("invalid string column in chunk: {}", e))
            })?;
            Ok(Some(column))
        }
        (Some(_), None) => Err(CodecError::Corrupt(
            "string streams out of sync: data present, offsets ended".into(),
        )),
        (None, Some(_)) => Err(CodecError::Corrupt(
            "string streams out of sync: offsets present, data ended".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn round_trip_i64(values: &[i64]) -> Option<Vec<i64>> {
        let mut buf = Cursor::new(Vec::new());
        write_i64_chunk(&mut buf, values, Codec::Lz4).unwrap();
        buf.set_position(0);
        read_i64_chunk(&mut buf).unwrap()
    }

    fn round_trip_f64(values: &[f64]) -> Option<Vec<f64>> {
        let mut buf = Cursor::new(Vec::new());
        write_f64_chunk(&mut buf, values, Codec::Lz4).unwrap();
        buf.set_position(0);
        read_f64_chunk(&mut buf).unwrap()
    }

    fn round_trip_str(chunk: &StringColumn) -> Option<StringColumn> {
        let mut data = Cursor::new(Vec::new());
        let mut offsets = Cursor::new(Vec::new());
        write_str_chunk(&mut data, &mut offsets, chunk, Codec::Lz4).unwrap();
        data.set_position(0);
        offsets.set_position(0);
        read_str_chunk(&mut data, &mut offsets).unwrap()
    }

    /// One well-framed block carrying `raw`, rewound and ready to read.
    fn block(raw: &[u8]) -> Cursor<Vec<u8>> {
        let mut buf = Cursor::new(Vec::new());
        write_block(&mut buf, raw, Codec::Lz4).unwrap();
        buf.set_position(0);
        buf
    }

    fn offsets_bytes(offsets: &[u32]) -> Vec<u8> {
        offsets.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn bits(values: &[f64]) -> Vec<u64> {
        values.iter().map(|v| v.to_bits()).collect()
    }

    #[test]
    fn i64_chunk_round_trips_values() {
        assert_eq!(round_trip_i64(&[1, -1, 0, 42]), Some(vec![1, -1, 0, 42]));
    }

    #[test]
    fn i64_chunk_round_trips_empty_slice() {
        assert_eq!(round_trip_i64(&[]), Some(Vec::new()));
    }

    #[test]
    fn i64_chunk_round_trips_extreme_values() {
        let values = [i64::MIN, i64::MAX, 0, -1];
        assert_eq!(round_trip_i64(&values), Some(values.to_vec()));
    }

    #[test]
    fn i64_chunk_round_trips_many_values() {
        let values: Vec<i64> = (0..10_000).collect();
        assert_eq!(round_trip_i64(&values), Some(values.clone()));
    }

    #[test]
    fn i64_chunk_reads_sequential_chunks_in_order() {
        let mut buf = Cursor::new(Vec::new());
        write_i64_chunk(&mut buf, &[1, 2], Codec::Lz4).unwrap();
        write_i64_chunk(&mut buf, &[], Codec::Lz4).unwrap();
        write_i64_chunk(&mut buf, &[3], Codec::Lz4).unwrap();
        buf.set_position(0);

        assert_eq!(read_i64_chunk(&mut buf).unwrap(), Some(vec![1, 2]));
        assert_eq!(read_i64_chunk(&mut buf).unwrap(), Some(Vec::new()));
        assert_eq!(read_i64_chunk(&mut buf).unwrap(), Some(vec![3]));
        assert_eq!(read_i64_chunk(&mut buf).unwrap(), None);
    }

    #[test]
    fn read_i64_chunk_returns_none_on_clean_eof() {
        let mut buf = Cursor::new(Vec::new());
        assert_eq!(read_i64_chunk(&mut buf).unwrap(), None);
    }

    #[test]
    fn read_i64_chunk_rejects_length_not_multiple_of_eight() {
        let mut buf = block(&[0u8; 12]);
        let err = read_i64_chunk(&mut buf).unwrap_err();
        assert!(
            err.to_string()
                .contains("i64 chunk of 12 bytes is not a multiple of 8")
        );
    }

    #[test]
    fn read_i64_chunk_propagates_block_level_corruption() {
        let mut buf = Cursor::new(vec![1u8, 2, 3, 4, 5]);
        let err = read_i64_chunk(&mut buf).unwrap_err();
        assert!(err.to_string().contains("truncated header"));
    }

    #[test]
    fn f64_chunk_round_trips_values() {
        assert_eq!(
            round_trip_f64(&[1.5, -2.25, 0.0]),
            Some(vec![1.5, -2.25, 0.0])
        );
    }

    #[test]
    fn f64_chunk_round_trips_empty_slice() {
        assert_eq!(round_trip_f64(&[]), Some(Vec::new()));
    }

    #[test]
    fn f64_chunk_preserves_nan_and_infinities_bitwise() {
        let values = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
        let got = round_trip_f64(&values).unwrap();
        assert_eq!(bits(&got), bits(&values));
    }

    #[test]
    fn f64_chunk_preserves_negative_zero_bitwise() {
        let values = [-0.0, 0.0];
        let got = round_trip_f64(&values).unwrap();
        assert_eq!(bits(&got), bits(&values));
    }

    #[test]
    fn f64_chunk_round_trips_extreme_magnitudes() {
        let values = [f64::MIN, f64::MAX, f64::EPSILON, f64::MIN_POSITIVE];
        assert_eq!(round_trip_f64(&values), Some(values.to_vec()));
    }

    #[test]
    fn f64_chunk_reads_sequential_chunks_in_order() {
        let mut buf = Cursor::new(Vec::new());
        write_f64_chunk(&mut buf, &[1.5, 2.5], Codec::Lz4).unwrap();
        write_f64_chunk(&mut buf, &[], Codec::Lz4).unwrap();
        write_f64_chunk(&mut buf, &[3.5], Codec::Lz4).unwrap();
        buf.set_position(0);

        assert_eq!(read_f64_chunk(&mut buf).unwrap(), Some(vec![1.5, 2.5]));
        assert_eq!(read_f64_chunk(&mut buf).unwrap(), Some(Vec::new()));
        assert_eq!(read_f64_chunk(&mut buf).unwrap(), Some(vec![3.5]));
        assert_eq!(read_f64_chunk(&mut buf).unwrap(), None);
    }

    #[test]
    fn read_f64_chunk_returns_none_on_clean_eof() {
        let mut buf = Cursor::new(Vec::new());
        assert_eq!(read_f64_chunk(&mut buf).unwrap(), None);
    }

    #[test]
    fn read_f64_chunk_rejects_length_not_multiple_of_eight() {
        let mut buf = block(&[0u8; 12]);
        let err = read_f64_chunk(&mut buf).unwrap_err();
        assert!(
            err.to_string()
                .contains("f64 chunk of 12 bytes is not a multiple of 8")
        );
    }

    #[test]
    fn str_chunk_round_trips_values() {
        let col = StringColumn::new_with_values(&["hello", "world", "foo"]);
        assert_eq!(round_trip_str(&col), Some(col.clone()));
    }

    #[test]
    fn str_chunk_round_trips_empty_column() {
        let col = StringColumn::new();
        assert_eq!(round_trip_str(&col), Some(StringColumn::new()));
    }

    #[test]
    fn str_chunk_round_trips_column_with_empty_strings() {
        let col = StringColumn::new_with_values(&["", "a", "", ""]);
        let got = round_trip_str(&col).unwrap();

        assert_eq!(got, col);
        assert_eq!(got.len(), 4);
        assert_eq!(got.get(0), "");
        assert_eq!(got.get(1), "a");
    }

    #[test]
    fn str_chunk_round_trips_multibyte_utf8() {
        let col = StringColumn::new_with_values(&["café", "日本語", ""]);
        let got = round_trip_str(&col).unwrap();

        assert_eq!(got, col);
        assert_eq!(got.get(0), "café");
        assert_eq!(got.get(1), "日本語");
    }

    #[test]
    fn str_chunk_round_trips_filtered_column() {
        let col = StringColumn::new_with_values(&["a", "b", "c", "d"])
            .filter(&[true, false, true, false]);
        let got = round_trip_str(&col).unwrap();

        assert_eq!(got, col);
        assert_eq!(got.len(), 2);
        assert_eq!(got.get(0), "a");
        assert_eq!(got.get(1), "c");
    }

    #[test]
    fn str_chunk_reads_sequential_chunks_in_order() {
        let first = StringColumn::new_with_values(&["a", "bc"]);
        let second = StringColumn::new();
        let third = StringColumn::new_with_values(&["日本語"]);

        let mut data = Cursor::new(Vec::new());
        let mut offsets = Cursor::new(Vec::new());
        write_str_chunk(&mut data, &mut offsets, &first, Codec::Lz4).unwrap();
        write_str_chunk(&mut data, &mut offsets, &second, Codec::Lz4).unwrap();
        write_str_chunk(&mut data, &mut offsets, &third, Codec::Lz4).unwrap();
        data.set_position(0);
        offsets.set_position(0);

        assert_eq!(
            read_str_chunk(&mut data, &mut offsets).unwrap(),
            Some(first)
        );
        assert_eq!(
            read_str_chunk(&mut data, &mut offsets).unwrap(),
            Some(second)
        );
        assert_eq!(
            read_str_chunk(&mut data, &mut offsets).unwrap(),
            Some(third)
        );
        assert_eq!(read_str_chunk(&mut data, &mut offsets).unwrap(), None);
    }

    #[test]
    fn read_str_chunk_returns_none_when_both_streams_at_eof() {
        let mut data = Cursor::new(Vec::new());
        let mut offsets = Cursor::new(Vec::new());
        assert_eq!(read_str_chunk(&mut data, &mut offsets).unwrap(), None);
    }

    #[test]
    fn read_str_chunk_errors_when_offsets_stream_ends_early() {
        let mut data = block(b"abc");
        let mut offsets = Cursor::new(Vec::new());

        let err = read_str_chunk(&mut data, &mut offsets).unwrap_err();

        assert!(
            err.to_string()
                .contains("string streams out of sync: data present, offsets ended")
        );
    }

    #[test]
    fn read_str_chunk_errors_when_data_stream_ends_early() {
        let mut data = Cursor::new(Vec::new());
        let mut offsets = block(&offsets_bytes(&[0, 3]));

        let err = read_str_chunk(&mut data, &mut offsets).unwrap_err();

        assert!(
            err.to_string()
                .contains("string streams out of sync: offsets present, data ended")
        );
    }

    #[test]
    fn read_str_chunk_detects_desync_after_a_successful_chunk() {
        let first = StringColumn::new_with_values(&["a", "b"]);

        let mut data = Cursor::new(Vec::new());
        let mut offsets = Cursor::new(Vec::new());
        write_str_chunk(&mut data, &mut offsets, &first, Codec::Lz4).unwrap();
        write_block(&mut data, b"orphan", Codec::Lz4).unwrap();
        data.set_position(0);
        offsets.set_position(0);

        assert_eq!(
            read_str_chunk(&mut data, &mut offsets).unwrap(),
            Some(first)
        );

        let err = read_str_chunk(&mut data, &mut offsets).unwrap_err();
        assert!(
            err.to_string()
                .contains("string streams out of sync: data present, offsets ended")
        );
    }

    #[test]
    fn read_str_chunk_rejects_offsets_block_not_multiple_of_four() {
        let mut data = block(b"abc");
        let mut offsets = block(&[0u8; 6]);

        let err = read_str_chunk(&mut data, &mut offsets).unwrap_err();

        assert!(
            err.to_string()
                .contains("string offset chunk of 6 bytes is not a multiple of 4")
        );
    }

    #[test]
    fn read_str_chunk_rejects_empty_offsets_block() {
        let mut data = block(&[]);
        let mut offsets = block(&[]);

        let err = read_str_chunk(&mut data, &mut offsets).unwrap_err();

        assert!(
            err.to_string()
                .contains("invalid string column in chunk: offsets must not be empty")
        );
    }

    #[test]
    fn read_str_chunk_rejects_offsets_violating_column_invariants() {
        let mut data = block(b"abc");
        let mut offsets = block(&offsets_bytes(&[0, 2]));

        let err = read_str_chunk(&mut data, &mut offsets).unwrap_err();

        assert!(
            err.to_string()
                .contains("invalid string column in chunk: last offset 2 != data length 3")
        );
    }

    #[test]
    fn read_str_chunk_propagates_data_block_corruption() {
        let mut data = Cursor::new(vec![1u8, 2, 3, 4, 5]);
        let mut offsets = block(&offsets_bytes(&[0, 3]));

        let err = read_str_chunk(&mut data, &mut offsets).unwrap_err();

        assert!(err.to_string().contains("truncated header"));
    }

    #[test]
    fn write_str_chunk_accepts_data_at_max_block_size() {
        let col = StringColumn::try_from_parts(
            vec![b'a'; MAX_BLOCK_SIZE],
            vec![0, MAX_BLOCK_SIZE as u32],
        )
        .unwrap();

        let got = round_trip_str(&col).unwrap();

        assert_eq!(got.len(), 1);
        assert_eq!(got.data_len(), MAX_BLOCK_SIZE);
        assert_eq!(got, col);
    }

    #[test]
    #[should_panic(expected = "string chunk too big")]
    fn write_str_chunk_panics_when_data_exceeds_max_block_size() {
        let col = StringColumn::try_from_parts(
            vec![b'a'; MAX_BLOCK_SIZE + 1],
            vec![0, MAX_BLOCK_SIZE as u32 + 1],
        )
        .unwrap();

        let mut data = Cursor::new(Vec::new());
        let mut offsets = Cursor::new(Vec::new());
        let _ = write_str_chunk(&mut data, &mut offsets, &col, Codec::Lz4);
    }

    #[test]
    #[should_panic(expected = "exceeds MAX_BLOCK_SIZE")]
    fn write_str_chunk_panics_when_offsets_block_exceeds_max_block_size() {
        // data_len() == 0 passes write_str_chunk's own guard, but the offsets
        // block is (rows + 1) * 4 = 16_777_220 bytes — write_block rejects it.
        let rows = MAX_BLOCK_SIZE / size_of::<u32>();
        let col = StringColumn::try_from_parts(Vec::new(), vec![0; rows + 1]).unwrap();

        let mut data = Cursor::new(Vec::new());
        let mut offsets = Cursor::new(Vec::new());
        let _ = write_str_chunk(&mut data, &mut offsets, &col, Codec::Lz4);
    }
}
