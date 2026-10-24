//! Нативный VoIP-медиапоток: libtgvoip (Telegram) + аудио через cpal.
//!
//! Android: `libtgvoip.so` (см. `android/build-tgvoip.sh`) с
//! `TGVOIP_USE_CUSTOM_CRYPTO` (крипта ниже), `TGVOIP_USE_CALLBACK_AUDIO_IO`
//! (PCM 48 кГц моно из cpal/AAudio) и встроенным WebRTC DSP (AEC + AGC2).
//! Десктоп: модуль компилируется, но `NativeCall::start` возвращает ошибку —
//! бинарник libtgvoip собирается только под Android.
//!
//! Ключ: SHA-256 от DH auth_key звонка (`phone.*`), эндпойнты — из
//! `PhoneConnection`; обмен ключами идёт по MTProto, медиа — по UDP
//! (obfuscated-протокол libtgvoip).

use crate::telegram::CallConnection;
use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Состояния TgVoip (см. TgVoip.h) → числа моста.
pub const STATE_WAIT_INIT: i32 = 0;
pub const STATE_WAIT_INIT_ACK: i32 = 1;
pub const STATE_ESTABLISHED: i32 = 2;
pub const STATE_FAILED: i32 = 3;
pub const STATE_RECONNECTING: i32 = 4;

/// SHA-256(auth_key) — ключ звонка, как в клиентах Telegram.
pub fn call_key_from_auth_key(auth_key: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(auth_key);
    h.finalize().into()
}

// --- Крипто-callbacks для libtgvoip (TGVOIP_USE_CUSTOM_CRYPTO) ---------------

/// # Safety
/// Вызывается C++ (libtgvoip); указатели валидны на время вызова.
#[no_mangle]
pub unsafe extern "C" fn tc_crypto_rand_bytes(buffer: *mut u8, length: usize) {
    if buffer.is_null() {
        return;
    }
    let out = std::slice::from_raw_parts_mut(buffer, length);
    if getrandom::fill(out).is_err() {
        // fallback: время + счётчик (не криптостойко, но лучше нулей)
        let mut x = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e3779b97f4a7c15);
        for b in out.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
    }
}

/// # Safety
/// `msg`/`output` — валидные буферы C++.
#[no_mangle]
pub unsafe extern "C" fn tc_crypto_sha1(msg: *mut u8, length: usize, output: *mut u8) {
    use sha1::{Digest, Sha1};
    if msg.is_null() || output.is_null() {
        return;
    }
    let data = std::slice::from_raw_parts(msg, length);
    let digest = Sha1::digest(data);
    std::ptr::copy_nonoverlapping(digest.as_ptr(), output, 20);
}

/// # Safety
/// `msg`/`output` — валидные буферы C++.
#[no_mangle]
pub unsafe extern "C" fn tc_crypto_sha256(msg: *mut u8, length: usize, output: *mut u8) {
    use sha2::{Digest, Sha256};
    if msg.is_null() || output.is_null() {
        return;
    }
    let data = std::slice::from_raw_parts(msg, length);
    let digest = Sha256::digest(data);
    std::ptr::copy_nonoverlapping(digest.as_ptr(), output, 32);
}

/// # Safety
/// Буферы `in_/out/key/iv` — валидные, `key` 32 байта.
#[no_mangle]
pub unsafe extern "C" fn tc_crypto_ige_encrypt(
    input: *mut u8,
    output: *mut u8,
    length: usize,
    key: *mut u8,
    iv: *mut u8,
) {
    if input.is_null() || output.is_null() || key.is_null() || iv.is_null() || length % 16 != 0 {
        return;
    }
    let mut buf = std::slice::from_raw_parts(input, length).to_vec();
    let k: [u8; 32] = std::slice::from_raw_parts(key, 32).try_into().unwrap();
    let mut v: [u8; 32] = std::slice::from_raw_parts(iv, 32).try_into().unwrap();
    grammers_crypto::aes::ige_encrypt(&mut buf, &k, &mut v);
    std::ptr::copy_nonoverlapping(buf.as_ptr(), output, length);
}

