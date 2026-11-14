#!/usr/bin/env python3
"""Measure microphone/loopback drift from a stereo sync-marker recording."""

from __future__ import annotations

import argparse
import array
import json
import math
import statistics
import sys
import wave
from pathlib import Path


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * fraction)))
    return ordered[index]


def read_envelopes(path: Path, window_ms: int) -> tuple[list[float], list[float], float]:
    with wave.open(str(path), "rb") as wav:
        if wav.getnchannels() != 2:
            raise ValueError(f"expected stereo WAV, found {wav.getnchannels()} channels")
        if wav.getsampwidth() != 2:
            raise ValueError(f"expected 16-bit PCM WAV, found {wav.getsampwidth() * 8}-bit")

        sample_rate = wav.getframerate()
        frames_per_window = max(1, round(sample_rate * window_ms / 1000))
        window_seconds = frames_per_window / sample_rate
        loopback_envelope: list[float] = []
        mic_envelope: list[float] = []

        while True:
            raw = wav.readframes(frames_per_window)
            if not raw:
                break

            samples = array.array("h")
            samples.frombytes(raw)
            if sys.byteorder != "little":
                samples.byteswap()

            frame_count = len(samples) // 2
            if frame_count == 0:
                continue

            loopback_energy = 0.0
            mic_energy = 0.0
            for frame in range(frame_count):
                left = samples[frame * 2] / 32768.0
                right = samples[frame * 2 + 1] / 32768.0
                loopback = left - right
                loopback_energy += loopback * loopback
                mic_energy += right * right

            loopback_envelope.append(math.sqrt(loopback_energy / frame_count))
            mic_envelope.append(math.sqrt(mic_energy / frame_count))

    return loopback_envelope, mic_envelope, window_seconds


def detect_markers(envelope: list[float], window_seconds: float) -> tuple[list[float], float, float]:
    noise_floor = statistics.median(envelope)
    marker_level = percentile(envelope, 0.995)
    dynamic_range = marker_level - noise_floor
    if dynamic_range <= 1e-5:
        return [], noise_floor, marker_level

    threshold = noise_floor + dynamic_range * 0.35
    groups: list[tuple[int, int]] = []
    start: int | None = None
    last_active: int | None = None

    for index, level in enumerate(envelope):
        if level >= threshold:
            if start is None:
                start = index
            last_active = index
        elif start is not None and last_active is not None and index - last_active > 2:
            groups.append((start, last_active))
            start = None
            last_active = None

    if start is not None and last_active is not None:
        groups.append((start, last_active))

    markers = []
    for group_start, group_end in groups:
        duration = (group_end - group_start + 1) * window_seconds
        if 0.05 <= duration <= 1.5:
            markers.append(group_start * window_seconds)

    return markers, noise_floor, marker_level


def pair_markers(loopback: list[float], mic: list[float], max_separation: float) -> list[tuple[float, float]]:
    pairs: list[tuple[float, float]] = []
    next_mic = 0

    for loopback_time in loopback:
        candidates = [
            (abs(mic[index] - loopback_time), index)
            for index in range(next_mic, len(mic))
            if abs(mic[index] - loopback_time) <= max_separation
        ]
        if not candidates:
            continue

        _, selected = min(candidates)
        pairs.append((loopback_time, mic[selected]))
        next_mic = selected + 1

    return pairs


def linear_regression(xs: list[float], ys: list[float]) -> tuple[float, float, list[float]]:
    x_mean = statistics.mean(xs)
    y_mean = statistics.mean(ys)
    variance = sum((value - x_mean) ** 2 for value in xs)
    if variance == 0:
        return 0.0, y_mean, [0.0 for _ in xs]

    slope = sum((x - x_mean) * (y - y_mean) for x, y in zip(xs, ys)) / variance
    intercept = y_mean - slope * x_mean
    residuals = [y - (intercept + slope * x) for x, y in zip(xs, ys)]
    return slope, intercept, residuals


