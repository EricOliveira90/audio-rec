//! Post-processing pipeline for audio recordings.
//!
//! Refactors the monolithic `merge_audio_streams_smart()` into discrete stages:
//! Merge → Normalize → Limiter → Trim → Finalize
//!
//! Each stage can be independently enabled/disabled via CLI flags.
//! The pipeline emits JSON processing events for each active stage.

use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::AudioFormat;
use crate::recorder::RecordingQuality;
use crate::status::JsonFileObserver;

/// Configuration for which pipeline stages are enabled
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub normalize: bool,
    pub limiter: bool,
    pub trim: bool,
    pub target_loudness: f64,
    pub output_format: AudioFormat,
    pub quality: RecordingQuality,
}

impl PipelineConfig {
    /// Create a default config with all stages enabled
    pub fn default_config(output_format: AudioFormat, quality: RecordingQuality) -> Self {
        Self {
            normalize: true,
            limiter: true,
            trim: true,
            target_loudness: -16.0,
            output_format,
            quality,
        }
    }

    /// Count the total number of active stages (for progress reporting)
    pub fn active_stage_count(&self) -> u8 {
        let mut count = 2u8; // Analyze + Merge are always active
        if self.normalize { count += 1; }
        if self.limiter { count += 1; }
        if self.trim { count += 1; }
        count += 1; // Finalize is always active
        // For M4A, encoding is a separate visible stage
        if matches!(self.output_format, AudioFormat::M4a) {
            count += 1;
        }
        count
    }

    /// Get the ordered list of active stage names
    pub fn active_stages(&self) -> Vec<&'static str> {
        let mut stages = vec!["analyzing", "merging"];
        if matches!(self.output_format, AudioFormat::M4a) {
            stages.push("encoding");
        }
        if self.normalize {
            stages.push("normalizing");
        }
        if self.limiter {
            stages.push("limiting");
        }
        if self.trim {
            stages.push("trimming");
        }
        stages.push("finalizing");
        stages
    }
}

/// Callback for emitting pipeline progress events
pub type EventEmitter = Box<dyn Fn(&str, &str) + Send + Sync>;

/// Post-processing pipeline that processes recorded audio through discrete stages.
pub struct PostProcessingPipeline {
    config: PipelineConfig,
    session_id: String,
    loopback_wav: PathBuf,
    mic_wav: PathBuf,
    output_path: PathBuf,
    loopback_has_audio: bool,
    mic_has_audio: bool,
    observer: Option<Arc<JsonFileObserver>>,
    event_emitter: Option<EventEmitter>,
}

impl PostProcessingPipeline {
    pub fn new(
        config: PipelineConfig,
        session_id: String,
        loopback_wav: PathBuf,
        mic_wav: PathBuf,
        output_path: PathBuf,
        loopback_has_audio: bool,
        mic_has_audio: bool,
    ) -> Self {
        Self {
            config,
            session_id,
            loopback_wav,
            mic_wav,
            output_path,
            loopback_has_audio,
            mic_has_audio,
            observer: None,
            event_emitter: None,
        }
    }

    /// Set the status observer for writing progress to status files
    pub fn with_observer(mut self, observer: Arc<JsonFileObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Set the event emitter for JSON stdout events
    pub fn with_event_emitter(mut self, emitter: EventEmitter) -> Self {
        self.event_emitter = Some(emitter);
        self
    }

    /// Get the pipeline configuration
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    /// Get the total number of stages
    pub fn total_stages(&self) -> u8 {
        self.config.active_stage_count()
    }

    /// Get the ordered list of active stage names
    pub fn active_stages(&self) -> Vec<&'static str> {
        self.config.active_stages()
    }

    /// Emit a processing event for the current stage
    fn emit_stage_event(&self, stage: &str, message: &str, stage_num: u8) {
        // Write to status file
        if let Some(ref observer) = self.observer {
            let _ = observer.write_processing_status_v2(
                &self.session_id,
                message,
                Some(stage_num),
                Some(self.total_stages()),
                Some(stage),
                None,
                None,
            );
        }

        // Emit JSON event
        if let Some(ref emitter) = self.event_emitter {
            emitter(stage, message);
        }

        tracing::info!("Stage {}/{}: {} - {}", stage_num, self.total_stages(), stage, message);
    }

