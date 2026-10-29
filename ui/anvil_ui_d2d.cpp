// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Native C++20 Direct2D + DirectWrite + DWM UI Chrome & Overlay Renderer.

#include "anvil_core_abi.h"

#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <d2d1.h>
#include <dwrite.h>
#include <dwmapi.h>
#include <math.h>

namespace {

static inline D2D1_COLOR_F to_d2d_color(AnvilUiColor c) {
    return D2D1::ColorF(
        static_cast<float>(c.r) / 255.0f,
        static_cast<float>(c.g) / 255.0f,
        static_cast<float>(c.b) / 255.0f,
        static_cast<float>(c.a) / 255.0f
    );
}

static inline D2D1_RECT_F to_d2d_rect(AnvilUiRect r) {
    return D2D1::RectF(r.x, r.y, r.x + r.w, r.y + r.h);
}

static int utf8_to_wide(const char* src, size_t len, wchar_t* dst, int dst_cap) {
    if (!src || len == 0 || dst_cap <= 1) {
        if (dst && dst_cap > 0) dst[0] = L'\0';
        return 0;
    }
    int n = MultiByteToWideChar(
        CP_UTF8,
        0,
        src,
        static_cast<int>(len),
        dst,
        dst_cap - 1
    );
    if (n < 0) n = 0;
    dst[n] = L'\0';
    return n;
}

struct D2dChromeRenderer {
    ID2D1Factory* d2d_factory = nullptr;
    IDWriteFactory* dwrite_factory = nullptr;
    IDWriteTextFormat* fmt_ui_regular = nullptr;
    IDWriteTextFormat* fmt_ui_semibold = nullptr;
    IDWriteTextFormat* fmt_ui_small = nullptr;
    IDWriteTextFormat* fmt_ui_badge = nullptr;
    ID2D1DCRenderTarget* dc_target = nullptr;
    ID2D1SolidColorBrush* brush = nullptr;

    HDC mem_dc = nullptr;
    HBITMAP dib_bitmap = nullptr;
    HGDIOBJ old_bitmap = nullptr;
    uint8_t* dib_bgra = nullptr;
    uint32_t dib_w = 0;
    uint32_t dib_h = 0;
    bool initialized = false;
    bool init_failed = false;

    bool ensure_init() {
        if (initialized) return true;
        if (init_failed) return false;

        HRESULT hr = D2D1CreateFactory(
            D2D1_FACTORY_TYPE_SINGLE_THREADED,
            __uuidof(ID2D1Factory),
            nullptr,
            reinterpret_cast<void**>(&d2d_factory)
        );
        if (FAILED(hr) || !d2d_factory) {
            init_failed = true;
            return false;
        }

        hr = DWriteCreateFactory(
            DWRITE_FACTORY_TYPE_SHARED,
            __uuidof(IDWriteFactory),
            reinterpret_cast<IUnknown**>(&dwrite_factory)
        );
        if (FAILED(hr) || !dwrite_factory) {
            init_failed = true;
            return false;
        }

        dwrite_factory->CreateTextFormat(
            L"Segoe UI",
            nullptr,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            12.5f,
            L"en-us",
            &fmt_ui_regular
        );
        dwrite_factory->CreateTextFormat(
            L"Segoe UI",
            nullptr,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            12.5f,
            L"en-us",
            &fmt_ui_semibold
        );
        dwrite_factory->CreateTextFormat(
            L"Segoe UI",
            nullptr,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            11.5f,
            L"en-us",
            &fmt_ui_small
        );
        dwrite_factory->CreateTextFormat(
            L"Segoe UI",
            nullptr,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            10.5f,
            L"en-us",
            &fmt_ui_badge
        );

        if (!fmt_ui_regular || !fmt_ui_semibold || !fmt_ui_small || !fmt_ui_badge) {
            init_failed = true;
            return false;
        }

        IDWriteInlineObject* ellipsis = nullptr;
        DWRITE_TRIMMING trimming = { DWRITE_TRIMMING_GRANULARITY_CHARACTER, 0, 0 };
        dwrite_factory->CreateEllipsisTrimmingSign(fmt_ui_regular, &ellipsis);
        for (IDWriteTextFormat* f : { fmt_ui_regular, fmt_ui_semibold, fmt_ui_small, fmt_ui_badge }) {
            f->SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            f->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            f->SetTrimming(&trimming, ellipsis);
        }
        if (ellipsis) ellipsis->Release();

        D2D1_RENDER_TARGET_PROPERTIES props = D2D1::RenderTargetProperties(
            D2D1_RENDER_TARGET_TYPE_DEFAULT,
            D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_IGNORE),
            96.0f,
            96.0f,
            D2D1_RENDER_TARGET_USAGE_NONE,
            D2D1_FEATURE_LEVEL_DEFAULT
        );
        hr = d2d_factory->CreateDCRenderTarget(&props, &dc_target);
        if (FAILED(hr) || !dc_target) {
            init_failed = true;
            return false;
        }
        dc_target->SetAntiialiasMode(D2D1_ANTI_ALIAS_MODE_PER_PRIMITIVE);
        dc_target->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);

        hr = dc_target->CreateSolidColorBrush(D2D1::ColorF(1.0f, 1.0f, 1.0f, 1.0f), &brush);
        if (FAILED(hr) || !brush) {
            init_failed = true;
            return false;
        }

