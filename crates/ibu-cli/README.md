# ibu-cli

Command-line toolkit for [IBU files](https://github.com/noamteyssier/ibu) — a binary format for barcode, UMI, and index data in high-throughput genomics, inspired by [BUS](https://github.com/BUStools/BUS-format).

Has some modifications to the format specifically around different variants of the record type.
For more information see the [IBU repo](https://github.com/noamteyssier/ibu).

```bash
cargo install ibu-cli
```

## Pipeline

The commands compose into the standard quantification pipeline:

```bash
ibu sort raw.ibu --pipe --dedup \
  | ibu umi --pipe \
  | ibu count --features features.tsv --output counts.tsv
```

Conventions shared by all commands:

- Input defaults to stdin, so commands chain with pipes. Inputs may be gzip- or zstd-compressed.
- Commands that emit binary IBU refuse to write to a terminal: pass `-o/--output` or opt in to stdout with `-p/--pipe`.
- `umi`, `consensus`, and `count` require sorted input and will reject unsorted files.
- Commands that correct or aggregate (`umi`, `consensus`, `count`) write summary statistics as JSON to stderr, or to a file with `-l/--log`.

## Commands

### `view`

Print an IBU file as text.
`-d` decodes barcodes and UMIs from 2-bit to nucleotides, `-H` prints only the header, and `-f features.tsv` maps index values to feature names.

### `cat`

Concatenate IBU files. Inputs must share barcode/UMI lengths and record type; records stream through untouched.
The output is marked unsorted, so re-sort before downstream steps.

### `sort`

Sort by (barcode, UMI, index).
Uses an on-disk merge sort bounded by `-m/--memory-limit-mb` (default 5 GiB), so file size is not limited by RAM; `--in-memory` skips the spill for small inputs and `-T` parallelizes the merge.
`-d/--dedup` collapses identical records into counted records during the sort (do this early), since single-cell data is heavily repetitive and every downstream step gets cheaper.

### `umi`

Correct sequencing errors in UMIs: within each (barcode, index) group, UMIs within Hamming distance 1 are clustered (transitively) and rewritten to the cluster's most abundant UMI.
Input must be sorted.

### `consensus`

For extended files (records carrying a sequence payload): collapse each (barcode, UMI, index) group to its most abundant sequence variant, resolving read-level disagreement to one sequence per molecule.
Input must be sorted.

### `count`

Count unique UMIs per (barcode, index) — the terminal step producing a count matrix.
Output is a TSV by default, or a 10x-style MatrixMarket directory with `--mtx`.
`-f features.tsv` maps indices to feature names, and a nonzero `-C/--feature-col` aggregates counts over that column's names (e.g. probes into genes).
For extended files, `-q` additionally writes per-sequence UMI counts.
