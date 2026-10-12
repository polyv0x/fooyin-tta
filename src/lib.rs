// SPDX-License-Identifier: GPL-3.0-or-later
//! TTA1 PCM decoding from seekable sources, without external codec libraries.
//! Samples are interleaved, signed, and right aligned in `i32` containers.

mod codec;
pub mod ffi;

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Invalid(&'static str),
    Unsupported(&'static str),
    ResourceLimit,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "TTA input error: {error}"),
            Self::Invalid(message) => write!(f, "Invalid TTA stream: {message}"),
            Self::Unsupported(message) => write!(f, "Unsupported TTA stream: {message}"),
            Self::ResourceLimit => f.write_str("TTA stream exceeds decoder resource limits"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Stream properties. `total_samples` counts sample frames, not channel samples.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamInfo {
    pub sample_rate: u32,
    pub total_samples: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
}

impl StreamInfo {
    pub fn duration_ms(self) -> u64 {
        u64::from(self.total_samples) * 1000 / u64::from(self.sample_rate)
    }
}

const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
const MAX_FRAME_SAMPLES: usize = 32 * 1024 * 1024;
const MAX_SEEK_ENTRIES: usize = 1_000_000;

fn u16_le(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}
fn u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn check_crc(data: &[u8], message: &'static str) -> Result<()> {
    if data.len() < 4 || codec::crc32(&data[..data.len() - 4]) != u32_le(&data[data.len() - 4..]) {
        return Err(Error::Invalid(message));
    }
    Ok(())
}

fn resize<T: Clone>(buffer: &mut Vec<T>, len: usize, fill: T) -> Result<()> {
    buffer
        .try_reserve(len.saturating_sub(buffer.len()))
        .map_err(|_| Error::ResourceLimit)?;
    buffer.resize(len, fill);
    Ok(())
}

/// Independent decoder state. Retains only the seek table and one audio frame.
pub struct Decoder<R> {
    input: R,
    info: StreamInfo,
    frame_samples: u32,
    offsets: Vec<u64>,
    compressed: Vec<u8>,
    decoded: Vec<i32>,
    loaded_frame: Option<usize>,
    position: u32,
}

impl<R: Read + Seek> Decoder<R> {
    /// Open and validate the header and seek table, including their CRCs.
    /// Leading ID3v2.2/2.3/2.4 tags are skipped; trailing tags are not decoded.
    pub fn new(mut input: R) -> Result<Self> {
        let file_size = input.seek(SeekFrom::End(0))?;
        input.seek(SeekFrom::Start(0))?;
        let mut prefix = [0u8; 10];
        input.read_exact(&mut prefix)?;
        let header_offset = if &prefix[..3] == b"ID3" {
            if !(2..=4).contains(&prefix[3])
                || prefix[4] == 255
                || prefix[6..10].iter().any(|b| b & 128 != 0)
            {
                return Err(Error::Invalid("invalid ID3v2 header"));
            }
            let size = prefix[6..10]
                .iter()
                .fold(0u64, |value, byte| (value << 7) | u64::from(*byte));
            10 + size
                + if prefix[3] == 4 && prefix[5] & 0x10 != 0 {
                    10
                } else {
                    0
                }
        } else {
            0
        };
        if header_offset + 22 > file_size {
            return Err(Error::Invalid("truncated TTA header"));
        }
        input.seek(SeekFrom::Start(header_offset))?;
        let mut header = [0u8; 22];
        input.read_exact(&mut header)?;
        if &header[..4] != b"TTA1" {
            return Err(Error::Invalid("missing TTA1 signature"));
        }
        check_crc(&header, "header checksum mismatch")?;
        match u16_le(&header[4..6]) {
            1 => {}
            2 => return Err(Error::Unsupported("encrypted files")),
            _ => return Err(Error::Unsupported("non-PCM audio")),
        }
        let info = StreamInfo {
            channels: u16_le(&header[6..8]),
            bits_per_sample: u16_le(&header[8..10]),
            sample_rate: u32_le(&header[10..14]),
            total_samples: u32_le(&header[14..18]),
        };
        if !(1..=32).contains(&info.channels) {
            return Err(Error::Unsupported("channel count must be 1..32"));
        }
        if ![8, 16, 24].contains(&info.bits_per_sample) {
            return Err(Error::Unsupported(
                "only 8-, 16-, and 24-bit PCM is supported",
            ));
        }
        if !(1..=768_000).contains(&info.sample_rate) {
            return Err(Error::Unsupported("sample rate must be 1..768000 Hz"));
        }
        let frame_samples = (u64::from(info.sample_rate) * 256 / 245) as u32;
        let frame_count = info.total_samples.div_ceil(frame_samples) as usize;
        if frame_count > MAX_SEEK_ENTRIES
            || frame_samples as usize * usize::from(info.channels) > MAX_FRAME_SAMPLES
        {
            return Err(Error::ResourceLimit);
        }
        let table_size = (frame_count + 1) * 4;
        let data_offset = header_offset + 22 + table_size as u64;
        if data_offset > file_size {
            return Err(Error::Invalid("truncated seek table"));
        }
        let mut table = Vec::new();
        resize(&mut table, table_size, 0)?;
        input.read_exact(&mut table)?;
        check_crc(&table, "seek table checksum mismatch")?;
        let mut offsets = Vec::new();
        offsets
            .try_reserve_exact(frame_count + 1)
            .map_err(|_| Error::ResourceLimit)?;
        offsets.push(data_offset);
        let mut offset = data_offset;
        for entry in table[..frame_count * 4].chunks_exact(4) {
            let size = u32_le(entry) as usize;
            if size < 5 {
                return Err(Error::Invalid("frame size is too small"));
            }
            if size > MAX_FRAME_BYTES {
                return Err(Error::ResourceLimit);
            }
            offset = offset
                .checked_add(size as u64)
                .ok_or(Error::Invalid("frame offset overflow"))?;
            if offset > file_size {
                return Err(Error::Invalid("frame extends past end of input"));
            }
            offsets.push(offset);
        }
        Ok(Self {
            input,
            info,
            frame_samples,
            offsets,
            compressed: Vec::new(),
            decoded: Vec::new(),
            loaded_frame: None,
            position: 0,
        })
    }

    pub fn info(&self) -> StreamInfo {
        self.info
    }

    /// Current position in sample frames, including any samples discarded by seeking.
    pub fn position(&self) -> u32 {
        self.position
    }

    /// Seek exactly to a sample frame, clamping to EOF. Decoding is deferred to read.
    pub fn seek(&mut self, sample: u64) {
        self.position = sample.min(u64::from(self.info.total_samples)) as u32;
    }

    pub fn seek_ms(&mut self, milliseconds: u64) {
        let sample = u128::from(milliseconds) * u128::from(self.info.sample_rate) / 1000;
        self.seek(sample.min(u128::from(self.info.total_samples)) as u64);
    }

    /// Read whole interleaved sample frames, at most one compressed TTA frame.
    /// Returns the number of channel samples written; zero means EOF or empty output.
    /// A nonempty output buffer must fit at least one complete sample frame.
    /// No samples from a corrupt frame are returned.
    pub fn read_samples(&mut self, output: &mut [i32]) -> Result<usize> {
        if output.is_empty() || self.position == self.info.total_samples {
            return Ok(0);
        }
        let channels = usize::from(self.info.channels);
        if output.len() < channels {
            return Err(Error::Invalid("output buffer cannot hold a sample frame"));
        }
        let frame = (self.position / self.frame_samples) as usize;
        if self.loaded_frame != Some(frame) {
            self.loaded_frame = None;
            let size = (self.offsets[frame + 1] - self.offsets[frame]) as usize;
            resize(&mut self.compressed, size, 0)?;
            self.input.seek(SeekFrom::Start(self.offsets[frame]))?;
            self.input.read_exact(&mut self.compressed)?;
            check_crc(&self.compressed, "audio frame checksum mismatch")?;
            let frame_start = frame as u32 * self.frame_samples;
            let samples = self
                .frame_samples
                .min(self.info.total_samples - frame_start) as usize
                * channels;
            resize(&mut self.decoded, samples, 0)?;
            codec::decode_frame(
                &self.compressed[..size - 4],
                self.info.channels,
                self.info.bits_per_sample,
                &mut self.decoded,
            )?;
            self.loaded_frame = Some(frame);
        }
        let offset = (self.position % self.frame_samples) as usize * channels;
        let count = (output.len() / channels * channels).min(self.decoded.len() - offset);
        output[..count].copy_from_slice(&self.decoded[offset..offset + count]);
        self.position += (count / channels) as u32;
        Ok(count)
    }
}
