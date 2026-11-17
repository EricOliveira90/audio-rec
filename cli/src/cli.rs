use std::path::PathBuf;
use std::str::FromStr;

use audio_rec_core::commands;
use audio_rec_core::commands::record::RecordOptions;
use audio_rec_core::config::RecorderConfig;
use audio_rec_core::domain::{AudioFormat, RecordingDuration};
use audio_rec_core::error::Result;
use audio_rec_core::output::UserOutput;
use audio_rec_core::recorder::RecordingQuality;

pub async fn run(args: Vec<String>) -> Result<()> {
    let output = UserOutput::new();

    if args.len() <= 1 {
        print_usage();
        return Ok(());
    }

    let command = &args[1];

    match command.as_str() {
        "record" => handle_record_command(&args, &output).await,
        "devices" => commands::devices::execute().await,
        "status" => commands::status::execute().await,
        "stop" => handle_stop_command(&args, &output).await,
        "mute" => handle_mute_command(&args, &output).await,
        "--help" | "-h" | "help" => {
            print_usage();
            Ok(())
        }
        _ => {
            output.error(&format!("Unknown command: {}", command));
            print_usage();
            std::process::exit(1);
        }
    }
}

/// Parse named flags from args slice, starting after the command name
fn parse_flag_value(args: &[String], flag: &str, short: Option<&str>) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        if arg == flag || short.map_or(false, |s| arg == s) {
            if i + 1 < args.len() {
                return Some(args[i + 1].clone());
            }
        }
    }
    None
}

/// Check if a boolean flag is present
fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

