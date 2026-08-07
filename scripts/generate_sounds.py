#!/usr/bin/env python3
"""Synthesize Herdr's built-in notification sounds.

The generated mp3 files in `assets/sounds/` are committed and embedded in the
binary, so this script only needs to run when a sound is added or reworked:

    python3 scripts/generate_sounds.py

`chime.mp3` and `request.mp3` are the original hand-made sounds and are left
alone; everything listed in SOUNDS below is regenerated from scratch. Requires
ffmpeg with libmp3lame on PATH.
"""

from __future__ import annotations

import argparse
import math
import struct
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

SAMPLE_RATE = 44100
REPO_ROOT = Path(__file__).resolve().parent.parent
SOUNDS_DIR = REPO_ROOT / "assets" / "sounds"


def note(freq, start, length, amp=1.0, decay=6.0, harmonic=0.0):
    """One struck note: `freq` Hz entering at `start` seconds."""
    return {
        "freq": freq,
        "start": start,
        "length": length,
        "amp": amp,
        "decay": decay,
        "harmonic": harmonic,
    }


# Every sound is a short stack of struck notes. Decay is the exponential rate,
# so a larger number is a shorter, drier tail.
SOUNDS = {
    # A struck bell: fundamental plus a fifth above it, ringing out slowly.
    "bell": [
        note(880.0, 0.0, 1.0, amp=0.55, decay=3.4),
        note(1320.0, 0.0, 1.0, amp=0.22, decay=5.0),
        note(2640.0, 0.0, 0.35, amp=0.06, decay=12.0),
    ],
    # One clean high blip. The shortest sound here.
    "ping": [
        note(1244.5, 0.0, 0.45, amp=0.5, decay=11.0, harmonic=0.12),
    ],
    # Two quick descending taps, wooden rather than metallic.
    "blip": [
        note(587.33, 0.0, 0.3, amp=0.45, decay=16.0, harmonic=0.3),
        note(440.0, 0.11, 0.35, amp=0.45, decay=14.0, harmonic=0.3),
    ],
    # A rising C major arpeggio that lands as a chord.
    "arpeggio": [
        note(523.25, 0.0, 0.9, amp=0.34, decay=4.2, harmonic=0.22),
        note(659.25, 0.13, 0.77, amp=0.34, decay=4.2, harmonic=0.22),
        note(783.99, 0.26, 0.64, amp=0.34, decay=4.2, harmonic=0.22),
    ],
    # Two low taps, closer to a knock on a door than a chime.
    "knock": [
        note(196.0, 0.0, 0.4, amp=0.6, decay=17.0, harmonic=0.45),
        note(174.61, 0.16, 0.45, amp=0.55, decay=15.0, harmonic=0.45),
    ],
}

ATTACK_SECONDS = 0.004
FADE_OUT_SECONDS = 0.02
# Peak amplitude every sound is normalized to. This matches the original
# `chime.mp3`, so switching sounds never changes how loud Herdr is.
TARGET_PEAK = 0.30


def render(notes):
    """Mix `notes` into a list of floats in roughly [-1.0, 1.0]."""
    total = max(n["start"] + n["length"] for n in notes)
    samples = [0.0] * int(total * SAMPLE_RATE)

    for n in notes:
        first = int(n["start"] * SAMPLE_RATE)
        count = int(n["length"] * SAMPLE_RATE)
        for i in range(count):
            t = i / SAMPLE_RATE
            envelope = math.exp(-n["decay"] * t)
            if t < ATTACK_SECONDS:
                envelope *= t / ATTACK_SECONDS
            value = math.sin(2 * math.pi * n["freq"] * t)
            if n["harmonic"]:
                # A fourth-harmonic partial that dies faster than the
                # fundamental is what reads as "wooden" instead of "sine".
                value += n["harmonic"] * math.exp(-3 * n["decay"] * t) * math.sin(
                    2 * math.pi * n["freq"] * 4 * t
                )
            samples[first + i] += n["amp"] * envelope * value

    fade = int(FADE_OUT_SECONDS * SAMPLE_RATE)
    for i in range(min(fade, len(samples))):
        samples[len(samples) - 1 - i] *= i / fade

    peak = max((abs(s) for s in samples), default=0.0)
    if peak > 0:
        samples = [s * (TARGET_PEAK / peak) for s in samples]
    return samples


def write_wav(samples, path):
    frames = b"".join(
        struct.pack("<h", max(-32768, min(32767, int(s * 32767)))) for s in samples
    )
    with wave.open(str(path), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(SAMPLE_RATE)
        out.writeframes(frames)


def encode_mp3(wav_path, mp3_path):
    subprocess.run(
        [
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            str(wav_path),
            "-codec:a",
            "libmp3lame",
            "-b:a",
            "128k",
            "-ac",
            "1",
            str(mp3_path),
        ],
        check=True,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "names",
        nargs="*",
        default=sorted(SOUNDS),
        help="sounds to regenerate (default: all)",
    )
    args = parser.parse_args()

    unknown = [name for name in args.names if name not in SOUNDS]
    if unknown:
        parser.error(f"unknown sound(s): {', '.join(unknown)}")

    SOUNDS_DIR.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        for name in args.names:
            samples = render(SOUNDS[name])
            wav_path = Path(tmp) / f"{name}.wav"
            mp3_path = SOUNDS_DIR / f"{name}.mp3"
            write_wav(samples, wav_path)
            encode_mp3(wav_path, mp3_path)
            print(f"{mp3_path.relative_to(REPO_ROOT)} ({mp3_path.stat().st_size} bytes)")


if __name__ == "__main__":
    sys.exit(main())