    /// Run the full pipeline: Merge → Normalize → Limiter → Trim → Finalize
    pub async fn run(&self, recordings_dir: &PathBuf) -> anyhow::Result<()> {
        let _total = self.total_stages();
        let mut current_stage = 1u8;

        // Stage: Analyze
        self.emit_stage_event("analyzing", "Analyzing audio streams", current_stage);
        current_stage += 1;

        // Stage: Merge
        self.emit_stage_event("merging", "Merging audio streams", current_stage);
        let merge_output = self.merge(recordings_dir).await?;
        current_stage += 1;

        // Stage: Encoding (M4A only — already done during merge for M4A)
        if matches!(self.config.output_format, AudioFormat::M4a) {
            // Encoding happens as part of merge for M4A
            current_stage += 1;
        }

        // Stage: Normalize (if enabled)
        let mut working_file = merge_output;
        if self.config.normalize {
            self.emit_stage_event("normalizing", "Normalizing loudness", current_stage);
            working_file = self.normalize(&working_file).await?;
            current_stage += 1;
        }

        // Stage: Limiter (if enabled)
        if self.config.limiter {
            self.emit_stage_event("limiting", "Applying limiter", current_stage);
            working_file = self.limit(&working_file).await?;
            current_stage += 1;
        }

        // Stage: Trim (if enabled)
        if self.config.trim {
            self.emit_stage_event("trimming", "Trimming silence", current_stage);
            working_file = self.trim(&working_file).await?;
            current_stage += 1;
        }

        // Stage: Finalize
        self.emit_stage_event("finalizing", "Saving recording", current_stage);
        self.finalize(&working_file).await?;

        Ok(())
    }

    /// Merge stage: combine loopback and mic WAV files
    async fn merge(&self, recordings_dir: &PathBuf) -> anyhow::Result<PathBuf> {
        let merge_output = if matches!(self.config.output_format, AudioFormat::M4a) {
            recordings_dir.join(format!("{}_merged.m4a", self.session_id))
        } else {
            recordings_dir.join(format!("{}_merged.wav", self.session_id))
        };

        crate::recorder::merge_audio_streams_smart(
            &self.loopback_wav,
            &self.mic_wav,
            &merge_output,
            self.loopback_has_audio,
            self.mic_has_audio,
            &self.config.quality,
            self.config.output_format,
            Some(&self.session_id),
            self.observer.clone(),
            self.total_stages(),
        )
        .await?;

        Ok(merge_output)
    }

    /// Normalize stage: EBU R128 loudness normalization via FFmpeg
    async fn normalize(&self, input: &PathBuf) -> anyhow::Result<PathBuf> {
        // Placeholder — will be implemented in Issue #12
        // For now, pass through unchanged
        tracing::debug!("Normalize stage: target {} LUFS (placeholder)", self.config.target_loudness);
        Ok(input.clone())
    }

    /// Limiter stage: soft audio limiter via FFmpeg alimiter filter
    async fn limit(&self, input: &PathBuf) -> anyhow::Result<PathBuf> {
        // Placeholder — will be implemented in Issue #13
        // For now, pass through unchanged
        tracing::debug!("Limiter stage (placeholder)");
        Ok(input.clone())
    }

    /// Trim stage: remove silence from beginning/end
    async fn trim(&self, input: &PathBuf) -> anyhow::Result<PathBuf> {
        let _ = crate::ffmpeg_encoder::trim_silence(input, -50.0, 2.0).await;
        Ok(input.clone())
    }

