//! MTProto-прокси (MTProxy): obfuscated2 + padded-intermediate.
//!
//! grammers умеет только SOCKS5 и жёстко использует `Full`-транспорт, а MTProxy
//! требует obfuscated2 (AES-256-CTR с секретом) и один из transport-тегов
//! (`0xdddddddd` padded-intermediate / `0xeeeeeeee` intermediate).
//!
//! Решение без форка grammers: локальный SOCKS5-мост. grammers подключается к
//! `127.0.0.1:<port>`, мост принимает SOCKS5 CONNECT (в адресе — целевой DC),
//! сам открывает TCP к MTProxy, шлёт 64-байтовый obfuscated-init с DC id и
//! тегом транспорта, далее перекладывает `Full`-кадры grammers в кадры
//! intermediate/padded-intermediate и обратно. Криптография MTProto при этом
//! остаётся end-to-end с DC — мост видит только транспортный слой.
//!
//! Формат ссылок: `tg://proxy?server=HOST&port=PORT&secret=HEX[&dc=N]`,
//! `https://t.me/proxy?...`, `mtproto://HOST:PORT?secret=HEX`.
//!
//! Секрет: 16 байт (hex 32) → intermediate; 17 байт (hex 34) с первым байтом
//! `0xdd` → padded-intermediate, `0xee` → intermediate.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use aes::cipher::{KeyIvInit, StreamCipher};
use aes::Aes256;
use anyhow::{anyhow, bail, Context, Result};
use ctr::Ctr128BE;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Транспорт между мостом и MTProxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyTransport {
    /// `0xeeeeeeee`: длина + payload без выравнивания.
    Intermediate,
    /// `0xdddddddd`: длина + payload + случайный padding 0..15.
    PaddedIntermediate,
}

impl ProxyTransport {
    fn tag(self) -> [u8; 4] {
        match self {
            Self::Intermediate => 0xeeee_eeeeu32.to_le_bytes(),
            Self::PaddedIntermediate => 0xdddd_ddddu32.to_le_bytes(),
        }
    }
}

/// Разобранная конфигурация MTProxy.
#[derive(Clone, Debug)]
pub struct MtProxyConfig {
    pub host: String,
    pub port: u16,
    /// 16-байтовый секрет (без ведущего байта-тега).
    pub secret: Vec<u8>,
    pub transport: ProxyTransport,
    /// Явный DC id (если задан в ссылке) — иначе определяется по адресу SOCKS5.
    pub dc: Option<i16>,
}

impl MtProxyConfig {
    /// Похоже ли значение на MTProxy-ссылку (а не socks5://).
    pub fn is_mtproxy_url(value: &str) -> bool {
        let v = value.trim().to_ascii_lowercase();
        v.starts_with("tg://proxy")
            || v.starts_with("https://t.me/proxy")
            || v.starts_with("http://t.me/proxy")
            || v.starts_with("mtproto://")
    }
}