/// # Safety
/// Буферы `in_/out/key/iv` — валидные, `key` 32 байта.
#[no_mangle]
pub unsafe extern "C" fn tc_crypto_ige_decrypt(
    input: *mut u8,
    output: *mut u8,
    length: usize,
    key: *mut u8,
    iv: *mut u8,
) {
    if input.is_null() || output.is_null() || key.is_null() || iv.is_null() || length % 16 != 0 {
        return;
    }
    let mut buf = std::slice::from_raw_parts(input, length).to_vec();
    let k: [u8; 32] = std::slice::from_raw_parts(key, 32).try_into().unwrap();
    let mut v: [u8; 32] = std::slice::from_raw_parts(iv, 32).try_into().unwrap();
    grammers_crypto::aes::ige_decrypt(&mut buf, &k, &mut v);
    std::ptr::copy_nonoverlapping(buf.as_ptr(), output, length);
}

/// AES-256-CTR в семантике OpenSSL `CRYPTO_ctr128_encrypt`:
/// состояние (`iv`, `ecount`, `num`) переносится между вызовами.
/// # Safety
/// Все указатели валидны; `key` 32 байта; `iv`/`ecount` 16 байт.
#[no_mangle]
pub unsafe extern "C" fn tc_crypto_aes_ctr(
    inout: *mut u8,
    length: usize,
    key: *mut u8,
    iv: *mut u8,
    ecount: *mut u8,
    num: *mut u32,
) {
    use aes::cipher::{BlockCipherEncrypt, KeyInit};
    if inout.is_null() || key.is_null() || iv.is_null() || ecount.is_null() || num.is_null() {
        return;
    }
    let buf = std::slice::from_raw_parts_mut(inout, length);
    let cipher = aes::Aes256::new_from_slice(std::slice::from_raw_parts(key, 32)).unwrap();
    let iv_s = std::slice::from_raw_parts_mut(iv, 16);
    let ec = std::slice::from_raw_parts_mut(ecount, 16);
    let used = &mut *num;

    let mut pos = 0usize;
    while pos < length {
        if *used == 0 {
            let mut block = aes::Block::default();
            block.copy_from_slice(ec);
            cipher.encrypt_block(&mut block);
            ec.copy_from_slice(&block);
            // ctr64_inc: big-endian ++ последних 8 байт
            let mut carry = 1u16;
            for i in (8..16).rev() {
                let v = iv_s[i] as u16 + carry;
                iv_s[i] = v as u8;
                carry = v >> 8;
                if carry == 0 {
                    break;
                }
            }
        }
        let n = (*used as usize).min(16);
        let take = (length - pos).min(16 - n);
        for i in 0..take {
            buf[pos + i] ^= ec[n + i];
        }
        pos += take;
        *used = (n + take) as u32;
        if *used == 16 {
            *used = 0;
        }
    }
}

// --- PCM мост cpal <-> libtgvoip -------------------------------------------

#[derive(Default)]
pub(crate) struct AudioShared {
    mic: Mutex<VecDeque<i16>>,
    spk: Mutex<VecDeque<i16>>,
    pub(crate) muted: AtomicBool,
}

impl AudioShared {
    pub(crate) fn input(&self, out: &mut [i16]) {
        let muted = self.muted.load(Ordering::Relaxed);
        let mut q = self.mic.lock().unwrap();
        for s in out.iter_mut() {
            *s = if muted { 0 } else { q.pop_front().unwrap_or(0) };
        }
    }

    pub(crate) fn output_mic(&self, samples: &[i16]) {
        let mut q = self.mic.lock().unwrap();
        if q.len() > 48_000 {
            q.clear();
        }
        q.extend(samples.iter().copied());
    }

    pub(crate) fn next_spk(&self) -> i16 {
        self.spk.lock().unwrap().pop_front().unwrap_or(0)
    }

    pub(crate) fn output(&self, samples: &[i16]) {
        let mut q = self.spk.lock().unwrap();
        if q.len() > 48_000 {
            q.clear();
        }
        q.extend(samples.iter().copied());
    }
}

