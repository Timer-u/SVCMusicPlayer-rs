/// ReplayGain 响度引擎与软拐点防削波限制器
#[derive(Debug, Clone)]
pub struct LoudnessProcessor {
    pub target_lufs: f32,
    pub current_gain_db: f32,
    pub is_static_gain: bool,
    smooth_energy: f32,
}

impl LoudnessProcessor {
    /// 创建响度处理器。若传入 precomputed_gain_db（来自标签或静态全曲扫描），则锁定为静态增益模式；
    /// 否则启用滑动窗口动态 EBU R128 / Gated RMS 追踪模式。
    pub fn new(target_lufs: f32, precomputed_gain_db: Option<f32>) -> Self {
        match precomputed_gain_db {
            Some(gain) => Self {
                target_lufs,
                current_gain_db: gain.clamp(-12.0, 12.0),
                is_static_gain: true,
                smooth_energy: 0.0,
            },
            None => Self {
                target_lufs,
                current_gain_db: 0.0,
                is_static_gain: false,
                smooth_energy: 0.0,
            },
        }
    }

    /// 从整曲单声道 48kHz i16 样本切片预计算静态增益 (单位: dB，范围 clamp 到 -12.0..=12.0)
    pub fn analyze_static_gain_db(samples: &[i16], target_lufs: f32) -> Option<f32> {
        if samples.is_empty() {
            return None;
        }

        const CHUNK_SIZE: usize = 4800; // 100ms @ 48kHz
        let silence_threshold = 1e-5f32; // -50 dBFS

        let mut valid_blocks = Vec::new();

        for chunk in samples.chunks(CHUNK_SIZE) {
            if chunk.is_empty() {
                continue;
            }
            let sum_sq: f64 = chunk
                .iter()
                .map(|&s| {
                    let norm = s as f64 / 32768.0;
                    norm * norm
                })
                .sum();
            let energy = (sum_sq / chunk.len() as f64) as f32;
            if energy >= silence_threshold {
                valid_blocks.push(energy);
            }
        }

        if valid_blocks.is_empty() {
            return None;
        }

        let avg_energy: f32 = valid_blocks.iter().sum::<f32>() / (valid_blocks.len() as f32);
        if avg_energy <= 0.0 {
            return None;
        }

        let lufs_est = -0.691 + 10.0 * avg_energy.log10();
        let gain_db = (target_lufs - lufs_est).clamp(-12.0, 12.0);
        Some(gain_db)
    }

    /// 解析常见 ReplayGain 字符串标签（如 "-3.21 dB" 或 "+1.5 dB"）为 f32
    pub fn parse_replaygain_tag(tag_value: &str) -> Option<f32> {
        let trimmed = tag_value.trim();
        let clean = trimmed
            .trim_end_matches(|c: char| c.is_alphabetic() || c.is_whitespace())
            .trim();

        clean.parse::<f32>().ok().map(|v| v.clamp(-18.0, 18.0))
    }

    /// 软拐点峰值限制器纯函数（供单元测试直接验证）：
    /// |x| <= 0.85 时线性通过；|x| > 0.85 时通过 tanh 软饱和渐近限制在 ±0.98 以内。
    pub fn soft_clip_sample(x: f32) -> f32 {
        const T: f32 = 0.85;
        const M: f32 = 0.98;
        const R: f32 = M - T; // 0.13

        let abs_x = x.abs();
        if abs_x <= T {
            x
        } else {
            let sign = if x < 0.0 { -1.0 } else { 1.0 };
            sign * (T + R * ((abs_x - T) / R).tanh())
        }
    }

