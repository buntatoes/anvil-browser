// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Native C++20 CSS Engine Kernels (Anvil-CSS Engine).
// Linked directly into the Rust core (`anvil-style`) as `anvil_css_engine`.

#include "anvil_css_engine.hpp"

extern "C" {

uint64_t anvil_css_bloom_hash(const uint8_t* data, size_t len) {
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

size_t anvil_css_skip_whitespace_and_comments(const uint8_t* data, size_t len) {
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

uint32_t anvil_css_mix_srgb_rgba8(uint32_t rgba_a, uint32_t rgba_b, float w) {
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

} // extern "C"
