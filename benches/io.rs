use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Write},
};

use ibu::{Header, Record};

// use ibu::{constructs::Ibu, Header, Reader, Record};

// Helper to create test data of different sizes
fn create_test_data(record_count: usize) -> (Header, Vec<Record>) {
    // Write header
    let header = Header::builder()
        .version(1)
        .bc_len(16)
        .umi_len(12)
        .sorted(false)
        .build()
        .unwrap();

    // Write records
    let mut records = Vec::new();
    for i in 0..record_count {
        let record = Record::builder()
            .index(i as u64)
            .barcode((i * 31) as u64) // Some arbitrary values
            .umi((i * 17) as u64)
            .build();
        records.push(record);
    }

    (header, records)
}

// Compare against raw bincode deserialization
fn bench_io(c: &mut Criterion) {
    let mut group = c.benchmark_group("io");
    let num_records = 10_000_000;
    let (header, records) = create_test_data(num_records);

    // benchmark raw writing
    group.bench_function("raw_write", |b| {
        b.iter(|| {
            let mut writer = File::create("test.ibu").map(BufWriter::new).unwrap();
            header.write_bytes(&mut writer).unwrap();
            for record in &records {
                record.write_bytes(&mut writer).unwrap();
            }
            writer.flush().unwrap();
            black_box(writer)
        });
    });

    // benchmark raw reading
    group.bench_function("raw_read", |b| {
        // Write the file
        let mut writer = File::create("test.ibu").map(BufWriter::new).unwrap();
        header.write_bytes(&mut writer).unwrap();
        for record in &records {
            record.write_bytes(&mut writer).unwrap();
        }
        writer.flush().unwrap();

        // Benchmark reading
        b.iter(|| {
            let mut reader = File::open("test.ibu").map(BufReader::new).unwrap();
            let header = Header::from_bytes(&mut reader).unwrap();
            let mut records = Vec::new();
            while let Some(record) = Record::from_bytes(&mut reader).unwrap() {
                records.push(record);
            }
            assert_eq!(records.len(), num_records);
            black_box((header, records))
        });
    });

    group.finish();
}

criterion_group!(benches, bench_io);
criterion_main!(benches);
