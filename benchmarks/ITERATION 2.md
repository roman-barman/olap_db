# Benchmark: Iteration 2, writing the on-disk tables

> Write-side results of the disk benchmark: insert timings, on-disk sizes,
> per-file compression breakdown, and codec-fallback evidence read from disk.
> Read-side matrix (cold/warm × None/LZ4) follows in a separate report.

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-08-19 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |

Dataset: 10M rows, seed 42, 8,192-row blocks (1,220 full + tail), schema
`id: Int64 (sequential), ts: Int64 (uniform 0..1M), url: String (/page/{0..1000}), dur: Int64 (1..10K)`.
Each table is written as **one part** via a single `insert(&blocks)` —
fragmentation cost is deliberately excluded (it is iteration 3's subject).

## Insert timings

| Table | Time | Throughput (~220 MB raw) |
|---|---|---|
| Codec::None | 266 ms | ~830 MB/s |
| Codec::Lz4  | 704 ms | ~310 MB/s |

## On-disk sizes

| Table | Size | Ratio |
|---|---|---|
| None | 369.0 MB | 1.00 |
| LZ4  | 199.1 MB | **1.85× smaller** |

### Per-file breakdown (part_0000)

| File | None | LZ4 | Compression | Why |
|---|---|---|---|---|
| id.bin | 77 MB | 39 MB | 2.0× | sequential i64: 6–7 shared high bytes between neighbors — LZ4 finds the windows |
| ts.bin | 77 MB | 46 MB | 1.7× | uniform 0..1M in i64 → **5 high bytes are always zero**; LZ4 eats zeros |
| dur.bin | 77 MB | 38 MB | 2.0× | 0..10K in i64 → 6 zero bytes per value |
| url.data.bin | 85 MB | 32 MB | 2.7× | 1,000 repeated urls — but the LZ4 dictionary lives inside one 8,192-row block, capping the win |
| url.offsets.bin | 39 MB | 39 MB | **1.0×** | ascending u32 with variable step: no byte repeats — LZ4 gets nothing |
| schema.txt | 4 KB | 4 KB | — | |

Header overhead: 4,880 blocks × 9 bytes ≈ 43 KB per numeric column — 0.05%,
the fixed 9-byte header is effectively free.

## Codec fallback: verified on disk

`write_block` falls back to None when `compress(raw).len() >= raw.len()`,
and the block's codec byte records the **actual** codec. Read straight from
the LZ4 table with `xxd -l 1`:

| File (lz4 table) | First codec byte | Meaning |
|---|---|---|
| url.offsets.bin | `00` | **fallback fired** — incompressible, stored as None |
| url.data.bin | `01` | compressed as LZ4 |

Corroborated by sizes: url.offsets.bin is byte-identical in both tables
(39 MB). The format is self-describing per block; the reader needs no table-
level codec knowledge. (Strictly, `xxd -l 1` shows the first of 1,220 blocks;
identical file sizes make per-block fallback the only plausible story.)

## Takeaways

1. **Compression is a CPU-for-bytes trade, and page cache hides the byte
   savings at write time** — LZ4 insert is 2.6× slower with zero write-time
   benefit; the payoff, if any, is at read time.
2. **Number "compression" here is mostly zero-byte padding**: i64 for values
   under a million wastes 5 bytes/value, and LZ4 merely claws that back.
   Delta / narrower types (backlog, iteration 6+) attack the same waste
   directly — measured motivation: id at 2.0× vs Delta's potential ~10–20×.
3. **url.offsets is the ideal Delta client and LZ4's blind spot** — ascending
   values, zero LZ4 gain, fallback fires. First concrete target for custom
   codecs.
4. **Block-local dictionaries cap string compression** (2.7× vs naive 5–10×
   expectation): compression ratio depends on block size — a tunable to
   revisit.

# Benchmark: Iteration 2, subtask 2.6 — full-scan read speed

> Read-side, part 1: full table scan (all four columns) over both on-disk
> tables, cold and warm. Companion to the write-side report.
> The projected-query matrix via `execute` follows separately.

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-08-21 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |

Method: scan of all columns (`id, ts, url, dur`) through `Table::scan`,
accumulating `num_rows` under `black_box`; `Table::open` outside the timed
region. Cold = first run after process start; warm = subsequent run
(page cache hot). Tables: one part each, 10M rows (see write-side report).

## Results

| Run | None (369 MB) | LZ4 (199 MB) | Faster |
|---|---|---|---|
| Cold | 449.0 ms | 279.8 ms | **LZ4, 1.6×** |
| Warm | 149.0 ms | 201.1 ms | **None, 1.35×** |

## Interpretation

1. **Cold: compression pays exactly where write-time hid it.** Reading from
   disk, None hauls 369 MB, LZ4 hauls 199 MB; the byte savings outweigh
   decompression cost. The write-side lesson inverts on the read side:
   CPU-for-bytes is a losing trade into the page cache and a winning one
   out of the disk.

2. **Warm: roles flip.** With all bytes in RAM, None is pure block assembly
   (149 ms); LZ4 adds ~52 ms of decompression over ~200 MB compressed —
   ~4 GB/s, LZ4's nameplate decompression speed observed live.

3. **Codec choice is a bet on the cache profile of the workload**: hot
   working sets favor None, cold scans favor LZ4. (Production systems
   default to LZ4 because working sets rarely fit in RAM at scale — and
   storage bytes cost money regardless.)

4. **Warm-None throughput ≈ 2.5 GB/s end-to-end** (369 MB / 149 ms) — well
   below memory bandwidth (~11.7 GB/s reference). Block assembly (column
   allocation, StringColumn reconstruction from chunks, per-block reader
   plumbing) is now a visible cost center, not just memcpy. Profiling
   candidate — backlog, not now.