extern "C" fn audio_input_cb(samples: *mut i16, count: usize, ctx: *mut std::ffi::c_void) {
    if samples.is_null() || ctx.is_null() {
        return;
    }
    let shared = unsafe { &*(ctx as *const AudioShared) };
    let out = unsafe { std::slice::from_raw_parts_mut(samples, count) };
    shared.input(out);
}

extern "C" fn audio_output_cb(samples: *mut i16, count: usize, ctx: *mut std::ffi::c_void) {
    if samples.is_null() || ctx.is_null() {
        return;
    }
    let shared = unsafe { &*(ctx as *const AudioShared) };
    let data = unsafe { std::slice::from_raw_parts(samples, count) };
    shared.output(data);
}

#[cfg(target_os = "android")]
mod ffi {
    use super::*;
    use std::ffi::c_void;

    pub type AudioCb = extern "C" fn(*mut i16, usize, *mut c_void);
    pub type StateCb = extern "C" fn(i32, *mut c_void);

    #[repr(C)]
    pub struct TcCrypto {
        pub rand_bytes: unsafe extern "C" fn(*mut u8, usize),
        pub sha1: unsafe extern "C" fn(*mut u8, usize, *mut u8),
        pub sha256: unsafe extern "C" fn(*mut u8, usize, *mut u8),
        pub aes_ige_encrypt: unsafe extern "C" fn(*mut u8, *mut u8, usize, *mut u8, *mut u8),
        pub aes_ige_decrypt: unsafe extern "C" fn(*mut u8, *mut u8, usize, *mut u8, *mut u8),
        pub aes_ctr_encrypt:
            unsafe extern "C" fn(*mut u8, usize, *mut u8, *mut u8, *mut u8, *mut u32),
    }

    pub const CRYPTO: TcCrypto = TcCrypto {
        rand_bytes: tc_crypto_rand_bytes,
        sha1: tc_crypto_sha1,
        sha256: tc_crypto_sha256,
        aes_ige_encrypt: tc_crypto_ige_encrypt,
        aes_ige_decrypt: tc_crypto_ige_decrypt,
        aes_ctr_encrypt: tc_crypto_aes_ctr,
    };

    #[link(name = "tgvoip")]
    extern "C" {
        pub fn tc_voip_create(
            count: i32,
            ipv4: *const *const std::ffi::c_char,
            ipv6: *const *const std::ffi::c_char,
            ports: *const i32,
            ids: *const i64,
            tags: *const u8,
            key: *const u8,
            key_len: i32,
            outgoing: i32,
            enable_p2p: i32,
            enable_aec: i32,
            enable_ns: i32,
            enable_agc: i32,
            input: AudioCb,
            output: AudioCb,
            audio_ctx: *mut c_void,
            crypto: *const TcCrypto,
        ) -> *mut c_void;
        pub fn tc_voip_set_state_cb(h: *mut c_void, cb: StateCb, ctx: *mut c_void);
        pub fn tc_voip_state(h: *mut c_void) -> i32;
        pub fn tc_voip_stats(h: *mut c_void, sent: *mut u64, received: *mut u64);
        pub fn tc_voip_destroy(h: *mut c_void);
        pub fn tc_voip_version() -> *const std::ffi::c_char;
    }
}

extern "C" fn state_cb(state: i32, ctx: *mut std::ffi::c_void) {
    if ctx.is_null() {
        return;
    }
    let shared = unsafe { &*(ctx as *const NativeShared) };
    shared.state.store(state, Ordering::Relaxed);
    if state == STATE_FAILED {
        shared.stop.store(true, Ordering::Relaxed);
    }
}

struct NativeShared {
    state: AtomicI32,
    muted: AtomicBool,
    stop: AtomicBool,
    sent: AtomicU64,
    received: AtomicU64,
    error: Mutex<String>,
}