def analyze(args: argparse.Namespace) -> tuple[dict[str, object], int]:
    loopback_envelope, mic_envelope, window_seconds = read_envelopes(args.recording, args.window_ms)
    loopback_markers, loopback_noise, loopback_level = detect_markers(
        loopback_envelope, window_seconds
    )
    mic_markers, mic_noise, mic_level = detect_markers(mic_envelope, window_seconds)
    pairs = pair_markers(loopback_markers, mic_markers, args.max_pair_separation)

    report: dict[str, object] = {
        "recording": str(args.recording.resolve()),
        "window_ms": args.window_ms,
        "loopback_markers": len(loopback_markers),
        "mic_markers": len(mic_markers),
        "paired_markers": len(pairs),
        "loopback_noise_rms": loopback_noise,
        "loopback_marker_rms": loopback_level,
        "mic_noise_rms": mic_noise,
        "mic_marker_rms": mic_level,
    }

    if len(pairs) < args.min_markers:
        report.update(
            status="INVALID",
            message=f"only {len(pairs)} marker pairs detected; need at least {args.min_markers}",
        )
        return report, 2

    loopback_times = [pair[0] for pair in pairs]
    offsets = [mic_time - loopback_time for loopback_time, mic_time in pairs]
    elapsed = [value - loopback_times[0] for value in loopback_times]
    slope, intercept, residuals = linear_regression(elapsed, offsets)
    span_seconds = elapsed[-1]
    drift_ms_per_hour = slope * 3_600_000.0
    drift_ppm = slope * 1_000_000.0
    end_to_end_drift_ms = (offsets[-1] - offsets[0]) * 1000.0
    residual_ms = [abs(value) * 1000.0 for value in residuals]
    p95_jitter_ms = percentile(residual_ms, 0.95)

    if span_seconds < args.min_measurement_seconds:
        status = "VALIDATION"
        exit_code = 0
        message = "markers detected; recording is too short for a drift verdict"
    elif abs(drift_ms_per_hour) > args.max_drift_ms_per_hour:
        status = "FAIL"
        exit_code = 1
        message = "relative drift exceeds threshold"
    else:
        status = "PASS"
        exit_code = 0
        message = "relative drift is within threshold"

    report.update(
        status=status,
        message=message,
        measurement_span_seconds=span_seconds,
        initial_offset_ms=offsets[0] * 1000.0,
        final_offset_ms=offsets[-1] * 1000.0,
        fitted_initial_offset_ms=intercept * 1000.0,
        end_to_end_drift_ms=end_to_end_drift_ms,
        drift_ms_per_hour=drift_ms_per_hour,
        drift_ppm=drift_ppm,
        p95_jitter_ms=p95_jitter_ms,
        max_drift_ms_per_hour=args.max_drift_ms_per_hour,
        pairs=[
            {
                "loopback_seconds": loopback_time,
                "mic_seconds": mic_time,
                "offset_ms": (mic_time - loopback_time) * 1000.0,
            }
            for loopback_time, mic_time in pairs
        ],
    )
    return report, exit_code


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("recording", type=Path)
    parser.add_argument("--window-ms", type=int, default=10)
    parser.add_argument("--min-markers", type=int, default=4)
    parser.add_argument("--max-pair-separation", type=float, default=2.0)
    parser.add_argument("--min-measurement-seconds", type=float, default=600.0)
    parser.add_argument("--max-drift-ms-per-hour", type=float, default=20.0)
    parser.add_argument("--report", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        report, exit_code = analyze(args)
    except (OSError, ValueError, wave.Error) as error:
        print(f"sync analysis failed: {error}", file=sys.stderr)
        return 2

    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    print(f"status: {report['status']}")
    print(
        "markers: "
        f"loopback={report['loopback_markers']} "
        f"microphone={report['mic_markers']} "
        f"paired={report['paired_markers']}"
    )
    if "drift_ms_per_hour" in report:
        print(f"initial offset: {report['initial_offset_ms']:.1f} ms")
        print(f"end-to-end drift: {report['end_to_end_drift_ms']:.1f} ms")
        print(f"fitted drift: {report['drift_ms_per_hour']:.1f} ms/hour")
        print(f"clock delta: {report['drift_ppm']:.1f} ppm")
        print(f"p95 residual jitter: {report['p95_jitter_ms']:.1f} ms")
    print(f"result: {report['message']}")
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
