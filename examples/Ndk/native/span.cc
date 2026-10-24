#include <stdint.h>
#include <string>

DROIDE_EXPORT int32_t span(int32_t n) {
    std::string text = "ndk";
    return n + (int32_t)text.size();
}
