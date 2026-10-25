// Décode une mosaïque RAW via LibRaw, sans dématriçage.
// Le retournement du boîtier est renvoyé tel quel : Rust l'applique.

#define LIBRAW_NODLL
#include "libraw/libraw.h"

#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

enum {
    MOSAIC_OK = 0,
    MOSAIC_OPEN = 1,
    MOSAIC_UNPACK = 2,
    MOSAIC_UNSUPPORTED = 3,
    MOSAIC_NOMEM = 4,
};

enum {
    THUMB_NONE = 0,
    THUMB_JPEG = 1,
    THUMB_RGB = 2,
    THUMB_GRAY = 3,
};

enum {
    BAYER_MONO = -1,
};

struct MosaicBuffer {
    int width;
    int height;
    int bayer;
    int flip;
    int status;
    float* pixels;
    unsigned char* thumb;
    int thumb_len;
    int thumb_w;
    int thumb_h;
    int thumb_kind;
    char message[192];
};

static void fail(MosaicBuffer* out, int status, const char* text) {
    out->status = status;
    if (text) {
        std::snprintf(out->message, sizeof(out->message), "%s", text);
    }
}

static int pattern_code(int c00, int c01, int c10) {
    auto green = [](int color) { return color == 1 || color == 3; };
    if (c00 == 0 && green(c01) && green(c10)) {
        return 1;
    }
    if (green(c00) && c01 == 0 && c10 == 2) {
        return 2;
    }
    if (green(c00) && c01 == 2 && c10 == 0) {
        return 3;
    }
    if (c00 == 2 && green(c01) && green(c10)) {
        return 0;
    }
    return -2;
}

static int black_at(LibRaw& raw, int row, int col, int top, int left) {
    const libraw_colordata_t& color = raw.imgdata.color;
    int channel = raw.COLOR(row, col);
    int black = static_cast<int>(color.black);
    if (channel >= 0 && channel < 4) {
        black += static_cast<int>(color.cblack[channel]);
    }
    if (color.cblack[4] && color.cblack[5] && row >= top && col >= left) {
        unsigned rr = static_cast<unsigned>(row - top);
        unsigned cc = static_cast<unsigned>(col - left);
        unsigned index = 6 + (rr % color.cblack[4]) * color.cblack[5] + (cc % color.cblack[5]);
        if (index < LIBRAW_CBLACK_SIZE) {
            black += static_cast<int>(color.cblack[index]);
        }
    }
    return black;
}

static void copy_thumb(LibRaw& raw, MosaicBuffer* out) {
    if (raw.unpack_thumb() != 0) {
        return;
    }
    const libraw_thumbnail_t& thumb = raw.imgdata.thumbnail;
    if (!thumb.thumb || thumb.tlength == 0) {
        return;
    }
    if (thumb.tformat == LIBRAW_THUMBNAIL_JPEG) {
        out->thumb = static_cast<unsigned char*>(std::malloc(thumb.tlength));
        if (!out->thumb) {
            return;
        }
        std::memcpy(out->thumb, thumb.thumb, thumb.tlength);
        out->thumb_len = static_cast<int>(thumb.tlength);
        out->thumb_w = thumb.twidth;
        out->thumb_h = thumb.theight;
        out->thumb_kind = THUMB_JPEG;
        return;
    }
    if (thumb.tformat != LIBRAW_THUMBNAIL_BITMAP && thumb.tformat != LIBRAW_THUMBNAIL_BITMAP16) {
        return;
    }
    int colors = thumb.tcolors == 1 ? 1 : 3;
    bool wide = thumb.tformat == LIBRAW_THUMBNAIL_BITMAP16;
    size_t pixels = static_cast<size_t>(thumb.twidth) * static_cast<size_t>(thumb.theight);
    size_t need = pixels * static_cast<size_t>(colors) * (wide ? 2 : 1);
    if (thumb.twidth == 0 || thumb.theight == 0 || need > thumb.tlength) {
        return;
    }
    out->thumb = static_cast<unsigned char*>(std::malloc(pixels * static_cast<size_t>(colors)));
    if (!out->thumb) {
        return;
    }
    if (!wide) {
        if (colors == 1) {
            std::memcpy(out->thumb, thumb.thumb, pixels);
        } else {
            std::memcpy(out->thumb, thumb.thumb, pixels * 3);
        }
    } else {
        const auto* src = reinterpret_cast<const std::uint16_t*>(thumb.thumb);
        for (size_t i = 0; i < pixels * static_cast<size_t>(colors); i++) {
            out->thumb[i] = static_cast<unsigned char>(src[i] >> 8);
        }
    }
    out->thumb_len = static_cast<int>(pixels * static_cast<size_t>(colors));
    out->thumb_w = thumb.twidth;
    out->thumb_h = thumb.theight;
    out->thumb_kind = colors == 1 ? THUMB_GRAY : THUMB_RGB;
}

