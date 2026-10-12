// SPDX-License-Identifier: GPL-3.0-or-later
use fooyin_tta::{Decoder, Error};
use std::io::Cursor;

struct Fixture {
    data: &'static [u8],
    pcm: &'static [u8],
    bits: u16,
    channels: u16,
    rate: u32,
}

fn fixtures() -> [Fixture; 5] {
    [
        Fixture {
            data: include_bytes!("fixtures/mono8.tta"),
            pcm: include_bytes!("fixtures/mono8.s32le"),
            bits: 8,
            channels: 1,
            rate: 44100,
        },
        Fixture {
            data: include_bytes!("fixtures/stereo8.tta"),
            pcm: include_bytes!("fixtures/stereo8.s32le"),
            bits: 8,
            channels: 2,
            rate: 16000,
        },
        Fixture {
            data: include_bytes!("fixtures/stereo16.tta"),
            pcm: include_bytes!("fixtures/stereo16.s32le"),
            bits: 16,
            channels: 2,
            rate: 8000,
        },
        Fixture {
            data: include_bytes!("fixtures/surround24.tta"),
            pcm: include_bytes!("fixtures/surround24.s32le"),
            bits: 24,
            channels: 6,
            rate: 48000,
        },
        Fixture {
            data: include_bytes!("fixtures/silence16.tta"),
            pcm: include_bytes!("fixtures/silence16.s32le"),
            bits: 16,
            channels: 1,
            rate: 8000,
        },
    ]
}

fn expected(fixture: &Fixture) -> Vec<i32> {
    fixture
        .pcm
        .chunks_exact(4)
        .map(|b| i32::from_le_bytes(b.try_into().unwrap()) >> (32 - fixture.bits))
        .collect()
}

fn decode(data: &[u8], chunk: usize) -> fooyin_tta::Result<Vec<i32>> {
    let mut decoder = Decoder::new(Cursor::new(data))?;
    let mut buffer = vec![0; chunk];
    let mut pcm = Vec::new();
    loop {
        let count = decoder.read_samples(&mut buffer)?;
        if count == 0 {
            break;
        }
        assert_eq!(count % usize::from(decoder.info().channels), 0);
        pcm.extend_from_slice(&buffer[..count]);
    }
    assert_eq!(decoder.position(), decoder.info().total_samples);
    assert_eq!(decoder.read_samples(&mut buffer)?, 0);
    Ok(pcm)
}

#[test]
fn bit_exact_against_independent_ffmpeg_fixtures() {
    for fixture in fixtures() {
        let info = Decoder::new(Cursor::new(fixture.data)).unwrap().info();
        assert_eq!(info.bits_per_sample, fixture.bits);
        assert_eq!(info.channels, fixture.channels);
        assert_eq!(info.sample_rate, fixture.rate);
        assert_eq!(
            info.total_samples as usize,
            fixture.pcm.len() / 4 / usize::from(fixture.channels)
        );
        for chunk in [usize::from(fixture.channels), 101, 65536] {
            assert_eq!(
                decode(fixture.data, chunk).unwrap(),
                expected(&fixture),
                "{}-bit/{} channels/chunk {chunk}",
                fixture.bits,
                fixture.channels
            );
        }
    }
}

#[test]
fn seeks_are_exact_across_frames_and_eof() {
    for fixture in fixtures() {
        let mut decoder = Decoder::new(Cursor::new(fixture.data)).unwrap();
        let expected = expected(&fixture);
        let total = decoder.info().total_samples;
        let frame = fixture.rate * 256 / 245;
        let mut output = vec![0; usize::from(fixture.channels) * 37];
        for position in [
            0,
            1,
            total - 1,
            frame.saturating_sub(1),
            frame,
            frame + 1,
            total,
            7,
            0,
            u32::MAX,
        ] {
            decoder.seek(u64::from(position));
            let position = position.min(total) as usize;
            assert_eq!(decoder.position() as usize, position);
            let count = decoder.read_samples(&mut output).unwrap();
            let offset = position * usize::from(fixture.channels);
            assert_eq!(&output[..count], &expected[offset..offset + count]);
            assert_eq!(count == 0, position == total as usize);
        }
        decoder.seek_ms(13);
        assert_eq!(decoder.position(), (fixture.rate * 13 / 1000).min(total));
        decoder.seek_ms(u64::MAX);
        assert_eq!(decoder.position(), total);
    }
}

