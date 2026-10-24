#include <stdint.h>

static int32_t clamp_byte(int32_t n) {
    if (n < 0) {
        return 0;
    }
    if (n > 255) {
        return 255;
    }
    return n;
}

/* Droide has no shifts. scale and pack are the color math. */
DROIDE_EXPORT int32_t scale(int32_t channel, int32_t light) {
    if (light < 0) {
        light = 0;
    }
    if (light > 100) {
        light = 100;
    }
    return clamp_byte(channel) * light / 100;
}

DROIDE_EXPORT int32_t pack(int32_t r, int32_t g, int32_t b) {
    uint32_t red = (uint32_t)clamp_byte(r);
    uint32_t green = (uint32_t)clamp_byte(g);
    uint32_t blue = (uint32_t)clamp_byte(b);
    return (int32_t)(0xFF000000u | (red << 16) | (green << 8) | blue);
}
