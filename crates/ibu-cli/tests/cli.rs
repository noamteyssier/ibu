//! End-to-end tests driving the `ibu` binary.

use std::path::Path;
use std::process::Command;

use ibu::{ExtRecord, Header, IbuRecord, Reader, Record, RecordCount, Writer};

fn ibu_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ibu"))
}

fn write_records<T: IbuRecord>(path: &Path, records: &[T]) {
    let header = Header::new(16, 12);
    let mut writer: Writer<_, T> =
        Writer::new(std::fs::File::create(path).unwrap(), header).unwrap();
    writer.write_batch(records).unwrap();
    writer.finish().unwrap();
}

/// An unsorted, heavily duplicated record set
fn test_records() -> Vec<Record> {
    (0..10_000u64)
        .rev()
        .map(|i| Record::new(i % 10, i % 7, i % 3))
        .collect()
}

#[test]
fn test_sort() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let output = dir.path().join("sorted.ibu");
    write_records(&input, &test_records());

    let status = ibu_bin()
        .arg("sort")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());

    let reader = Reader::new(std::fs::File::open(&output).unwrap()).unwrap();
    assert!(reader.header().sorted());
    let records: Vec<Record> = reader
        .iter_records()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(records.len(), 10_000);
    assert!(records.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn test_sort_default_output_path() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let derived = dir.path().join("in.sort.ibu");
    write_records(&input, &test_records());

    // no -o and no -p: output path is derived from the input
    let out = ibu_bin().arg("sort").arg(&input).output().unwrap();
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Writing sorted output to:"),
        "derived output path should be announced on stderr"
    );

    let reader = Reader::new(std::fs::File::open(&derived).unwrap()).unwrap();
    assert!(reader.header().sorted());
    let records: Vec<Record> = reader
        .iter_records()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(records.len(), 10_000);
}

#[test]
fn test_sort_stdin_requires_output_target() {
    use std::process::Stdio;

    // stdin input with neither -o nor -p is an error (no name to derive)
    let out = ibu_bin().arg("sort").stdin(Stdio::null()).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("requires an output target"));
}

#[test]
fn test_sort_dedup() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let external = dir.path().join("dedup.ibu");
    let in_memory = dir.path().join("dedup_mem.ibu");
    write_records(&input, &test_records());

    for (output, extra) in [(&external, None), (&in_memory, Some("--in-memory"))] {
        let mut cmd = ibu_bin();
        cmd.args(["sort", "--dedup"])
            .arg(&input)
            .arg("-o")
            .arg(output);
        if let Some(extra) = extra {
            cmd.arg(extra);
        }
        assert!(cmd.status().unwrap().success());

        let reader = Reader::new(std::fs::File::open(output).unwrap()).unwrap();
        assert!(reader.header().sorted());
        assert!(reader.header().counts());
        let records: Vec<RecordCount> = reader
            .iter_record_counts()
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(records.windows(2).all(|w| w[0] < w[1]));
        let total: u64 = records.iter().map(|r| r.count).sum();
        assert_eq!(total, 10_000);
    }

    // both sort strategies produce identical files
    assert_eq!(
        std::fs::read(&external).unwrap(),
        std::fs::read(&in_memory).unwrap()
    );
}

#[test]
fn test_cat() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.ibu");
    let b = dir.path().join("b.ibu");
    let output = dir.path().join("cat.ibu");
    write_records(&a, &[Record::new(1, 2, 3), Record::new(4, 5, 6)]);
    write_records(&b, &[Record::new(7, 8, 9)]);

    let status = ibu_bin()
        .arg("cat")
        .args([&a, &b])
        .arg("-o")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());

    let reader = Reader::new(std::fs::File::open(&output).unwrap()).unwrap();
    let records: Vec<Record> = reader
        .iter_records()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        records,
        vec![
            Record::new(1, 2, 3),
            Record::new(4, 5, 6),
            Record::new(7, 8, 9)
        ]
    );
}

