# Benchmark: Iteration 1, baseline (before projection pushdown)

> Zero mark of the project's performance history. Recorded **before** fixing
> `execute` (it filters every column of the block instead of the single one needed).

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-07-22 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |
| Method | median of 7 runs, first run discarded (warm-up), `black_box` |

## Dataset

- 10,000,000 rows, deterministic seed = 42
- Schema: `id: Int64` (sequential), `ts: Int64` (uniform 0..1M), `url: String` (`/page/{0..1000}`), `dur: Int64` (1..10,000)
- Columnar table: blocks of 8,192 rows (1,220 full blocks + a 5,440-row tail)
- Contender: naive `Vec<Row>` with a straightforward loop
- Cross-check: both engines returned identical answers on every query ✅

## Results

### `sum(dur) WHERE ts > X`

| Selectivity | Columnar | Row-based | Ratio (row/col) |
|---|---|---|---|
| ~1%  | 56.3 ms  | 25.6 ms | **0.5x** |
| ~50% | 279.6 ms | 45.8 ms | **0.2x** |
| ~99% | 267.0 ms | 27.0 ms | **0.1x** |

### `sum(dur)` without a filter

| Query | Columnar | Row-based | Ratio (row/col) |
|---|---|---|---|
| no filter | 6.8 ms | 23.4 ms | **3.4x** |

Columnar aggregation throughput: 80 MB / 6.8 ms ≈ **11.7 GB/s** —
memory speed; SIMD is working, the aggregation core is healthy.

### `count() WHERE ts > X`

| Selectivity | Columnar | Row-based | Ratio (row/col) |
|---|---|---|---|
| ~1%  | 58.5 ms  | 21.8 ms | **0.4x** |
| ~50% | 289.5 ms | 26.1 ms | **0.1x** |
| ~99% | 273.9 ms | 21.8 ms | **0.1x** |

## Interpretation

**The columnar engine loses on every filtered query — and this is the measured
cost of a known issue, not of the architecture.**

1. **Diagnosis.** `execute` calls `Block::filter(&mask)`, which copies
   **all four** columns of the block, including the heavy `url: String`.
   Filtering a string column means a `clone()` (an allocation) for every
   surviving row. At 99% selectivity that is ~9.9M `String` allocations whose
   results nobody uses: `sum(dur)` only looks at `dur`, and `count` only at
   the length.

2. **Confirmation from the numbers.** Without a filter (no `Block::filter`)
   the columnar engine is 3.4x faster. With a filter, time explodes from
   6.8 ms to 270–290 ms — even though the `ts` pass itself (building the mask)
   costs ~7 ms. The difference is the cost of cloning columns nobody needs.

3. **Selectivity dependence.** 1% (56 ms) is faster than 50–99% (~280 ms)
   because there are proportionally fewer clones. The row engine shows its own
   effect: 50% (45.8 ms) is slower than the edges (~26 ms) — the `r.ts > x`
   branch is unpredictable there, and branch-predictor misses take their toll.

4. **The fast paths in `Column::filter`** (all-true / all-false mask) never
   fired on random data — as expected; they are designed for sorted data and
   will pay off with the sparse index (iteration 4).

## Planned fix

Projection pushdown in `execute`: filter only the aggregate's column instead
of the whole block — `block.column(agg).filter(&mask)`. `Block::filter` stays
in the API as the general mechanism for consumers that need several columns.

Post-fix forecast: columnar filtered sum at ~15–35 ms,
ratio 1–3x in favor of columns at every selectivity, peaking at ~1%.

## Lessons

- The benchmark paid for itself on the very first run: it turned a code-review
  note ("all columns get filtered") into a measured cost of ~270 ms.
- "The executor must filter only what is consumed downstream" — a hands-on
  discovery of projection pushdown, one of the key optimizations in analytical
  databases. To be continued in iteration 2: don't read unneeded files from disk.

# Benchmark: Iteration 1, after projection pushdown

> Second mark of the project's performance history. Recorded **after** the
> `execute` fix: only the aggregate's column is filtered
> (`block.column(agg).filter(&mask)`), not the whole block.
> For the "before" baseline, see `benchmark-iteration-1-en.md`.

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-07-23 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |
| Method | median of 7 runs, first run discarded (warm-up), `black_box` |

Dataset and contender unchanged (10M rows, seed 42, 8,192-row blocks,
naive `Vec<Row>`). Cross-check passed on every query ✅

## Results

### `sum(dur) WHERE ts > X`

| Selectivity | Columnar | Row-based | Ratio (row/col) | Before (col.) | Speedup |
|---|---|---|---|---|---|
| ~1%  | 18.6 ms | 24.7 ms | **1.3x** | 56.3 ms  | 3.0× |
| ~50% | 45.6 ms | 56.3 ms | **1.2x** | 279.6 ms | 6.1× |
| ~99% | 20.4 ms | 24.3 ms | **1.2x** | 267.0 ms | **13.1×** |

