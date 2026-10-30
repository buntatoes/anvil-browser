// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Native C++20 CSS Engine Kernels (Prism Engine).
// Linked directly into the Rust core (`anvil-style`) as `prism_css_engine`.

#include "prism_css_engine.hpp"
#include <math.h>

extern "C" {

uint64_t prism_css_bloom_hash(const uint8_t* data, size_t len) {
    if (!data) {
        return 0ULL;
    }
    uint64_t hash = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < len; ++i) {
        uint8_t b = data[i];
        if (b >= 'A' && b <= 'Z') {
            b = static_cast<uint8_t>(b + 32);
        }
        hash ^= static_cast<uint64_t>(b);
        hash *= 0x100000001b3ULL;
    }
    return hash;
}

size_t prism_css_skip_whitespace_and_comments(const uint8_t* data, size_t len) {
    if (!data) {
        return 0;
    }
    size_t i = 0;
    while (i < len) {
        uint8_t c = data[i];
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f') {
            ++i;
            continue;
        }
        if (c == '/' && (i + 1) < len && data[i + 1] == '*') {
            i += 2;
            while ((i + 1) < len && !(data[i] == '*' && data[i + 1] == '/')) {
                ++i;
            }
            i = ((i + 1) < len) ? (i + 2) : len;
            continue;
        }
        break;
    }
    return i;
}

static inline uint8_t clamp_u8(float v) {
    if (v <= 0.0f) return 0;
    if (v >= 255.0f) return 255;
    return static_cast<uint8_t>(v + 0.5f);
}

uint32_t prism_css_mix_srgb_rgba8(uint32_t rgba_a, uint32_t rgba_b, float w) {
    if (w < 0.0f) w = 0.0f;
    if (w > 1.0f) w = 1.0f;
    const float inv = 1.0f - w;
    const float r1 = static_cast<float>((rgba_a >> 24) & 0xFFu);
    const float g1 = static_cast<float>((rgba_a >> 16) & 0xFFu);
    const float b1 = static_cast<float>((rgba_a >> 8) & 0xFFu);
    const float a1 = static_cast<float>(rgba_a & 0xFFu);

    const float r2 = static_cast<float>((rgba_b >> 24) & 0xFFu);
    const float g2 = static_cast<float>((rgba_b >> 16) & 0xFFu);
    const float b2 = static_cast<float>((rgba_b >> 8) & 0xFFu);
    const float a2 = static_cast<float>(rgba_b & 0xFFu);

    const uint32_t r = clamp_u8(r1 * inv + r2 * w);
    const uint32_t g = clamp_u8(g1 * inv + g2 * w);
    const uint32_t b = clamp_u8(b1 * inv + b2 * w);
    const uint32_t a = clamp_u8(a1 * inv + a2 * w);
    return (r << 24) | (g << 16) | (b << 8) | a;
}

static bool ascii_eq_ignore_case(const uint8_t* a, size_t len, const char* lit, size_t lit_len) {
    if (len != lit_len) {
        return false;
    }
    for (size_t i = 0; i < len; ++i) {
        uint8_t c = a[i];
        if (c >= 'A' && c <= 'Z') {
            c = static_cast<uint8_t>(c + 32);
        }
        if (c != static_cast<uint8_t>(lit[i])) {
            return false;
        }
    }
    return true;
}

uint32_t prism_css_classify_pseudo_element(const uint8_t* data, size_t len) {
    if (!data || len == 0) {
        return 0;
    }
    while (len > 0 && (data[len - 1] == ' ' || data[len - 1] == '\t' || data[len - 1] == '\n' || data[len - 1] == '\r')) {
        --len;
    }
    size_t colon_pos = len;
    while (colon_pos > 0 && data[colon_pos - 1] != ':') {
        --colon_pos;
    }
    if (colon_pos == 0) {
        return 0;
    }
    const uint8_t* ident = data + colon_pos;
    const size_t ident_len = len - colon_pos;
    if (ascii_eq_ignore_case(ident, ident_len, "before", 6)) return 1;
    if (ascii_eq_ignore_case(ident, ident_len, "after", 5)) return 2;
    if (ascii_eq_ignore_case(ident, ident_len, "first-line", 10)) return 3;
    if (ascii_eq_ignore_case(ident, ident_len, "first-letter", 12)) return 4;
    if (ascii_eq_ignore_case(ident, ident_len, "placeholder", 11)) return 5;
    if (ascii_eq_ignore_case(ident, ident_len, "selection", 9)) return 6;
    if (ascii_eq_ignore_case(ident, ident_len, "marker", 6)) return 7;
    if (ascii_eq_ignore_case(ident, ident_len, "backdrop", 8)) return 8;
    return 0;
}

float prism_css_resolve_length_px(
    float value,
    uint32_t unit_code,
    float font_size_px,
    float root_font_size_px,
    float viewport_w,
    float viewport_h,
    float container_px
) {
    switch (unit_code) {
        case 0: // px
            return value;
        case 1: // em
            return value * font_size_px;
        case 2: // rem
            return value * root_font_size_px;
        case 3: // %
            return (value * 0.01f) * container_px;
        case 4: // vw
            return (value * 0.01f) * viewport_w;
        case 5: // vh
            return (value * 0.01f) * viewport_h;
        case 6: { // vmin
            const float vmin = (viewport_w < viewport_h) ? viewport_w : viewport_h;
            return (value * 0.01f) * vmin;
        }
        case 7: { // vmax
            const float vmax = (viewport_w > viewport_h) ? viewport_w : viewport_h;
            return (value * 0.01f) * vmax;
        }
        case 8: // ch
        case 9: // ex
            return value * (font_size_px * 0.5f);
        case 10: // pt
            return value * (96.0f / 72.0f);
        case 11: // pc
            return value * 16.0f;
        case 12: // cm
            return value * (96.0f / 2.54f);
        case 13: // mm
            return value * (96.0f / 25.4f);
        case 14: // in
            return value * 96.0f;
        case 15: // q
            return value * (96.0f / 101.6f);
        default:
            return value;
    }
}

float prism_css_eval_clamp(float min_px, float preferred_px, float max_px) {
    float clamped = (preferred_px < max_px) ? preferred_px : max_px;
    if (clamped < min_px) {
        clamped = min_px;
    }
    return clamped;
}

uint32_t prism_css_sample_radial_gradient(
    float norm_x,
    float norm_y,
    uint32_t inner_rgba,
    uint32_t outer_rgba
) {
    const float dx = (norm_x - 0.5f) * 2.0f;
    const float dy = (norm_y - 0.5f) * 2.0f;
    float dist = sqrtf(dx * dx + dy * dy);
    if (dist < 0.0f) dist = 0.0f;
    if (dist > 1.0f) dist = 1.0f;
    return prism_css_mix_srgb_rgba8(inner_rgba, outer_rgba, dist);
}

uint32_t prism_css_pack_specificity(
    uint32_t id_count,
    uint32_t class_count,
    uint32_t element_count
) {
    const uint32_t a = (id_count > 1023u) ? 1023u : id_count;
    const uint32_t b = (class_count > 1023u) ? 1023u : class_count;
    const uint32_t c = (element_count > 1023u) ? 1023u : element_count;
    return (a << 20) | (b << 10) | c;
}

} // extern "C"
