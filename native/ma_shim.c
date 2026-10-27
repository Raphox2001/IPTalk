/*
 * Kleine C-Schnittstelle zwischen IPTalk (Rust) und miniaudio.
 * Rust sieht nur undurchsichtige Zeiger und einfache Zahlen.
 */
#define MINIAUDIO_IMPLEMENTATION
#include "miniaudio.h"

#include <stdlib.h>
#include <string.h>

#define IPT_SAMPLE_RATE 48000

enum {
    IPT_BACKEND_AUTO = 0,
    IPT_BACKEND_WASAPI = 1,
    IPT_BACKEND_PULSE = 2,
    IPT_BACKEND_ALSA = 3,
    IPT_BACKEND_JACK = 4
};

typedef void (*ipt_capture_cb)(void* user, const float* input, ma_uint32 frames);
typedef void (*ipt_playback_cb)(void* user, float* output, ma_uint32 frames);

typedef struct {
    ma_context context;
    ma_device_info* playback;
    ma_uint32 playback_count;
    ma_device_info* capture;
    ma_uint32 capture_count;
} ipt_ctx;

typedef struct {
    ma_device device;
    int is_capture;
    void* user;
    ipt_capture_cb capture_cb;
    ipt_playback_cb playback_cb;
} ipt_stream;

const char* ipt_result_desc(int result)
{
    return ma_result_description((ma_result)result);
}

int ipt_ctx_new(int backend, ipt_ctx** out)
{
    ma_backend list[3];
    ma_uint32 count = 0;
    ma_context_config cfg;
    ipt_ctx* ctx;
    ma_result r;

    switch (backend) {
        case IPT_BACKEND_WASAPI: list[count++] = ma_backend_wasapi; break;
        case IPT_BACKEND_PULSE:  list[count++] = ma_backend_pulseaudio; break;
        case IPT_BACKEND_ALSA:   list[count++] = ma_backend_alsa; break;
        case IPT_BACKEND_JACK:   list[count++] = ma_backend_jack; break;
        default:
#if defined(_WIN32)
            list[count++] = ma_backend_wasapi;
#else
            list[count++] = ma_backend_pulseaudio;
            list[count++] = ma_backend_alsa;
            list[count++] = ma_backend_jack;
#endif
            break;
    }

    ctx = (ipt_ctx*)calloc(1, sizeof(ipt_ctx));
    if (ctx == NULL) return MA_OUT_OF_MEMORY;

    cfg = ma_context_config_init();
    cfg.threadPriority = ma_thread_priority_realtime;
    cfg.pulse.pApplicationName = "IPTalk";
    cfg.jack.pClientName = "IPTalk";

    r = ma_context_init(list, count, &cfg, &ctx->context);
    if (r != MA_SUCCESS) {
        free(ctx);
        return r;
    }
    *out = ctx;
    return MA_SUCCESS;
}

void ipt_ctx_free(ipt_ctx* ctx)
{
    if (ctx == NULL) return;
    ma_context_uninit(&ctx->context);
    free(ctx);
}

const char* ipt_ctx_backend_name(ipt_ctx* ctx)
{
    return ma_get_backend_name(ctx->context.backend);
}

int ipt_ctx_refresh(ipt_ctx* ctx)
{
    return ma_context_get_devices(&ctx->context,
        &ctx->playback, &ctx->playback_count,
        &ctx->capture, &ctx->capture_count);
}

ma_uint32 ipt_ctx_device_count(ipt_ctx* ctx, int capture)
{
    return capture ? ctx->capture_count : ctx->playback_count;
}

const char* ipt_ctx_device_name(ipt_ctx* ctx, int capture, ma_uint32 index)
{
    ma_device_info* list = capture ? ctx->capture : ctx->playback;
    ma_uint32 count = capture ? ctx->capture_count : ctx->playback_count;
    if (index >= count) return "";
    return list[index].name;
}

static void ipt_data_callback(ma_device* device, void* output, const void* input, ma_uint32 frames)
{
    ipt_stream* s = (ipt_stream*)device->pUserData;
    if (s->is_capture) {
        s->capture_cb(s->user, (const float*)input, frames);
    } else {
        s->playback_cb(s->user, (float*)output, frames);
    }
}

