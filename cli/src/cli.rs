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

    let options = RecordOptions {
        duration,
        audio_format,
        quality,
        output_path,
        mic_muted,
        trim_silence,
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
    println!("  stop      Stop an active recording");
    println!("  mute      Toggle microphone mute on an active recording");
    println!("  status    Show system audio device info");
    println!();
    println!("RECORD OPTIONS:");
    println!("  -o, --output <path>       Output file path (default: cwd/auto-name.wav)");
    println!("  -d, --duration <secs>     Recording duration in seconds (default: indefinite)");
    println!("  -f, --format <wav|m4a>    Audio format (default: wav, or inferred from --output)");
    println!("  -q, --quality <level>     Quality: quick, standard, professional, high (default: professional)");
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
    println!("  audio-rec stop");
    println!("  audio-rec mute");
    println!();
    println!("JSON PROTOCOL (stdout):");
    println!("  All output is JSON, one object per line (events emitted only on state changes):");
    println!("  {{\"type\": \"started\", \"session_id\": \"...\", \"file_path\": \"...\"}}");
    println!("  {{\"type\": \"audio_state\", \"loopback_has_audio\": true, \"mic_has_audio\": false, \"is_mic_muted\": false}}");
    println!("  {{\"type\": \"muted\", \"is_muted\": true}}");
    println!("  {{\"type\": \"completed\", \"file_path\": \"...\", \"file_size_mb\": \"2.34\"}}");
    println!("  {{\"type\": \"error\", \"message\": \"...\"}}");
    println!();
}
