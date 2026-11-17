use audio_rec_core::domain::AudioFormat;
use audio_rec_core::recorder::{merge_audio_streams_smart, AudioAlignment, RecordingQuality};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SAMPLE_RATE: u32 = 48_000;
const MARKER_FRAMES: usize = 240;
const MARKER_LEVEL: i16 = 12_000;
const MAX_SEPARATION_FRAMES: usize = 2;

#[derive(Clone, Copy)]
struct SyncCase {
    name: &'static str,
    mic_start_seconds: f64,
    mic_timeline_ratio: f64,
    marker_seconds: [f64; 2],
}

fn marker_starts(seconds: &[f64; 2]) -> [usize; 2] {
    seconds.map(|time| (time * f64::from(SAMPLE_RATE)).round() as usize)
}

fn write_loopback_fixture(path: &Path, case: SyncCase) {
    let markers = marker_starts(&case.marker_seconds);
    let final_frame = markers[1] + SAMPLE_RATE as usize / 2;
    let spec = WavSpec {
        channels: 2,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).unwrap();

    for frame in 0..final_frame {
        let pulse = if markers
            .iter()
            .any(|start| (*start..*start + MARKER_FRAMES).contains(&frame))
        {
            MARKER_LEVEL
        } else {
            0
        };
        writer.write_sample(pulse).unwrap();
        writer.write_sample(0i16).unwrap();
    }
    writer.finalize().unwrap();
}

fn write_mic_fixture(path: &Path, case: SyncCase) {
    let markers = case.marker_seconds.map(|time| {
        ((time - case.mic_start_seconds) * f64::from(SAMPLE_RATE) / case.mic_timeline_ratio).round()
            as usize
    });
    let final_frame = markers[1] + SAMPLE_RATE as usize / 2;
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).unwrap();

    for frame in 0..final_frame {
        let pulse = if markers
            .iter()
            .any(|start| (*start..*start + MARKER_FRAMES).contains(&frame))
        {
            -MARKER_LEVEL
        } else {
            0
        };
        writer.write_sample(pulse).unwrap();
    }
    writer.finalize().unwrap();
}

fn write_silent_fixture(path: &Path, channels: u16, seconds: f64) {
    let spec = WavSpec {
        channels,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).unwrap();
    let frame_count = (seconds * f64::from(SAMPLE_RATE)).round() as usize;
    for _ in 0..frame_count * usize::from(channels) {
        writer.write_sample(0i16).unwrap();
    }
    writer.finalize().unwrap();
}

fn stereo_samples(path: &Path) -> Vec<[i16; 2]> {
    let reader = WavReader::open(path).unwrap();
    assert_eq!(reader.spec().channels, 2);
    let samples: Vec<i16> = reader
        .into_samples::<i16>()
        .map(|sample| sample.unwrap())
        .collect();
    samples
        .chunks_exact(2)
        .map(|channels| [channels[0], channels[1]])
        .collect()
}
fn event_frames(path: &Path) -> (Vec<usize>, Vec<usize>) {
    let reader = WavReader::open(path).unwrap();
    assert_eq!(reader.spec().channels, 2);
    let samples: Vec<i16> = reader
        .into_samples::<i16>()
        .map(|sample| sample.unwrap())
        .collect();

    let mut loopback_events = Vec::new();
    let mut mic_events = Vec::new();
    let mut loopback_active = false;
    let mut mic_active = false;
    for (frame, channels) in samples.chunks_exact(2).enumerate() {
        let left = i32::from(channels[0]);
        let right = i32::from(channels[1]);
        let next_loopback_active = (left - right).abs() > 1_000;
        let next_mic_active = right.abs() > 1_000;
        if next_loopback_active && !loopback_active {
            loopback_events.push(frame);
        }
        if next_mic_active && !mic_active {
            mic_events.push(frame);
        }
        loopback_active = next_loopback_active;
        mic_active = next_mic_active;
    }
    (loopback_events, mic_events)
}

fn unique_test_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let test_dir = std::env::current_dir()
        .unwrap()
        .join("target/test-artifacts")
        .join(format!("audio-rec-sync-{name}-{unique}"));
    std::fs::create_dir_all(&test_dir).unwrap();
    test_dir
}

