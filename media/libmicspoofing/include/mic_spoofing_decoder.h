#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

int mic_spoofing_decoder_start(int source_fd, uint32_t *out_sample_rate,
        uint32_t *out_channel_count);

#ifdef __cplusplus
}
#endif