/// Разбирает `tg://proxy`, `t.me/proxy` или `mtproto://` ссылку.
pub fn parse_proxy_url(value: &str) -> Result<MtProxyConfig> {
    let value = value.trim();
    if value.is_empty() {
        bail!("пустая ссылка MTProxy");
    }
    let lower = value.to_ascii_lowercase();
    let (host, port, query) = if lower.starts_with("tg://proxy") {
        let query = value
            .split_once('?')
            .map(|(_, q)| q)
            .ok_or_else(|| anyhow!("в ссылке tg://proxy нет параметров"))?;
        let (host, port) = parse_query_host_port(query)?;
        (host, port, query.to_string())
    } else if lower.starts_with("https://t.me/proxy") || lower.starts_with("http://t.me/proxy") {
        let query = value
            .split_once('?')
            .map(|(_, q)| q)
            .ok_or_else(|| anyhow!("в ссылке t.me/proxy нет параметров"))?;
        let (host, port) = parse_query_host_port(query)?;
        (host, port, query.to_string())
    } else if lower.starts_with("mtproto://") {
        let rest = &value["mtproto://".len()..];
        let (authority, query) = match rest.split_once('?') {
            Some((a, q)) => (a, q.to_string()),
            None => (rest, String::new()),
        };
        let authority = authority.trim_end_matches('/');
        let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
            let port: u16 = p.parse().context("порт MTProxy")?;
            (h.to_string(), port)
        } else {
            (authority.to_string(), 443)
        };
        (host, port, query)
    } else {
        bail!("неизвестная схема MTProxy-ссылки");
    };

    let params = parse_query(&query);
    let secret_hex = params
        .iter()
        .find(|(k, _)| k == "secret")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| anyhow!("в MTProxy-ссылке нет secret"))?;
    let (secret, transport) = parse_secret(&secret_hex)?;
    let dc = params
        .iter()
        .find(|(k, _)| k == "dc")
        .and_then(|(_, v)| v.parse::<i16>().ok());
    if host.trim().is_empty() {
        bail!("в MTProxy-ссылке нет server");
    }
    Ok(MtProxyConfig {
        host: host.trim().to_string(),
        port,
        secret,
        transport,
        dc,
    })
}

fn parse_query_host_port(query: &str) -> Result<(String, u16)> {
    let params = parse_query(query);
    let host = params
        .iter()
        .find(|(k, _)| k == "server" || k == "host")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| anyhow!("в MTProxy-ссылке нет server"))?;
    let port = params
        .iter()
        .find(|(k, _)| k == "port")
        .and_then(|(_, v)| v.parse::<u16>().ok())
        .unwrap_or(443);
    Ok((host, port))
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.push((percent_decode(k), percent_decode(v)));
    }
    out
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Разбирает hex-секрет MTProxy (16 или 17 байт).
pub fn parse_secret(hex_secret: &str) -> Result<(Vec<u8>, ProxyTransport)> {
    let hex_secret = hex_secret.trim();
    if !hex_secret.len().is_multiple_of(2) || hex_secret.is_empty() {
        bail!("secret MTProxy должен быть hex-строкой чётной длины");
    }
    let mut bytes = Vec::with_capacity(hex_secret.len() / 2);
    let raw = hex_secret.as_bytes();
    for pair in raw.chunks(2) {
        let hi = hex_val(pair[0]).ok_or_else(|| anyhow!("secret MTProxy: не hex"))?;
        let lo = hex_val(pair[1]).ok_or_else(|| anyhow!("secret MTProxy: не hex"))?;
        bytes.push(hi << 4 | lo);
    }
    match bytes.len() {
        16 => Ok((bytes, ProxyTransport::Intermediate)),
        17 => {
            let tag = bytes.remove(0);
            let transport = match tag {
                0xdd => ProxyTransport::PaddedIntermediate,
                0xee => ProxyTransport::Intermediate,
                // Неизвестный тег: Telegram-клиенты по умолчанию берут padded.
                _ => ProxyTransport::PaddedIntermediate,
            };
            Ok((bytes, transport))
        }
        n => bail!("secret MTProxy: ожидалось 16 или 17 байт, получено {n}"),
    }
}

/// Состояние AES-256-CTR в обе стороны (как в obfuscated2).
struct ObfuscatedCipher {
    tx: Ctr128BE<Aes256>,
    rx: Ctr128BE<Aes256>,
}

impl ObfuscatedCipher {
    fn from_init(init: &[u8; 64], secret: &[u8]) -> Result<Self> {
        let mut rev = *init;
        rev.reverse();
        let tx_key = derive_key(&init[8..40], secret);
        let rx_key = derive_key(&rev[8..40], secret);
        Ok(Self {
            tx: Ctr128BE::<Aes256>::new_from_slices(&tx_key, &init[40..56])
                .map_err(|_| anyhow!("AES-CTR: неверный ключ/IV"))?,
            rx: Ctr128BE::<Aes256>::new_from_slices(&rx_key, &rev[40..56])
                .map_err(|_| anyhow!("AES-CTR: неверный ключ/IV"))?,
        })
    }

