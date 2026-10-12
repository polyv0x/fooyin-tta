// SPDX-License-Identifier: GPL-3.0-or-later
// TTA algorithm adapted from the BSD-licensed True Audio reference decoder.
// Copyright (c) 2004 True Audio Software. See LICENSES/tta-reference.txt.

use crate::{Error, Result};

const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut i = 0;
    while i < table.len() {
        let mut value = i as u32;
        let mut bit = 0;
        while bit < 8 {
            value = (value >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(value & 1));
            bit += 1;
        }
        table[i] = value;
        i += 1;
    }
    table
}

const CRC_TABLE: [u32; 256] = crc_table();

pub(crate) fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(u32::MAX, |crc, byte| {
        (crc >> 8) ^ CRC_TABLE[((crc ^ u32::from(*byte)) & 255) as usize]
    })
}

struct Bits<'a> {
    bytes: &'a [u8],
    offset: usize,
    cache: u64,
    count: u32,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            cache: 0,
            count: 0,
        }
    }

    fn read(&mut self, count: u32) -> Result<u32> {
        if count > 32 {
            return Err(Error::Invalid("invalid Rice parameter"));
        }
        while self.count < count {
            let byte = *self
                .bytes
                .get(self.offset)
                .ok_or(Error::Invalid("truncated frame bits"))?;
            self.cache |= u64::from(byte) << self.count;
            self.count += 8;
            self.offset += 1;
        }
        let value = self.cache & ((1u64 << count) - 1);
        self.cache >>= count;
        self.count -= count;
        Ok(value as u32)
    }

    fn unary(&mut self) -> Result<u32> {
        let mut value = 0u32;
        loop {
            if self.count == 0 {
                self.cache = u64::from(
                    *self
                        .bytes
                        .get(self.offset)
                        .ok_or(Error::Invalid("unterminated Rice code"))?,
                );
                self.offset += 1;
                self.count = 8;
            }
            let ones = self.cache.trailing_ones().min(self.count);
            value = value
                .checked_add(ones)
                .ok_or(Error::Invalid("Rice code overflow"))?;
            self.cache >>= ones;
            self.count -= ones;
            if self.count != 0 {
                self.cache >>= 1;
                self.count -= 1;
                return Ok(value);
            }
        }
    }

    fn finish(&self) -> Result<()> {
        // A frame ends with at most seven zero padding bits before its CRC.
        if self.offset != self.bytes.len() || self.count > 7 || self.cache != 0 {
            return Err(Error::Invalid("unexpected data after frame samples"));
        }
        Ok(())
    }
}

struct Rice {
    k: [u32; 2],
    sum: [u32; 2],
}

impl Rice {
    fn new() -> Self {
        Self {
            k: [10; 2],
            sum: [1 << 14; 2],
        }
    }

    fn update(&mut self, level: usize, value: u32) {
        self.sum[level] = self.sum[level]
            .wrapping_add(value)
            .wrapping_sub(self.sum[level] >> 4);
        let k = &mut self.k[level];
        // The reference's shift table saturates at 2^31.
        if *k > 0 && self.sum[level] < (1 << (*k + 4).min(31)) {
            *k -= 1;
        } else if *k < 31 && self.sum[level] > (1 << (*k + 5).min(31)) {
            *k += 1;
        }
    }

    fn decode(&mut self, bits: &mut Bits<'_>) -> Result<i32> {
        let unary = bits.unary()?;
        let level = usize::from(unary != 0);
        let quotient = unary.saturating_sub(1);
        let k = self.k[level];
        let wide = (u64::from(quotient) << k) | u64::from(bits.read(k)?);
        let mut value = u32::try_from(wide).map_err(|_| Error::Invalid("Rice value overflow"))?;
        if level == 1 {
            self.update(1, value);
            value = value
                .checked_add(1 << self.k[0])
                .ok_or(Error::Invalid("Rice value overflow"))?;
        }
        self.update(0, value);
        Ok(if value & 1 != 0 {
            ((value >> 1) + 1) as i32
        } else {
            -((value >> 1) as i32)
        })
    }
}

