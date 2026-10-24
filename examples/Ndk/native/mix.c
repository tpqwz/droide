#include <stdint.h>
#include <string.h>

/* Marker on the definition. The body may call the C library. */
DROIDE_EXPORT int32_t mix(int32_t n) {
    return n + (int32_t)strlen("ndk");
}