        initialized = true;
        return true;
    }

    bool ensure_surface(uint32_t w, uint32_t h) {
        if (!ensure_init()) return false;
        if (mem_dc && dib_bitmap && dib_bgra && dib_w == w && dib_h == h) {
            return true;
        }
        if (old_bitmap && mem_dc) {
            SelectObject(mem_dc, old_bitmap);
            old_bitmap = nullptr;
        }
        if (dib_bitmap) {
            DeleteObject(dib_bitmap);
            dib_bitmap = nullptr;
            dib_bgra = nullptr;
        }
        if (!mem_dc) {
            mem_dc = CreateCompatibleDC(nullptr);
            if (!mem_dc) return false;
        }

        BITMAPINFO bmi = {};
        bmi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
        bmi.bmiHeader.biWidth = static_cast<LONG>(w);
        bmi.bmiHeader.biHeight = -static_cast<LONG>(h); // top-down DIB
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        void* bits = nullptr;
        dib_bitmap = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &bits, nullptr, 0);
        if (!dib_bitmap || !bits) return false;

        old_bitmap = SelectObject(mem_dc, dib_bitmap);
        dib_bgra = static_cast<uint8_t*>(bits);
        dib_w = w;
        dib_h = h;
        return true;
    }

    void copy_rows_rgba_to_bgra(const uint8_t* rgba, uint32_t w, uint32_t y0, uint32_t y1) {
        for (uint32_t y = y0; y < y1; ++y) {
            const size_t row_off = static_cast<size_t>(y) * static_cast<size_t>(w) * 4u;
            const uint8_t* src = rgba + row_off;
            uint8_t* dst = dib_bgra + row_off;
            for (uint32_t x = 0; x < w; ++x) {
                dst[x * 4 + 0] = src[x * 4 + 2];
                dst[x * 4 + 1] = src[x * 4 + 1];
                dst[x * 4 + 2] = src[x * 4 + 0];
                dst[x * 4 + 3] = 255;
            }
        }
    }

    void copy_rows_bgra_to_rgba(uint8_t* rgba, uint32_t w, uint32_t y0, uint32_t y1) {
        for (uint32_t y = y0; y < y1; ++y) {
            const size_t row_off = static_cast<size_t>(y) * static_cast<size_t>(w) * 4u;
            const uint8_t* src = dib_bgra + row_off;
            uint8_t* dst = rgba + row_off;
            for (uint32_t x = 0; x < w; ++x) {
                dst[x * 4 + 0] = src[x * 4 + 2];
                dst[x * 4 + 1] = src[x * 4 + 1];
                dst[x * 4 + 2] = src[x * 4 + 0];
                dst[x * 4 + 3] = 255;
            }
        }
    }

    void fill_rect(D2D1_RECT_F r, AnvilUiColor col) {
        brush->SetColor(to_d2d_color(col));
        dc_target->FillRectangle(r, brush);
    }

    void fill_rounded(AnvilUiRect r, float radius, float border_w, AnvilUiColor fill, AnvilUiColor border) {
        D2D1_ROUNDED_RECT rr = D2D1::RoundedRect(to_d2d_rect(r), radius, radius);
        brush->SetColor(to_d2d_color(fill));
        dc_target->FillRoundedRectangle(rr, brush);
        if (border_w > 0.0f) {
            brush->SetColor(to_d2d_color(border));
            dc_target->DrawRoundedRectangle(rr, brush, border_w);
        }
    }

    void draw_shadow_card(AnvilUiRect r, float radius, AnvilUiColor fill, AnvilUiColor border) {
        for (int i = 4; i >= 1; --i) {
            float expand = static_cast<float>(i) * 2.2f;
            uint8_t alpha = static_cast<uint8_t>(28 * (5 - i));
            AnvilUiRect sr = {
                r.x - expand * 0.5f,
                r.y + static_cast<float>(i) * 1.6f,
                r.w + expand,
                r.h + expand
            };
            D2D1_ROUNDED_RECT srr = D2D1::RoundedRect(to_d2d_rect(sr), radius + expand * 0.4f, radius + expand * 0.4f);
            brush->SetColor(D2D1::ColorF(0.0f, 0.0f, 0.0f, static_cast<float>(alpha) / 255.0f));
            dc_target->FillRoundedRectangle(srr, brush);
        }
        fill_rounded(r, radius, 1.0f, fill, border);
    }

    void draw_utf8_text(
        const char* text,
        size_t len,
        AnvilUiRect r,
        IDWriteTextFormat* fmt,
        AnvilUiColor col,
        DWRITE_TEXT_ALIGNMENT align = DWRITE_TEXT_ALIGNMENT_LEADING
    ) {
        if (!text || len == 0 || r.w <= 1.0f) return;
        wchar_t wbuf[512];
        int wlen = utf8_to_wide(text, len, wbuf, 512);
        if (wlen <= 0) return;
        fmt->SetTextAlignment(align);
        brush->SetColor(to_d2d_color(col));
        dc_target->DrawText(
            wbuf,
            static_cast<UINT32>(wlen),
            fmt,
            to_d2d_rect(r),
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP
        );
        fmt->SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
    }

    float measure_and_draw_segment(
        const char* text,
        size_t len,
        float x,
        float y,
        float max_w,
        float h,
        IDWriteTextFormat* fmt,
        AnvilUiColor col
    ) {
        if (!text || len == 0 || max_w <= 1.0f) return 0.0f;
        wchar_t wbuf[512];
        int wlen = utf8_to_wide(text, len, wbuf, 512);
        if (wlen <= 0) return 0.0f;

        IDWriteTextLayout* layout = nullptr;
        HRESULT hr = dwrite_factory->CreateTextLayout(
            wbuf,
            static_cast<UINT32>(wlen),
            fmt,
            max_w,
            h,
            &layout
        );
        if (FAILED(hr) || !layout) return 0.0f;

        DWRITE_TEXT_METRICS metrics = {};
        layout->GetMetrics(&metrics);
        brush->SetColor(to_d2d_color(col));
        dc_target->DrawTextLayout(
            D2D1::Point2F(x, y),
            layout,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP
        );
        layout->Release();
        return metrics.widthIncludingTrailingWhitespace;
    }

    void draw_line(float x1, float y1, float x2, float y2, float stroke, AnvilUiColor col) {
        brush->SetColor(to_d2d_color(col));
        dc_target->DrawLine(D2D1::Point2F(x1, y1), D2D1::Point2F(x2, y2), brush, stroke);
    }

    void draw_close_x(AnvilUiRect r, float half_sz, float stroke, AnvilUiColor col) {
        float cx = r.x + r.w * 0.5f;
        float cy = r.y + r.h * 0.5f;
        draw_line(cx - half_sz, cy - half_sz, cx + half_sz, cy + half_sz, stroke, col);
        draw_line(cx + half_sz, cy - half_sz, cx - half_sz, cy + half_sz, stroke, col);
    }

    void draw_plus(AnvilUiRect r, float half_sz, float stroke, AnvilUiColor col) {
        float cx = r.x + r.w * 0.5f;
        float cy = r.y + r.h * 0.5f;
        draw_line(cx - half_sz, cy, cx + half_sz, cy, stroke, col);
        draw_line(cx, cy - half_sz, cx, cy + half_sz, stroke, col);
    }

    void draw_nav_button_shell(AnvilUiRect r, uint8_t enabled, uint8_t hovered, uint8_t active, const AnvilUiChromeState* s, AnvilUiColor* out_fg) {
        float radius = r.h * 0.5f;
        if (active) {
            fill_rounded(r, radius, 1.0f, s->control_active_bg, s->accent);
            *out_fg = s->accent;
        } else if (enabled && hovered) {
            fill_rounded(r, radius, 1.0f, s->control_hover_bg, s->divider);
            *out_fg = s->ink_primary;
        } else {
            *out_fg = enabled ? s->ink_primary : s->ink_dim;
        }
    }
};