/// Запущенный нативный звонок. Держит поток медиа; `stop()` завершает.
pub struct NativeCall {
    shared: Arc<NativeShared>,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl NativeCall {
    pub fn start(
        endpoints: &[CallConnection],
        key: [u8; 32],
        outgoing: bool,
        muted: bool,
        p2p: bool,
    ) -> Result<Self> {
        if endpoints.is_empty() {
            anyhow::bail!("нет эндпойнтов для медиа");
        }
        let shared = Arc::new(NativeShared {
            state: AtomicI32::new(STATE_WAIT_INIT),
            muted: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            sent: AtomicU64::new(0),
            received: AtomicU64::new(0),
            error: Mutex::new(String::new()),
        });
        let eps = endpoints.to_vec();
        let shared_thread = shared.clone();
        let join = std::thread::Builder::new()
            .name("tc-voip".into())
            .spawn(move || media_thread(eps, key, outgoing, muted, p2p, shared_thread))
            .context("не удалось запустить медиа-поток")?;
        Ok(Self {
            shared,
            join: Mutex::new(Some(join)),
        })
    }

    pub fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::Relaxed);
    }

    pub fn state(&self) -> i32 {
        self.shared.state.load(Ordering::Relaxed)
    }

    pub fn stats(&self) -> (u64, u64) {
        (
            self.shared.sent.load(Ordering::Relaxed),
            self.shared.received.load(Ordering::Relaxed),
        )
    }

    pub fn error(&self) -> String {
        self.shared.error.lock().unwrap().clone()
    }

    pub fn is_active(&self) -> bool {
        !self.shared.stop.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.join.lock().unwrap().take() {
            let _ = h.join();
        }
    }
}

