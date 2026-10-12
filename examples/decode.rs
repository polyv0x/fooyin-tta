// SPDX-License-Identifier: GPL-3.0-or-later
//! Decode to full-scale signed 32-bit little-endian PCM on stdout.
use fooyin_tta::Decoder;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: decode INPUT.tta [SEEK_MILLISECONDS] > output.s32le")?;
    let mut decoder = Decoder::new(BufReader::new(File::open(path)?))?;
    if let Some(milliseconds) = args.next() {
        decoder.seek_ms(
            milliseconds
                .to_str()
                .ok_or("invalid seek argument")?
                .parse()?,
        );
    }
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let info = decoder.info();
    eprintln!(
        "{} Hz, {} channels, {} bits, {} sample frames",
        info.sample_rate, info.channels, info.bits_per_sample, info.total_samples
    );
    let mut output = BufWriter::new(io::stdout().lock());
    let mut samples = vec![0; 4096 * usize::from(info.channels)];
    loop {
        let count = decoder.read_samples(&mut samples)?;
        if count == 0 {
            break;
        }
        for sample in &samples[..count] {
            output.write_all(
                &sample
                    .wrapping_shl(u32::from(32 - info.bits_per_sample))
                    .to_le_bytes(),
            )?;
        }
    }
    output.flush()?;
    Ok(())
}
