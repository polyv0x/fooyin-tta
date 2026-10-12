#!/usr/bin/env python3
"""Optional larger FFmpeg comparison; pass a compiled Rust `decode` example.

This checks combinations outside the small committed fixture set, including
32 channels, high sample rates, frame boundaries, and seeking. Requires FFmpeg.
"""
from itertools import product
from pathlib import Path
import struct
import subprocess
import sys
import tempfile

from generate_fixtures import samples


def main():
    decoder = Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/examples/decode").resolve()
    count = 0
    with tempfile.TemporaryDirectory(prefix="tta-differential-") as temp:
        source = Path(temp) / "input.raw"
        encoded = Path(temp) / "encoded.tta"
        for bits, channels, (rate, frames) in product(
            # A 257-sample tail avoids FFmpeg's TTA encoder buffer assertion
            # on very short (one/two-sample) final 8-bit frames.
            (8, 16, 24), (1, 2, 3, 8, 32), ((8000, 16975), (192000, 4097), (768000, 257))
        ):
            pcm = list(samples(bits, channels, frames, False))
            source.write_bytes(b"".join(
                bytes([value + 128]) if bits == 8 else value.to_bytes(bits // 8, "little", signed=True)
                for value in pcm
            ))
            expected = b"".join(struct.pack("<i", value << (32 - bits)) for value in pcm)
            subprocess.run([
                "ffmpeg", "-v", "error", "-y", "-f", {8: "u8", 16: "s16le", 24: "s24le"}[bits],
                "-ar", str(rate), "-ac", str(channels), "-i", str(source),
                "-c:a", "tta", str(encoded),
            ], check=True)
            decoded = subprocess.run([str(decoder), str(encoded)], capture_output=True, check=True).stdout
            assert decoded == expected, (bits, channels, rate, frames, "PCM mismatch")
            for milliseconds in (1, 1043, 1044, 1045, 1046, 2088, 100000):
                decoded = subprocess.run(
                    [str(decoder), str(encoded), str(milliseconds)], capture_output=True, check=True
                ).stdout
                offset = min(milliseconds * rate // 1000, frames) * channels * 4
                assert decoded == expected[offset:], (bits, channels, rate, milliseconds, "seek mismatch")
            count += 1
    print(f"{count} independent FFmpeg encoding cases and {count * 7} seek comparisons passed")


if __name__ == "__main__":
    main()
