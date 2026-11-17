#!/usr/bin/env python3
"""Exercise FFmpeg startup-offset and clock-drift alignment with synthetic WAVs."""

from __future__ import annotations

import argparse
import array
import subprocess
import wave
from dataclasses import dataclass
from pathlib import Path


SAMPLE_RATE = 48_000
MARKER_FRAMES = 240
MARKER_LEVEL = 12_000
MAX_SEPARATION_FRAMES = 2


@dataclass(frozen=True)
class SyncCase:
    name: str
    mic_start_seconds: float
    mic_seconds_per_loopback_second: float
    marker_seconds: tuple[float, float]


def write_loopback(path: Path, case: SyncCase) -> None:
    final_frame = round((case.marker_seconds[-1] + 0.5) * SAMPLE_RATE)
    marker_starts = {round(seconds * SAMPLE_RATE) for seconds in case.marker_seconds}
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(2)
        wav.setsampwidth(2)
        wav.setframerate(SAMPLE_RATE)
        samples = array.array("h")
        for frame in range(final_frame):
            pulse = MARKER_LEVEL if any(
                start <= frame < start + MARKER_FRAMES for start in marker_starts
            ) else 0
            samples.extend((pulse, 0))
        wav.writeframes(samples.tobytes())


def write_microphone(path: Path, case: SyncCase) -> None:
    marker_starts = {
        round(
            (seconds - case.mic_start_seconds)
            * SAMPLE_RATE
            / case.mic_seconds_per_loopback_second
        )
        for seconds in case.marker_seconds
    }
    final_frame = max(marker_starts) + round(0.5 * SAMPLE_RATE)
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(SAMPLE_RATE)
        samples = array.array("h")
        for frame in range(final_frame):
            pulse = -MARKER_LEVEL if any(
                start <= frame < start + MARKER_FRAMES for start in marker_starts
            ) else 0
            samples.append(pulse)
        wav.writeframes(samples.tobytes())


def affine_filter(case: SyncCase) -> str:
    delay_frames = abs(round(case.mic_start_seconds * SAMPLE_RATE))
    corrected_mic_rate = SAMPLE_RATE / case.mic_seconds_per_loopback_second
    loopback_delay = (
        f"adelay=delays={delay_frames}S:all=1,"
        if case.mic_start_seconds < 0
        else ""
    )
    mic_delay = (
        f"adelay=delays={delay_frames}S:all=1,"
        if case.mic_start_seconds > 0
        else ""
    )
    return (
        f"[0:a]{loopback_delay}aformat=channel_layouts=stereo[loopback];"
        f"[1:a]asetrate={corrected_mic_rate:.12f},aresample={SAMPLE_RATE},"
        f"{mic_delay}aformat=channel_layouts=mono,asplit=2[ml][mr];"
        "[loopback][ml][mr]amerge=inputs=3,"
        "pan=stereo|c0<c0+c2|c1<c1+c2[aout]"
    )


def merge(loopback: Path, microphone: Path, output: Path, case: SyncCase) -> None:
    codec_args = (
        ["-c:a", "aac", "-b:a", "192k", "-movflags", "faststart"]
        if output.suffix == ".m4a"
        else ["-c:a", "pcm_s16le"]
    )
    command = [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        str(loopback),
        "-i",
        str(microphone),
        "-filter_complex",
        affine_filter(case),
        "-map",
        "[aout]",
        "-ar",
        str(SAMPLE_RATE),
        *codec_args,
        "-y",
        str(output),
    ]
    subprocess.run(command, check=True)