### `sum(dur)` without a filter

| Query | Columnar | Row-based | Ratio (row/col) |
|---|---|---|---|
| no filter | 7.0 ms | 26.4 ms | **3.7x** |

### `count() WHERE ts > X`

| Selectivity | Columnar | Row-based | Ratio (row/col) | Before (col.) |
|---|---|---|---|---|
| ~1%  | 18.3 ms | 24.0 ms | **1.3x** | 58.5 ms |
| ~50% | 44.0 ms | 26.5 ms | **0.6x** | 289.5 ms |
| ~99% | 19.7 ms | 26.6 ms | **1.3x** | 273.9 ms |

## Forecast check

Forecast from the "before" report: columnar filtered sum at ~15–35 ms,
ratio 1–3x in favor of columns at every selectivity. Actual: 18–46 ms,
ratio 1.2–1.3x on sum — forecast confirmed. A single line of code delivered
up to a 13x speedup: that is the measured cost of cloning unneeded
`String` columns.

## Interpreting the remaining effects

1. **Why ~50% is slower than the edges (45 ms vs ~19 ms).** The url clones
   are gone; the remainder is the branch predictor struggling with an
   unpredictable mask inside our own pipeline (`filter_map` in
   `Column::filter`), plus the `eval_predicate` and `cap`-counting passes.
   A possible cure — branch-free filtering (always write, advance the index
   by `m as usize`) — is deferred to the backlog until iteration 6
   (vectorization), to be decided by a criterion duel.

2. **The count ~50% anomaly: the only 0.6x ratio — credit to the contender,
   not a flaw of columns.** The row-based `count()` compiles to branch-free
   code (compare → increment), so its times are flat across selectivities
   (24–26.5 ms), unlike its sum counterpart (56 ms at 50%). Meanwhile the
   columnar count still drags the full pipeline: mask → column filter → `len`.

## Optimization backlog (not now)

- **Aggregate pushdown for count**: count the trues in the mask without
  materializing a column — `cap` is already computed in `filter`. Removes
  the 0.6x anomaly.
- **Branch-free filtering** in `Column::filter` — a criterion-duel candidate
  for iteration 6.
- **Mask fast paths** (all-true/all-false) never fire on random data; they
  will pay off on sorted data with the sparse index (iteration 4).

## Iteration 1 verdict

The columnar engine beats the row-based one on every sum query (1.2–1.3x
filtered, 3.7x unfiltered) with aggregation throughput of ~11.4 GB/s
(80 MB / 7.0 ms) — memory-bound, the core is healthy. The main lesson of the
iteration: **the executor must process only the data that is consumed
downstream** — projection pushdown delivered up to 13x and will continue on
disk (iteration 2: don't read unneeded files) and in the index
(iteration 4: don't read unneeded granules).

# Benchmark: Iteration 2, subtask 2.0 — string filter BEFORE migration

> Baseline for measuring the effect of migrating the string column
> from `Vec<String>` to `data: Vec<u8> + offsets: Vec<u32>`.
> Recorded **before** the cascade: the column still stores `Vec<String>`.

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-07-29 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |
| Method | median of 7 runs, first run discarded (warm-up), `black_box` |

Dataset and contender unchanged (10M rows, seed 42, 8,192-row blocks,
naive `Vec<Row>`). Cross-check passed ✅

## Query

`count() WHERE url == "/page/42"`

- Selectivity ~0.1%: urls are uniform over one thousand values → ~10K matches
- After projection pushdown, materialization is negligible (count over ~10K
  rows of the id column) — the measurement consists almost entirely of
  `eval_predicate` over `url`: 10M string-vs-constant comparisons.
  We measure exactly what we are about to migrate.

## Result

| Query | Columnar | Row-based | Ratio (row/col) |
|---|---|---|---|
| `count WHERE url == "/page/42"` | 50.2 ms | 54.8 ms | **1.1x** |

## Interpretation

1. **A string predicate is ~7x more expensive than a numeric one.**
   A mask over a numeric column (`ts > X`) is estimated at ~7 ms — from the
   project's reference number in the no-filter run (an 80 MB pass at
   ~11.7 GB/s); the string predicate over `url` costs 50 ms for the same
   number of rows. The tax is twofold: dereferencing the `String` pointer
   (a random jump into the heap → a cache miss per row) plus a
   variable-length memcmp instead of a single SIMD compare instruction.

2. **A ratio of only 1.1x — columnar in form only.** The `String` structs
   in `Vec<String>` are laid out contiguously, but the *string bytes* are
   scattered across the heap just like the row-based contender's. The main
   advantage of columns — dense data layout — does not apply to the string
   column in its current representation. That is exactly what the
   `data + offsets` migration fixes.

## Post-migration forecast

- A pass over a contiguous buffer: ~140 MB of data + 40 MB of offsets,
  sequential reads, the prefetcher does its job.
- The `bytes_at` bonus: early rejection by length
  (`end - start != x.len()`) before touching the bytes; urls are 7–10
  characters long → a noticeable share of candidates is discarded
  without a memcmp.
- **Prediction: 12–25 ms (2–4x off the current 50 ms).** The row engine
  does not change → the ratio should grow to ~2.5–4x.

## Estimation method (back-of-the-envelope)

The project's reference number: memory bandwidth of ~11.7 GB/s (from the
no-filter run). Any pipeline stage is estimated as "bytes read/written ÷
memory bandwidth", since analytical workloads are memory-bound.
An estimate-vs-fact gap of several times is not an estimation error but a
finding: here it pointed at random heap access in `Vec<String>`, where the
sequential-read yardstick does not apply.

