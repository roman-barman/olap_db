use lz4_flex::block::{compress, decompress, get_maximum_output_size};
use std::io::{Read, Write};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Codec {
    None = 0,
    Lz4 = 1,
}

impl Codec {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Codec::None => "none",
            Codec::Lz4 => "lz4",
        }
    }
}

impl FromStr for Codec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Codec::None),
            "lz4" => Ok(Codec::Lz4),
            _ => Err(format!("invalid codec: {}", s)),
        }
    }
}

impl TryFrom<u8> for Codec {
    type Error = CodecError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        const _: () = {
            match Codec::None {
                Codec::None | Codec::Lz4 => {}
            }
        };
        match value {
            x if x == Codec::None as u8 => Ok(Codec::None),
            x if x == Codec::Lz4 as u8 => Ok(Codec::Lz4),
            _ => Err(CodecError::Corrupt(format!("unknown codec byte {value}"))),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CodecError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt block: {0}")]
    Corrupt(String),
}

pub(crate) const MAX_BLOCK_SIZE: usize = 16 * 1024 * 1024;

pub(crate) fn write_block(w: &mut impl Write, raw: &[u8], codec: Codec) -> Result<(), CodecError> {
    assert!(
        raw.len() <= MAX_BLOCK_SIZE,
        "write_block: {} bytes exceeds MAX_BLOCK_SIZE — caller bug",
        raw.len()
    );

    let compressed_owned;
    let (compressed, codec) = match codec {
        Codec::Lz4 => {
            compressed_owned = compress(raw);
            if compressed_owned.len() >= raw.len() {
                (raw, Codec::None)
            } else {
                (compressed_owned.as_slice(), Codec::Lz4)
            }
        }
        Codec::None => (raw, Codec::None),
    };
    let c_len = u32::try_from(compressed.len()).expect("compressed size exceeds u32");
    let r_len = u32::try_from(raw.len()).expect("raw size exceeds u32");

    w.write_all(&[codec as u8])?;
    w.write_all(&c_len.to_le_bytes())?;
    w.write_all(&r_len.to_le_bytes())?;
    w.write_all(&compressed)?;

    Ok(())
}

pub(crate) fn read_block(r: &mut impl Read) -> Result<Option<Vec<u8>>, CodecError> {
    let mut header = [0u8; 9];
    let mut filled = 0;
    while filled < 9 {
        let n = r.read(&mut header[filled..])?;
        if n == 0 {
            return if filled == 0 {
                Ok(None)
            } else {
                Err(CodecError::Corrupt(format!(
                    "truncated header: got {filled} of 9 bytes"
                )))
            };
        }
        filled += n;
    }

    let codec = Codec::try_from(header[0])?;
    let c_len = u32::from_le_bytes(header[1..5].try_into().unwrap()) as usize;
    let r_len = u32::from_le_bytes(header[5..9].try_into().unwrap()) as usize;

    if r_len > MAX_BLOCK_SIZE {
        return Err(CodecError::Corrupt(format!(
            "raw length {r_len} exceeds MAX_BLOCK_SIZE {MAX_BLOCK_SIZE}"
        )));
    }

    match codec {
        Codec::Lz4 => {
            if c_len > get_maximum_output_size(r_len) {
                return Err(CodecError::Corrupt(format!(
                    "compressed length {c_len} implausible for raw length {r_len}"
                )));
            }

            let mut compressed = vec![0u8; c_len];
            r.read_exact(&mut compressed).map_err(|e| match e.kind() {
                std::io::ErrorKind::UnexpectedEof => {
                    CodecError::Corrupt(format!("truncated body: expected {c_len} bytes"))
                }
                _ => CodecError::Io(e),
            })?;

            let raw = decompress(&compressed, r_len)
                .map_err(|e| CodecError::Corrupt(format!("lz4 decompress failed: {e}")))?;

            Ok(Some(raw))
        }
        Codec::None => {
            if r_len != c_len {
                return Err(CodecError::Corrupt(format!(
                    "raw length {r_len} not equals compressed length {c_len}"
                )));
            }

            let mut raw = vec![0u8; c_len];
            r.read_exact(&mut raw).map_err(|e| match e.kind() {
                std::io::ErrorKind::UnexpectedEof => {
                    CodecError::Corrupt(format!("truncated body: expected {c_len} bytes"))
                }
                _ => CodecError::Io(e),
            })?;

            Ok(Some(raw))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn round_trip(raw: &[u8]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        write_block(&mut buf, raw).unwrap();
        buf.set_position(0);
        read_block(&mut buf).unwrap().unwrap()
    }

    #[test]
    fn round_trip_empty_input() {
        assert_eq!(round_trip(&[]), Vec::<u8>::new());
    }

    #[test]
    fn round_trip_small_input() {
        assert_eq!(round_trip(b"hello world"), b"hello world".to_vec());
    }

    #[test]
    fn round_trip_highly_compressible_input() {
        let raw = vec![0xABu8; 10_000];
        assert_eq!(round_trip(&raw), raw);
    }

    #[test]
    fn round_trip_incompressible_input() {
        let raw: Vec<u8> = (0..4096u32)
            .map(|i| (i as u8).wrapping_mul(37).wrapping_add(11))
            .collect();
        assert_eq!(round_trip(&raw), raw);
    }

    #[test]
    fn round_trip_at_max_block_size_boundary() {
        let raw = vec![0x7u8; MAX_BLOCK_SIZE];
        let got = round_trip(&raw);
        assert_eq!(got.len(), MAX_BLOCK_SIZE);
        assert_eq!(got, raw);
    }

    #[test]
    #[should_panic(expected = "exceeds MAX_BLOCK_SIZE")]
    fn write_block_panics_when_raw_exceeds_max_block_size() {
        let raw = vec![0u8; MAX_BLOCK_SIZE + 1];
        let mut buf = Cursor::new(Vec::new());
        let _ = write_block(&mut buf, &raw);
    }

    #[test]
    fn read_block_returns_none_on_clean_eof() {
        let mut buf = Cursor::new(Vec::new());
        assert!(matches!(read_block(&mut buf), Ok(None)));
    }

    #[test]
    fn read_block_errors_on_truncated_header() {
        let mut buf = Cursor::new(vec![1u8, 2, 3, 4, 5]);
        let err = read_block(&mut buf).unwrap_err();
        assert!(
            err.to_string()
                .contains("truncated header: got 5 of 8 bytes")
        );
    }

    #[test]
    fn read_block_errors_when_raw_length_exceeds_max_block_size() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_le_bytes()); // c_len
        bytes.extend_from_slice(&(MAX_BLOCK_SIZE as u32 + 1).to_le_bytes()); // r_len
        bytes.push(0); // body byte, never read
        let mut buf = Cursor::new(bytes);
        let err = read_block(&mut buf).unwrap_err();
        assert!(err.to_string().contains("exceeds MAX_BLOCK_SIZE"));
    }

    #[test]
    fn read_block_errors_when_compressed_length_implausible() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&100u32.to_le_bytes()); // c_len, implausible for r_len=0
        bytes.extend_from_slice(&0u32.to_le_bytes()); // r_len
        let mut buf = Cursor::new(bytes);
        let err = read_block(&mut buf).unwrap_err();
        assert!(err.to_string().contains("implausible for raw length"));
    }

    #[test]
    fn read_block_errors_on_truncated_body() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&10u32.to_le_bytes()); // c_len, plausible for r_len=1
        bytes.extend_from_slice(&1u32.to_le_bytes()); // r_len
        bytes.extend_from_slice(&[0u8; 3]); // only 3 of 10 body bytes present
        let mut buf = Cursor::new(bytes);
        let err = read_block(&mut buf).unwrap_err();
        assert!(
            err.to_string()
                .contains("truncated body: expected 10 bytes")
        );
    }

    #[test]
    fn read_block_errors_on_invalid_lz4_stream() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_le_bytes()); // c_len, plausible for r_len=1
        bytes.extend_from_slice(&1u32.to_le_bytes()); // r_len
        bytes.push(0xFF); // token requiring extension bytes that aren't present
        let mut buf = Cursor::new(bytes);
        let err = read_block(&mut buf).unwrap_err();
        assert!(err.to_string().contains("lz4 decompress failed"));
    }

    #[test]
    fn read_block_reads_multiple_sequential_blocks_in_order() {
        let mut buf = Cursor::new(Vec::new());
        write_block(&mut buf, b"first").unwrap();
        write_block(&mut buf, b"").unwrap();
        let third = vec![9u8; 500];
        write_block(&mut buf, &third).unwrap();
        buf.set_position(0);

        assert_eq!(read_block(&mut buf).unwrap(), Some(b"first".to_vec()));
        assert_eq!(read_block(&mut buf).unwrap(), Some(Vec::new()));
        assert_eq!(read_block(&mut buf).unwrap(), Some(third));
        assert!(matches!(read_block(&mut buf), Ok(None)));
    }

    // Note: CodecError::Io passthrough (write-side `?` operators, and the
    // `_ => CodecError::Io(e)` arm in read_block) is not covered here.
    // Cursor<Vec<u8>> never produces arbitrary io::Error kinds, so exercising
    // this path would require a hand-rolled mock Read/Write.
}
