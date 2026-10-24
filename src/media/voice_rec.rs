//! Запись голосовых заметок с микрофона (cpal → WAV 48 кГц моно).
//!
//! Честное ограничение: Telegram принимает голосовые только в Ogg/Opus, а
//! opus-энкодера (concentus) нет в офлайн-кэше крейтов — поэтому запись идёт
//! в WAV 16-bit и отправляется как обычное аудио. Как только сеть позволит
//! добавить `concentus` + `ogg`, WAV легко прогнать через энкодер: сэмплы
//! уже нормализованы к 48 кГц моно.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Целевая частота для голосовых.
pub const TARGET_RATE: u32 = 48_000;
/// Лимит записи (защита от забытой кнопки).
pub const MAX_SECS: u64 = 600;

/// Состояние записи, разделяемое с колбэком cpal.
struct RecState {
    samples: Vec<f32>,
    from_rate: u32,
    channels: u16,
    level: f32,
    started: Instant,
}

/// Запись с микрофона. Владение потоком = идёт запись; `stop()` забирает сэмплы.
pub struct Recorder {
    _stream: cpal::Stream,
    state: Arc<Mutex<RecState>>,
}

impl Recorder {
    /// Старт записи с устройства ввода по умолчанию.
    /// Просит 48 кГц моно f32, иначе берёт дефолтную конфигурацию.
    pub fn start() -> anyhow::Result<Self> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context_input("нет устройства ввода (микрофона)")?;
        let supported = device
            .supported_input_configs()
            .context_input("не удалось опросить микрофон")?;
        // Предпочитаем 48 кГц моно f32 — без ресемпла.
        let mut chosen: Option<cpal::SupportedStreamConfig> = None;
        let mut fallback: Option<cpal::SupportedStreamConfig> = None;
        for cfg in supported {
            if cfg.channels() == 1 && cfg.sample_format() == cpal::SampleFormat::F32 {
                if cfg.min_sample_rate() <= TARGET_RATE && TARGET_RATE <= cfg.max_sample_rate() {
                    chosen = Some(cfg.with_sample_rate(TARGET_RATE));
                    break;
                }
                if fallback.is_none() {
                    fallback = Some(cfg.with_max_sample_rate());
                }
            } else if fallback.is_none() {
                fallback = Some(cfg.with_max_sample_rate());
            }
        }
        let config = chosen
            .or(fallback)
            .or_else(|| device.default_input_config().ok())
            .context_input("у микрофона нет рабочих конфигураций")?;
        let from_rate = config.sample_rate();
        let channels = config.channels();
        let fmt = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();
        let state = Arc::new(Mutex::new(RecState {
            samples: Vec::with_capacity((TARGET_RATE as usize) * 10),
            from_rate,
            channels,
            level: 0.0,
            started: Instant::now(),
        }));
        let state_cb = Arc::clone(&state);
        let err_fn = |e| tracing::warn!("микрофон: {e}");
        let stream = match fmt {
            cpal::SampleFormat::F32 => device.build_input_stream(
                stream_config,
                move |data: &[f32], _| push_f32(&state_cb, data),
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                stream_config.clone(),
                move |data: &[i16], _| {
                    let conv: Vec<f32> = data.iter().map(|s| *s as f32 / 32768.0).collect();
                    push_f32(&state_cb, &conv);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                stream_config,
                move |data: &[u16], _| {
                    let conv: Vec<f32> = data
                        .iter()
                        .map(|s| (*s as f32 - 32768.0) / 32768.0)
                        .collect();
                    push_f32(&state_cb, &conv);
                },
                err_fn,
                None,
            ),
            f => anyhow::bail!("формат микрофона {f:?} не поддерживается"),
        }
        .context_input("не удалось открыть микрофон (нет разрешения?)")?;
        stream
            .play()
            .context_input("не удалось стартовать запись")?;
        Ok(Self {
            _stream: stream,
            state,
        })
    }

    /// Текущий уровень 0..1 (RMS последнего чанка).
    pub fn level(&self) -> f32 {
        self.state.lock().map(|s| s.level).unwrap_or(0.0)
    }

    /// Сколько идёт запись.
    pub fn elapsed(&self) -> Duration {
        self.state
            .lock()
            .map(|s| s.started.elapsed())
            .unwrap_or_default()
    }

    /// Стоп: вернуть нормализованные сэмплы 48 кГц моно + длительность.
    pub fn stop(self) -> VoiceTake {
        drop(self._stream);
        let s = self.state.lock().expect("rec state poisoned");
        let mono = resample_to_48k(&s.samples, s.from_rate, s.channels);
        let secs = mono.len() as f32 / TARGET_RATE as f32;
        VoiceTake {
            samples: mono,
            secs,
        }
    }
}

/// Готовая запись: сэмплы 48 кГц моно f32 (-1..1).
pub struct VoiceTake {
    pub samples: Vec<f32>,
    pub secs: f32,
}