# Benchmark: Iteration 2, subtask 2.0 — string filter AFTER migration

> Companion to `benchmark-2-0-string-before-en.md`. The string column now
> stores `data: Vec<u8> + offsets: Vec<u32>`; the predicate compares byte
> slices via `bytes_at` (no UTF-8 validation, no per-row allocations).
> Cross-check passed before and after ✅ — the migration is correct.

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-07-30 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |
| Method | median of 7 runs, first run discarded (warm-up), `black_box` |

## Result

`count() WHERE url == "/page/42"` (selectivity ~0.1%)

| Stage | Before | After | Change |
|---|---|---|---|
| Full query (columnar) | 50.2 ms | 48.4 ms | **−4%** |
| Row-based contender | 54.8 ms | 54.9 ms | unchanged |
| Ratio (row/col) | 1.1x | 1.1x | unchanged |

Isolated measurement (new): **mask only (`eval_predicate` over url) = 38.2 ms**,
i.e. 79% of the query. The remaining ~10 ms: filtering the 80 MB id column
by mask (~7–8 ms) + cap counting + count itself.

## Forecast check — a miss, and why

Forecast: 12–25 ms (2–4x speedup). Actual: 48.4 ms (−4%). A miss by an
order of magnitude in effect size — which means the mental model was wrong.
The post-mortem:

1. **The task was never memory-bound.** 38.2 ms / 10M rows = 3.8 ns ≈ 17–18
   CPU cycles per row, while reading the ~18 bytes/row of data + offsets at
   the reference 11.7 GB/s costs ~1.5 ns. The bottleneck is per-row
   *processing*, not data layout — and the migration changes layout only.

2. **Why "before" wasn't as bad as theorized.** The cache-miss horror story
   assumed `String` bytes scattered across the heap. In reality the benchmark
   pushes 10M small strings back-to-back in a fresh process — the allocator
   places them nearly sequentially, and the prefetcher forgives the walk.
   The theory of random heap access was correct in general and inapplicable
   to this allocation pattern. **Lesson: memory predictions must be checked
   against the actual allocation pattern, not the worst case.**

3. **Where the 17–18 cycles go** (per row): two offsets loads with bounds
   checks, slice construction with a range check, then `&[u8] == &[u8]` —
   which compares lengths first. Url lengths are 7–10 bytes and the constant
   is 8, so the length test passes for ~10% of rows: an unpredictable branch
   (~10/90) costing 15–20 cycles per miss, then an 8-byte memcmp for the
   survivors. The "free early rejection by length" from the forecast is, on
   this data, an unpredictable branch per row — the opposite of free.

## What the migration did buy

- The on-disk format prerequisite: `data + offsets` serializes as two flat
  dumps (`url.data.bin`, `url.offsets.bin`) — the actual goal of 2.0.
- `filter` without per-row allocations (byte-range copies), `push_str`
  without allocation.
- Predicate is now compute-bound with dense data — the correct starting
  point for vectorized string operations later.

## Optimization backlog (updated)

- **Vectorized length pre-pass for string equality**: first pass compares
  lengths as numbers over the offsets column (`offsets[i+1] - offsets[i] == k`
  — pure SIMD-friendly arithmetic), second pass memcmps only the ~10% that
  survive. Turns the unpredictable per-row branch into a dense numeric scan.
  This is how mature engines build string ops from columnar primitives.
- **Aggregate pushdown for count** (from iteration 1 backlog) — now priced:
  ~8 ms of this query (filtering 80 MB of id to take its length).
- Branch-free filtering, mask fast paths on sorted data — unchanged.

## Verdict

The migration is a **format win, not a speed win** — and that is fine:
2.0 was a prerequisite for disk (iteration 2), not an optimization. The
measured story (forecast 12–25 ms → fact 48.4 ms → dissection → diagnosis)
is kept as documentation of how to respond to a model miss: isolate, count
cycles, name the lesson. Next: 2.1, the block codec — writing bytes to disk,
which is what `data + offsets` was for.