    fn encrypt(&mut self, data: &mut [u8]) {
        self.tx.apply_keystream(data);
    }

    #[cfg(test)]
    fn decrypt(&mut self, data: &mut [u8]) {
        self.rx.apply_keystream(data);
    }
}

fn derive_key(raw: &[u8], secret: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(raw);
    if !secret.is_empty() {
        hasher.update(secret);
    }
    hasher.finalize().into()
}

const FORBIDDEN_FIRST_INTS: [[u8; 4]; 7] = [
    *b"HEAD",
    *b"POST",
    *b"GET ",
    *b"OPTI",
    [0x16, 0x03, 0x01, 0x02], // TLS ClientHello
    [0xdd, 0xdd, 0xdd, 0xdd],
    [0xee, 0xee, 0xee, 0xee],
];

/// Генерирует 64-байтовый obfuscated-init и cipher для дальнейшего потока.
fn build_init(
    secret: &[u8],
    dc: i16,
    transport: ProxyTransport,
) -> Result<([u8; 64], ObfuscatedCipher)> {
    finalize_init(random_init(dc, transport)?, secret)
}

fn finalize_init(init: [u8; 64], secret: &[u8]) -> Result<([u8; 64], ObfuscatedCipher)> {
    let mut cipher = ObfuscatedCipher::from_init(&init, secret)?;
    let mut encrypted = init;
    cipher.encrypt(&mut encrypted);
    let mut final_init = init;
    final_init[56..64].copy_from_slice(&encrypted[56..64]);
    Ok((final_init, cipher))
}

/// Случайный init с transport-тегом и DC id (до шифрования хвоста).
fn random_init(dc: i16, transport: ProxyTransport) -> Result<[u8; 64]> {
    let mut init = [0u8; 64];
    loop {
        getrandom::fill(&mut init).map_err(|e| anyhow!("getrandom: {e}"))?;
        if init[0] == 0xef {
            continue;
        }
        if FORBIDDEN_FIRST_INTS
            .iter()
            .any(|forbidden| forbidden == &init[..4])
        {
            continue;
        }
        if init[4..8] == [0u8; 4] {
            continue;
        }
        break;
    }
    init[56..60].copy_from_slice(&transport.tag());
    init[60..62].copy_from_slice(&dc.to_le_bytes());
    Ok(init)
}

/// Строит кадр выбранного транспорта из payload MTProto.
fn encode_transport_frame(payload: &[u8], transport: ProxyTransport) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 20);
    match transport {
        ProxyTransport::Intermediate => {
            out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            out.extend_from_slice(payload);
        }
        ProxyTransport::PaddedIntermediate => {
            let mut rnd = [0u8; 1];
            let _ = getrandom::fill(&mut rnd);
            let pad = (rnd[0] % 16) as usize;
            out.extend_from_slice(&((payload.len() + pad) as u32).to_le_bytes());
            out.extend_from_slice(payload);
            if pad > 0 {
                let mut padding = vec![0u8; pad];
                let _ = getrandom::fill(&mut padding);
                out.extend_from_slice(&padding);
            }
        }
    }
    out
}

