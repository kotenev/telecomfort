// Реализация proxy-функций WebRTC AudioProcessing для версии webrtc_dsp
// из libtgvoip (в upstream файл .cc отсутствует).
#include "modules/audio_processing/include/audio_processing.h"
#include "modules/audio_processing/include/audio_frame_proxies.h"

namespace webrtc {

int ProcessAudioFrame(AudioProcessing* ap, AudioFrame* frame) {
    return ap->ProcessStream(frame);
}

int ProcessReverseAudioFrame(AudioProcessing* ap, AudioFrame* frame) {
    return ap->ProcessReverseStream(frame);
}

}  // namespace webrtc
