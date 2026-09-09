# Benchmark: Iteration 3, subtask 3.0 — the cost of fragmentation

> Baseline "before" measurement for merges. A table of 1,220 single-block
> parts vs the single-part table from iteration 2 — same data, same codec
> (None), same queries. Merges (3.3–3.5) must win against these numbers.

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-08-27 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |
| Dataset | 10M rows, seed 42, 8,192-row blocks, Codec::None |
| Fragmented table | one `insert` per block → 1,220 parts of one block each |

## Results

| Metric | 1 part | 1,220 parts | Fragmentation tax |
|---|---|---|---|
| Insert (10M rows) | 266 ms | 1,387 ms | **5.2×** |
| sum no-filter, warm | 11.5 ms | 143.4 ms | **12.5×** |
| vs row engine (warm) | 1.7x faster | **0.1x — 7× slower** | |

Cross-check passed (fragmented table returns identical answers). ✅

## Interpretation

**Write side: ~0.9 ms per extra part.** The +1.12 s over one part spreads
across 1,219 extra parts — dominated by filesystem metadata operations
(create_dir, 4× File::create, schema.txt write, flushes, rename), not by
data bytes. This is one half of why real systems shout "too many parts."

**Read side: ~0.11 ms per part opened.** +132 ms across 1,220 opens =
schema.txt parse + File::open (one column under projection) + final
num_rows check. Notably cheap per part — but multiplied by part count it
turns an 11.5 ms query into 143 ms and hands the row engine a 7× lead.

## Acceptance criterion for the iteration

After `optimize()` merges the fragmented table back into one part, this
same benchmark line must return to ~11.5 ms. The number to beat is now
fixed in advance.