/// Убирает padding padded-intermediate, возвращая точный MTProto payload.
///
/// Зашифрованное сообщение всегда `24 + 16*k` байт (auth_key_id + msg_key +
/// блоки AES-IGE), поэтому padding однозначен: `(len - 24) % 16`.
/// Незашифрованное (auth_key_id == 0) несёт явную длину тела.
fn strip_padding(payload: &[u8]) -> &[u8] {
    let n = payload.len();
    if n >= 20 && payload[..8].iter().all(|&b| b == 0) {
        let body_len =
            u32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]]) as usize;
        if 20 + body_len <= n {
            return &payload[..20 + body_len];
        }
    }
    if n >= 24 {
        let pad = (n - 24) % 16;
        return &payload[..n - pad];
    }
    payload
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Полный `Full`-кадр для grammers.
fn encode_full_frame(payload: &[u8], seq: u32) -> Vec<u8> {
    let len = payload.len() + 12;
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(payload);
    let crc = crc32(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

/// Достаёт один `Full`-кадр из буфера (возвращает payload).
fn take_full_frame(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    if buf.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if !(12..=8 * 1024 * 1024).contains(&len) {
        return None;
    }
    if buf.len() < len {
        return None;
    }
    let frame: Vec<u8> = buf.drain(..len).collect();
    Some(frame[8..len - 4].to_vec())
}

/// Достаёт один кадр intermediate/padded из буфера.
fn take_transport_frame(buf: &mut Vec<u8>, transport: ProxyTransport) -> Option<Vec<u8>> {
    if buf.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > 8 * 1024 * 1024 {
        return None;
    }
    if buf.len() < 4 + len {
        return None;
    }
    let frame: Vec<u8> = buf.drain(..4 + len).collect();
    let payload = &frame[4..];
    Some(match transport {
        ProxyTransport::Intermediate => payload.to_vec(),
        ProxyTransport::PaddedIntermediate => strip_padding(payload).to_vec(),
    })
}

/// DC id по адресу назначения из SOCKS5-запроса grammers.
fn dc_for_addr(addr: Option<IpAddr>) -> i16 {
    match addr {
        Some(IpAddr::V4(ip)) => dc_for_ipv4(ip),
        _ => 2,
    }
}

fn dc_for_ipv4(ip: Ipv4Addr) -> i16 {
    let o = ip.octets();
    match (o[0], o[1], o[2], o[3]) {
        (149, 154, 175, 100) => 3,
        (149, 154, 175, _) => 1,
        (149, 154, 167, 91) => 4,
        (149, 154, 167, 92) => 4,
        (149, 154, 167, _) => 2,
        (149, 154, 171, _) => 5,
        (91, 108, 56, 130) => 5,
        (91, 108, 56, 149) => 5,
        (91, 108, 56, _) => 5,
        (91, 108, 57, _) => 5,
        _ => 2,
    }
}

/// Запущенный локальный SOCKS5-мост.
#[derive(Clone, Debug)]
pub struct MtProxyTunnel {
    pub socks_addr: SocketAddr,
}

/// Поднимает локальный SOCKS5-мост, туннелирующий соединения через MTProxy.
pub async fn start(config: MtProxyConfig) -> Result<MtProxyTunnel> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .context("MTProxy: не удалось открыть локальный SOCKS5-порт")?;
    let socks_addr = listener.local_addr()?;
    let config = Arc::new(config);
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let config = Arc::clone(&config);
                    tokio::spawn(async move {
                        if let Err(err) = handle_connection(stream, config).await {
                            tracing::debug!("MTProxy-мост: соединение закрыто: {err:#}");
                        }
                    });
                }
                Err(err) => {
                    tracing::warn!("MTProxy-мост: accept: {err}");
                    break;
                }
            }
        }
    });
    Ok(MtProxyTunnel { socks_addr })
}

async fn handle_connection(mut client: TcpStream, config: Arc<MtProxyConfig>) -> Result<()> {
    let dest = socks5_handshake(&mut client).await?;
    let dc = config.dc.unwrap_or_else(|| dc_for_addr(dest));

    let proxy = TcpStream::connect((config.host.as_str(), config.port))
        .await
        .with_context(|| format!("MTProxy: подключение к {}:{}", config.host, config.port))?;
    proxy.set_nodelay(true).ok();
    client.set_nodelay(true).ok();

    let (init, cipher) = build_init(&config.secret, dc, config.transport)?;
    let ObfuscatedCipher { mut tx, mut rx } = cipher;
    let (mut proxy_read, mut proxy_write) = proxy.into_split();
    proxy_write
        .write_all(&init)
        .await
        .context("MTProxy: отправка obfuscated-init")?;

    socks5_reply(&mut client, true).await?;

    let (mut client_read, mut client_write) = client.into_split();
    let transport = config.transport;

    let mut up = tokio::spawn(async move {
        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let n = client_read.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            while let Some(payload) = take_full_frame(&mut buf) {
                let mut frame = encode_transport_frame(&payload, transport);
                tx.apply_keystream(&mut frame);
                proxy_write.write_all(&frame).await?;
            }
        }
        Ok::<(), anyhow::Error>(())
    });

    let mut down = tokio::spawn(async move {
        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        let mut chunk = [0u8; 16 * 1024];
        let mut seq = 0u32;
        loop {
            let n = proxy_read.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            let mut data = chunk[..n].to_vec();
            rx.apply_keystream(&mut data);
            buf.extend_from_slice(&data);
            while let Some(payload) = take_transport_frame(&mut buf, transport) {
                let frame = encode_full_frame(&payload, seq);
                seq = seq.wrapping_add(1);
                client_write.write_all(&frame).await?;
            }
        }
        Ok::<(), anyhow::Error>(())
    });

    tokio::select! {
        r = &mut up => { down.abort(); r??; }
        r = &mut down => { up.abort(); r??; }
    }
    Ok(())
}

