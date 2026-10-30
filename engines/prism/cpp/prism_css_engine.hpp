// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Native C++20 CSS Engine Kernels (Prism Engine).
// Linked directly into the Rust core (`anvil-style`) as `prism_css_engine`.

#ifndef PRISM_CSS_ENGINE_HPP
#define PRISM_CSS_ENGINE_HPP

#include <stddef.h>
#include <stdint.h>

extern "C" {

// Compute a 64-bit ASCII-lowercased FNV-1a bloom hash for selector class/id/tag matching.
uint64_t prism_css_bloom_hash(const uint8_t* data, size_t len);

// Scan past leading CSS whitespace and `/* ... */` block comments, returning the byte offset.
size_t prism_css_skip_whitespace_and_comments(const uint8_t* data, size_t len);

// Mix two packed RGBA8 colors (`0xRRGGBBAA`) in sRGB space with weight `w` in [0.0, 1.0].
uint32_t prism_css_mix_srgb_rgba8(uint32_t rgba_a, uint32_t rgba_b, float w);

// Classify a selector trailing pseudo-element token:
//   0 = none, 1 = ::before, 2 = ::after, 3 = ::first-line, 4 = ::first-letter,
//   5 = ::placeholder, 6 = ::selection, 7 = ::marker, 8 = ::backdrop.
uint32_t prism_css_classify_pseudo_element(const uint8_t* data, size_t len);

// Resolve a CSS Values & Units Level 4 dimension into pixels:
//   unit_code: 0=px, 1=em, 2=rem, 3=%, 4=vw, 5=vh, 6=vmin, 7=vmax,
//              8=ch, 9=ex, 10=pt, 11=pc, 12=cm, 13=mm, 14=in, 15=q.
float prism_css_resolve_length_px(
    float value,
    uint32_t unit_code,
    float font_size_px,
    float root_font_size_px,
    float viewport_w,
    float viewport_h,
    float container_px
);

// Evaluate CSS `clamp(min_px, preferred_px, max_px)` per CSS Values Level 4.
float prism_css_eval_clamp(float min_px, float preferred_px, float max_px);

// Sample a 2-stop radial gradient at normalized UV coordinates in [0.0, 1.0].
uint32_t prism_css_sample_radial_gradient(
    float norm_x,
    float norm_y,
    uint32_t inner_rgba,
    uint32_t outer_rgba
);

// Pack Selectors Level 4 specificity (ids, classes/attrs/pseudo-classes, elements/pseudo-elements).
uint32_t prism_css_pack_specificity(
    uint32_t id_count,
    uint32_t class_count,
    uint32_t element_count
);

} // extern "C"

#endif // PRISM_CSS_ENGINE_HPP