def marker_frames(path: Path) -> tuple[list[int], list[int]]:
    with wave.open(str(path), "rb") as wav:
        if wav.getnchannels() != 2 or wav.getsampwidth() != 2:
            raise AssertionError("expected stereo 16-bit PCM output")
        samples = array.array("h")
        samples.frombytes(wav.readframes(wav.getnframes()))

    loopback_markers: list[int] = []
    mic_markers: list[int] = []
    loopback_active = False
    mic_active = False
    for frame in range(len(samples) // 2):
        left = samples[frame * 2]
        right = samples[frame * 2 + 1]
        next_loopback_active = abs(left - right) > 1_000
        next_mic_active = abs(right) > 1_000
        if next_loopback_active and not loopback_active:
            loopback_markers.append(frame)
        if next_mic_active and not mic_active:
            mic_markers.append(frame)
        loopback_active = next_loopback_active
        mic_active = next_mic_active
    return loopback_markers, mic_markers


def run_case(case: SyncCase, keep: Path | None, extension: str = "wav") -> None:
    fixture_root = keep or Path.cwd() / "target" / "audio-sync-fixtures"
    directory = fixture_root / case.name
    directory.mkdir(parents=True, exist_ok=True)

    loopback = directory / "loopback.wav"
    microphone = directory / "microphone.wav"
    output = directory / f"merged.{extension}"
    write_loopback(loopback, case)
    write_microphone(microphone, case)
    merge(loopback, microphone, output, case)
    if extension == "m4a":
        decoded = directory / "merged-decoded.wav"
        subprocess.run(
            [
                "ffmpeg", "-hide_banner", "-loglevel", "error", "-i", str(output),
                "-c:a", "pcm_s16le", "-y", str(decoded),
            ],
            check=True,
        )
    else:
        decoded = output
    loopback_markers, mic_markers = marker_frames(decoded)

    if len(loopback_markers) != 2 or len(mic_markers) != 2:
        raise AssertionError(
            f"{case.name}: expected two markers per stream, got "
            f"loopback={loopback_markers}, microphone={mic_markers}"
        )

    separations = [
        abs(loopback - microphone)
        for loopback, microphone in zip(loopback_markers, mic_markers)
    ]
    print(
        f"{case.name}: early={separations[0]} frames, "
        f"late={separations[1]} frames"
    )
    if max(separations) > MAX_SEPARATION_FRAMES:
        raise AssertionError(
            f"{case.name}: marker separation exceeds "
            f"{MAX_SEPARATION_FRAMES} frames: {separations}"
        )


def run_fractional_tail_case(keep: Path | None) -> None:
    fixture_root = keep or Path.cwd() / "target" / "audio-sync-fixtures"
    directory = fixture_root / "fractional-millisecond-tail"
    directory.mkdir(parents=True, exist_ok=True)
    loopback = directory / "loopback.wav"
    microphone = directory / "microphone.wav"
    output = directory / "merged.wav"
    mic_frames = SAMPLE_RATE + 47

    with wave.open(str(loopback), "wb") as wav:
        wav.setnchannels(2)
        wav.setsampwidth(2)
        wav.setframerate(SAMPLE_RATE)
        wav.writeframes(array.array("h", [0] * SAMPLE_RATE * 2).tobytes())
    with wave.open(str(microphone), "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(SAMPLE_RATE)
        samples = array.array(
            "h",
            [0] * (mic_frames - 20) + [-MARKER_LEVEL] * 20,
        )
        wav.writeframes(samples.tobytes())

    common_duration = mic_frames / SAMPLE_RATE
    filter_graph = (
        f"[0:a]aresample={SAMPLE_RATE},aformat=channel_layouts=stereo,"
        f"apad=whole_dur={common_duration:.9f}[left];"
        f"[1:a]aresample={SAMPLE_RATE},aformat=channel_layouts=mono,"
        f"apad=whole_dur={common_duration:.9f},asplit=2[ml][mr];"
        "[left][ml][mr]amerge=inputs=3,"
        "pan=stereo|c0<c0+c2|c1<c1+c2[aout]"
    )
    subprocess.run(
        [
            "ffmpeg", "-hide_banner", "-loglevel", "error",
            "-i", str(loopback), "-i", str(microphone),
            "-filter_complex", filter_graph, "-map", "[aout]",
            "-ar", str(SAMPLE_RATE), "-c:a", "pcm_s16le",
            "-y", str(output),
        ],
        check=True,
    )

    with wave.open(str(output), "rb") as wav:
        output_frames = wav.getnframes()
        samples = array.array("h")
        samples.frombytes(wav.readframes(output_frames))
    tail_marker = any(
        abs(samples[frame * 2 + 1]) > 1_000
        for frame in range(SAMPLE_RATE, output_frames)
    )
    print(
        "fractional-millisecond-tail: "
        f"frames={output_frames}, tail_marker={tail_marker}"
    )
    if output_frames < mic_frames or not tail_marker:
        raise AssertionError("fractional-millisecond tail was truncated")


def first_active_frame(path: Path) -> int:
    with wave.open(str(path), "rb") as wav:
        channels = wav.getnchannels()
        samples = array.array("h")
        samples.frombytes(wav.readframes(wav.getnframes()))
    return next(
        frame
        for frame in range(len(samples) // channels)
        if abs(samples[frame * channels]) > 1_000
    )


def run_single_stream_cases(keep: Path | None) -> None:
    fixture_root = keep or Path.cwd() / "target" / "audio-sync-fixtures"
    directory = fixture_root / "single-stream-alignment"
    directory.mkdir(parents=True, exist_ok=True)
    loopback = directory / "loopback.wav"
    microphone = directory / "microphone.wav"
    output = directory / "merged.wav"
    case = SyncCase("single-stream", 0.0, 1.0, (0.25, 0.75))
    write_loopback(loopback, case)
    write_microphone(microphone, case)

    delay_frames = round(0.1 * SAMPLE_RATE)
    common_duration = 1.35
    loopback_filter = (
        f"[0:a]aresample={SAMPLE_RATE},adelay=delays={delay_frames}S:all=1,"
        f"aformat=channel_layouts=stereo,"
        f"apad=whole_dur={common_duration:.9f}[aout]"
    )
    subprocess.run(
        [
            "ffmpeg", "-hide_banner", "-loglevel", "error",
            "-i", str(loopback), "-filter_complex", loopback_filter,
            "-map", "[aout]", "-ar", str(SAMPLE_RATE),
            "-c:a", "pcm_s16le", "-y", str(output),
        ],
        check=True,
    )
    loopback_first = first_active_frame(output)
    expected_loopback = round(0.35 * SAMPLE_RATE)
    if abs(loopback_first - expected_loopback) > MAX_SEPARATION_FRAMES:
        raise AssertionError("loopback-only alignment was not applied")

    corrected_rate = SAMPLE_RATE / 1.01
    common_duration = 1.25 * 1.01 + 0.1
    mic_filter = (
        f"[0:a]asetrate={corrected_rate:.12f},aresample={SAMPLE_RATE},"
        f"adelay=delays={delay_frames}S:all=1,aformat=channel_layouts=mono,"
        f"apad=whole_dur={common_duration:.9f},asplit=2[l][r];"
        "[l][r]amerge=inputs=2,pan=stereo|c0=c0|c1=c1[aout]"
    )
    subprocess.run(
        [
            "ffmpeg", "-hide_banner", "-loglevel", "error",
            "-i", str(microphone), "-filter_complex", mic_filter,
            "-map", "[aout]", "-ar", str(SAMPLE_RATE),
            "-c:a", "pcm_s16le", "-y", str(output),
        ],
        check=True,
    )
    microphone_first = first_active_frame(output)
    expected_microphone = round((0.25 * 1.01 + 0.1) * SAMPLE_RATE)
    if abs(microphone_first - expected_microphone) > MAX_SEPARATION_FRAMES:
        raise AssertionError("microphone-only affine alignment was not applied")
    print(
        "single-stream-alignment: "
        f"loopback={loopback_first}, microphone={microphone_first}"
    )

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--keep", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    cases = [
        SyncCase(
            name="loopback-first-mic-slow",
            mic_start_seconds=0.2,
            mic_seconds_per_loopback_second=1.01,
            marker_seconds=(1.0, 5.0),
        ),
        SyncCase(
            name="mic-first-mic-fast",
            mic_start_seconds=-0.2,
            mic_seconds_per_loopback_second=0.99,
            marker_seconds=(1.0, 5.0),
        ),
        SyncCase(
            name="measured-460ms-6_7ppm",
            mic_start_seconds=0.460,
            mic_seconds_per_loopback_second=1.0000067,
            marker_seconds=(1.0, 5.0),
        ),
    ]
    for case in cases:
        run_case(case, args.keep)
    run_case(cases[0], args.keep, extension="m4a")
    run_fractional_tail_case(args.keep)
    run_single_stream_cases(args.keep)
    print("FFmpeg affine sync fixture passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