async fn socks5_handshake(stream: &mut TcpStream) -> Result<Option<IpAddr>> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await?;
    if head[0] != 5 {
        bail!("SOCKS5: неверная версия {}", head[0]);
    }
    let n = head[1] as usize;
    let mut methods = vec![0u8; n];
    stream.read_exact(&mut methods).await?;
    stream.write_all(&[5, 0]).await?;

    let mut req = [0u8; 4];
    stream.read_exact(&mut req).await?;
    if req[0] != 5 || req[1] != 1 {
        socks5_reply(stream, false).await.ok();
        bail!("SOCKS5: поддерживается только CONNECT");
    }
    let addr = match req[3] {
        1 => {
            let mut b = [0u8; 4];
            stream.read_exact(&mut b).await?;
            Some(IpAddr::V4(Ipv4Addr::from(b)))
        }
        3 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut domain = vec![0u8; len[0] as usize];
            stream.read_exact(&mut domain).await?;
            None
        }
        4 => {
            let mut b = [0u8; 16];
            stream.read_exact(&mut b).await?;
            Some(IpAddr::V6(b.into()))
        }
        other => bail!("SOCKS5: неизвестный ATYP {other}"),
    };
    let mut port = [0u8; 2];
    stream.read_exact(&mut port).await?;
    Ok(addr)
}