static D2dChromeRenderer g_d2d;

} // namespace

extern "C" void anvil_ui_dwm_configure_window(void* hwnd_ptr, int dark_mode, int frameless) {
    HWND hwnd = static_cast<HWND>(hwnd_ptr);
    if (!hwnd) return;

    BOOL dark = dark_mode ? TRUE : FALSE;
    DwmSetWindowAttribute(hwnd, 20 /* DWMWA_USE_IMMERSIVE_DARK_MODE */, &dark, sizeof(dark));

    // Windows 11 rounded corners (DWMWA_WINDOW_CORNER_PREFERENCE = 33, DWMWCP_ROUND = 2)
    DWORD corner_pref = 2;
    DwmSetWindowAttribute(hwnd, 33, &corner_pref, sizeof(corner_pref));

    if (frameless) {
        MARGINS margins = { 0, 0, 1, 0 };
        DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}

extern "C" int anvil_ui_render_top_chrome_d2d(
    uint8_t* rgba_pixels,
    uint32_t width,
    uint32_t height,
    const AnvilUiChromeState* s
) {
    if (!rgba_pixels || !s || width == 0 || height == 0) return 0;
    uint32_t header_h = static_cast<uint32_t>(s->top_header_height + 0.5f);
    if (header_h == 0 || header_h > height) header_h = height;

    if (!g_d2d.ensure_surface(width, height)) return 0;

    // Copy existing top header rows (preserving the rendered Anvil brand icon) into DIB
    g_d2d.copy_rows_rgba_to_bgra(rgba_pixels, width, 0, header_h);

    RECT bind_rc = { 0, 0, static_cast<LONG>(width), static_cast<LONG>(header_h) };
    if (FAILED(g_d2d.dc_target->BindDC(g_d2d.mem_dc, &bind_rc))) {
        return 0;
    }

    g_d2d.dc_target->BeginDraw();

    // 1. Tab Strip Background (keep 8..32 x 7..31 logo visible by painting strip around/behind it)
    g_d2d.fill_rect(D2D1::RectF(36.0f, 0.0f, s->window_width, s->tab_strip_height), s->strip_bg);
    g_d2d.fill_rect(D2D1::RectF(0.0f, 0.0f, 36.0f, 6.0f), s->strip_bg);
    g_d2d.fill_rect(D2D1::RectF(0.0f, 32.0f, 36.0f, s->tab_strip_height), s->strip_bg);
    g_d2d.fill_rect(D2D1::RectF(0.0f, 6.0f, 7.0f, 32.0f), s->strip_bg);
    g_d2d.fill_rect(D2D1::RectF(33.0f, 6.0f, 36.0f, 32.0f), s->strip_bg);

    // 2. Modern Curved Tabs
    for (size_t i = 0; i < s->tab_count && i < ANVIL_UI_MAX_TABS; ++i) {
        const AnvilUiTabInfo& tab = s->tabs[i];
        AnvilUiColor bg = tab.is_active ? s->nav_top_bg : (tab.is_hovered ? s->control_bg : s->tab_idle_bg);
        AnvilUiColor fg = (tab.is_active || tab.is_hovered) ? s->ink_primary : s->ink_muted;
        AnvilUiColor border = tab.is_active ? s->divider : (tab.is_hovered ? s->divider : s->strip_bg);

        if (tab.is_active) {
            AnvilUiRect r_ext = { tab.rect.x, tab.rect.y, tab.rect.w, tab.rect.h + 4.0f };
            g_d2d.fill_rounded(r_ext, 8.0f, 1.0f, bg, border);
            // Seamless bottom join with nav bar
            g_d2d.fill_rect(
                D2D1::RectF(tab.rect.x + 1.0f, s->tab_strip_height - 2.0f, tab.rect.x + tab.rect.w - 1.0f, s->tab_strip_height + 2.0f),
                s->nav_top_bg
            );
            // Top active accent pill
            AnvilUiRect bar = { tab.rect.x + 12.0f, tab.rect.y + 1.0f, (tab.rect.w - 24.0f > 8.0f ? tab.rect.w - 24.0f : 8.0f), 2.5f };
            g_d2d.fill_rounded(bar, 1.25f, 0.0f, s->accent, s->accent);
        } else {
            AnvilUiRect r_idle = { tab.rect.x, tab.rect.y + 2.0f, tab.rect.w, tab.rect.h - 4.0f };
            g_d2d.fill_rounded(r_idle, 7.0f, 0.0f, bg, border);
            // Subtle vertical separator between inactive tabs
            if (i + 1 < s->tab_count && !s->tabs[i + 1].is_active) {
                float sep_x = tab.rect.x + tab.rect.w + 2.0f;
                g_d2d.draw_line(sep_x, tab.rect.y + 8.0f, sep_x, tab.rect.y + tab.rect.h - 8.0f, 1.0f, s->divider);
            }
        }

        // Security protocol dot
        float dot_cx = tab.rect.x + 13.0f;
        float dot_cy = tab.rect.y + tab.rect.h * 0.5f;
        g_d2d.brush->SetColor(to_d2d_color(tab.dot_color));
        g_d2d.dc_target->FillEllipse(D2D1::Ellipse(D2D1::Point2F(dot_cx, dot_cy), 3.2f, 3.2f), g_d2d.brush);

        // Tab title via DirectWrite
        float text_right_pad = tab.can_close ? 28.0f : 10.0f;
        AnvilUiRect text_rc = {
            tab.rect.x + 22.0f,
            tab.rect.y + 1.0f,
            (tab.rect.w - 22.0f - text_right_pad > 8.0f) ? (tab.rect.w - 22.0f - text_right_pad) : 8.0f,
            tab.rect.h - 2.0f
        };
        g_d2d.draw_utf8_text(
            tab.title_ptr,
            tab.title_len,
            text_rc,
            tab.is_active ? g_d2d.fmt_ui_semibold : g_d2d.fmt_ui_regular,
            fg
        );

        if (tab.can_close) {
            if (tab.is_close_hovered) {
                g_d2d.fill_rounded(tab.close_rect, 4.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            g_d2d.draw_close_x(
                tab.close_rect,
                3.5f,
                1.4f,
                tab.is_close_hovered ? s->ink_primary : s->ink_muted
            );
        }
    }

    // New Tab (+) button
    if (s->new_tab_hovered) {
        g_d2d.fill_rounded(s->new_tab_rect, s->new_tab_rect.h * 0.5f, 0.0f, s->control_hover_bg, s->divider);
    }
    g_d2d.draw_plus(s->new_tab_rect, 5.0f, 1.5f, s->new_tab_hovered ? s->ink_primary : s->ink_muted);

    // 3. Windows 11 Top-Right Caption Buttons (Minimize, Maximize/Restore, Close)
    if (s->win_min_rect.w > 0.0f) {
        if (s->win_min_hovered) {
            g_d2d.fill_rect(to_d2d_rect(s->win_min_rect), s->control_hover_bg);
        }
        float mcx = s->win_min_rect.x + s->win_min_rect.w * 0.5f;
        float mcy = s->win_min_rect.y + s->win_min_rect.h * 0.5f;
        g_d2d.draw_line(mcx - 5.0f, mcy, mcx + 5.0f, mcy, 1.2f, s->ink_primary);

        if (s->win_max_hovered) {
            g_d2d.fill_rect(to_d2d_rect(s->win_max_rect), s->control_hover_bg);
        }
        float xcx = s->win_max_rect.x + s->win_max_rect.w * 0.5f;
        float xcy = s->win_max_rect.y + s->win_max_rect.h * 0.5f;
        g_d2d.brush->SetColor(to_d2d_color(s->ink_primary));
        g_d2d.dc_target->DrawRectangle(D2D1::RectF(xcx - 4.5f, xcy - 4.5f, xcx + 4.5f, xcy + 4.5f), g_d2d.brush, 1.2f);

        AnvilUiColor close_fg = s->ink_primary;
        if (s->win_close_hovered) {
            AnvilUiColor crimson = { 232, 17, 35, 255 };
            g_d2d.fill_rect(to_d2d_rect(s->win_close_rect), crimson);
            close_fg = { 255, 255, 255, 255 };
        }
        g_d2d.draw_close_x(s->win_close_rect, 4.5f, 1.3f, close_fg);
    }

    // 4. Navigation Bar Background
    g_d2d.fill_rect(
        D2D1::RectF(0.0f, s->tab_strip_height, s->window_width, s->top_bar_height),
        s->nav_bg
    );
    g_d2d.fill_rect(
        D2D1::RectF(0.0f, s->top_bar_height - 1.0f, s->window_width, s->top_bar_height),
        s->divider
    );
    if (s->is_loading) {
        g_d2d.fill_rect(
            D2D1::RectF(0.0f, s->top_bar_height - 2.0f, s->window_width, s->top_bar_height),
            s->accent
        );
    }

    // Navigation vector buttons (Back, Forward, Reload, Home)
    AnvilUiColor fg = s->ink_primary;
    g_d2d.draw_nav_button_shell(s->back_rect, s->can_go_back, s->back_hovered, 0, s, &fg);
    {
        float cx = s->back_rect.x + s->back_rect.w * 0.5f;
        float cy = s->back_rect.y + s->back_rect.h * 0.5f;
        g_d2d.draw_line(cx - 5.5f, cy, cx + 5.5f, cy, 1.6f, fg);
        g_d2d.draw_line(cx - 5.5f, cy, cx - 1.0f, cy - 4.5f, 1.6f, fg);
        g_d2d.draw_line(cx - 5.5f, cy, cx - 1.0f, cy + 4.5f, 1.6f, fg);
    }

    g_d2d.draw_nav_button_shell(s->forward_rect, s->can_go_forward, s->forward_hovered, 0, s, &fg);
    {
        float cx = s->forward_rect.x + s->forward_rect.w * 0.5f;
        float cy = s->forward_rect.y + s->forward_rect.h * 0.5f;
        g_d2d.draw_line(cx - 5.5f, cy, cx + 5.5f, cy, 1.6f, fg);
        g_d2d.draw_line(cx + 5.5f, cy, cx + 1.0f, cy - 4.5f, 1.6f, fg);
        g_d2d.draw_line(cx + 5.5f, cy, cx + 1.0f, cy + 4.5f, 1.6f, fg);
    }

    g_d2d.draw_nav_button_shell(s->reload_rect, s->has_current_page, s->reload_hovered, 0, s, &fg);
    {
        float cx = s->reload_rect.x + s->reload_rect.w * 0.5f;
        float cy = s->reload_rect.y + s->reload_rect.h * 0.5f;
        g_d2d.brush->SetColor(to_d2d_color(fg));
        g_d2d.dc_target->DrawEllipse(D2D1::Ellipse(D2D1::Point2F(cx, cy), 5.2f, 5.2f), g_d2d.brush, 1.6f);
        // Mask top-right gap and draw arrowhead
        g_d2d.draw_line(cx + 2.5f, cy - 5.8f, cx + 5.8f, cy - 2.5f, 1.6f, fg);
        g_d2d.draw_line(cx + 1.5f, cy - 2.5f, cx + 5.8f, cy - 2.5f, 1.6f, fg);
    }

    g_d2d.draw_nav_button_shell(s->home_rect, 1, s->home_hovered, 0, s, &fg);
    {
        float cx = s->home_rect.x + s->home_rect.w * 0.5f;
        float cy = s->home_rect.y + s->home_rect.h * 0.5f;
        g_d2d.draw_line(cx - 6.0f, cy - 0.5f, cx, cy - 6.0f, 1.6f, fg);
        g_d2d.draw_line(cx, cy - 6.0f, cx + 6.0f, cy - 0.5f, 1.6f, fg);
        g_d2d.brush->SetColor(to_d2d_color(fg));
        g_d2d.dc_target->DrawRectangle(D2D1::RectF(cx - 4.0f, cy - 0.5f, cx + 4.0f, cy + 5.5f), g_d2d.brush, 1.5f);
    }

    // 5. Full-Capsule Pill Omnibox
    AnvilUiColor omni_border = s->is_editing_address ? s->accent : (s->omnibox_hovered ? s->accent_cyan : s->divider);
    float omni_border_w = s->is_editing_address ? 1.5f : 1.0f;
    g_d2d.fill_rounded(s->omnibox_rect, s->omnibox_rect.h * 0.5f, omni_border_w, s->field_bg, omni_border);

    // Security Badge Pill inside left of Omnibox
    AnvilUiRect sec_inner = {
        s->security_badge_rect.x + 3.0f,
        s->security_badge_rect.y + 3.0f,
        s->security_badge_rect.w - 6.0f,
        s->security_badge_rect.h - 6.0f
    };
    g_d2d.fill_rounded(
        sec_inner,
        sec_inner.h * 0.5f,
        1.0f,
        s->sec_bg,
        s->sec_hovered ? s->accent : s->sec_fg
    );
    g_d2d.draw_utf8_text(
        s->sec_label_ptr,
        s->sec_label_len,
        sec_inner,
        g_d2d.fmt_ui_badge,
        s->sec_fg,
        DWRITE_TEXT_ALIGNMENT_CENTER
    );

    // Omnibox Text (Domain Highlighting when idle, or input/placeholder)
    float text_x = s->address_rect.x + 8.0f;
    float text_max_w = s->address_rect.w - (s->show_clear_btn ? 32.0f : 12.0f);
    if (text_max_w < 16.0f) text_max_w = 16.0f;

    if (s->is_editing_address && s->is_input_select_all && s->selection_char_count > 0) {
        float sel_w = static_cast<float>(s->selection_char_count) * 7.2f + 8.0f;
        if (sel_w > text_max_w) sel_w = text_max_w;
        AnvilUiRect sel_rc = { text_x - 2.0f, s->address_rect.y + 4.0f, sel_w, s->address_rect.h - 8.0f };
        g_d2d.fill_rounded(sel_rc, 5.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
    }

    if (s->use_domain_highlight && s->url_domain_len > 0) {
        float cur_x = text_x;
        float rem_w = text_max_w;
        if (s->url_scheme_len > 0 && rem_w > 8.0f) {
            float w1 = g_d2d.measure_and_draw_segment(
                s->url_scheme_ptr,
                s->url_scheme_len,
                cur_x,
                s->address_rect.y,
                rem_w,
                s->address_rect.h,
                g_d2d.fmt_ui_regular,
                s->ink_muted
            );
            cur_x += w1;
            rem_w -= w1;
        }
        if (rem_w > 8.0f) {
            float w2 = g_d2d.measure_and_draw_segment(
                s->url_domain_ptr,
                s->url_domain_len,
                cur_x,
                s->address_rect.y,
                rem_w,
                s->address_rect.h,
                g_d2d.fmt_ui_semibold,
                s->ink_primary
            );
            cur_x += w2;
            rem_w -= w2;
        }
        if (s->url_path_len > 0 && rem_w > 8.0f) {
            AnvilUiRect path_rc = { cur_x, s->address_rect.y, rem_w, s->address_rect.h };
            g_d2d.draw_utf8_text(
                s->url_path_ptr,
                s->url_path_len,
                path_rc,
                g_d2d.fmt_ui_regular,
                s->ink_muted
            );
        }
    } else {
        AnvilUiRect addr_text_rc = { text_x, s->address_rect.y, text_max_w, s->address_rect.h };
        g_d2d.draw_utf8_text(
            s->display_text_ptr,
            s->display_text_len,
            addr_text_rc,
            g_d2d.fmt_ui_regular,
            s->display_text_muted ? s->ink_muted : s->ink_primary
        );
    }

    if (s->show_clear_btn) {
        if (s->clear_hovered) {
            g_d2d.fill_rounded(s->clear_address_rect, s->clear_address_rect.h * 0.5f, 0.0f, s->control_hover_bg, s->control_hover_bg);
        }
        g_d2d.draw_close_x(s->clear_address_rect, 3.5f, 1.4f, s->clear_hovered ? s->accent : s->ink_muted);
    }

    // Bookmark Star inside right of Omnibox
    if (s->star_hovered || s->is_bookmarked) {
        AnvilUiRect star_inner = {
            s->star_rect.x + 3.0f,
            s->star_rect.y + 3.0f,
            s->star_rect.w - 6.0f,
            s->star_rect.h - 6.0f
        };
        if (s->star_hovered) {
            g_d2d.fill_rounded(star_inner, star_inner.h * 0.5f, 0.0f, s->control_hover_bg, s->accent);
        }
    }
    {
        AnvilUiColor star_col = (s->is_bookmarked || s->star_hovered) ? s->accent : s->ink_muted;
        float cx = s->star_rect.x + s->star_rect.w * 0.5f;
        float cy = s->star_rect.y + s->star_rect.h * 0.5f;
        g_d2d.draw_line(cx, cy - 5.5f, cx, cy + 5.5f, 1.5f, star_col);
        g_d2d.draw_line(cx - 5.5f, cy, cx + 5.5f, cy, 1.5f, star_col);
        g_d2d.draw_line(cx - 3.5f, cy - 3.5f, cx + 3.5f, cy + 3.5f, 1.4f, star_col);
        g_d2d.draw_line(cx + 3.5f, cy - 3.5f, cx - 3.5f, cy + 3.5f, 1.4f, star_col);
    }

    // 6. Right Action Cluster
    // 6a. Shields Pill (if visible)
    if (s->shields_pill_rect.w > 0.0f) {
        AnvilUiColor sh_bg = s->shields_active ? s->control_active_bg : (s->shields_hovered ? s->control_hover_bg : s->control_bg);
        AnvilUiColor sh_border = (s->shields_active || s->shields_hovered) ? s->accent : s->divider;
        g_d2d.fill_rounded(s->shields_pill_rect, s->shields_pill_rect.h * 0.5f, 1.0f, sh_bg, sh_border);
        g_d2d.draw_utf8_text(
            s->shields_text_ptr,
            s->shields_text_len,
            s->shields_pill_rect,
            g_d2d.fmt_ui_badge,
            s->accent,
            DWRITE_TEXT_ALIGNMENT_CENTER
        );
    }

    // 6b. Zoom Pill
    {
        AnvilUiColor zm_bg = s->zoom_hovered ? s->control_hover_bg : s->control_bg;
        AnvilUiColor zm_border = (s->zoom_non_default || s->zoom_hovered) ? s->accent : s->divider;
        g_d2d.fill_rounded(s->zoom_pill_rect, s->zoom_pill_rect.h * 0.5f, 1.0f, zm_bg, zm_border);
        g_d2d.draw_utf8_text(
            s->zoom_text_ptr,
            s->zoom_text_len,
            s->zoom_pill_rect,
            g_d2d.fmt_ui_small,
            (s->zoom_non_default || s->zoom_hovered) ? s->accent : s->ink_primary,
            DWRITE_TEXT_ALIGNMENT_CENTER
        );
    }

    // 6c. Settings Button
    g_d2d.draw_nav_button_shell(s->settings_btn_rect, 1, s->settings_hovered, s->settings_active, s, &fg);
    {
        float x0 = s->settings_btn_rect.x + 7.0f;
        float y0 = s->settings_btn_rect.y + 9.0f;
        g_d2d.draw_line(x0, y0, x0 + 14.0f, y0, 1.5f, fg);
        g_d2d.draw_line(x0, y0 + 5.0f, x0 + 14.0f, y0 + 5.0f, 1.5f, fg);
        g_d2d.draw_line(x0, y0 + 10.0f, x0 + 14.0f, y0 + 10.0f, 1.5f, fg);
        g_d2d.brush->SetColor(to_d2d_color(s->accent));
        g_d2d.dc_target->FillEllipse(D2D1::Ellipse(D2D1::Point2F(x0 + 4.0f, y0), 2.2f, 2.2f), g_d2d.brush);
        g_d2d.dc_target->FillEllipse(D2D1::Ellipse(D2D1::Point2F(x0 + 10.0f, y0 + 5.0f), 2.2f, 2.2f), g_d2d.brush);
        g_d2d.dc_target->FillEllipse(D2D1::Ellipse(D2D1::Point2F(x0 + 6.0f, y0 + 10.0f), 2.2f, 2.2f), g_d2d.brush);
    }

    // 6d. Downloads Button
    g_d2d.draw_nav_button_shell(s->downloads_btn_rect, 1, s->downloads_hovered, s->downloads_active, s, &fg);
    {
        float cx = s->downloads_btn_rect.x + s->downloads_btn_rect.w * 0.5f;
        float cy = s->downloads_btn_rect.y + s->downloads_btn_rect.h * 0.5f;
        g_d2d.draw_line(cx, cy - 5.5f, cx, cy + 2.5f, 1.6f, fg);
        g_d2d.draw_line(cx - 3.5f, cy - 1.0f, cx, cy + 2.5f, 1.6f, fg);
        g_d2d.draw_line(cx + 3.5f, cy - 1.0f, cx, cy + 2.5f, 1.6f, fg);
        g_d2d.draw_line(cx - 5.5f, cy + 5.0f, cx + 5.5f, cy + 5.0f, 1.6f, fg);
        if (s->downloads_has_entries) {
            AnvilUiColor emerald = { 110, 231, 183, 255 };
            g_d2d.brush->SetColor(to_d2d_color(emerald));
            g_d2d.dc_target->FillEllipse(
                D2D1::Ellipse(D2D1::Point2F(s->downloads_btn_rect.x + s->downloads_btn_rect.w - 5.5f, s->downloads_btn_rect.y + 5.5f), 2.8f, 2.8f),
                g_d2d.brush
            );
        }
    }

    // 6e. Inspector Button (<>)
    g_d2d.draw_nav_button_shell(s->inspector_btn_rect, 1, s->inspector_hovered, s->inspector_active, s, &fg);
    {
        float cx = s->inspector_btn_rect.x + s->inspector_btn_rect.w * 0.5f;
        float cy = s->inspector_btn_rect.y + s->inspector_btn_rect.h * 0.5f;
        g_d2d.draw_line(cx - 2.0f, cy - 4.0f, cx - 6.0f, cy, 1.5f, fg);
        g_d2d.draw_line(cx - 6.0f, cy, cx - 2.0f, cy + 4.0f, 1.5f, fg);
        g_d2d.draw_line(cx + 2.0f, cy - 4.0f, cx + 6.0f, cy, 1.5f, fg);
        g_d2d.draw_line(cx + 6.0f, cy, cx + 2.0f, cy + 4.0f, 1.5f, fg);
    }

    // 6f. App Menu Button (hamburger)
    g_d2d.draw_nav_button_shell(s->app_menu_btn_rect, 1, s->app_menu_hovered, s->app_menu_active, s, &fg);
    {
        float x0 = s->app_menu_btn_rect.x + 7.0f;
        float y0 = s->app_menu_btn_rect.y + 9.0f;
        g_d2d.draw_line(x0, y0, x0 + 14.0f, y0, 1.6f, fg);
        g_d2d.draw_line(x0, y0 + 5.0f, x0 + 14.0f, y0 + 5.0f, 1.6f, fg);
        g_d2d.draw_line(x0, y0 + 10.0f, x0 + 14.0f, y0 + 10.0f, 1.6f, fg);
    }

    // 7. Bookmarks Bar (when populated)
    if (s->bookmark_count > 0 && s->top_header_height > s->top_bar_height) {
        g_d2d.fill_rect(
            D2D1::RectF(0.0f, s->top_bar_height, s->window_width, s->top_header_height),
            s->nav_bg
        );
        g_d2d.fill_rect(
            D2D1::RectF(0.0f, s->top_header_height - 1.0f, s->window_width, s->top_header_height),
            s->divider
        );
        for (size_t i = 0; i < s->bookmark_count && i < ANVIL_UI_MAX_BOOKMARKS; ++i) {
            const AnvilUiBookmarkInfo& bm = s->bookmarks[i];
            if (bm.rect.x + bm.rect.w > s->window_width - 8.0f) break;
            g_d2d.fill_rounded(
                bm.rect,
                bm.rect.h * 0.5f,
                1.0f,
                bm.is_hovered ? s->control_hover_bg : s->control_bg,
                bm.is_hovered ? s->accent : s->divider
            );
            AnvilUiRect bm_txt = { bm.rect.x + 8.0f, bm.rect.y, bm.rect.w - 16.0f, bm.rect.h };
            g_d2d.draw_utf8_text(bm.label_ptr, bm.label_len, bm_txt, g_d2d.fmt_ui_small, s->ink_primary);
        }
    }

    if (FAILED(g_d2d.dc_target->EndDraw())) {
        return 0;
    }

    g_d2d.copy_rows_bgra_to_rgba(rgba_pixels, width, 0, header_h);
    return 1;
}

extern "C" int anvil_ui_render_overlays_d2d(
    uint8_t* rgba_pixels,
    uint32_t width,
    uint32_t height,
    const AnvilUiChromeState* s
) {
    if (!rgba_pixels || !s || width == 0 || height == 0) return 0;
    bool any_overlay = s->show_floating_status_pill
        || s->find_open
        || s->suggestion_count > 0
        || s->app_menu_open
        || s->context_menu_open
        || s->select_dropdown_open;
    if (!any_overlay) return 1;

    if (!g_d2d.ensure_surface(width, height)) return 0;

    uint32_t y_min = static_cast<uint32_t>(s->top_bar_height);
    if (y_min >= height) y_min = 0;
    g_d2d.copy_rows_rgba_to_bgra(rgba_pixels, width, y_min, height);

    RECT bind_rc = { 0, 0, static_cast<LONG>(width), static_cast<LONG>(height) };
    if (FAILED(g_d2d.dc_target->BindDC(g_d2d.mem_dc, &bind_rc))) {
        return 0;
    }

    g_d2d.dc_target->BeginDraw();

    // 1. Floating Bottom-Left Link-Hover / Loading Status Pill
    if (s->show_floating_status_pill && s->status_pill_len > 0) {
        float max_pill_w = s->window_width * 0.62f;
        if (max_pill_w < 180.0f) max_pill_w = 180.0f;
        float est_w = static_cast<float>(s->status_pill_len) * 6.8f + 24.0f;
        if (est_w > max_pill_w) est_w = max_pill_w;
        if (est_w < 120.0f) est_w = 120.0f;
        AnvilUiRect pill = { 12.0f, s->window_height - 34.0f, est_w, 24.0f };
        g_d2d.draw_shadow_card(pill, 12.0f, s->overlay_bg, s->divider);
        AnvilUiRect txt_rc = { pill.x + 11.0f, pill.y, pill.w - 22.0f, pill.h };
        g_d2d.draw_utf8_text(s->status_pill_ptr, s->status_pill_len, txt_rc, g_d2d.fmt_ui_small, s->ink_muted);
    }

    // 2. Floating Find-in-Page Bar
    if (s->find_open) {
        g_d2d.draw_shadow_card(s->find_bar_rect, 10.0f, s->overlay_bg, s->accent);
        AnvilUiRect q_rc = { s->find_bar_rect.x + 12.0f, s->find_bar_rect.y, 154.0f, s->find_bar_rect.h };
        g_d2d.draw_utf8_text(s->find_query_ptr, s->find_query_len, q_rc, g_d2d.fmt_ui_regular, s->ink_primary);

        AnvilUiRect c_rc = { s->find_bar_rect.x + 166.0f, s->find_bar_rect.y, 58.0f, s->find_bar_rect.h };
        g_d2d.draw_utf8_text(s->find_count_ptr, s->find_count_len, c_rc, g_d2d.fmt_ui_small, s->accent, DWRITE_TEXT_ALIGNMENT_TRAILING);

        auto draw_find_btn = [&](AnvilUiRect r, uint8_t hov, const char* g) {
            g_d2d.fill_rounded(r, 6.0f, 1.0f, hov ? s->control_hover_bg : s->control_bg, hov ? s->accent : s->divider);
            g_d2d.draw_utf8_text(g, 1, r, g_d2d.fmt_ui_semibold, s->ink_primary, DWRITE_TEXT_ALIGNMENT_CENTER);
        };
        draw_find_btn(s->find_prev_rect, s->find_prev_hovered, "<");
        draw_find_btn(s->find_next_rect, s->find_next_hovered, ">");
        g_d2d.fill_rounded(
            s->find_close_rect,
            6.0f,
            1.0f,
            s->find_close_hovered ? s->control_hover_bg : s->control_bg,
            s->find_close_hovered ? s->accent : s->divider
        );
        g_d2d.draw_close_x(s->find_close_rect, 3.5f, 1.4f, s->ink_primary);
    }

    // 3. Floating Omnibox Autocomplete Suggestion Card
    if (s->suggestion_count > 0) {
        g_d2d.draw_shadow_card(s->suggestions_rect, 10.0f, s->overlay_bg, s->accent);
        for (size_t i = 0; i < s->suggestion_count && i < ANVIL_UI_MAX_SUGGESTIONS; ++i) {
            const AnvilUiSuggestionInfo& sug = s->suggestions[i];
            if (sug.is_selected || sug.is_hovered) {
                g_d2d.fill_rounded(
                    sug.rect,
                    6.0f,
                    0.0f,
                    sug.is_selected ? s->control_active_bg : s->control_hover_bg,
                    s->accent
                );
            }
            AnvilUiRect badge_rc = { sug.rect.x + 6.0f, sug.rect.y + 4.0f, 38.0f, 16.0f };
            g_d2d.fill_rounded(badge_rc, 4.0f, 1.0f, s->control_bg, s->divider);
            g_d2d.draw_utf8_text(
                sug.badge_ptr,
                sug.badge_len,
                badge_rc,
                g_d2d.fmt_ui_badge,
                s->accent_cyan,
                DWRITE_TEXT_ALIGNMENT_CENTER
            );

            float tx = sug.rect.x + 52.0f;
            float max_w = (sug.rect.w - 58.0f > 20.0f) ? (sug.rect.w - 58.0f) : 20.0f;
            AnvilUiColor title_col = (sug.is_selected || sug.is_hovered) ? s->accent : s->ink_primary;
            if (sug.is_search || sug.url_len == 0) {
                AnvilUiRect trc = { tx, sug.rect.y, max_w, sug.rect.h };
                g_d2d.draw_utf8_text(sug.title_ptr, sug.title_len, trc, g_d2d.fmt_ui_regular, title_col);
            } else {
                float w1 = g_d2d.measure_and_draw_segment(
                    sug.title_ptr,
                    sug.title_len,
                    tx,
                    sug.rect.y,
                    max_w * 0.55f,
                    sug.rect.h,
                    g_d2d.fmt_ui_semibold,
                    title_col
                );
                float rem = max_w - w1 - 10.0f;
                if (rem > 24.0f) {
                    AnvilUiRect urc = { tx + w1 + 8.0f, sug.rect.y, rem, sug.rect.h };
                    g_d2d.draw_utf8_text(sug.url_ptr, sug.url_len, urc, g_d2d.fmt_ui_small, s->ink_muted);
                }
            }
        }
    }

    // 4. Floating Hamburger App Menu Card
    if (s->app_menu_open) {
        g_d2d.draw_shadow_card(s->app_menu_rect, 10.0f, s->overlay_bg, s->divider);
        AnvilUiRect hdr_rc = { s->app_menu_rect.x + 14.0f, s->app_menu_rect.y + 2.0f, s->app_menu_rect.w - 28.0f, 26.0f };
        g_d2d.draw_utf8_text("ANVIL BROWSER", 13, hdr_rc, g_d2d.fmt_ui_badge, s->accent);
        g_d2d.draw_line(
            s->app_menu_rect.x + 8.0f,
            s->app_menu_rect.y + 30.0f,
            s->app_menu_rect.x + s->app_menu_rect.w - 8.0f,
            s->app_menu_rect.y + 30.0f,
            1.0f,
            s->divider
        );

        for (size_t i = 0; i < s->app_menu_row_count && i < ANVIL_UI_MAX_MENU_ROWS; ++i) {
            const AnvilUiMenuRowInfo& row = s->app_menu_rows[i];
            if (row.has_divider_above) {
                g_d2d.draw_line(row.rect.x + 6.0f, row.rect.y - 1.0f, row.rect.x + row.rect.w - 6.0f, row.rect.y - 1.0f, 1.0f, s->divider);
            }
            if (row.is_hovered) {
                g_d2d.fill_rounded(row.rect, 6.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 76.0f, row.rect.h };
            g_d2d.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                g_d2d.fmt_ui_regular,
                row.is_hovered ? s->accent : s->ink_primary
            );
            AnvilUiRect src = { row.rect.x + row.rect.w - 68.0f, row.rect.y, 58.0f, row.rect.h };
            g_d2d.draw_utf8_text(
                row.shortcut_ptr,
                row.shortcut_len,
                src,
                g_d2d.fmt_ui_small,
                row.is_hovered ? s->accent_cyan : s->ink_muted,
                DWRITE_TEXT_ALIGNMENT_TRAILING
            );
        }
    }

    // 5. Floating Right-Click Context Menu Card
    if (s->context_menu_open) {
        g_d2d.draw_shadow_card(s->context_menu_rect, 10.0f, s->overlay_bg, s->divider);
        for (size_t i = 0; i < s->context_menu_row_count && i < ANVIL_UI_MAX_MENU_ROWS; ++i) {
            const AnvilUiMenuRowInfo& row = s->context_menu_rows[i];
            if (row.has_divider_above) {
                g_d2d.draw_line(row.rect.x + 6.0f, row.rect.y - 1.0f, row.rect.x + row.rect.w - 6.0f, row.rect.y - 1.0f, 1.0f, s->divider);
            }
            if (row.is_hovered) {
                g_d2d.fill_rounded(row.rect, 6.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 78.0f, row.rect.h };
            g_d2d.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                g_d2d.fmt_ui_regular,
                row.is_hovered ? s->accent : s->ink_primary
            );
            AnvilUiRect src = { row.rect.x + row.rect.w - 72.0f, row.rect.y, 62.0f, row.rect.h };
            g_d2d.draw_utf8_text(
                row.shortcut_ptr,
                row.shortcut_len,
                src,
                g_d2d.fmt_ui_small,
                row.is_hovered ? s->accent_cyan : s->ink_muted,
                DWRITE_TEXT_ALIGNMENT_TRAILING
            );
        }
    }

    // 6. Floating <select> Dropdown Card
    if (s->select_dropdown_open) {
        g_d2d.draw_shadow_card(s->select_dropdown_rect, 8.0f, s->overlay_bg, s->accent);
        for (size_t i = 0; i < s->select_dropdown_row_count && i < ANVIL_UI_MAX_SELECT_ROWS; ++i) {
            const AnvilUiSelectRowInfo& row = s->select_dropdown_rows[i];
            if (row.is_hovered || row.is_selected) {
                g_d2d.fill_rounded(
                    row.rect,
                    5.0f,
                    0.0f,
                    row.is_hovered ? s->control_hover_bg : s->control_bg,
                    s->control_hover_bg
                );
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 20.0f, row.rect.h };
            g_d2d.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                row.is_selected ? g_d2d.fmt_ui_semibold : g_d2d.fmt_ui_regular,
                (row.is_hovered || row.is_selected) ? s->accent : s->ink_primary
            );
        }
    }

    if (FAILED(g_d2d.dc_target->EndDraw())) {
        return 0;
    }

    g_d2d.copy_rows_bgra_to_rgba(rgba_pixels, width, y_min, height);
    return 1;
}

#else

extern "C" void anvil_ui_dwm_configure_window(void*, int, int) {}
extern "C" int anvil_ui_render_top_chrome_d2d(uint8_t*, uint32_t, uint32_t, const AnvilUiChromeState*) { return 0; }
extern "C" int anvil_ui_render_overlays_d2d(uint8_t*, uint32_t, uint32_t, const AnvilUiChromeState*) { return 0; }

#endif
