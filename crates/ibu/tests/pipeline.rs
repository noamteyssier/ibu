//! End-to-end test of the composable iterator pipeline:
//! sort -> dedup -> UMI correction -> consensus -> counting, without any
//! intermediate files, checked against the reader-to-writer routines.

use std::io::Cursor;

use ibu::consensus::{consensus_parallel, consensus_parallel_iter};
use ibu::count::{BarcodeUmiCounter, BarcodeUmiCounts};
use ibu::umi::{correct_umis_parallel, correct_umis_parallel_iter};
use ibu::{external_sort, DedupExt, ExtRecord, ExtRecordCount, Header, Reader, Writer};

const UMI_LEN: usize = 12;

/// An unsorted extended record stream with duplicate observations, UMI errors
/// (Hamming-distance-1 neighbors), and sequence variants.
fn simulated_records() -> Vec<ExtRecord> {
    let mut records = Vec::new();
    for barcode in 0..20u64 {
        for umi_base in [0b0000u64, 0b1100, 0b110000] {
            for index in 0..3u64 {
                let dominant = ExtRecord::from_sequence(barcode, umi_base, index, b"ACGTACGT")
                    .expect("valid sequence");
                records.extend(std::iter::repeat_n(dominant, 4));
                // a sequence variant of the dominant molecule
                records.push(
                    ExtRecord::from_sequence(barcode, umi_base, index, b"ACGTACGA")
                        .expect("valid sequence"),
                );
                // a UMI error one base off the dominant UMI
                records.push(
                    ExtRecord::from_sequence(barcode, umi_base ^ 0b01, index, b"ACGTACGT")
                        .expect("valid sequence"),
                );
            }
        }
    }
    // interleave the stream so it is thoroughly unsorted
    let mid = records.len() / 2;
    let (left, right) = records.split_at(mid);
    left.iter()
        .zip(right.iter())
        .flat_map(|(a, b)| [*b, *a])
        .collect()
}

/// The same pipeline through the reader-to-writer routines, one buffer per stage.
fn reference_counts(records: &[ExtRecord], threads: usize) -> BarcodeUmiCounts {
    let mut header = Header::new(16, UMI_LEN as u32);
    header.set_sorted();

    let mut sorted = records.to_vec();
    sorted.sort_unstable();

    let mut writer: Writer<_, ExtRecordCount> = Writer::new(Vec::new(), header).unwrap();
    writer
        .write_iter(
            ibu::dedup_sorted(sorted.into_iter())
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .into_iter(),
        )
        .unwrap();
    writer.finish().unwrap();

    let reader = Reader::new(Cursor::new(writer.into_inner())).unwrap();
    let mut corrected: Writer<_, ExtRecordCount> = Writer::new(Vec::new(), header).unwrap();
    correct_umis_parallel(reader, &mut corrected, threads).unwrap();

    let reader = Reader::new(Cursor::new(corrected.into_inner())).unwrap();
    let mut consolidated: Writer<_, ExtRecordCount> = Writer::new(Vec::new(), header).unwrap();
    consensus_parallel(reader, &mut consolidated, threads).unwrap();

    let reader = Reader::new(Cursor::new(consolidated.into_inner())).unwrap();
    BarcodeUmiCounter::new()
        .consume(reader.iter_ext_record_counts().unwrap())
        .unwrap()
}

#[test]
fn test_iterator_pipeline_matches_reader_writer_pipeline() {
    let records = simulated_records();
    let expected = reference_counts(&records, 1);

    for threads in [1, 4] {
        let sorted = external_sort(records.clone().into_iter().map(Ok), 256, threads)
            .unwrap()
            .dedup();
        let corrected = correct_umis_parallel_iter(sorted, UMI_LEN, threads);
        let consensus = consensus_parallel_iter(corrected, threads);
        let counts = BarcodeUmiCounter::new().consume(consensus).unwrap();

        assert_eq!(counts.stats(), expected.stats());
        assert_eq!(
            counts.iter_counts().collect::<Vec<_>>(),
            expected.iter_counts().collect::<Vec<_>>()
        );
        assert_eq!(
            counts.iter_seq_counts().collect::<Vec<_>>(),
            expected.iter_seq_counts().collect::<Vec<_>>()
        );
    }
}

#[test]
fn test_iterator_pipeline_stats_match_reader_writer_stats() {
    let records = simulated_records();

    // reader/writer route stats
    let mut header = Header::new(16, UMI_LEN as u32);
    header.set_sorted();
    let mut sorted = records.clone();
    sorted.sort_unstable();
    let mut writer: Writer<_, ExtRecord> = Writer::new(Vec::new(), header).unwrap();
    writer.write_batch(&sorted).unwrap();
    writer.finish().unwrap();

    let reader = Reader::new(Cursor::new(writer.into_inner())).unwrap();
    let mut corrected: Writer<_, ExtRecord> = Writer::new(Vec::new(), header).unwrap();
    let expected_umi = correct_umis_parallel(reader, &mut corrected, 1).unwrap();

    let reader = Reader::new(Cursor::new(corrected.into_inner())).unwrap();
    let mut consolidated: Writer<_, ExtRecord> = Writer::new(Vec::new(), header).unwrap();
    let expected_consensus = consensus_parallel(reader, &mut consolidated, 1).unwrap();

    // iterator route stats
    let mut corrected = correct_umis_parallel_iter(sorted.into_iter().map(Ok), UMI_LEN, 4);
    let corrected_records: Vec<ExtRecord> = corrected.by_ref().collect::<Result<_, _>>().unwrap();
    assert_eq!(corrected.stats(), expected_umi);

    let mut consensus = consensus_parallel_iter(corrected_records.into_iter().map(Ok), 4);
    consensus.by_ref().for_each(|r| {
        r.unwrap();
    });
    assert_eq!(consensus.stats(), expected_consensus);
}