    /// Finalize stage: copy to final output location
    async fn finalize(&self, working_file: &PathBuf) -> anyhow::Result<()> {
        if let Some(parent) = self.output_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        // Copy then remove (works across drives)
        std::fs::copy(working_file, &self.output_path)?;
        let _ = std::fs::remove_file(working_file);

        tracing::info!("Final file saved to: {:?}", self.output_path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(format: AudioFormat) -> PipelineConfig {
        PipelineConfig {
            normalize: true,
            limiter: true,
            trim: true,
            target_loudness: -16.0,
            output_format: format,
            quality: RecordingQuality::professional(),
        }
    }

    // ── PipelineConfig tests ──

    #[test]
    fn pipeline_config_all_stages_enabled_wav_count() {
        let config = test_config(AudioFormat::Wav);
        // analyze + merge + normalize + limiter + trim + finalize = 6
        assert_eq!(config.active_stage_count(), 6);
    }

    #[test]
    fn pipeline_config_all_stages_enabled_m4a_count() {
        let config = test_config(AudioFormat::M4a);
        // analyze + merge + encoding + normalize + limiter + trim + finalize = 7
        assert_eq!(config.active_stage_count(), 7);
    }

    #[test]
    fn pipeline_config_no_normalize_reduces_count() {
        let mut config = test_config(AudioFormat::Wav);
        config.normalize = false;
        assert_eq!(config.active_stage_count(), 5);
    }

    #[test]
    fn pipeline_config_no_limiter_reduces_count() {
        let mut config = test_config(AudioFormat::Wav);
        config.limiter = false;
        assert_eq!(config.active_stage_count(), 5);
    }

    #[test]
    fn pipeline_config_no_trim_reduces_count() {
        let mut config = test_config(AudioFormat::Wav);
        config.trim = false;
        assert_eq!(config.active_stage_count(), 5);
    }

    #[test]
    fn pipeline_config_all_disabled_minimum_count() {
        let mut config = test_config(AudioFormat::Wav);
        config.normalize = false;
        config.limiter = false;
        config.trim = false;
        // analyze + merge + finalize = 3
        assert_eq!(config.active_stage_count(), 3);
    }

    #[test]
    fn pipeline_config_active_stages_wav_all_enabled() {
        let config = test_config(AudioFormat::Wav);
        let stages = config.active_stages();
        assert_eq!(stages, vec!["analyzing", "merging", "normalizing", "limiting", "trimming", "finalizing"]);
    }

    #[test]
    fn pipeline_config_active_stages_m4a_all_enabled() {
        let config = test_config(AudioFormat::M4a);
        let stages = config.active_stages();
        assert_eq!(stages, vec!["analyzing", "merging", "encoding", "normalizing", "limiting", "trimming", "finalizing"]);
    }

    #[test]
    fn pipeline_config_active_stages_with_disabled() {
        let mut config = test_config(AudioFormat::Wav);
        config.normalize = false;
        config.limiter = false;
        let stages = config.active_stages();
        assert_eq!(stages, vec!["analyzing", "merging", "trimming", "finalizing"]);
    }

    #[test]
    fn pipeline_config_default_target_loudness() {
        let config = PipelineConfig::default_config(AudioFormat::Wav, RecordingQuality::professional());
        assert_eq!(config.target_loudness, -16.0);
        assert!(config.normalize);
        assert!(config.limiter);
        assert!(config.trim);
    }

    // ── PostProcessingPipeline construction tests ──

    #[test]
    fn pipeline_construction_sets_total_stages() {
        let config = test_config(AudioFormat::Wav);
        let pipeline = PostProcessingPipeline::new(
            config,
            "test-session".to_string(),
            PathBuf::from("/tmp/loopback.wav"),
            PathBuf::from("/tmp/mic.wav"),
            PathBuf::from("/tmp/output.wav"),
            true,
            true,
        );
        assert_eq!(pipeline.total_stages(), 6);
    }

    #[test]
    fn pipeline_active_stages_matches_config() {
        let mut config = test_config(AudioFormat::Wav);
        config.normalize = false;
        let pipeline = PostProcessingPipeline::new(
            config,
            "test-session".to_string(),
            PathBuf::from("/tmp/loopback.wav"),
            PathBuf::from("/tmp/mic.wav"),
            PathBuf::from("/tmp/output.wav"),
            true,
            false,
        );
        let stages = pipeline.active_stages();
        assert!(!stages.contains(&"normalizing"));
        assert!(stages.contains(&"merging"));
        assert!(stages.contains(&"finalizing"));
    }

    #[test]
    fn pipeline_event_emitter_receives_events() {
        use std::sync::Mutex;

        let events: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let config = test_config(AudioFormat::Wav);
        let pipeline = PostProcessingPipeline::new(
            config,
            "test-session".to_string(),
            PathBuf::from("/tmp/loopback.wav"),
            PathBuf::from("/tmp/mic.wav"),
            PathBuf::from("/tmp/output.wav"),
            true,
            true,
        )
        .with_event_emitter(Box::new(move |stage, message| {
            events_clone.lock().unwrap().push((stage.to_string(), message.to_string()));
        }));

        // Manually call emit to test the emitter works
        pipeline.emit_stage_event("analyzing", "Test message", 1);

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, "analyzing");
        assert_eq!(captured[0].1, "Test message");
    }
}