struct Filter {
    coefficients: [i32; 8],
    differences: [i32; 9],
    delay: [i32; 9],
    error: i32,
    shift: u32,
}

impl Filter {
    fn new(bits: u16) -> Self {
        Self {
            coefficients: [0; 8],
            differences: [0; 9],
            delay: [0; 9],
            error: 0,
            shift: if bits == 16 { 9 } else { 10 },
        }
    }

    fn decode(&mut self, residual: i32) -> i32 {
        let mut sum = 1i32 << (self.shift - 1);
        for i in 0..8 {
            // TTA relies on two's-complement 32-bit wrapping arithmetic.
            self.coefficients[i] = match self.error.cmp(&0) {
                std::cmp::Ordering::Less => self.coefficients[i].wrapping_sub(self.differences[i]),
                std::cmp::Ordering::Greater => {
                    self.coefficients[i].wrapping_add(self.differences[i])
                }
                std::cmp::Ordering::Equal => self.coefficients[i],
            };
            sum = sum.wrapping_add(self.delay[i].wrapping_mul(self.coefficients[i]));
        }
        self.error = residual;
        let value = residual.wrapping_add(sum >> self.shift);
        self.differences[8] = ((self.delay[7] >> 30) | 1) << 2;
        self.differences[7] = ((self.delay[6] >> 30) | 1) << 1;
        self.differences[6] = ((self.delay[5] >> 30) | 1) << 1;
        self.differences[5] = (self.delay[4] >> 30) | 1;
        self.delay[8] = value;
        self.delay[7] = self.delay[8].wrapping_sub(self.delay[7]);
        self.delay[6] = self.delay[7].wrapping_sub(self.delay[6]);
        self.delay[5] = self.delay[6].wrapping_sub(self.delay[5]);
        self.delay.copy_within(1..9, 0);
        self.differences.copy_within(1..9, 0);
        value
    }
}

struct Channel {
    rice: Rice,
    filter: Filter,
    previous: i32,
}

pub(crate) fn decode_frame(
    data: &[u8],
    channels: u16,
    depth: u16,
    output: &mut [i32],
) -> Result<()> {
    let mut bits = Bits::new(data);
    let mut state: Vec<_> = (0..channels)
        .map(|_| Channel {
            rice: Rice::new(),
            filter: Filter::new(depth),
            previous: 0,
        })
        .collect();
    let predictor_shift = if depth == 8 { 4 } else { 5 };
    let min_sample = -(1i32 << (depth - 1));
    let max_sample = (1i32 << (depth - 1)) - 1;
    for samples in output.chunks_exact_mut(usize::from(channels)) {
        for (sample, channel) in samples.iter_mut().zip(&mut state) {
            let residual = channel.rice.decode(&mut bits)?;
            let filtered = channel.filter.decode(residual);
            let predicted = ((i64::from(channel.previous) * ((1 << predictor_shift) - 1))
                >> predictor_shift) as i32;
            *sample = filtered.wrapping_add(predicted);
            channel.previous = *sample;
        }
        if samples.len() > 1 {
            let last = samples.len() - 1;
            // Division must truncate toward zero, including negative odd values.
            samples[last] = samples[last].wrapping_add(samples[last - 1] / 2);
            for i in (0..last).rev() {
                samples[i] = samples[i + 1].wrapping_sub(samples[i]);
            }
        }
        if samples
            .iter()
            .any(|sample| !(min_sample..=max_sample).contains(sample))
        {
            return Err(Error::Invalid("decoded sample exceeds declared bit depth"));
        }
    }
    bits.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_crc_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn truncated_rice_codes_fail() {
        assert!(Bits::new(&[255; 32]).unary().is_err());
        assert!(Bits::new(&[]).read(1).is_err());
        assert!(Bits::new(&[0; 8]).read(33).is_err());
    }
}
