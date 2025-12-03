use std::io::BufWriter;
use std::sync::Arc;
use std::time::Instant;
use std::{fs::File, sync::Mutex};

use ibu::{Header, MmapReader, ParallelProcessor, ParallelReader, Record, Writer};

#[derive(Clone, Default)]
pub struct Processor {
    local_count: [u64; 3],
    global_count: Arc<Mutex<[u64; 3]>>,
}
impl Processor {
    pub fn final_counts(&self) -> [u64; 3] {
        let mut counts = [0; 3];
        let guard = self.global_count.lock().unwrap();
        counts.copy_from_slice(&*guard);
        counts
    }
}
impl ParallelProcessor for Processor {
    fn process_record(&mut self, record: Record) -> ibu::Result<()> {
        self.local_count[0] += record.barcode;
        self.local_count[1] += record.umi;
        self.local_count[2] += record.index;
        Ok(())
    }
    fn on_batch_complete(&mut self) -> ibu::Result<()> {
        let mut guard = self.global_count.lock().unwrap();
        guard[0] += self.local_count[0];
        guard[1] += self.local_count[1];
        guard[2] += self.local_count[2];
        self.local_count = [0; 3];
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configuration
    let num_records = 1_000_000_000; // 100M records = ~2.4GB
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

    // ============ PARALLEL TEST ============
    println!("Processing...");
    let proc = Processor::default();
    let reader = MmapReader::new(filename)?;

    let start = Instant::now();
    reader.process_parallel(proc.clone(), 0)?;
    let proc_elapsed = start.elapsed();
    println!("Number of records processed: {:?}", proc.final_counts());
    println!(
        "Processing duration: {:.5}s",
        proc_elapsed.as_millis() as f64 / 1000.0
    );
    Ok(())
}