async fn socks5_reply(stream: &mut TcpStream, ok: bool) -> Result<()> {
    let code = if ok { 0 } else { 1 };
    stream.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_16_byte_secret_as_intermediate() {
        let (secret, transport) = parse_secret("0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(secret.len(), 16);
        assert_eq!(transport, ProxyTransport::Intermediate);
    }

    #[test]
    fn parses_17_byte_dd_secret_as_padded() {
        let (secret, transport) = parse_secret("dd0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(secret.len(), 16);
        assert_eq!(secret[0], 0x01);
        assert_eq!(transport, ProxyTransport::PaddedIntermediate);
    }

    #[test]
    fn rejects_bad_secret() {
        assert!(parse_secret("zz").is_err());
        assert!(parse_secret("0011").is_err());
    }

    #[test]
    fn parses_tg_proxy_link() {
        let cfg = parse_proxy_url(
            "tg://proxy?server=proxy.example&port=8443&secret=dd0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert_eq!(cfg.host, "proxy.example");
        assert_eq!(cfg.port, 8443);
        assert_eq!(cfg.transport, ProxyTransport::PaddedIntermediate);
    }

    #[test]
    fn parses_mtproto_link_with_dc() {
        let cfg = parse_proxy_url(
            "mtproto://proxy.example:443?secret=0123456789abcdef0123456789abcdef&dc=4",
        )
        .unwrap();
        assert_eq!(cfg.dc, Some(4));
        assert_eq!(cfg.transport, ProxyTransport::Intermediate);
    }

    #[test]
    fn recognizes_mtproxy_urls() {
        assert!(MtProxyConfig::is_mtproxy_url("tg://proxy?x=1"));
        assert!(MtProxyConfig::is_mtproxy_url("https://t.me/proxy?x=1"));
        assert!(MtProxyConfig::is_mtproxy_url("mtproto://h:1?x"));
        assert!(!MtProxyConfig::is_mtproxy_url("socks5://127.0.0.1:1"));
    }

    #[test]
    fn init_has_tag_and_dc_and_is_not_forbidden() {
        let init = random_init(2, ProxyTransport::PaddedIntermediate).unwrap();
        assert_eq!(&init[56..60], &0xdddd_ddddu32.to_le_bytes());
        assert_eq!(&init[60..62], &2i16.to_le_bytes());
        assert_ne!(init[0], 0xef);
        assert!(!FORBIDDEN_FIRST_INTS.iter().any(|f| f == &init[..4]));
        assert_ne!(init[4..8], [0u8; 4]);
    }

    #[test]
    fn build_init_encrypts_tail_and_keeps_header() {
        let secret = [7u8; 16];
        let plain = random_init(3, ProxyTransport::Intermediate).unwrap();
        let (final_init, _cipher) = finalize_init(plain, &secret).unwrap();
        assert_ne!(&final_init[56..64], &plain[56..64]);
        assert_eq!(&final_init[..56], &plain[..56]);
    }

    #[test]
    fn ctr_roundtrip() {
        let init = [42u8; 64];
        let secret = [3u8; 16];
        let mut a = ObfuscatedCipher::from_init(&init, &secret).unwrap();
        let mut b = ObfuscatedCipher::from_init(&init, &secret).unwrap();
        let mut data = b"hello mtproto proxy".to_vec();
        let original = data.clone();
        a.encrypt(&mut data);
        assert_ne!(data, original);
        b.decrypt(&mut data);
        assert_eq!(data, original);
    }

    #[test]
    fn full_frame_roundtrip_and_crc() {
        let payload = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let frame = encode_full_frame(&payload, 7);
        assert_eq!(frame.len(), payload.len() + 12);
        let len = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
        assert_eq!(len, frame.len());
        assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 7);
        let crc = u32::from_le_bytes(frame[len - 4..].try_into().unwrap());
        assert_eq!(crc, crc32(&frame[..len - 4]));
        let mut buf = frame;
        assert_eq!(take_full_frame(&mut buf).unwrap(), payload);
        assert!(buf.is_empty());
    }

    #[test]
    fn padded_frame_strips_padding_for_encrypted_payload() {
        for extra_blocks in 0..4 {
            let mut payload = vec![0u8; 24 + 16 * extra_blocks];
            payload[0] = 1; // auth_key_id != 0
            for pad in 0..16usize {
                let mut frame = ((payload.len() + pad) as u32).to_le_bytes().to_vec();
                frame.extend_from_slice(&payload);
                frame.extend(std::iter::repeat(0xAAu8).take(pad));
                let mut buf = frame;
                let got =
                    take_transport_frame(&mut buf, ProxyTransport::PaddedIntermediate).unwrap();
                assert_eq!(got, payload, "blocks={extra_blocks} pad={pad}");
            }
        }
    }

    #[test]
    fn padded_frame_strips_plain_payload_by_explicit_len() {
        let body = vec![9u8; 12];
        let mut payload = vec![0u8; 8];
        payload.extend_from_slice(&1234i64.to_le_bytes()); // msg_id
        payload.extend_from_slice(&(body.len() as u32).to_le_bytes());
        payload.extend_from_slice(&body);
        for pad in 0..16usize {
            let mut frame = ((payload.len() + pad) as u32).to_le_bytes().to_vec();
            frame.extend_from_slice(&payload);
            frame.extend(std::iter::repeat(0xBBu8).take(pad));
            let mut buf = frame;
            let got = take_transport_frame(&mut buf, ProxyTransport::PaddedIntermediate).unwrap();
            assert_eq!(got, payload, "pad={pad}");
        }
    }

    #[test]
    fn intermediate_frame_roundtrip() {
        let payload = vec![5u8; 16];
        let frame = encode_transport_frame(&payload, ProxyTransport::Intermediate);
        let mut buf = frame;
        assert_eq!(
            take_transport_frame(&mut buf, ProxyTransport::Intermediate).unwrap(),
            payload
        );
    }

    #[test]
    fn dc_mapping() {
        assert_eq!(dc_for_ipv4(Ipv4Addr::new(149, 154, 175, 50)), 1);
        assert_eq!(dc_for_ipv4(Ipv4Addr::new(149, 154, 175, 100)), 3);
        assert_eq!(dc_for_ipv4(Ipv4Addr::new(149, 154, 167, 51)), 2);
        assert_eq!(dc_for_ipv4(Ipv4Addr::new(149, 154, 167, 91)), 4);
        assert_eq!(dc_for_ipv4(Ipv4Addr::new(91, 108, 56, 130)), 5);
    }

    #[tokio::test]
    async fn socks5_bridge_roundtrips_frames_through_fake_proxy() {
        let secret = vec![0x11u8; 16];
        let transport = ProxyTransport::PaddedIntermediate;
        let payload_in = vec![7u8; 24];
        let mut payload_out = vec![0u8; 24 + 16];
        payload_out[0] = 1;

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server_secret = secret.clone();
        let server_payload_out = payload_out.clone();
        let server_payload_in = payload_in.clone();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut plain = [0u8; 64];
            sock.read_exact(&mut plain).await.unwrap();
            let mut rev = plain;
            rev.reverse();
            let tx_key = derive_key(&plain[8..40], &server_secret);
            let rx_key = derive_key(&rev[8..40], &server_secret);
            let mut rx = Ctr128BE::<Aes256>::new_from_slices(&tx_key, &plain[40..56]).unwrap();
            let mut tx = Ctr128BE::<Aes256>::new_from_slices(&rx_key, &rev[40..56]).unwrap();
            // Клиент потратил 64 байта keystream на сам init — сервер обязан
            // сдвинуть rx на те же 64 байта перед чтением первого пакета.
            rx.apply_keystream(&mut [0u8; 64]);

            let mut hdr = [0u8; 4];
            sock.read_exact(&mut hdr).await.unwrap();
            rx.apply_keystream(&mut hdr);
            let len = u32::from_le_bytes(hdr) as usize;
            let mut body = vec![0u8; len];
            sock.read_exact(&mut body).await.unwrap();
            rx.apply_keystream(&mut body);
            assert_eq!(strip_padding(&body), server_payload_in.as_slice());

            let mut frame = (server_payload_out.len() as u32).to_le_bytes().to_vec();
            frame.extend_from_slice(&server_payload_out);
            tx.apply_keystream(&mut frame);
            sock.write_all(&frame).await.unwrap();
        });

        let config = MtProxyConfig {
            host: Ipv4Addr::LOCALHOST.to_string(),
            port,
            secret,
            transport,
            dc: Some(2),
        };
        let tunnel = start(config).await.unwrap();
        let mut client = TcpStream::connect(tunnel.socks_addr).await.unwrap();
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut methods = [0u8; 2];
        client.read_exact(&mut methods).await.unwrap();
        assert_eq!(methods, [5, 0]);
        client
            .write_all(&[5, 1, 0, 1, 149, 154, 167, 51, 0x01, 0xbb])
            .await
            .unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply[..2], &[5, 0]);

        let frame = encode_full_frame(&payload_in, 0);
        client.write_all(&frame).await.unwrap();

        let mut hdr = [0u8; 4];
        client.read_exact(&mut hdr).await.unwrap();
        let len = u32::from_le_bytes(hdr) as usize;
        let mut rest = vec![0u8; len - 4];
        client.read_exact(&mut rest).await.unwrap();
        let mut full = hdr.to_vec();
        full.extend_from_slice(&rest);
        let mut buf = full;
        assert_eq!(take_full_frame(&mut buf).unwrap(), payload_out);
        server.await.unwrap();
    }
}
