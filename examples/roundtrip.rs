use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::time::Instant;

use ibu::{load_to_vec, Header, Reader, Record, Writer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configuration
    let num_records = 500_000_000; // 100M records = ~2.4GB
    let filename = "test_roundtrip.ibu";

    println!("IBU Roundtrip Test");
    println!("==================");
    println!("Records: {}", num_records);
    println!(
        "File size: ~{:.2} GB\n",
        (num_records * 24) as f64 / 1_000_000_000.0
    );

    // Create header
    let mut header = Header::new(16, 12);
    header.set_sorted();

    // ========== WRITE TEST ==========
    println!("Writing...");
    let write_start = Instant::now();

    {
        let file = File::create(filename)?;
        let buf_writer = BufWriter::with_capacity(4 * 1024 * 1024, file);
        let mut writer = Writer::new(buf_writer, header)?;

        for i in 0..num_records {
            let record = Record::new(
                i % 1_000_000,        // barcode
                (i * 31) % 1_000_000, // umi (some pattern)
                i,                    // index
            );

            // Progress indicator
            if i % 10_000_000 == 0 && i > 0 {
                let elapsed = write_start.elapsed().as_secs_f64();
                let rate = i as f64 / elapsed / 1_000_000.0;
                print!("\r  Written: {} M records ({:.2} M/s)", i / 1_000_000, rate);
                std::io::Write::flush(&mut std::io::stdout())?;
            }
            writer.write_record(&record)?;
        }
        writer.finish()?;
    }

    let write_duration = write_start.elapsed();
    let write_rate = num_records as f64 / write_duration.as_secs_f64() / 1_000_000.0;
    let write_bandwidth =
        (num_records * 24) as f64 / write_duration.as_secs_f64() / 1_000_000_000.0;

    println!("\r  ✓ Write complete");
    println!("  Duration: {:.2}s", write_duration.as_secs_f64());
    println!("  Rate: {:.2} M records/s", write_rate);
    println!("  Bandwidth: {:.2} GB/s\n", write_bandwidth);

    // ========== READ TEST ==========
    println!("Reading...");
    let read_start = Instant::now();

    let mut records_read = 0u64;
    let mut checksum = 0u64; // Simple checksum to verify data

    {
        let file = File::open(filename)?;
        let buf_reader = BufReader::with_capacity(4 * 1024 * 1024, file);
        let reader = Reader::new(buf_reader)?;

        // Verify header
        let read_header = reader.header();
        assert_eq!(read_header.bc_len, header.bc_len);
        assert_eq!(read_header.umi_len, header.umi_len);
        assert_eq!(read_header.sorted(), header.sorted());

        for result in reader {
            let record = result?;
            records_read += 1;

            // Calculate checksum (XOR of all fields)
            checksum ^= record.barcode;
            checksum ^= record.umi;
            checksum ^= record.index;

            // Progress indicator
            if records_read % 10_000_000 == 0 {
                let elapsed = read_start.elapsed().as_secs_f64();
                let rate = records_read as f64 / elapsed / 1_000_000.0;
                print!(
                    "\r  Read: {} M records ({:.2} M/s)",
                    records_read / 1_000_000,
                    rate
                );
                std::io::Write::flush(&mut std::io::stdout())?;
            }
        }
    }

    let read_duration = read_start.elapsed();
    let read_rate = records_read as f64 / read_duration.as_secs_f64() / 1_000_000.0;
    let read_bandwidth = (records_read * 24) as f64 / read_duration.as_secs_f64() / 1_000_000_000.0;

    println!("\r  ✓ Read complete");
    println!("  Duration: {:.2}s", read_duration.as_secs_f64());
    println!("  Rate: {:.2} M records/s", read_rate);
    println!("  Bandwidth: {:.2} GB/s\n", read_bandwidth);

    // ========== VERIFICATION ==========
    println!("Verification:");
    println!("  Records written: {}", num_records);
    println!("  Records read: {}", records_read);
    println!("  Checksum: 0x{:016X}", checksum);

    assert_eq!(records_read, num_records, "Record count mismatch!");
    println!("  ✓ Record count matches\n");

    // ========== Direct Load ===========
    let start = Instant::now();
    let records = load_to_vec(filename)?;
    let elapsed = start.elapsed();
    let load_rate = records.len() as f64 / elapsed.as_secs_f64() / 1_000_000.0;
    let load_bandwidth = (records.len() * 24) as f64 / elapsed.as_secs_f64() / 1_000_000_000.0;

    println!("Direct Load:");
    println!("  Duration: {:.2}s", elapsed.as_secs_f64());
    println!("  Rate: {:.2} M records/s", load_rate);
    println!("  Bandwidth: {:.2} GB/s\n", load_bandwidth);

    // ========== CLEANUP ==========
    std::fs::remove_file(filename)?;
    println!("✓ Test complete - file cleaned up");

    Ok(())
}
