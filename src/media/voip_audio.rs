//! PCM-мост cpal (AAudio/CoreAudio/...) ↔ libtgvoip: 48 кГц, моно, i16.
//! Поток владеет `cpal::Stream` (не Send) и живёт до `AudioGuard::drop`.

use super::voip_native::AudioShared;
use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct AudioGuard {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for AudioGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

/// Запустить микрофон и динамик; ошибка — если не удалось открыть устройства.
pub fn spawn(shared: Arc<AudioShared>) -> Result<AudioGuard> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let join = std::thread::Builder::new()
        .name("tc-voip-audio".into())
        .spawn(move || run(shared, stop_thread))
        .context("audio thread")?;
    Ok(AudioGuard {
        stop,
        join: Some(join),
    })
}

fn run(shared: Arc<AudioShared>, stop: Arc<AtomicBool>) {
    if let Err(e) = run_inner(&shared, &stop) {
        eprintln!("voip audio: {e:#}");
        // устройств нет — libtgvoip продолжит без звука
    }
}

fn run_inner(shared: &Arc<AudioShared>, stop: &Arc<AtomicBool>) -> Result<()> {
    let host = cpal::default_host();
    let input = host.default_input_device().context("нет микрофона")?;
    let output = host.default_output_device().context("нет динамика")?;

    let in_cfg = pick_config(&input, true)?;
    let out_cfg = pick_config(&output, false)?;

    let in_stream = build_input(&input, &in_cfg, shared.clone())?;
    let out_stream = build_output(&output, &out_cfg, shared.clone())?;
    in_stream.play().context("старт микрофона")?;
    out_stream.play().context("старт динамика")?;

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    drop(in_stream);
    drop(out_stream);
    Ok(())
}

/// Пытаемся 48 кГц/моно; иначе — дефолтная конфигурация устройства.
fn pick_config(device: &cpal::Device, input: bool) -> Result<cpal::StreamConfig> {
    let wanted = cpal::StreamConfig {
        channels: 1,
        sample_rate: 48_000u32,
        buffer_size: cpal::BufferSize::Default,
    };
    let ranges: Vec<(u32, u32)> = if input {
        device
            .supported_input_configs()
            .map(|it| {
                it.map(|r| (r.min_sample_rate(), r.max_sample_rate()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        device
            .supported_output_configs()
            .map(|it| {
                it.map(|r| (r.min_sample_rate(), r.max_sample_rate()))
                    .collect()
            })
            .unwrap_or_default()
    };
    if ranges
        .iter()
        .any(|(min, max)| *min <= 48_000 && *max >= 48_000)
    {
        return Ok(wanted);
    }
    let def = if input {
        device.default_input_config()
    } else {
        device.default_output_config()
    }
    .context("конфигурация аудио")?;
    Ok(def.config())
}

fn build_input(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    shared: Arc<AudioShared>,
) -> Result<cpal::Stream> {
    let channels = cfg.channels as usize;
    let err_fn = |e| eprintln!("voip mic: {e}");
    let stream = match device.default_input_config()?.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            cfg.clone(),
            move |data: &[f32], _| {
                let mut mono = Vec::with_capacity(data.len() / channels.max(1));
                for frame in data.chunks(channels.max(1)) {
                    let v: f32 = frame.iter().sum::<f32>() / channels.max(1) as f32;
                    mono.push((v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                }
                shared.output_mic(&mono);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            cfg.clone(),
            move |data: &[i16], _| {
                let mut mono = Vec::with_capacity(data.len() / channels.max(1));
                for frame in data.chunks(channels.max(1)) {
                    mono.push(frame.iter().copied().sum::<i16>() / channels.max(1) as i16);
                }
                shared.output_mic(&mono);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_input_stream(
            cfg.clone(),
            move |data: &[u16], _| {
                let mut mono = Vec::with_capacity(data.len() / channels.max(1));
                for frame in data.chunks(channels.max(1)) {
                    let v = frame.iter().map(|x| *x as i32 - 32768).sum::<i32>()
                        / channels.max(1) as i32;
                    mono.push(v.clamp(-32768, 32767) as i16);
                }
                shared.output_mic(&mono);
            },
            err_fn,
            None,
        )?,
        other => anyhow::bail!("неподдерживаемый формат микрофона: {other:?}"),
    };
    Ok(stream)
}

fn build_output(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    shared: Arc<AudioShared>,
) -> Result<cpal::Stream> {
    let channels = cfg.channels as usize;
    let err_fn = |e| eprintln!("voip spk: {e}");
    let stream = match device.default_output_config()?.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            cfg.clone(),
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(channels.max(1)) {
                    let s = shared.next_spk() as f32 / i16::MAX as f32;
                    for v in frame.iter_mut() {
                        *v = s;
                    }
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream(
            cfg.clone(),
            move |data: &mut [i16], _| {
                for frame in data.chunks_mut(channels.max(1)) {
                    let s = shared.next_spk();
                    for v in frame.iter_mut() {
                        *v = s;
                    }
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_output_stream(
            cfg.clone(),
            move |data: &mut [u16], _| {
                for frame in data.chunks_mut(channels.max(1)) {
                    let s = (shared.next_spk() as i32 + 32768).clamp(0, 65535) as u16;
                    for v in frame.iter_mut() {
                        *v = s;
                    }
                }
            },
            err_fn,
            None,
        )?,
        other => anyhow::bail!("неподдерживаемый формат динамика: {other:?}"),
    };
    Ok(stream)
}