#[test]
fn test_cat_rejects_mismatched_record_types() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.ibu");
    let b = dir.path().join("b.ibu");
    write_records(&a, &[Record::new(1, 2, 3)]);
    write_records(&b, &[ExtRecord::from_sequence(1, 2, 3, b"ACGT").unwrap()]);

    let out = ibu_bin()
        .arg("cat")
        .args([&a, &b])
        .arg("-p")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("record types do not match"));
}

#[test]
fn test_view() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    // barcode "AAAC..." = 0b01 << 4 (base 2 = C), umi all A
    write_records(&input, &[Record::new(16, 0, 42)]);

    // encoded view
    let out = ibu_bin().arg("view").arg(&input).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("# barcode_len: 16"));
    assert!(text.contains("16\t0\t42"));

    // decoded view without the header block
    let out = ibu_bin()
        .args(["view", "-d", "-S"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.trim(), "AACAAAAAAAAAAAAA\tAAAAAAAAAAAA\t42");
}

#[test]
fn test_view_ext_counted() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let record = ExtRecord::from_sequence(0, 0, 7, b"ACGTACGT").unwrap();
    write_records(&input, &[record.to_counted(99)]);

    let out = ibu_bin().args(["view", "-S"]).arg(&input).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    // barcode, umi, index, sequence, count
    assert_eq!(text.trim(), "0\t0\t7\tACGTACGT\t99");
}

/// Sorted records where each (barcode, index) group has a dominant UMI plus a
/// rare HD=1 error UMI that should be corrected into it.
fn umi_error_records() -> Vec<Record> {
    let mut records = Vec::new();
    for barcode in 0..50u64 {
        for index in 0..4u64 {
            let dominant = 0b0000; // "AA..."
            let error = 0b0001; // single-base substitution (HD=1)
            for _ in 0..10 {
                records.push(Record::new(barcode, dominant, index));
            }
            records.push(Record::new(barcode, error, index));
        }
    }
    records.sort_unstable();
    records
}

#[test]
fn test_umi_correction() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let derived = dir.path().join("in.umi.ibu");
    write_records(&input, &umi_error_records());

    let out = ibu_bin().arg("umi").arg(&input).output().unwrap();
    assert!(out.status.success());

    // stats JSON lands on stderr: 200 of 2200 reads corrected
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("\"total\": 2200"), "stderr: {stderr}");
    assert!(stderr.contains("\"corrected\": 200"), "stderr: {stderr}");

    let reader = Reader::new(std::fs::File::open(&derived).unwrap()).unwrap();
    assert!(reader.header().sorted());
    let records: Vec<Record> = reader
        .iter_records()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(records.len(), 2200);
    assert!(
        records.windows(2).all(|w| w[0] <= w[1]),
        "output not sorted"
    );
    assert!(
        records.iter().all(|r| r.umi == 0),
        "uncorrected UMIs remain"
    );
}

#[test]
fn test_umi_correction_threaded_matches_single() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let single = dir.path().join("single.ibu");
    let threaded = dir.path().join("threaded.ibu");
    write_records(&input, &umi_error_records());

    for (output, threads) in [(&single, "1"), (&threaded, "4")] {
        let status = ibu_bin()
            .arg("umi")
            .arg(&input)
            .arg("-o")
            .arg(output)
            .args(["-T", threads])
            .status()
            .unwrap();
        assert!(status.success());
    }

    // ticketed writer keeps output deterministic across thread counts
    assert_eq!(
        std::fs::read(&single).unwrap(),
        std::fs::read(&threaded).unwrap()
    );
}

#[test]
fn test_umi_correction_counted_records() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let output = dir.path().join("out.ibu");

    // the error UMIs have more rows but fewer reads - counts decide abundance
    let records = vec![
        RecordCount::new(Record::new(0, 0b0000, 0), 100),
        RecordCount::new(Record::new(0, 0b0001, 0), 2),
        RecordCount::new(Record::new(0, 0b0010, 0), 3),
    ];
    write_records(&input, &records);

    let out = ibu_bin()
        .arg("umi")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("\"total\": 105"), "stderr: {stderr}");
    assert!(stderr.contains("\"corrected\": 5"), "stderr: {stderr}");

    let reader = Reader::new(std::fs::File::open(&output).unwrap()).unwrap();
    assert!(reader.header().counts());
    let corrected: Vec<RecordCount> = reader
        .iter_record_counts()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // the counted stream stays deduplicated: one merged record per triple
    assert_eq!(corrected, vec![RecordCount::new(Record::new(0, 0, 0), 105)]);
}