async fn run_sync_case(case: SyncCase, output_format: AudioFormat) {
    let test_dir = unique_test_dir(case.name);
    let loopback = test_dir.join("loopback.wav");
    let mic = test_dir.join("mic.wav");
    let encoded = test_dir.join(format!("merged.{}", output_format.extension()));
    write_loopback_fixture(&loopback, case);
    write_mic_fixture(&mic, case);

    let alignment = AudioAlignment::affine(case.mic_start_seconds, case.mic_timeline_ratio);
    merge_audio_streams_smart(
        &loopback,
        &mic,
        &encoded,
        true,
        true,
        &RecordingQuality::professional(),
        output_format,
        None,
        None,
        3,
        &alignment,
    )
    .await
    .unwrap();

    let decoded = if matches!(output_format, AudioFormat::M4a) {
        let decoded = test_dir.join("decoded.wav");
        let status = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&encoded)
            .args(["-c:a", "pcm_s16le", "-y"])
            .arg(&decoded)
            .status()
            .unwrap();
        assert!(status.success());
        decoded
    } else {
        encoded
    };

    let (loopback_events, mic_events) = event_frames(&decoded);
    assert_eq!(loopback_events.len(), 2, "{} loopback markers", case.name);
    assert_eq!(mic_events.len(), 2, "{} microphone markers", case.name);
    for (index, (loopback, mic)) in loopback_events.iter().zip(&mic_events).enumerate() {
        let separation = loopback.abs_diff(*mic);
        assert!(
            separation <= MAX_SEPARATION_FRAMES,
            "{} marker {index} is separated by {separation} frames (loopback={loopback}, mic={mic})",
            case.name
        );
    }

    std::fs::remove_dir_all(test_dir).unwrap();
}

#[tokio::test]
async fn merge_aligns_same_event_when_loopback_started_first() {
    run_sync_case(
        SyncCase {
            name: "loopback-first-fixed-offset",
            mic_start_seconds: 0.2,
            mic_timeline_ratio: 1.0,
            marker_seconds: [1.0, 5.0],
        },
        AudioFormat::Wav,
    )
    .await;
}

#[tokio::test]
async fn merge_aligns_early_and_late_markers_when_microphone_clock_is_slow() {
    run_sync_case(
        SyncCase {
            name: "loopback-first-mic-slow",
            mic_start_seconds: 0.2,
            mic_timeline_ratio: 1.01,
            marker_seconds: [1.0, 5.0],
        },
        AudioFormat::Wav,
    )
    .await;
}

#[tokio::test]
async fn merge_aligns_early_and_late_markers_when_microphone_started_first_and_is_fast() {
    run_sync_case(
        SyncCase {
            name: "mic-first-mic-fast",
            mic_start_seconds: -0.2,
            mic_timeline_ratio: 0.99,
            marker_seconds: [1.0, 5.0],
        },
        AudioFormat::Wav,
    )
    .await;
}

#[tokio::test]
async fn merge_aligns_measured_startup_offset_and_clock_delta() {
    run_sync_case(
        SyncCase {
            name: "measured-460ms-6_7ppm",
            mic_start_seconds: 0.460,
            mic_timeline_ratio: 1.0000067,
            marker_seconds: [1.0, 5.0],
        },
        AudioFormat::Wav,
    )
    .await;
}

#[tokio::test]
async fn merge_applies_affine_alignment_when_encoding_m4a() {
    run_sync_case(
        SyncCase {
            name: "m4a-mic-first-slow",
            mic_start_seconds: -0.2,
            mic_timeline_ratio: 1.01,
            marker_seconds: [1.0, 5.0],
        },
        AudioFormat::M4a,
    )
    .await;
}
#[tokio::test]
async fn merge_preserves_loopback_when_microphone_is_muted() {
    let test_dir = unique_test_dir("muted-microphone");
    let loopback = test_dir.join("loopback.wav");
    let microphone = test_dir.join("microphone.wav");
    let output = test_dir.join("merged.wav");
    let case = SyncCase {
        name: "muted-microphone",
        mic_start_seconds: 0.0,
        mic_timeline_ratio: 1.0,
        marker_seconds: [0.25, 0.75],
    };
    write_loopback_fixture(&loopback, case);
    write_silent_fixture(&microphone, 1, 1.25);

    merge_audio_streams_smart(
        &loopback,
        &microphone,
        &output,
        true,
        false,
        &RecordingQuality::professional(),
        AudioFormat::Wav,
        None,
        None,
        3,
        &AudioAlignment::affine(-0.1, 1.0),
    )
    .await
    .unwrap();

    let samples = stereo_samples(&output);
    let first_marker = samples
        .iter()
        .position(|channels| channels[0].abs() > 1_000)
        .unwrap();
    assert!(first_marker.abs_diff((0.35 * f64::from(SAMPLE_RATE)) as usize) <= 2);
    std::fs::remove_dir_all(test_dir).unwrap();
}

