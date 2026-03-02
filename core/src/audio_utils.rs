//! Audio processing utilities shared across recording modules

/// Calculate RMS (Root Mean Square) for i16 samples
pub fn calculate_rms_i16(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// Calculate RMS (Root Mean Square) for i32 samples
pub fn calculate_rms_i32(samples: &[i32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// Calculate RMS (Root Mean Square) for f32 samples
pub fn calculate_rms_f32(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rms_i16_silence() {
        let samples = vec![0i16; 100];
        assert_eq!(calculate_rms_i16(&samples), 0.0);
    }

    #[test]
    fn test_rms_i16_signal() {
        let samples = vec![1000i16, -1000, 1000, -1000];
        let rms = calculate_rms_i16(&samples);
        assert!((rms - 1000.0).abs() < 0.01);
    }

    #[test]
    fn test_rms_i16_empty() {
        let samples: Vec<i16> = vec![];
        assert_eq!(calculate_rms_i16(&samples), 0.0);
    }

    #[test]
    fn test_rms_i32_silence() {
        let samples = vec![0i32; 100];
        assert_eq!(calculate_rms_i32(&samples), 0.0);
    }

    #[test]
    fn test_rms_i32_signal() {
        let samples = vec![50000i32, -50000, 50000, -50000];
        let rms = calculate_rms_i32(&samples);
        assert!((rms - 50000.0).abs() < 0.01);
    }

    #[test]
    fn test_rms_i32_empty() {
        let samples: Vec<i32> = vec![];
        assert_eq!(calculate_rms_i32(&samples), 0.0);
    }

    #[test]
    fn test_rms_f32_silence() {
        let samples = vec![0.0f32; 100];
        assert_eq!(calculate_rms_f32(&samples), 0.0);
    }

    #[test]
    fn test_rms_f32_signal() {
        let samples = vec![0.5f32, -0.5, 0.5, -0.5];
        let rms = calculate_rms_f32(&samples);
        assert!((rms - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_rms_f32_empty() {
        let samples: Vec<f32> = vec![];
        assert_eq!(calculate_rms_f32(&samples), 0.0);
    }
}