/*
 * Öffnet ein Mono-Gerät mit 48 kHz/f32. device_index < 0 bedeutet Standardgerät.
 * miniaudio rechnet auf das native Format des Geräts um.
 */
int ipt_stream_open(ipt_ctx* ctx, int capture, int device_index, ma_uint32 period_frames,
                    int exclusive, void* user, ipt_capture_cb ccb, ipt_playback_cb pcb,
                    ipt_stream** out)
{
    ma_device_config cfg;
    ipt_stream* s;
    ma_result r;
    const ma_device_id* id = NULL;

    if (device_index >= 0) {
        ma_device_info* list = capture ? ctx->capture : ctx->playback;
        ma_uint32 count = capture ? ctx->capture_count : ctx->playback_count;
        if ((ma_uint32)device_index < count) id = &list[device_index].id;
    }

    s = (ipt_stream*)calloc(1, sizeof(ipt_stream));
    if (s == NULL) return MA_OUT_OF_MEMORY;
    s->is_capture = capture;
    s->user = user;
    s->capture_cb = ccb;
    s->playback_cb = pcb;

    cfg = ma_device_config_init(capture ? ma_device_type_capture : ma_device_type_playback);
    cfg.sampleRate = IPT_SAMPLE_RATE;
    cfg.periodSizeInFrames = period_frames;
    cfg.periods = 2;
    cfg.performanceProfile = ma_performance_profile_low_latency;
    cfg.noPreSilencedOutputBuffer = MA_FALSE;
    cfg.noFixedSizedCallback = MA_TRUE; /* keine zusätzliche Zwischenpufferung */
    cfg.dataCallback = ipt_data_callback;
    cfg.pUserData = s;

    cfg.capture.pDeviceID = id;
    cfg.capture.format = ma_format_f32;
    cfg.capture.channels = 1;
    cfg.capture.shareMode = exclusive ? ma_share_mode_exclusive : ma_share_mode_shared;

    cfg.playback.pDeviceID = id;
    cfg.playback.format = ma_format_f32;
    cfg.playback.channels = 1;
    cfg.playback.shareMode = exclusive ? ma_share_mode_exclusive : ma_share_mode_shared;

    /* IAudioClient3-Low-Latency funktioniert nur ohne WASAPI-eigenen Resampler. */
    cfg.wasapi.noAutoConvertSRC = MA_TRUE;
    cfg.wasapi.noDefaultQualitySRC = MA_TRUE;
    cfg.wasapi.usage = ma_wasapi_usage_pro_audio;

    cfg.pulse.pStreamNameCapture = "IPTalk Mikrofon";
    cfg.pulse.pStreamNamePlayback = "IPTalk Kumpel";

    r = ma_device_init(&ctx->context, &cfg, &s->device);
    if (r != MA_SUCCESS) {
        free(s);
        return r;
    }
    *out = s;
    return MA_SUCCESS;
}

int ipt_stream_start(ipt_stream* s)
{
    return ma_device_start(&s->device);
}

void ipt_stream_free(ipt_stream* s)
{
    if (s == NULL) return;
    ma_device_uninit(&s->device);
    free(s);
}

/* Tatsächlich ausgehandelte Werte, für Anzeige und Latenzschätzung. */
void ipt_stream_info(ipt_stream* s, ma_uint32* period, ma_uint32* periods, ma_uint32* rate, int* exclusive)
{
    if (s->is_capture) {
        *period = s->device.capture.internalPeriodSizeInFrames;
        *periods = s->device.capture.internalPeriods;
        *rate = s->device.capture.internalSampleRate;
        *exclusive = s->device.capture.shareMode == ma_share_mode_exclusive;
    } else {
        *period = s->device.playback.internalPeriodSizeInFrames;
        *periods = s->device.playback.internalPeriods;
        *rate = s->device.playback.internalSampleRate;
        *exclusive = s->device.playback.shareMode == ma_share_mode_exclusive;
    }
}
