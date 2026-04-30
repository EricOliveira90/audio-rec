PS C:\Code\audio-rec> cargo run --release -- record
warning: unused import: `windows::Win32::System::Threading::*`
   --> core\src\wasapi\process_resolver.rs:116:9
    |
116 |     use windows::Win32::System::Threading::*;
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = note: `#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default

warning: field `frame_buffer` is never read
  --> core\src\audio_processing\noise_suppress.rs:44:5
   |
41 | pub struct NoiseSuppressor {
   |            --------------- field in this struct
...
44 |     frame_buffer: Vec<f32>,
   |     ^^^^^^^^^^^^
   |
   = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: constant `AUDIOCLIENT_ACTIVATION_TYPE_DEFAULT` is never used
  --> core\src\wasapi\process_loopback.rs:44:7
   |
44 | const AUDIOCLIENT_ACTIVATION_TYPE_DEFAULT: u32 = 0;
   |       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: constant `PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE` is never used
  --> core\src\wasapi\process_loopback.rs:52:7
   |
52 | const PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE: u32 = 1;
   |       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: `audio-rec-core` (lib) generated 4 warnings (run `cargo fix --lib -p audio-rec-core` to apply 1 suggestion)
    Finished `release` profile [optimized] target(s) in 0.94s
     Running `target\release\audio-rec.exe record`
2026-04-09T13:37:33.283837Z  INFO Audio Recorder Manager CLI started
{"capture_mode":"system","duration":-1,"file_path":"C:\\Code\\audio-rec\\2026-04-09-10_37_recording.wav","format":"wav","mic_muted":false,"quality":"Professional (48kHz Stereo)","session_id":"rec-20260409_103737","trim_silence":true,"type":"started"}
2026-04-09T13:37:37.773431Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Starting WASAPI dual-channel recording: 2026-04-09-10_37_recording.wav (0 seconds, mic_muted: false)
2026-04-09T13:37:37.821449Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: System audio sample rate: 48000 Hz - will match microphone to this
2026-04-09T13:37:37.822702Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Microphone recorder initialized successfully with matched sample rate
2026-04-09T13:37:37.828126Z  INFO WASAPI Microphone native format: 16000 Hz, 1 channels, 32 bits, tag: 0xFFFE, detected: Float32
2026-04-09T13:37:37.828414Z  INFO Using native microphone format - FFmpeg will resample to 48000 Hz during merge
2026-04-09T13:37:37.828577Z  INFO Initializing audio client with native format...
2026-04-09T13:37:37.832743Z  INFO WASAPI format: 48000 Hz, 2 channels, 32 bits, format tag: 0xFFFE, detected: Float32
2026-04-09T13:37:37.854744Z  INFO Recording with: Float32 format
2026-04-09T13:37:37.866128Z  INFO Audio detected! Level: 0.1568
2026-04-09T13:37:38.364739Z  INFO Audio client initialized successfully
2026-04-09T13:37:38.365154Z  INFO Getting capture client service...
2026-04-09T13:37:38.365548Z  INFO Capture client obtained successfully
2026-04-09T13:37:38.365989Z  INFO Creating WAV writer for microphone...
2026-04-09T13:37:38.367542Z  INFO WAV writer created successfully
2026-04-09T13:37:38.367853Z  INFO Starting microphone audio client...
2026-04-09T13:37:38.368957Z  INFO Audio client started successfully
2026-04-09T13:37:38.369542Z  INFO Recording microphone: 16000 Hz, 1 channels, Float32 format
2026-04-09T13:37:38.369930Z  INFO Microphone audio client started, entering recording loop...
{"capture_mode":"system","is_mic_muted":false,"loopback_has_audio":true,"mic_has_audio":false,"session_id":"rec-20260409_103737","type":"audio_state"}
2026-04-09T13:38:39.208101Z  INFO Microphone audio detected! Level: 0.0216
{"capture_mode":"system","is_mic_muted":false,"loopback_has_audio":true,"mic_has_audio":true,"session_id":"rec-20260409_103737","type":"audio_state"}
2026-04-09T14:32:51.209183Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Stop signal received for session rec-20260409_103737
2026-04-09T14:32:51.214599Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: ⏱️  Start time offset: 514 ms (loopback started first)
2026-04-09T14:32:51.272579Z  INFO Microphone recording loop finished: 309311 iterations, 309311 packets received, 309311 frames captured
2026-04-09T14:32:52.077696Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Recording completed after 3314s, starting post-processing
2026-04-09T14:32:52.078657Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Recording completed successfully! user_output=true category="success"
Recording completed successfully!
{"elapsed_secs":3314,"message":"Post-processing started","session_id":"rec-20260409_103737","type":"processing"}
2026-04-09T14:32:52.080312Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Waiting for temporary files to be ready...
2026-04-09T14:32:52.204634Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Temporary files are ready for processing
2026-04-09T14:32:52.205088Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Stage 1/4: Analyzing Audio - Detected system audio and microphone
2026-04-09T14:32:52.208219Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Post-processing started
2026-04-09T14:32:52.209411Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: Merging audio channels... user_output=true prefix="Processing"
[Processing] Merging audio channels...
2026-04-09T14:32:52.421764Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: ═══════════════════════════════════════════════════════════
2026-04-09T14:32:52.422490Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: 🎧 AUDIO MERGE PROCESS STARTED
2026-04-09T14:32:52.423232Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: ═══════════════════════════════════════════════════════════
2026-04-09T14:32:52.423530Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: 📊 Merge Configuration:
2026-04-09T14:32:52.423818Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Loopback (System Audio): ✓ Present
2026-04-09T14:32:52.424157Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Microphone (User Audio):  ✓ Present
2026-04-09T14:32:52.424485Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Output Format:          WAV
2026-04-09T14:32:52.424836Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Output Sample Rate:     48000 Hz
2026-04-09T14:32:52.425111Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Output Channels:        2 (Stereo)
2026-04-09T14:32:52.425326Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Start offset:           514 ms (loopback started first, delaying loopback)
2026-04-09T14:32:52.609892Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: 📁 Input file sizes:
2026-04-09T14:32:52.610363Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Loopback WAV: 636163328 bytes (621253.25 KB)
2026-04-09T14:32:52.610727Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Microphone WAV: 106009964 bytes (103525.36 KB)
2026-04-09T14:32:52.611167Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: ───────────────────────────────────────────────────────────
2026-04-09T14:32:52.611464Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: ⏳ Starting merge operation...
2026-04-09T14:32:52.611782Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: 📝 Stage 2/4: Preparing merge (detecting durations)...
2026-04-09T14:32:52.614837Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: Attempting duration detection for: "C:\\Code\\audio-rec\\storage\\recordings\\rec-20260409_103737_loopback.wav"
2026-04-09T14:32:52.615986Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:   Duration from WAV header: 3313350 ms (3313.35s) - INSTANT
2026-04-09T14:32:52.616327Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: Attempting duration detection for: "C:\\Code\\audio-rec\\storage\\recordings\\rec-20260409_103737_mic.wav"
2026-04-09T14:32:52.617002Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:   Duration from WAV header: 3312810 ms (3312.81s) - INSTANT
2026-04-09T14:32:52.617862Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: 📊 Duration detection:
2026-04-09T14:32:52.618124Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:   ├─ Loopback: 3313350 ms
2026-04-09T14:32:52.618423Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:   ├─ Microphone: 3312810 ms
2026-04-09T14:32:52.618675Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:   └─ Using maximum: 3313350 ms
2026-04-09T14:32:52.618901Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: Duration detection result: 3313350 ms
2026-04-09T14:32:52.619155Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: Progress monitoring: ENABLED (using effective duration: 3313350 ms)
2026-04-09T14:32:52.619424Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: 📋 Merge Strategy: Dual-mono stereo (L=loopback, R=microphone)
2026-04-09T14:35:39.831344Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: 🔊 LUFS Measurement:
2026-04-09T14:35:39.831645Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Loopback (system): -18.7 LUFS
2026-04-09T14:35:39.831777Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Microphone (user): -29.4 LUFS
2026-04-09T14:35:39.831981Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Mic gain compensation: 10.8 dB
2026-04-09T14:35:39.832220Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: 🔀 Starting FFmpeg merge and encode...
2026-04-09T14:35:39.846071Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: ⏱️  Estimated processing time: ~11 minutes (based on 3313s audio at ~5x speed)
2026-04-09T14:35:39.846330Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     Audio duration: 3313350ms, Progress monitoring: ACTIVE
2026-04-09T14:35:43.071591Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     📊 Progress checkpoint: 79% | Elapsed: 3s | ETA: ~0 min 0 sec
2026-04-09T14:35:44.687941Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: 📝 Stage 3/4: Finalizing
2026-04-09T14:35:45.073105Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: 📊 Merge Results:
2026-04-09T14:35:45.073366Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Output file:     "rec-20260409_103737_merged.wav"
2026-04-09T14:35:45.073560Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Output format:   WAV
2026-04-09T14:35:45.073701Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Output size:     606.81 MB
2026-04-09T14:35:45.073922Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Time elapsed:    172.08s
2026-04-09T14:35:45.074155Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}:     • Sample rate:     48000 Hz
2026-04-09T14:35:45.074478Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: ═══════════════════════════════════════════════════════════
2026-04-09T14:35:45.074660Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: Audio processing completed: "C:\\Code\\audio-rec\\storage\\recordings\\rec-20260409_103737_merged.wav"
2026-04-09T14:35:45.074794Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:merge_audio{loopback_has_audio=true mic_has_audio=true}: Audio processing completed! user_output=true category="success"
Audio processing completed!
2026-04-09T14:35:45.774196Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Temporary recording files cleaned up
2026-04-09T14:35:45.775177Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Trimming silence from recording...
2026-04-09T14:35:45.777387Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Trimming silence... user_output=true prefix="Processing"
[Processing] Trimming silence...
{"message":"Trimming silence","session_id":"rec-20260409_103737","type":"processing"}
2026-04-09T14:35:45.778019Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: ═══════════════════════════════════════════════════════════
2026-04-09T14:35:45.778168Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: ✂️  SILENCE TRIMMING STARTED
2026-04-09T14:35:45.778295Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: ═══════════════════════════════════════════════════════════
2026-04-09T14:35:45.778440Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:     Input:              "rec-20260409_103737_merged.wav"
2026-04-09T14:35:45.778563Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:     Threshold:          -50 dB
2026-04-09T14:35:45.778679Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:     Max internal gap:   2s
2026-04-09T14:35:53.463590Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: ✂️  Silence trimming completed in 7.50s
2026-04-09T14:35:53.463841Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}:     Output size: 556.75 MB
2026-04-09T14:35:53.464082Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: ═══════════════════════════════════════════════════════════
2026-04-09T14:35:53.464240Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Silence trimming completed
2026-04-09T14:36:01.767943Z  INFO recording_session{session_id=rec-20260409_103737 format=Wav duration_secs=0}: Final file saved to: "C:\\Code\\audio-rec\\2026-04-09-10_37_recording.wav"
{"duration_secs":-1,"file_path":"C:\\Code\\audio-rec\\2026-04-09-10_37_recording.wav","file_size_mb":"556.75","filename":"2026-04-09-10_37_recording.wav","format":"wav","message":"Recording completed successfully","session_id":"rec-20260409_103737","type":"completed"}
2026-04-09T14:36:02.038413Z  INFO CLI completed successfull