#[test]
fn test_umi_correction_rejects_unsorted() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    write_records(&input, &[Record::new(5, 0, 0), Record::new(1, 0, 0)]);

    let out = ibu_bin().arg("umi").arg(&input).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unsorted"));
}

/// Sorted extended records where each (barcode, UMI) group has a dominant
/// sequence plus a rare variant that should be consolidated into it.
fn consensus_variant_records() -> Vec<ExtRecord> {
    let mut records = Vec::new();
    for barcode in 0..50u64 {
        for umi in 0..4u64 {
            for _ in 0..10 {
                records.push(ExtRecord::from_sequence(barcode, umi, 0, b"ACGT").unwrap());
            }
            records.push(ExtRecord::from_sequence(barcode, umi, 0, b"ACGG").unwrap());
        }
    }
    records.sort_unstable();
    records
}

#[test]
fn test_consensus() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let derived = dir.path().join("in.consensus.ibu");
    write_records(&input, &consensus_variant_records());

    let out = ibu_bin().arg("consensus").arg(&input).output().unwrap();
    assert!(out.status.success());

    // stats JSON lands on stderr: 200 of 2200 reads consolidated
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("\"total\": 2200"), "stderr: {stderr}");
    assert!(stderr.contains("\"consolidated\": 200"), "stderr: {stderr}");

    let reader = Reader::new(std::fs::File::open(&derived).unwrap()).unwrap();
    assert!(reader.header().sorted());
    let records: Vec<ExtRecord> = reader
        .iter_ext_records()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(records.len(), 2200);
    assert!(
        records.windows(2).all(|w| w[0] <= w[1]),
        "output not sorted"
    );
    assert!(
        records
            .iter()
            .all(|r| r.decode_sequence().unwrap().seq() == b"ACGT"),
        "unconsolidated sequence variants remain"
    );
}

#[test]
fn test_consensus_threaded_matches_single() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let single = dir.path().join("single.ibu");
    let threaded = dir.path().join("threaded.ibu");
    write_records(&input, &consensus_variant_records());

    for (output, threads) in [(&single, "1"), (&threaded, "4")] {
        let status = ibu_bin()
            .arg("consensus")
            .arg(&input)
            .arg("-o")
            .arg(output)
            .args(["-T", threads])
            .status()
            .unwrap();
        assert!(status.success());
    }

    // ticketed writer keeps output deterministic across thread counts
    assert_eq!(
        std::fs::read(&single).unwrap(),
        std::fs::read(&threaded).unwrap()
    );
}

#[test]
fn test_consensus_counted_records() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let output = dir.path().join("out.ibu");

    // the other variants have more rows but fewer reads - counts decide abundance
    let records = vec![
        ExtRecord::from_sequence(0, 0, 0, b"ACGG")
            .unwrap()
            .to_counted(100),
        ExtRecord::from_sequence(0, 0, 0, b"ACGT")
            .unwrap()
            .to_counted(2),
        ExtRecord::from_sequence(0, 0, 0, b"TTTT")
            .unwrap()
            .to_counted(3),
    ];
    write_records(&input, &records);

    let out = ibu_bin()
        .arg("consensus")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("\"total\": 105"), "stderr: {stderr}");
    assert!(stderr.contains("\"consolidated\": 5"), "stderr: {stderr}");

    let reader = Reader::new(std::fs::File::open(&output).unwrap()).unwrap();
    assert!(reader.header().counts());
    let consolidated: Vec<_> = reader
        .iter_ext_record_counts()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // the counted stream stays deduplicated: one merged record per triple
    assert_eq!(consolidated.len(), 1);
    assert_eq!(consolidated[0].count, 105);
    assert_eq!(
        consolidated[0].record.decode_sequence().unwrap().seq(),
        b"ACGG"
    );
}