static int extract_mosaic(LibRaw& raw, MosaicBuffer* out) {
    if (raw.is_sraw() || raw.is_fuji_rotated() || raw.is_floating_point()) {
        fail(out, MOSAIC_UNSUPPORTED, "mosaique non rectangulaire");
        return out->status;
    }
    unsigned filters = raw.imgdata.idata.filters;
    int colors = raw.imgdata.idata.colors;
    bool mono = filters == 0 && colors <= 1;
    if (!mono && filters < 1000) {
        fail(out, MOSAIC_UNSUPPORTED, "motif non Bayer");
        return out->status;
    }

    int rc = raw.unpack();
    if (rc != 0) {
        fail(out, MOSAIC_UNPACK, LibRaw::strerror(rc));
        return out->status;
    }
    const unsigned short* image = raw.imgdata.rawdata.raw_image;
    if (!image) {
        fail(out, MOSAIC_UNSUPPORTED, "pas de mosaïque");
        return out->status;
    }

    const libraw_image_sizes_t& sizes = raw.imgdata.sizes;
    int left = sizes.left_margin;
    int top = sizes.top_margin;
    int width = sizes.width > 0 ? sizes.width : static_cast<int>(sizes.raw_width) - left;
    int height = sizes.height > 0 ? sizes.height : static_cast<int>(sizes.raw_height) - top;
    if (width > 1 && (width % 2) != 0) {
        width -= 1;
    }
    if (height > 1 && (height % 2) != 0) {
        height -= 1;
    }
    if (width < 2 || height < 2 || left < 0 || top < 0 || left + width > sizes.raw_width || top + height > sizes.raw_height) {
        fail(out, MOSAIC_UNSUPPORTED, "cadre brut invalide");
        return out->status;
    }

    int bayer = BAYER_MONO;
    if (!mono) {
        bayer = pattern_code(raw.COLOR(top, left), raw.COLOR(top, left + 1), raw.COLOR(top + 1, left));
        if (bayer < 0) {
            fail(out, MOSAIC_UNSUPPORTED, "motif Bayer illisible");
            return out->status;
        }
    }

    int pitch = static_cast<int>(sizes.raw_pitch);
    if (pitch < sizes.raw_width * static_cast<int>(sizeof(unsigned short))) {
        pitch = sizes.raw_width * static_cast<int>(sizeof(unsigned short));
    }
    const auto* base = reinterpret_cast<const unsigned char*>(image);
    int white = static_cast<int>(raw.imgdata.color.maximum);
    if (white <= 0) {
        unsigned bits = raw.imgdata.color.raw_bps;
        white = (bits >= 8 && bits <= 16) ? (1 << bits) - 1 : 65535;
    }

    std::vector<float> pixels(static_cast<size_t>(width) * static_cast<size_t>(height));
    for (int row = 0; row < height; row++) {
        const auto* line = reinterpret_cast<const unsigned short*>(base + static_cast<size_t>(top + row) * static_cast<size_t>(pitch));
        for (int col = 0; col < width; col++) {
            int black = black_at(raw, top + row, left + col, top, left);
            int span = white - black;
            if (span < 1) {
                span = 1;
            }
            float value = (static_cast<int>(line[left + col]) - black) * (65535.0f / static_cast<float>(span));
            if (value < 0.0f) {
                value = 0.0f;
            }
            if (value > 65535.0f) {
                value = 65535.0f;
            }
            pixels[static_cast<size_t>(row) * static_cast<size_t>(width) + static_cast<size_t>(col)] = value;
        }
    }

    out->pixels = static_cast<float*>(std::malloc(pixels.size() * sizeof(float)));
    if (!out->pixels) {
        fail(out, MOSAIC_NOMEM, "mémoire");
        return out->status;
    }
    std::memcpy(out->pixels, pixels.data(), pixels.size() * sizeof(float));
    out->width = width;
    out->height = height;
    out->bayer = bayer;
    out->flip = sizes.flip;
    out->status = MOSAIC_OK;
    return MOSAIC_OK;
}

template <typename Path>
static int decode_path(const Path* path, int full, MosaicBuffer* out) {
    std::memset(out, 0, sizeof(*out));
    out->bayer = BAYER_MONO;
    if (!path) {
        fail(out, MOSAIC_OPEN, "chemin vide");
        return out->status;
    }
    try {
        LibRaw raw;
        int rc = raw.open_file(path);
        if (rc != 0) {
            fail(out, MOSAIC_OPEN, LibRaw::strerror(rc));
            return out->status;
        }
        if (!full) {
            copy_thumb(raw, out);
            if (out->thumb_kind == THUMB_NONE) {
                fail(out, MOSAIC_OPEN, "miniature absente");
                return out->status;
            }
            out->status = MOSAIC_OK;
            return MOSAIC_OK;
        }
        return extract_mosaic(raw, out);
    } catch (const std::bad_alloc&) {
        fail(out, MOSAIC_NOMEM, "mémoire");
        return out->status;
    } catch (...) {
        fail(out, MOSAIC_UNPACK, "exception");
        return out->status;
    }
}

extern "C" {

int mosaic_decode_utf8(const char* path, int full, MosaicBuffer* out) {
    return decode_path(path, full, out);
}

#if defined(_WIN32)
int mosaic_decode_utf16(const wchar_t* path, int full, MosaicBuffer* out) {
    return decode_path(path, full, out);
}
#endif

void mosaic_free(MosaicBuffer* out) {
    if (!out) {
        return;
    }
    std::free(out->pixels);
    std::free(out->thumb);
    out->pixels = nullptr;
    out->thumb = nullptr;
    out->thumb_len = 0;
}

}
