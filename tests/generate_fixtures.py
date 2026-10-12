#!/usr/bin/env python3
"""Regenerate deterministic TTA fixtures with FFmpeg (not needed to run tests).

The inputs are generated here, not taken from copyrighted recordings. Verify
FFmpeg's decoded PCM against the original before saving each reference file.
"""
import hashlib
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent / "fixtures"
CASES = [
    ("mono8", 8, 1, 44100, 257, False),
    ("stereo8", 8, 2, 16000, 1027, False),
    ("stereo16", 16, 2, 8000, 2 * (8000 * 256 // 245) + 37, False),
    ("surround24", 24, 6, 48000, 2048, False),
    ("silence16", 16, 1, 8000, 2 * (8000 * 256 // 245), True),
]


def samples(bits, channels, frames, silence):
    state = 0x12345678
    limit = 1 << (bits - 1)
    for frame in range(frames):
        for channel in range(channels):
            state = (1664525 * state + 1013904223) & 0xFFFFFFFF
            if silence or frame % 97 < 8:
                yield 0
            elif frame % 97 < 16:
                yield [-limit, limit - 1, -1, 1][(frame + channel) % 4]
            elif frame % 97 < 48:
                yield ((frame * 113 + channel * 317) % (2 * limit)) - limit
            else:
                yield (state >> (32 - bits)) - limit


def main():
    ROOT.mkdir(exist_ok=True)
    hashes = []
    with tempfile.TemporaryDirectory(prefix="tta-fixtures-") as temp:
        for name, bits, channels, rate, frames, silence in CASES:
            pcm = list(samples(bits, channels, frames, silence))
            raw = b"".join(
                bytes([value + 128]) if bits == 8 else value.to_bytes(bits // 8, "little", signed=True)
                for value in pcm
            )
            expected = b"".join(struct.pack("<i", value << (32 - bits)) for value in pcm)
            path = ROOT / f"{name}.tta"
            source = Path(temp) / "input.raw"
            source.write_bytes(raw)
            subprocess.run([
                "ffmpeg", "-v", "error", "-y", "-f", {8: "u8", 16: "s16le", 24: "s24le"}[bits],
                "-ar", str(rate), "-ac", str(channels), "-i", str(source),
                "-map_metadata", "-1", "-c:a", "tta", str(path),
            ], check=True)
            actual = subprocess.check_output([
                "ffmpeg", "-v", "error", "-i", str(path), "-c:a", "pcm_s32le", "-f", "s32le", "-",
            ])
            assert actual == expected, f"FFmpeg round trip failed for {name}"
            assert int.from_bytes(path.read_bytes()[8:10], "little") == bits
            (ROOT / f"{name}.s32le").write_bytes(expected)
            for extension in ("tta", "s32le"):
                file = ROOT / f"{name}.{extension}"
                hashes.append(f"{hashlib.sha256(file.read_bytes()).hexdigest()}  {file.name}")
            print(f"{name}: {bits}-bit, {channels} channels, {rate} Hz, {frames} sample frames")
    (ROOT / "SHA256SUMS").write_text("\n".join(hashes) + "\n")


if __name__ == "__main__":
    main()