// Independent bitwise CRC implementation used to construct malformed fixtures
// with valid checksums, so parser tests reach beyond the initial CRC check.
fn crc(data: &[u8]) -> u32 {
    let mut result = !0u32;
    for byte in data {
        result ^= u32::from(*byte);
        for _ in 0..8 {
            result = if result & 1 != 0 {
                (result >> 1) ^ 0xedb8_8320
            } else {
                result >> 1
            };
        }
    }
    !result
}

fn update_crc(data: &mut [u8]) {
    let end = data.len() - 4;
    let checksum = crc(&data[..end]);
    data[end..].copy_from_slice(&checksum.to_le_bytes());
}

#[test]
fn skips_leading_id3_and_ignores_trailing_tags() {
    let fixture = &fixtures()[0];
    for version in [2, 3, 4] {
        for footer in [false, true] {
            if footer && version != 4 {
                continue;
            }
            let mut data = vec![
                b'I',
                b'D',
                b'3',
                version,
                0,
                if footer { 0x10 } else { 0 },
                0,
                0,
                0,
                17,
            ];
            data.extend_from_slice(&[0; 17]);
            if footer {
                data.extend_from_slice(&[b'3', b'D', b'I', 4, 0, 0x10, 0, 0, 0, 17]);
            }
            data.extend_from_slice(fixture.data);
            data.extend_from_slice(b"APETAGEXignored trailing metadata");
            assert_eq!(decode(&data, 43).unwrap(), expected(fixture));
        }
    }
}

#[test]
fn checks_header_table_and_final_frame_checksums() {
    let fixture = &fixtures()[0];
    for byte in [10, 22, 32] {
        let mut data = fixture.data.to_vec();
        data[byte] ^= 1;
        assert!(decode(&data, 1024).is_err());
    }
    let mut data = fixture.data.to_vec();
    let size = u32::from_le_bytes(data[22..26].try_into().unwrap()) as usize;
    data[30 + size - 1] ^= 128;
    let mut decoder = Decoder::new(Cursor::new(data)).unwrap();
    let mut output = [12345; 1024];
    assert!(decoder.read_samples(&mut output).is_err());
    assert_eq!(output, [12345; 1024]);
    assert_eq!(decoder.position(), 0);
}

#[test]
fn truncated_inputs_and_invalid_parameters_fail_without_panics() {
    let fixture = &fixtures()[0];
    let audio_end = 30 + u32::from_le_bytes(fixture.data[22..26].try_into().unwrap()) as usize;
    for length in 0..audio_end {
        assert!(
            decode(&fixture.data[..length], 1024).is_err(),
            "length {length}"
        );
    }
    for (offset, replacement) in [
        (4, vec![2, 0]),
        (6, vec![0, 0]),
        (6, vec![33, 0]),
        (8, vec![32, 0]),
        (10, vec![0; 4]),
        (10, u32::MAX.to_le_bytes().to_vec()),
    ] {
        let mut data = fixture.data.to_vec();
        data[offset..offset + replacement.len()].copy_from_slice(&replacement);
        update_crc(&mut data[..22]);
        assert!(matches!(
            Decoder::new(Cursor::new(data)),
            Err(Error::Unsupported(_))
        ));
    }
    for size in [0u32, 4, u32::MAX] {
        let mut data = fixture.data.to_vec();
        data[22..26].copy_from_slice(&size.to_le_bytes());
        update_crc(&mut data[22..30]);
        assert!(Decoder::new(Cursor::new(data)).is_err());
    }
}

#[test]
fn empty_stream_and_small_output_buffers() {
    let mut data = fixtures()[0].data[..22].to_vec();
    data[14..18].fill(0);
    update_crc(&mut data);
    data.extend_from_slice(&[0; 4]);
    assert_eq!(decode(&data, 1).unwrap(), Vec::<i32>::new());
    let mut decoder = Decoder::new(Cursor::new(fixtures()[3].data)).unwrap();
    assert_eq!(decoder.read_samples(&mut []).unwrap(), 0);
    assert!(decoder.read_samples(&mut [0; 5]).is_err());
    assert_eq!(decoder.position(), 0);
}

#[test]
fn malformed_frame_with_valid_crc_does_not_read_past_payload() {
    let mut data = fixtures()[0].data.to_vec();
    let size = u32::from_le_bytes(data[22..26].try_into().unwrap()) as usize;
    data[30..30 + size - 4].fill(255);
    update_crc(&mut data[30..30 + size]);
    assert!(decode(&data, 1024).is_err());
}

#[test]
fn separate_decoders_can_run_concurrently() {
    let threads: Vec<_> = fixtures()
        .into_iter()
        .map(|fixture| {
            std::thread::spawn(move || {
                assert_eq!(decode(fixture.data, 103).unwrap(), expected(&fixture));
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
