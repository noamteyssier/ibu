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