#[test]
fn test_consensus_rejects_unsorted() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    write_records(
        &input,
        &[
            ExtRecord::from_sequence(5, 0, 0, b"ACGT").unwrap(),
            ExtRecord::from_sequence(1, 0, 0, b"ACGT").unwrap(),
        ],
    );

    let out = ibu_bin().arg("consensus").arg(&input).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unsorted"));
}

#[test]
fn test_consensus_rejects_classic_records() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    write_records(&input, &[Record::new(0, 0, 0)]);

    let out = ibu_bin().arg("consensus").arg(&input).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("extended"));
}

#[test]
fn test_sort_pipe_roundtrip() {
    use std::io::Write as _;
    use std::process::Stdio;

    // stream an unsorted file through `ibu sort -p` and read the stdout bytes back
    let records = test_records();
    let header = Header::new(16, 12);
    let mut writer: Writer<_, Record> = Writer::new(Vec::new(), header).unwrap();
    writer.write_batch(&records).unwrap();
    writer.finish().unwrap();
    let input_bytes = writer.into_inner();

    let mut child = ibu_bin()
        .args(["sort", "-p"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&input_bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());

    let reader = Reader::new(std::io::Cursor::new(out.stdout)).unwrap();
    assert!(reader.header().sorted());
    let sorted: Vec<Record> = reader
        .iter_records()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(sorted.len(), records.len());
    assert!(sorted.windows(2).all(|w| w[0] <= w[1]));
}

/// A sorted record set with a known UMI structure:
/// barcode 0 holds two UMIs on index 0 (one duplicated) and one on index 1;
/// barcode 1 holds one UMI on index 1.
fn count_records() -> Vec<Record> {
    vec![
        Record::new(0, 0, 0),
        Record::new(0, 0, 0),
        Record::new(0, 1, 0),
        Record::new(0, 2, 1),
        Record::new(1, 0, 1),
    ]
}

fn read_to_string(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn test_count() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let output = dir.path().join("counts.tsv");
    let log = dir.path().join("stats.json");
    write_records(&input, &count_records());

    let status = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("-l")
        .arg(&log)
        .status()
        .unwrap();
    assert!(status.success());

    assert_eq!(read_to_string(&output), "0\t0\t2\n0\t1\t1\n1\t1\t1\n");

    let stats = read_to_string(&log);
    assert!(stats.contains("\"reads\": 5"));
    assert!(stats.contains("\"umis\": 4"));
    assert!(stats.contains("\"counted\": 4"));
    assert!(stats.contains("\"tied\": 0"));
}

#[test]
fn test_count_decode_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let output = dir.path().join("counts.tsv");
    write_records(&input, &count_records());

    let status = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .args(["--decode", "--suffix", "1"])
        .status()
        .unwrap();
    assert!(status.success());

    // barcode 0 decodes to 16 A's (bc_len=16); barcode 1 has a trailing C
    assert_eq!(
        read_to_string(&output),
        "AAAAAAAAAAAAAAAA-1\t0\t2\n\
         AAAAAAAAAAAAAAAA-1\t1\t1\n\
         CAAAAAAAAAAAAAAA-1\t1\t1\n"
    );
}

#[test]
fn test_count_counted_records_weigh_by_multiplicity() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let output = dir.path().join("counts.tsv");
    // one UMI split across two indices: the stored counts break the tie
    write_records(
        &input,
        &[
            RecordCount::new(Record::new(0, 0, 0), 5),
            RecordCount::new(Record::new(0, 0, 1), 2),
        ],
    );

    let status = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(read_to_string(&output), "0\t0\t1\n");
}

#[test]
fn test_count_features_and_aggregation() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let features = dir.path().join("features.tsv");
    let output = dir.path().join("counts.tsv");
    write_records(&input, &count_records());
    std::fs::write(&features, "p0\tgA\np1\tgB\n").unwrap();

    // feature column 0: per-probe names
    let status = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("-f")
        .arg(&features)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(read_to_string(&output), "0\tp0\t2\n0\tp1\t1\n1\tp1\t1\n");

    // aggregating on column 1 with distinct names is a no-op relabeling
    let status = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("-f")
        .arg(&features)
        .args(["-C", "1"])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(read_to_string(&output), "0\tgA\t2\n0\tgB\t1\n1\tgB\t1\n");

    // aggregating probes sharing a name merges their counts
    std::fs::write(&features, "p0\tgA\np1\tgA\n").unwrap();
    let status = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("-f")
        .arg(&features)
        .args(["-C", "1"])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(read_to_string(&output), "0\tgA\t3\n1\tgA\t1\n");
}