impl Drop for NativeCall {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "android")]
fn media_thread(
    endpoints: Vec<CallConnection>,
    key: [u8; 32],
    outgoing: bool,
    muted: bool,
    p2p: bool,
    shared: Arc<NativeShared>,
) {
    use std::ffi::{c_void, CString};

    let audio = Arc::new(AudioShared::default());
    audio.muted.store(muted, Ordering::Relaxed);
    shared.muted.store(muted, Ordering::Relaxed);
    // PCM-устройства держим в этом же потоке (cpal::Stream !Send).
    let _audio_guard = match crate::media::voip_audio::spawn(audio.clone()) {
        Ok(g) => Some(g),
        Err(e) => {
            *shared.error.lock().unwrap() = format!("audio: {e}");
            shared.state.store(STATE_FAILED, Ordering::Relaxed);
            shared.stop.store(true, Ordering::Relaxed);
            return;
        }
    };

    let cstrs: Vec<CString> = endpoints
        .iter()
        .map(|e| CString::new(e.ip.clone()).unwrap_or_default())
        .collect();
    let empty = CString::default();
    let ipv4: Vec<*const std::ffi::c_char> = cstrs.iter().map(|c| c.as_ptr()).collect();
    let ipv6: Vec<*const std::ffi::c_char> = endpoints.iter().map(|_| empty.as_ptr()).collect();
    let ports: Vec<i32> = endpoints.iter().map(|e| e.port).collect();
    let ids: Vec<i64> = endpoints
        .iter()
        .enumerate()
        .map(|(i, e)| if e.id != 0 { e.id } else { i as i64 + 1 })
        .collect();
    let mut tags = vec![0u8; endpoints.len() * 16];
    for (i, e) in endpoints.iter().enumerate() {
        let n = e.peer_tag.len().min(16);
        tags[i * 16..i * 16 + n].copy_from_slice(&e.peer_tag[..n]);
    }

    let handle = unsafe {
        ffi::tc_voip_create(
            endpoints.len() as i32,
            ipv4.as_ptr(),
            ipv6.as_ptr(),
            ports.as_ptr(),
            ids.as_ptr(),
            tags.as_ptr(),
            key.as_ptr(),
            key.len() as i32,
            outgoing as i32,
            p2p as i32,
            0,
            0,
            0,
            audio_input_cb,
            audio_output_cb,
            Arc::as_ptr(&audio) as *mut c_void,
            &ffi::CRYPTO as *const ffi::TcCrypto,
        )
    };
    if handle.is_null() {
        *shared.error.lock().unwrap() = "libtgvoip: не удалось создать сессию".into();
        shared.state.store(STATE_FAILED, Ordering::Relaxed);
        shared.stop.store(true, Ordering::Relaxed);
        return;
    }
    unsafe {
        ffi::tc_voip_set_state_cb(handle, state_cb, Arc::as_ptr(&shared) as *mut c_void);
    }

    while !shared.stop.load(Ordering::Relaxed) {
        let mut sent = 0u64;
        let mut recv = 0u64;
        unsafe { ffi::tc_voip_stats(handle, &mut sent, &mut recv) };
        shared.sent.store(sent, Ordering::Relaxed);
        shared.received.store(recv, Ordering::Relaxed);
        let st = unsafe { ffi::tc_voip_state(handle) };
        shared.state.store(st, Ordering::Relaxed);
        audio
            .muted
            .store(shared.muted.load(Ordering::Relaxed), Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    unsafe { ffi::tc_voip_destroy(handle) };
}

#[cfg(not(target_os = "android"))]
fn media_thread(
    _endpoints: Vec<CallConnection>,
    _key: [u8; 32],
    _outgoing: bool,
    _muted: bool,
    _p2p: bool,
    shared: Arc<NativeShared>,
) {
    *shared.error.lock().unwrap() =
        "VoIP-медиа доступно только в Android-сборке (libtgvoip)".into();
    shared.state.store(STATE_FAILED, Ordering::Relaxed);
    shared.stop.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_key_is_sha256() {
        let k = call_key_from_auth_key(b"auth");
        let expect: [u8; 32] = {
            use sha2::{Digest, Sha256};
            Sha256::digest(b"auth").into()
        };
        assert_eq!(k, expect);
    }

    #[test]
    fn ige_roundtrip_matches_grammers() {
        let key = [7u8; 32];
        let iv = [1u8; 32];
        let plain = vec![9u8; 64];
        let mut enc = plain.clone();
        let mut iv_e = iv;
        grammers_crypto::aes::ige_encrypt(&mut enc, &key, &mut iv_e);

        let mut out = vec![0u8; 64];
        let mut iv_d = iv;
        unsafe {
            tc_crypto_ige_decrypt(
                enc.as_mut_ptr(),
                out.as_mut_ptr(),
                enc.len(),
                key.as_ptr() as *mut u8,
                iv_d.as_mut_ptr(),
            );
        }
        assert_eq!(out, plain);

        let mut out_enc = vec![0u8; 64];
        let mut iv_enc = iv;
        unsafe {
            tc_crypto_ige_encrypt(
                plain.as_ptr() as *mut u8,
                out_enc.as_mut_ptr(),
                plain.len(),
                key.as_ptr() as *mut u8,
                iv_enc.as_mut_ptr(),
            );
        }
        assert_eq!(out_enc, enc);
    }

    #[test]
    fn aes_ctr_is_stream_consistent() {
        let key = [3u8; 32];
        let base_iv = [5u8; 16];
        let plain: Vec<u8> = (0..77u8).collect();

        let mut one = plain.clone();
        let (mut iv1, mut ec1, mut n1) = (base_iv, [0u8; 16], 0u32);
        unsafe {
            tc_crypto_aes_ctr(
                one.as_mut_ptr(),
                one.len(),
                key.as_ptr() as *mut u8,
                iv1.as_mut_ptr(),
                ec1.as_mut_ptr(),
                &mut n1,
            );
        }

        let mut parts = plain.clone();
        let (mut iv2, mut ec2, mut n2) = (base_iv, [0u8; 16], 0u32);
        let (a, b) = parts.split_at_mut(13);
        unsafe {
            tc_crypto_aes_ctr(
                a.as_mut_ptr(),
                a.len(),
                key.as_ptr() as *mut u8,
                iv2.as_mut_ptr(),
                ec2.as_mut_ptr(),
                &mut n2,
            );
            tc_crypto_aes_ctr(
                b.as_mut_ptr(),
                b.len(),
                key.as_ptr() as *mut u8,
                iv2.as_mut_ptr(),
                ec2.as_mut_ptr(),
                &mut n2,
            );
        }
        assert_eq!(one, parts);
        assert_ne!(one, plain);
    }
}