    /// 处理一帧 (如 960 samples @ 48kHz) 的 i16 PCM 采样，写出应用了 ReplayGain（若 enabled=true）、
    /// 用户音量 (user_volume_ratio: 0.0..=1.0) 以及软拐点防爆音限幅 (Soft-Knee Peak Limiter) 后的 f32 样本。
    /// 返回当前实际生效的 ReplayGain 增益值 (dB)。
    pub fn process_frame(
        &mut self,
        input_i16: &[i16],
        output_f32: &mut [f32],
        replay_gain_enabled: bool,
        user_volume_ratio: f32,
    ) -> f32 {
        let len = input_i16.len().min(output_f32.len());
        if len == 0 {
            return if replay_gain_enabled { self.current_gain_db } else { 0.0 };
        }

        if !replay_gain_enabled {
            for i in 0..len {
                let sample_norm = input_i16[i] as f32 / 32768.0;
                output_f32[i] = Self::soft_clip_sample(sample_norm * user_volume_ratio);
            }
            return 0.0;
        }

        // ReplayGain enabled
        if !self.is_static_gain {
            let sum_sq: f64 = input_i16[..len]
                .iter()
                .map(|&s| {
                    let norm = s as f64 / 32768.0;
                    norm * norm
                })
                .sum();
            let frame_energy = (sum_sq / len as f64) as f32;

            if frame_energy > 1e-5 {
                if self.smooth_energy <= 0.0 {
                    self.smooth_energy = frame_energy;
                } else {
                    self.smooth_energy = 0.96 * self.smooth_energy + 0.04 * frame_energy;
                }
                let target_db = (self.target_lufs - (-0.691 + 10.0 * self.smooth_energy.log10()))
                    .clamp(-12.0, 12.0);
                let diff = target_db - self.current_gain_db;
                self.current_gain_db += diff.clamp(-0.03, 0.03);
            }
        }

        let rg_linear = 10.0f32.powf(self.current_gain_db / 20.0);
        let total_gain = rg_linear * user_volume_ratio;

        for i in 0..len {
            let sample_norm = input_i16[i] as f32 / 32768.0;
            output_f32[i] = Self::soft_clip_sample(sample_norm * total_gain);
        }

        self.current_gain_db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_replaygain_tag() {
        assert_eq!(LoudnessProcessor::parse_replaygain_tag("-3.21 dB"), Some(-3.21));
        assert_eq!(LoudnessProcessor::parse_replaygain_tag("+1.5 db"), Some(1.5));
        assert_eq!(LoudnessProcessor::parse_replaygain_tag(" -14.2 LUFS "), Some(-14.2));
        assert_eq!(LoudnessProcessor::parse_replaygain_tag("+25.0 dB"), Some(18.0)); // clamped
        assert_eq!(LoudnessProcessor::parse_replaygain_tag("-30.0 dB"), Some(-18.0)); // clamped
        assert_eq!(LoudnessProcessor::parse_replaygain_tag("invalid"), None);
    }

    #[test]
    fn test_soft_clip_boundary() {
        // Linear region
        assert_eq!(LoudnessProcessor::soft_clip_sample(0.0), 0.0);
        assert_eq!(LoudnessProcessor::soft_clip_sample(0.5), 0.5);
        assert_eq!(LoudnessProcessor::soft_clip_sample(-0.85), -0.85);
        assert_eq!(LoudnessProcessor::soft_clip_sample(0.85), 0.85);

        // Saturation region: never exceeds 0.98
        let clipped_pos = LoudnessProcessor::soft_clip_sample(1.5);
        assert!(clipped_pos > 0.85 && clipped_pos <= 0.98);

        let clipped_neg = LoudnessProcessor::soft_clip_sample(-10.0);
        assert!(clipped_neg < -0.85 && clipped_neg >= -0.98);

        let clipped_huge = LoudnessProcessor::soft_clip_sample(100.0);
        assert!(clipped_huge <= 0.98 && clipped_huge > 0.979);
    }

    #[test]
    fn test_analyze_static_gain_silence_and_signal() {
        let silence = vec![0i16; 9600];
        assert_eq!(LoudnessProcessor::analyze_static_gain_db(&silence, -16.0), None);

        // Sine wave at about -6 dBFS
        let mut signal = Vec::with_capacity(9600);
        for i in 0..9600 {
            let sample = (16000.0 * (2.0 * std::f32::consts::PI * 440.0 * (i as f32 / 48000.0)).sin()) as i16;
            signal.push(sample);
        }
        let gain = LoudnessProcessor::analyze_static_gain_db(&signal, -16.0);
        assert!(gain.is_some());
        let g = gain.unwrap();
        assert!(g >= -12.0 && g <= 12.0);
    }

    #[test]
    fn test_process_frame_volume_and_gain() {
        let mut proc = LoudnessProcessor::new(-16.0, Some(-3.0));
        assert!(proc.is_static_gain);
        assert_eq!(proc.current_gain_db, -3.0);

        let input = vec![10000i16; 960];
        let mut output = vec![0.0f32; 960];

        // Disabled
        let applied_gain = proc.process_frame(&input, &mut output, false, 1.0);
        assert_eq!(applied_gain, 0.0);
        let expected_val = 10000.0 / 32768.0;
        assert!((output[0] - expected_val).abs() < 1e-4);

        // Enabled
        let applied_gain = proc.process_frame(&input, &mut output, true, 1.0);
        assert_eq!(applied_gain, -3.0);
        let linear_gain = 10.0f32.powf(-3.0 / 20.0);
        let expected_val = (10000.0 / 32768.0) * linear_gain;
        assert!((output[0] - expected_val).abs() < 1e-4);
    }
}