#[test]
fn test_count_rejects_index_outside_features() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let features = dir.path().join("features.tsv");
    write_records(&input, &count_records());
    // only one feature, but records carry index 1
    std::fs::write(&features, "p0\n").unwrap();

    let out = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("-f")
        .arg(&features)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("exceeds the maximum expected index"));
}

fn read_gzip_to_string(path: &Path) -> String {
    use std::io::Read as _;
    let (mut reader, _format) = niffler::from_path(path).unwrap();
    let mut contents = String::new();
    reader.read_to_string(&mut contents).unwrap();
    contents
}

#[test]
fn test_count_mtx() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let features = dir.path().join("features.tsv");
    let outdir = dir.path().join("mtx");
    write_records(&input, &count_records());
    std::fs::write(&features, "p0\np1\n").unwrap();

    let status = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("-o")
        .arg(&outdir)
        .arg("-f")
        .arg(&features)
        .arg("--mtx")
        .status()
        .unwrap();
    assert!(status.success());

    assert_eq!(
        read_gzip_to_string(&outdir.join("features.tsv.gz")),
        "p0\np1\n"
    );
    assert_eq!(
        read_gzip_to_string(&outdir.join("barcodes.tsv.gz")),
        "AAAAAAAAAAAAAAAA\nCAAAAAAAAAAAAAAA\n"
    );

    let mtx = read_gzip_to_string(&outdir.join("matrix.mtx.gz"));
    let lines: Vec<&str> = mtx.lines().collect();
    assert!(lines[0].starts_with("%%MatrixMarket"));
    assert_eq!(lines[2], "2 2 3"); // features, barcodes, non-zero entries
    assert_eq!(&lines[3..], ["1 1 2", "2 1 1", "2 2 1"]);
}

#[test]
fn test_count_mtx_requires_output_and_features() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    write_records(&input, &count_records());

    let out = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("--mtx")
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn test_count_seq_output() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let output = dir.path().join("counts.tsv");
    let seq_output = dir.path().join("seq_counts.tsv");
    // three UMIs on (barcode 0, index 0): two carry ACGT, one carries AGGT
    write_records(
        &input,
        &[
            ExtRecord::from_sequence(0, 0, 0, b"ACGT").unwrap(),
            ExtRecord::from_sequence(0, 0, 0, b"ACGT").unwrap(),
            ExtRecord::from_sequence(0, 1, 0, b"AGGT").unwrap(),
            ExtRecord::from_sequence(0, 2, 0, b"ACGT").unwrap(),
        ],
    );

    let status = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("-q")
        .arg(&seq_output)
        .status()
        .unwrap();
    assert!(status.success());

    assert_eq!(read_to_string(&output), "0\t0\t3\n");
    assert_eq!(
        read_to_string(&seq_output),
        "0\t0\tACGT\t2\n0\t0\tAGGT\t1\n"
    );
}

#[test]
fn test_count_seq_output_rejects_classic_records() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    let seq_output = dir.path().join("seq_counts.tsv");
    write_records(&input, &count_records());

    let out = ibu_bin()
        .arg("count")
        .arg(&input)
        .arg("-q")
        .arg(&seq_output)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("extended"));
}

#[test]
fn test_count_rejects_unsorted() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    write_records(&input, &[Record::new(5, 0, 0), Record::new(1, 0, 0)]);

    let out = ibu_bin().arg("count").arg(&input).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unsorted"));
}

#[test]
fn test_count_rejects_empty_input() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.ibu");
    write_records::<Record>(&input, &[]);

    let out = ibu_bin().arg("count").arg(&input).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("No records found"));
}