#[tokio::test]
async fn merge_preserves_microphone_when_loopback_is_silent() {
    let test_dir = unique_test_dir("silent-loopback");
    let loopback = test_dir.join("loopback.wav");
    let microphone = test_dir.join("microphone.wav");
    let output = test_dir.join("merged.wav");
    let case = SyncCase {
        name: "silent-loopback",
        mic_start_seconds: 0.0,
        mic_timeline_ratio: 1.0,
        marker_seconds: [0.25, 0.75],
    };
    write_silent_fixture(&loopback, 2, 1.25);
    write_mic_fixture(&microphone, case);

    merge_audio_streams_smart(
        &loopback,
        &microphone,
        &output,
        false,
        true,
        &RecordingQuality::professional(),
        AudioFormat::Wav,
        None,
        None,
        3,
        &AudioAlignment::affine(0.1, 1.01),
    )
    .await
    .unwrap();

    let samples = stereo_samples(&output);
    let first_marker = samples
        .iter()
        .position(|channels| channels[0].abs() > 1_000)
        .unwrap();
    let expected_marker = ((0.25 * 1.01 + 0.1) * f64::from(SAMPLE_RATE)).round() as usize;
    assert!(first_marker.abs_diff(expected_marker) <= 2);
    assert!(samples.iter().all(|channels| channels[0] == channels[1]));
    std::fs::remove_dir_all(test_dir).unwrap();
}

#[tokio::test]
async fn merge_creates_valid_output_when_both_streams_are_silent() {
    let test_dir = unique_test_dir("both-silent");
    let loopback = test_dir.join("loopback.wav");
    let microphone = test_dir.join("microphone.wav");
    let output = test_dir.join("merged.wav");
    write_silent_fixture(&loopback, 2, 1.0);
    write_silent_fixture(&microphone, 1, 1.0);

    merge_audio_streams_smart(
        &loopback,
        &microphone,
        &output,
        false,
        false,
        &RecordingQuality::professional(),
        AudioFormat::Wav,
        None,
        None,
        3,
        &AudioAlignment::none("both streams are silent"),
    )
    .await
    .unwrap();

    let samples = stereo_samples(&output);
    assert!(!samples.is_empty());
    assert!(samples.iter().all(|channels| *channels == [0, 0]));
    std::fs::remove_dir_all(test_dir).unwrap();
}

#[tokio::test]
async fn silence_trimming_keeps_audio_and_removes_silent_edges() {
    let test_dir = unique_test_dir("silence-trimming");
    let recording = test_dir.join("recording.wav");
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(&recording, spec).unwrap();
    for frame in 0..SAMPLE_RATE as usize * 3 {
        let sample = if (SAMPLE_RATE as usize..SAMPLE_RATE as usize * 2).contains(&frame) {
            MARKER_LEVEL
        } else {
            0
        };
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();

    audio_rec_core::ffmpeg_encoder::trim_silence(&recording, -50.0, 2.0)
        .await
        .unwrap();

    let reader = WavReader::open(&recording).unwrap();
    assert!(reader.duration() < SAMPLE_RATE * 2);
    assert!(reader
        .into_samples::<i16>()
        .any(|sample| sample.unwrap().abs() > 1_000));
    std::fs::remove_dir_all(test_dir).unwrap();
}
#[tokio::test]
async fn merge_preserves_fractional_millisecond_tail_after_alignment() {
    let test_dir = unique_test_dir("fractional-millisecond-tail");
    let loopback = test_dir.join("loopback.wav");
    let microphone = test_dir.join("microphone.wav");
    let output = test_dir.join("merged.wav");
    write_silent_fixture(&loopback, 2, 1.0);

    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(&microphone, spec).unwrap();
    let mic_frames = SAMPLE_RATE + 47;
    for frame in 0..mic_frames {
        let sample = if frame >= mic_frames - 20 {
            MARKER_LEVEL
        } else {
            0
        };
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();

    merge_audio_streams_smart(
        &loopback,
        &microphone,
        &output,
        true,
        true,
        &RecordingQuality::professional(),
        AudioFormat::Wav,
        None,
        None,
        3,
        &AudioAlignment::none("tail preservation test"),
    )
    .await
    .unwrap();

    let samples = stereo_samples(&output);
    assert!(samples.len() >= mic_frames as usize);
    assert!(samples[SAMPLE_RATE as usize..]
        .iter()
        .any(|channels| channels[1].abs() > 1_000));
    std::fs::remove_dir_all(test_dir).unwrap();
}