async fn handle_record_command(args: &[String], output: &UserOutput) -> Result<()> {
    let config = RecorderConfig::new();

    // Parse --duration / -d (default: indefinite)
    let duration = if let Some(d_str) = parse_flag_value(args, "--duration", Some("-d")) {
        match d_str.parse::<i64>() {
            Ok(d) => match RecordingDuration::from_secs(d) {
                Ok(dur) => dur,
                Err(e) => {
                    output.error(&e.to_string());
                    std::process::exit(1);
                }
            },
            Err(_) => {
                output.error(&format!("Invalid duration '{}'. Must be a number.", d_str));
                std::process::exit(1);
            }
        }
    } else {
        RecordingDuration::Indefinite
    };

    // Parse --output / -o (default: cwd + auto filename)
    let output_path = parse_flag_value(args, "--output", Some("-o")).map(PathBuf::from);

    // Determine format: from --format flag, or inferred from output path extension, or default wav
    let audio_format = if let Some(fmt_str) = parse_flag_value(args, "--format", Some("-f")) {
        match AudioFormat::from_str(&fmt_str) {
            Ok(fmt) => fmt,
            Err(e) => {
                output.error(&e.to_string());
                std::process::exit(1);
            }
        }
    } else if let Some(ref path) = output_path {
        // Infer from output file extension
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| AudioFormat::from_str(ext).ok())
            .unwrap_or(AudioFormat::Wav)
    } else {
        AudioFormat::Wav
    };

    // Parse --quality / -q (default: professional)
    let quality = if let Some(q_str) = parse_flag_value(args, "--quality", Some("-q")) {
        match q_str.to_lowercase().as_str() {
            "quick" => RecordingQuality::quick(),
            "standard" => RecordingQuality::standard(),
            "professional" => RecordingQuality::professional(),
            "high" => RecordingQuality::high(),
            _ => {
                output.error(&format!(
                    "Invalid quality '{}'. Options: quick, standard, professional, high",
                    q_str
                ));
                std::process::exit(1);
            }
        }
    } else {
        RecordingQuality::professional()
    };

    // Parse boolean flags
    let mic_muted = has_flag(args, "--mic-muted");
    let trim_silence = !has_flag(args, "--no-trim"); // On by default
    let normalize = !has_flag(args, "--no-normalize"); // On by default
    let limiter = !has_flag(args, "--no-limiter"); // On by default
    let denoise = !has_flag(args, "--no-denoise"); // On by default
    let noise_gate = !has_flag(args, "--no-noise-gate"); // On by default

    // Parse --input-device / -id (optional device index for microphone)
    let input_device_index = if let Some(idx_str) = parse_flag_value(args, "--input-device", Some("-id")) {
        match idx_str.parse::<u32>() {
            Ok(idx) => Some(idx),
            Err(_) => {
                output.error(&format!(
                    "Invalid input device index '{}'. Must be a non-negative integer. Run 'audio-rec devices' to see available devices.",
                    idx_str
                ));
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    // Parse --output-device / -od (optional device index for loopback/output)
    let output_device_index = if let Some(idx_str) = parse_flag_value(args, "--output-device", Some("-od")) {
        match idx_str.parse::<u32>() {
            Ok(idx) => Some(idx),
            Err(_) => {
                output.error(&format!(
                    "Invalid output device index '{}'. Must be a non-negative integer. Run 'audio-rec devices' to see available devices.",
                    idx_str
                ));
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    // Parse --target-loudness (default: -16.0 LUFS)
    let target_loudness = if let Some(val_str) = parse_flag_value(args, "--target-loudness", None) {
        match val_str.parse::<f64>() {
            Ok(val) => val,
            Err(_) => {
                output.error(&format!("Invalid target loudness '{}'. Must be a number (e.g., -16).", val_str));
                std::process::exit(1);
            }
        }
    } else {
        -16.0
    };

    // Parse --gate-threshold (default: 0.5)
    let gate_threshold = if let Some(val_str) = parse_flag_value(args, "--gate-threshold", None) {
        match val_str.parse::<f64>() {
            Ok(val) => val,
            Err(_) => {
                output.error(&format!("Invalid gate threshold '{}'. Must be a number.", val_str));
                std::process::exit(1);
            }
        }
    } else {
        0.5
    };

    // Parse --app and --app-window (process-specific capture)
    let app = parse_flag_value(args, "--app", None);
    let app_window = parse_flag_value(args, "--app-window", None);

    // Mutual exclusivity: --app and --app-window cannot both be specified
    if app.is_some() && app_window.is_some() {
        output.error("Cannot specify both --app and --app-window. Use --app for process name (e.g., Zoom.exe) or --app-window for window title (e.g., \"Zoom Meeting\"), but not both.");
        std::process::exit(1);
    }

    let options = RecordOptions {
        duration,
        audio_format,
        quality,
        output_path,
        mic_muted,
        trim_silence,
        input_device_index,
        output_device_index,
        normalize,
        limiter,
        denoise,
        noise_gate,
        target_loudness,
        gate_threshold,
        app,
        app_window,
    };

    commands::record::execute_with_output(options, config, output.clone()).await
}

async fn handle_stop_command(args: &[String], _output: &UserOutput) -> Result<()> {
    let config = RecorderConfig::new();
    let session_id = if args.len() > 2 && !args[2].starts_with("--") {
        Some(args[2].clone())
    } else {
        None
    };

    commands::stop::execute(session_id, config).await
}

async fn handle_mute_command(args: &[String], _output: &UserOutput) -> Result<()> {
    let config = RecorderConfig::new();
    let session_id = if args.len() > 2 && !args[2].starts_with("--") {
        Some(args[2].clone())
    } else {
        None
    };

    commands::mute::execute(session_id, config).await
}

fn print_usage() {
    println!("============================================================");
    println!("Audio Recorder Manager");
    println!("============================================================");
    println!();
    println!("Records system audio and microphone on Windows via WASAPI.");
    println!("Designed to be spawned by other tools.");
    println!();
    println!("COMMANDS:");
    println!("  record    Start recording audio");
    println!("  devices   List all available audio input and output devices");
    println!("  stop      Stop an active recording");
    println!("  mute      Toggle microphone mute on an active recording");
    println!("  status    Show active recording session info");
    println!();
    println!("RECORD OPTIONS:");
    println!("  -o, --output <path>       Output file path (default: cwd/auto-name.wav)");
    println!("  -d, --duration <secs>     Recording duration in seconds (default: indefinite)");
    println!("  -f, --format <wav|m4a>    Audio format (default: wav, or inferred from --output)");
    println!("  -q, --quality <level>     Quality: quick, standard, professional, high (default: professional)");
    println!("  -id, --input-device <idx>  Input device index (microphone). Use 'devices' to list.");
    println!("  -od, --output-device <idx> Output device index (loopback). Use 'devices' to list.");
    println!("      --mic-muted           Start with microphone muted");
    println!("      --no-trim             Disable silence trimming (trimming is on by default)");
    println!();
    println!("STOP / MUTE OPTIONS:");
    println!("  [session_id]              Target a specific session (default: latest active)");
    println!();
    println!("GLOBAL OPTIONS:");
    println!("  -l, --log-level <level>   Log level: trace, debug, info, warn, error");
    println!("      --no-log-file         Disable file logging");
    println!("      --no-log-terminal     Disable terminal logging");
    println!();
    println!("EXAMPLES:");
    println!("  audio-rec record");
    println!("  audio-rec record -d 30");
    println!("  audio-rec record -o C:\\recordings\\meeting.wav");
    println!("  audio-rec record -o meeting.m4a -d 60 --mic-muted");
    println!("  audio-rec record --no-trim -q standard");
    println!("  audio-rec record -id 0 -od 1        # Use specific input/output devices");
    println!("  audio-rec record --input-device 0    # Use specific microphone");
    println!("  audio-rec devices                    # List available audio devices");
    println!("  audio-rec stop");
    println!("  audio-rec mute");
    println!("  audio-rec status");
    println!();
    println!("JSON PROTOCOL (stdout):");
    println!("  All output is JSON, one object per line (events emitted only on state changes):");
    println!("  {{\"type\": \"started\", \"session_id\": \"...\", \"file_path\": \"...\"}}");
    println!("  {{\"type\": \"audio_state\", \"loopback_has_audio\": true, \"mic_has_audio\": false, \"is_mic_muted\": false,");
    println!("   \"loopback_device_lost\": false, \"mic_device_lost\": false, \"device_recoveries\": 0}}");
    println!("  {{\"type\": \"warning\", \"code\": \"device_lost\" | \"device_recovered\" | \"stream_failed\" | \"process_exited\", \"message\": \"...\"}}");
    println!("  {{\"type\": \"muted\", \"is_muted\": true}}");
    println!("  {{\"type\": \"completed\", \"file_path\": \"...\", \"file_size_mb\": \"2.34\", \"warnings\": [], \"device_recoveries\": 0}}");
    println!("  {{\"type\": \"error\", \"message\": \"...\"}}");
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_flag_value tests ──

    #[test]
    fn test_parse_flag_value_long_flag() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "--input-device".into(),
            "0".into(),
        ];
        assert_eq!(
            parse_flag_value(&args, "--input-device", Some("-id")),
            Some("0".to_string())
        );
    }

    #[test]
    fn test_parse_flag_value_short_flag() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "-id".into(),
            "2".into(),
        ];
        assert_eq!(
            parse_flag_value(&args, "--input-device", Some("-id")),
            Some("2".to_string())
        );
    }

    #[test]
    fn test_parse_flag_value_output_device_short() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "-od".into(),
            "1".into(),
        ];
        assert_eq!(
            parse_flag_value(&args, "--output-device", Some("-od")),
            Some("1".to_string())
        );
    }

    #[test]
    fn test_parse_flag_value_missing_returns_none() {
        let args: Vec<String> = vec!["audio-rec".into(), "record".into()];
        assert_eq!(
            parse_flag_value(&args, "--input-device", Some("-id")),
            None
        );
    }

    #[test]
    fn test_parse_flag_value_no_conflict_between_o_and_od() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "-o".into(),
            "output.wav".into(),
            "-od".into(),
            "1".into(),
        ];
        // -o should match --output, not --output-device
        assert_eq!(
            parse_flag_value(&args, "--output", Some("-o")),
            Some("output.wav".to_string())
        );
        // -od should match --output-device
        assert_eq!(
            parse_flag_value(&args, "--output-device", Some("-od")),
            Some("1".to_string())
        );
    }

    #[test]
    fn test_parse_flag_value_both_device_flags() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "-id".into(),
            "0".into(),
            "-od".into(),
            "1".into(),
        ];
        assert_eq!(
            parse_flag_value(&args, "--input-device", Some("-id")),
            Some("0".to_string())
        );
        assert_eq!(
            parse_flag_value(&args, "--output-device", Some("-od")),
            Some("1".to_string())
        );
    }

    // ── RecordOptions construction tests ──

    /// Helper to create default RecordOptions for tests
    fn default_options() -> RecordOptions {
        RecordOptions {
            duration: RecordingDuration::Indefinite,
            audio_format: AudioFormat::Wav,
            quality: RecordingQuality::professional(),
            output_path: None,
            mic_muted: false,
            trim_silence: true,
            input_device_index: None,
            output_device_index: None,
            normalize: true,
            limiter: true,
            denoise: true,
            noise_gate: true,
            target_loudness: -16.0,
            gate_threshold: 0.5,
            app: None,
            app_window: None,
        }
    }

    #[test]
    fn test_record_options_default_device_indices_are_none() {
        let options = default_options();
        assert!(options.input_device_index.is_none());
        assert!(options.output_device_index.is_none());
    }

    #[test]
    fn test_record_options_with_device_indices() {
        let mut options = default_options();
        options.input_device_index = Some(0);
        options.output_device_index = Some(1);
        assert_eq!(options.input_device_index, Some(0));
        assert_eq!(options.output_device_index, Some(1));
    }

    #[test]
    fn test_record_options_quality_features_on_by_default() {
        let options = default_options();
        assert!(options.normalize);
        assert!(options.limiter);
        assert!(options.denoise);
        assert!(options.noise_gate);
    }

    #[test]
    fn test_record_options_default_target_loudness() {
        let options = default_options();
        assert_eq!(options.target_loudness, -16.0);
    }

    #[test]
    fn test_record_options_default_gate_threshold() {
        let options = default_options();
        assert_eq!(options.gate_threshold, 0.5);
    }

    #[test]
    fn test_record_options_app_defaults_to_none() {
        let options = default_options();
        assert!(options.app.is_none());
        assert!(options.app_window.is_none());
    }

    // ── has_flag tests ──

    #[test]
    fn test_has_flag_present() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "--mic-muted".into(),
        ];
        assert!(has_flag(&args, "--mic-muted"));
    }

    #[test]
    fn test_has_flag_absent() {
        let args: Vec<String> = vec!["audio-rec".into(), "record".into()];
        assert!(!has_flag(&args, "--mic-muted"));
    }

    // ── Quality feature flag tests ──

    #[test]
    fn test_no_normalize_flag_detected() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "--no-normalize".into(),
        ];
        assert!(has_flag(&args, "--no-normalize"));
    }

    #[test]
    fn test_no_limiter_flag_detected() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "--no-limiter".into(),
        ];
        assert!(has_flag(&args, "--no-limiter"));
    }

    #[test]
    fn test_no_denoise_flag_detected() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "--no-denoise".into(),
        ];
        assert!(has_flag(&args, "--no-denoise"));
    }

    #[test]
    fn test_no_noise_gate_flag_detected() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "--no-noise-gate".into(),
        ];
        assert!(has_flag(&args, "--no-noise-gate"));
    }

    #[test]
    fn test_target_loudness_flag_parsed() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "--target-loudness".into(),
            "-23".into(),
        ];
        assert_eq!(
            parse_flag_value(&args, "--target-loudness", None),
            Some("-23".to_string())
        );
    }

    #[test]
    fn test_gate_threshold_flag_parsed() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "--gate-threshold".into(),
            "0.7".into(),
        ];
        assert_eq!(
            parse_flag_value(&args, "--gate-threshold", None),
            Some("0.7".to_string())
        );
    }

    #[test]
    fn test_app_flag_parsed() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "--app".into(),
            "Zoom.exe".into(),
        ];
        assert_eq!(
            parse_flag_value(&args, "--app", None),
            Some("Zoom.exe".to_string())
        );
    }

    #[test]
    fn test_app_window_flag_parsed() {
        let args: Vec<String> = vec![
            "audio-rec".into(),
            "record".into(),
            "--app-window".into(),
            "Zoom Meeting".into(),
        ];
        assert_eq!(
            parse_flag_value(&args, "--app-window", None),
            Some("Zoom Meeting".to_string())
        );
    }
}
