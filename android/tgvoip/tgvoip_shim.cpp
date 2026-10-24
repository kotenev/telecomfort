// C-ABI шим над libtgvoip (TgVoip.h) для Rust-ядра telecomfort.
// Сборка: см. android/build-tgvoip.sh (NDK, TGVOIP_USE_CUSTOM_CRYPTO,
// TGVOIP_USE_CALLBACK_AUDIO_IO, WebRTC DSP включён).
#include "TgVoip.h"
#include "VoIPController.h"

#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

extern "C" {

typedef void (*tc_audio_cb)(int16_t* samples, size_t count, void* ctx);
typedef void (*tc_state_cb)(int state, void* ctx);

typedef struct TcCrypto {
    void (*rand_bytes)(uint8_t* buffer, size_t length);
    void (*sha1)(uint8_t* msg, size_t length, uint8_t* output);
    void (*sha256)(uint8_t* msg, size_t length, uint8_t* output);
    void (*aes_ige_encrypt)(uint8_t* in, uint8_t* out, size_t length, uint8_t* key, uint8_t* iv);
    void (*aes_ige_decrypt)(uint8_t* in, uint8_t* out, size_t length, uint8_t* key, uint8_t* iv);
    void (*aes_ctr_encrypt)(uint8_t* inout, size_t length, uint8_t* key, uint8_t* iv,
                            uint8_t* ecount, uint32_t* num);
} TcCrypto;

struct TcVoipHandle {
    std::unique_ptr<TgVoip> impl;
    tc_state_cb state_cb = nullptr;
    void* state_ctx = nullptr;
    int last_state = 0;
};

/// Создать и сразу запустить VoIP-сессию.
/// Возвращает nullptr при ошибке (см. tc_voip_last_error).
void* tc_voip_create(
    int count,
    const char* const* ipv4,
    const char* const* ipv6,
    const int32_t* ports,
    const int64_t* ids,
    const uint8_t* tags, // count * 16
    const uint8_t* key,
    int key_len,
    int outgoing,
    int enable_p2p,
    int enable_aec,
    int enable_ns,
    int enable_agc,
    tc_audio_cb input_cb,
    tc_audio_cb output_cb,
    void* audio_ctx,
    const TcCrypto* crypto) {

    if (count <= 0 || key == nullptr || key_len <= 0) {
        return nullptr;
    }
    if (crypto != nullptr) {
        tgvoip::VoIPController::crypto.rand_bytes = crypto->rand_bytes;
        tgvoip::VoIPController::crypto.sha1 = crypto->sha1;
        tgvoip::VoIPController::crypto.sha256 = crypto->sha256;
        tgvoip::VoIPController::crypto.aes_ige_encrypt = crypto->aes_ige_encrypt;
        tgvoip::VoIPController::crypto.aes_ige_decrypt = crypto->aes_ige_decrypt;
        tgvoip::VoIPController::crypto.aes_ctr_encrypt = crypto->aes_ctr_encrypt;
    }

    std::vector<TgVoipEndpoint> endpoints;
    endpoints.reserve(count);
    for (int i = 0; i < count; i++) {
        TgVoipEndpoint ep;
        ep.endpointId = ids ? ids[i] : static_cast<int64_t>(i + 1);
        ep.host.ipv4 = ipv4 && ipv4[i] ? ipv4[i] : "";
        ep.host.ipv6 = ipv6 && ipv6[i] ? ipv6[i] : "";
        ep.port = static_cast<uint16_t>(ports ? ports[i] : 0);
        ep.type = TgVoipEndpointType::UdpRelay;
        if (tags) {
            std::memcpy(ep.peerTag, tags + i * 16, 16);
        }
        endpoints.push_back(ep);
    }

    TgVoipConfig config;
    config.initializationTimeout = 30.0;
    config.receiveTimeout = 20.0;
    config.dataSaving = TgVoipDataSaving::Never;
    config.enableP2P = enable_p2p != 0;
    config.enableAEC = enable_aec != 0;
    config.enableNS = enable_ns != 0;
    config.enableAGC = enable_agc != 0;
    config.enableVolumeControl = true;
    config.enableCallUpgrade = true;
    config.maxApiLayer = 0;

    TgVoipEncryptionKey enc;
    enc.value.assign(key, key + key_len);
    enc.isOutgoing = outgoing != 0;

    TgVoipPersistentState persistent;

    TgVoipCrypto voip_crypto{};
    if (crypto != nullptr) {
        voip_crypto.rand_bytes = crypto->rand_bytes;
        voip_crypto.sha1 = crypto->sha1;
        voip_crypto.sha256 = crypto->sha256;
        voip_crypto.aes_ige_encrypt = crypto->aes_ige_encrypt;
        voip_crypto.aes_ige_decrypt = crypto->aes_ige_decrypt;
        voip_crypto.aes_ctr_encrypt = crypto->aes_ctr_encrypt;
    }

    TgVoipAudioDataCallbacks audio;
    if (input_cb && audio_ctx) {
        audio.input = [input_cb, audio_ctx](int16_t* samples, size_t count) {
            input_cb(samples, count, audio_ctx);
        };
    }
    if (output_cb && audio_ctx) {
        audio.output = [output_cb, audio_ctx](int16_t* samples, size_t count) {
            output_cb(samples, count, audio_ctx);
        };
    }

    auto* handle = new TcVoipHandle();
    try {
        handle->impl = TgVoip::makeInstance(
            config,
            persistent,
            endpoints,
            nullptr,
            TgVoipNetworkType::WiFi,
            enc,
            voip_crypto,
            audio);
    } catch (...) {
        delete handle;
        return nullptr;
    }
    return handle;
}

/// Раздельная установка колбэка состояния (до первого обновления).
void tc_voip_set_state_cb(void* raw, tc_state_cb cb, void* ctx) {
    auto* h = static_cast<TcVoipHandle*>(raw);
    if (!h) return;
    h->state_cb = cb;
    h->state_ctx = ctx;
    if (h->impl) {
        h->impl->setOnStateUpdated([h](TgVoipState s) {
            h->last_state = static_cast<int>(s);
            if (h->state_cb) h->state_cb(h->last_state, h->state_ctx);
        });
    }
}

void tc_voip_set_signal_cb(void* raw, void (*cb)(int, void*), void* ctx) {
    auto* h = static_cast<TcVoipHandle*>(raw);
    if (!h || !h->impl) return;
    h->impl->setOnSignalBarsUpdated([cb, ctx](int bars) {
        if (cb) cb(bars, ctx);
    });
}

int tc_voip_state(void* raw) {
    auto* h = static_cast<TcVoipHandle*>(raw);
    return h ? h->last_state : -1;
}

void tc_voip_set_mute(void* raw, int mute) {
    auto* h = static_cast<TcVoipHandle*>(raw);
    if (h && h->impl) h->impl->setMuteMicrophone(mute != 0);
}

void tc_voip_set_network(void* raw, int type) {
    auto* h = static_cast<TcVoipHandle*>(raw);
    if (!h || !h->impl) return;
    TgVoipNetworkType t = TgVoipNetworkType::Unknown;
    switch (type) {
        case 1: t = TgVoipNetworkType::Gprs; break;
        case 2: t = TgVoipNetworkType::Edge; break;
        case 3: t = TgVoipNetworkType::ThirdGeneration; break;
        case 4: t = TgVoipNetworkType::Hspa; break;
        case 5: t = TgVoipNetworkType::Lte; break;
        case 6: t = TgVoipNetworkType::WiFi; break;
        case 7: t = TgVoipNetworkType::Ethernet; break;
        case 8: t = TgVoipNetworkType::OtherHighSpeed; break;
        case 9: t = TgVoipNetworkType::OtherLowSpeed; break;
        case 10: t = TgVoipNetworkType::Dialup; break;
        case 11: t = TgVoipNetworkType::OtherMobile; break;
        default: break;
    }
    h->impl->setNetworkType(t);
}

void tc_voip_set_output_gain(void* raw, float level) {
    auto* h = static_cast<TcVoipHandle*>(raw);
    if (h && h->impl) h->impl->setOutputVolume(level);
}

void tc_voip_set_input_gain(void* raw, float level) {
    auto* h = static_cast<TcVoipHandle*>(raw);
    if (h && h->impl) h->impl->setInputVolume(level);
}

void tc_voip_stats(void* raw, uint64_t* sent, uint64_t* received) {
    auto* h = static_cast<TcVoipHandle*>(raw);
    if (!h || !h->impl) {
        if (sent) *sent = 0;
        if (received) *received = 0;
        return;
    }
    auto stats = h->impl->getTrafficStats();
    if (sent) *sent = stats.bytesSentWifi + stats.bytesSentMobile;
    if (received) *received = stats.bytesReceivedWifi + stats.bytesReceivedMobile;
}

/// Остановить сессию и освободить память. Вызывать один раз.
void tc_voip_destroy(void* raw) {
    auto* h = static_cast<TcVoipHandle*>(raw);
    if (!h) return;
    if (h->impl) {
        h->impl->stop();
        h->impl.reset();
    }
    delete h;
}

const char* tc_voip_version(void) {
    static std::string version = TgVoip::getVersion();
    return version.c_str();
}

} // extern "C"