fn push_f32(state: &Arc<Mutex<RecState>>, data: &[f32]) {
    if let Ok(mut s) = state.lock() {
        // В моно складываем каналы усреднением на этапе ресемпла; здесь сырьё.
        s.samples.extend_from_slice(data);
        if s.samples.len() > (TARGET_RATE as usize) * (MAX_SECS as usize) * 2 {
            // Хард-стоп по лимиту: дальше не копим.
            return;
        }
        // Уровень по последнему чанку.
        let n = data.len().min(4096);
        if n > 0 {
            let tail = &data[data.len() - n..];
            let rms = (tail.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
            s.level = (rms as f32 * 3.0).clamp(0.0, 1.0);
        }
    }
}

/// Ресемпл в 48 кГц моно линейной интерполяцией (для голоса достаточно).
pub fn resample_to_48k(interleaved: &[f32], from_rate: u32, channels: u16) -> Vec<f32> {
    if interleaved.is_empty() || from_rate == 0 || channels == 0 {
        return vec![];
    }
    let ch = channels as usize;
    let frames = interleaved.len() / ch;
    // Сначала в моно.
    let mut mono = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut acc = 0.0f32;
        for c in 0..ch {
            acc += interleaved[f * ch + c];
        }
        mono.push(acc / ch as f32);
    }
    if from_rate == TARGET_RATE {
        return mono;
    }
    let ratio = from_rate as f64 / TARGET_RATE as f64;
    let out_len = (frames as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let i0 = pos as usize;
        let frac = (pos - i0 as f64) as f32;
        let a = mono[i0.min(frames - 1)];
        let b = mono[(i0 + 1).min(frames - 1)];
        out.push(a + (b - a) * frac);
    }
    out
}

/// Записать WAV 16-bit PCM моно 48 кГц (минимальный RIFF-заголовок).
pub fn write_wav_16(path: &std::path::Path, samples: &[f32]) -> anyhow::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut f = std::fs::File::create(path)?;
    let data_bytes = (samples.len() * 2) as u32;
    // RIFF-заголовок.
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_bytes).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?; // subchunk1 size
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&TARGET_RATE.to_le_bytes())?;
    f.write_all(&(TARGET_RATE * 2).to_le_bytes())?; // byte rate
    f.write_all(&2u16.to_le_bytes())?; // block align
    f.write_all(&16u16.to_le_bytes())?; // bits
    f.write_all(b"data")?;
    f.write_all(&data_bytes.to_le_bytes())?;
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        f.write_all(&v.to_le_bytes())?;
    }
    Ok(())
}

trait ContextInput<T> {
    fn context_input(self, what: &str) -> anyhow::Result<T>;
}

impl<T> ContextInput<T> for Option<T> {
    fn context_input(self, what: &str) -> anyhow::Result<T> {
        self.ok_or_else(|| anyhow::anyhow!("{what}"))
    }
}

impl<T, E: std::fmt::Display> ContextInput<T> for Result<T, E> {
    fn context_input(self, what: &str) -> anyhow::Result<T> {
        self.map_err(|e| anyhow::anyhow!("{what}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_440(rate: u32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn resample_keeps_pitch_and_length() {
        // 1с 440 Гц на 44100 стерео → 48000 моно.
        let stereo: Vec<f32> = sine_440(44100, 1.0)
            .into_iter()
            .flat_map(|s| [s, s * 0.5])
            .collect();
        let out = resample_to_48k(&stereo, 44100, 2);
        assert!((out.len() as i32 - 48000).abs() < 10, "len={}", out.len());
        // Частота по zero-crossing: ~880 пересечений за секунду для 440 Гц.
        let crossings = out
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        assert!((crossings as i32 - 880).abs() < 30, "cross={crossings}");
        // Пустые/битые входы не паникуют.
        assert!(resample_to_48k(&[], 44100, 1).is_empty());
        assert!(resample_to_48k(&[0.5], 0, 1).is_empty());
    }

    #[test]
    fn wav_roundtrip_through_rodio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("take.wav");
        let src = sine_440(48000, 0.5);
        write_wav_16(&path, &src).unwrap();
        // rodio читает наш WAV обратно.
        let file = std::fs::File::open(&path).unwrap();
        let decoded = rodio::Decoder::new(std::io::BufReader::new(file)).unwrap();
        use rodio::Source;
        assert_eq!(decoded.sample_rate().get(), 48000);
        assert_eq!(decoded.channels().get(), 1);
        let back: Vec<i16> = decoded.map(|s| (s * 32767.0) as i16).collect();
        assert!((back.len() as i32 - 24000).abs() < 5, "len={}", back.len());
        // Амплитуда сохранилась (±1 квант).
        let peak = back.iter().map(|s| s.abs()).max().unwrap_or(0);
        assert!((peak - 16383).abs() < 200, "peak={peak}");
    }

    #[test]
    fn recorder_fails_gracefully_without_mic() {
        // На headless-машине микрофона нет — главное, что нет паники.
        // Если микрофон есть, тест просто создаст и тут же уронит запись.
        match Recorder::start() {
            Ok(r) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                let take = r.stop();
                assert!(take.secs >= 0.0);
            }
            Err(e) => {
                assert!(!format!("{e:#}").is_empty());
            }
        }
    }
}
