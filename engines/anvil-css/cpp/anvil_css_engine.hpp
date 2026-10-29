// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Native C++20 CSS Engine Kernels (Anvil-CSS Engine).
// Linked directly into the Rust core (`anvil-style`) as `anvil_css_engine`.

#ifndef ANVIL_CSS_ENGINE_HPP
#define ANVIL_CSS_ENGINE_HPP

#include <stddef.h>
#include <stdint.h>

extern "C" {

// Compute a 64-bit ASCII-lowercased FNV-1a bloom hash for selector class/id/tag matching.
uint64_t anvil_css_bloom_hash(const uint8_t* data, size_t len);

// Scan past leading CSS whitespace and `/* ... */` block comments, returning the byte offset.
size_t anvil_css_skip_whitespace_and_comments(const uint8_t* data, size_t len);

// Mix two packed RGBA8 colors (`0xRRGGBBAA`) in sRGB space with weight `w` in [0.0, 1.0].
uint32_t anvil_css_mix_srgb_rgba8(uint32_t rgba_a, uint32_t rgba_b, float w);

} // extern "C"

#endif // ANVIL_CSS_ENGINE_HPP
