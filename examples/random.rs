use std::{fs::File, time::Instant};

use anyhow::Result;
use clap::Parser;
use ibu::{Header, Record, Writer, HEADER_SIZE, RECORD_SIZE};
use rand::{rngs::SmallRng, Rng, SeedableRng};

#[derive(Parser)]
struct Args {
    /// Output file path
    #[clap(required = true)]
    path: String,
    /// Number of records to generate (in millions)
    #[clap(long, default_value_t = 1.0)]
    records: f64,
    #[clap(long, default_value_t = 1_000)]
    barcodes: u64,
    #[clap(long, default_value_t = 10_000)]
    max_index: u64,
    #[clap(long, default_value_t = 16)]
    bc_len: u32,
    #[clap(long, default_value_t = 12)]
    umi_len: u32,
    #[clap(long)]
    seed: Option<u64>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let header = Header::new(args.bc_len, args.umi_len);
    header.validate()?;

    let handle = File::create(&args.path)?;
    let mut writer = Writer::new(handle, header)?;
    let mut rng = if let Some(seed) = args.seed {
        SmallRng::seed_from_u64(seed)
    } else {
        SmallRng::from_os_rng()
    };

    let start = Instant::now();
    let num_records = (args.records * 1_000_000.0) as usize;
    for _ in 0..num_records {
        let barcode = rng.random_range(0..args.barcodes);
        let index = rng.random_range(0..args.max_index);
        let umi = rng.random();
        let record = Record::new(barcode, umi, index);
        writer.write_record(&record)?;
    }
    writer.finish()?;
    let elapsed = start.elapsed();

    let total_bytes = HEADER_SIZE + (num_records * RECORD_SIZE);

    eprintln!("Finished generating {} records", num_records);
    eprintln!("Elapsed time: {:?}", elapsed);
    eprintln!(
        "Bandwidth: {:.2} Gb/s",
        total_bytes as f64 / elapsed.as_millis() as f64 * 1000.0 / 1_000_000_000.0
    );

    Ok(())
}
