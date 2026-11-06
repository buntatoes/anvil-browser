// SPDX-License-Identifier: Apache-2.0
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
        IDWriteTextFormat* fmts[4] = { fmt_ui_regular, fmt_ui_semibold, fmt_ui_small, fmt_ui_badge };
        for (int i = 0; i < 4; ++i) {
            IDWriteTextFormat* f = fmts[i];
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
        dc_target->SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
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

    // 1. Seamless Tab Strip Background (logo is alpha-blended cleanly after EndDraw)
    g_d2d.fill_rect(D2D1::RectF(0.0f, 0.0f, s->window_width, s->tab_strip_height), s->strip_bg);

    // 2. Modern Curved Tabs
    for (size_t i = 0; i < s->tab_count && i < ANVIL_UI_MAX_TABS; ++i) {
        const AnvilUiTabInfo& tab = s->tabs[i];
        if (tab.rect.w <= 0.0f) continue;
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
        if (tab.is_pinned) {
            g_d2d.brush->SetColor(to_d2d_color(s->accent));
            g_d2d.dc_target->DrawEllipse(D2D1::Ellipse(D2D1::Point2F(dot_cx, dot_cy), 5.4f, 5.4f), g_d2d.brush, 1.2f);
        }

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

    // Tab Search / Switcher (v) button
    if (s->tab_search_btn_rect.w > 0.0f) {
        if (s->tab_search_btn_hovered || s->tab_search_active) {
            g_d2d.fill_rounded(
                s->tab_search_btn_rect,
                s->tab_search_btn_rect.h * 0.5f,
                s->tab_search_active ? 1.0f : 0.0f,
                s->tab_search_active ? s->control_active_bg : s->control_hover_bg,
                s->tab_search_active ? s->accent : s->divider
            );
        }
        float scx = s->tab_search_btn_rect.x + s->tab_search_btn_rect.w * 0.5f;
        float scy = s->tab_search_btn_rect.y + s->tab_search_btn_rect.h * 0.5f;
        AnvilUiColor sfg = s->tab_search_active ? s->accent : (s->tab_search_btn_hovered ? s->ink_primary : s->ink_muted);
        g_d2d.draw_line(scx - 4.0f, scy - 1.5f, scx, scy + 2.5f, 1.5f, sfg);
        g_d2d.draw_line(scx + 4.0f, scy - 1.5f, scx, scy + 2.5f, 1.5f, sfg);
    }

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
        float split_x = s->window_width * 0.65f;
        g_d2d.fill_rect(
            D2D1::RectF(0.0f, s->top_bar_height - 2.5f, split_x, s->top_bar_height),
            s->accent
        );
        g_d2d.fill_rect(
            D2D1::RectF(split_x, s->top_bar_height - 2.5f, s->window_width, s->top_bar_height),
            s->accent_cyan
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
    if (s->is_loading) {
        g_d2d.draw_close_x(s->reload_rect, 4.5f, 1.6f, s->accent);
    } else {
        float cx = s->reload_rect.x + s->reload_rect.w * 0.5f;
        float cy = s->reload_rect.y + s->reload_rect.h * 0.5f;
        g_d2d.brush->SetColor(to_d2d_color(fg));
        g_d2d.dc_target->DrawEllipse(D2D1::Ellipse(D2D1::Point2F(cx, cy), 5.2f, 5.2f), g_d2d.brush, 1.6f);
        // Mask top-right gap and draw arrowhead
        g_d2d.draw_line(cx + 2.5f, cy - 5.8f, cx + 5.8f, cy - 2.5f, 1.6f, fg);
        g_d2d.draw_line(cx + 1.5f, cy - 2.5f, cx + 5.8f, cy - 2.5f, 1.6f, fg);
    }

    if (s->home_rect.w > 0.0f) {
    g_d2d.draw_nav_button_shell(s->home_rect, 1, s->home_hovered, 0, s, &fg);
    {
        float cx = s->home_rect.x + s->home_rect.w * 0.5f;
        float cy = s->home_rect.y + s->home_rect.h * 0.5f;
        g_d2d.draw_line(cx - 6.0f, cy - 0.5f, cx, cy - 6.0f, 1.6f, fg);
        g_d2d.draw_line(cx, cy - 6.0f, cx + 6.0f, cy - 0.5f, 1.6f, fg);
        g_d2d.brush->SetColor(to_d2d_color(fg));
        g_d2d.dc_target->DrawRectangle(D2D1::RectF(cx - 4.0f, cy - 0.5f, cx + 4.0f, cy + 5.5f), g_d2d.brush, 1.5f);
    }
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
    float right_reserve = (s->show_clear_btn ? 32.0f : 12.0f) + (s->show_reader_btn ? 30.0f : 0.0f);
    float text_max_w = s->address_rect.w - right_reserve;
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
    } else if (s->show_reader_btn && s->reader_btn_rect.w > 0.0f) {
        AnvilUiRect rdr_inner = {
            s->reader_btn_rect.x + 2.0f,
            s->reader_btn_rect.y + 3.0f,
            s->reader_btn_rect.w - 4.0f,
            s->reader_btn_rect.h - 6.0f
        };
        if (s->reader_active || s->reader_hovered) {
            g_d2d.fill_rounded(
                rdr_inner,
                rdr_inner.h * 0.5f,
                s->reader_active ? 1.0f : 0.0f,
                s->reader_active ? s->control_active_bg : s->control_hover_bg,
                s->accent
            );
        }
        AnvilUiColor rdr_col = (s->reader_active || s->reader_hovered) ? s->accent : s->ink_muted;
        float rcx = s->reader_btn_rect.x + s->reader_btn_rect.w * 0.5f;
        float rcy = s->reader_btn_rect.y + s->reader_btn_rect.h * 0.5f;
        g_d2d.brush->SetColor(to_d2d_color(rdr_col));
        g_d2d.dc_target->DrawRectangle(D2D1::RectF(rcx - 5.5f, rcy - 6.0f, rcx + 5.5f, rcy + 6.0f), g_d2d.brush, 1.3f);
        g_d2d.draw_line(rcx - 3.2f, rcy - 3.0f, rcx + 3.2f, rcy - 3.0f, 1.3f, rdr_col);
        g_d2d.draw_line(rcx - 3.2f, rcy, rcx + 3.2f, rcy, 1.3f, rdr_col);
        g_d2d.draw_line(rcx - 3.2f, rcy + 3.0f, rcx + 1.5f, rcy + 3.0f, 1.3f, rdr_col);
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
    if (s->settings_btn_rect.w > 0.0f) {
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
    if (s->inspector_btn_rect.w > 0.0f) {
    g_d2d.draw_nav_button_shell(s->inspector_btn_rect, 1, s->inspector_hovered, s->inspector_active, s, &fg);
    {
        float cx = s->inspector_btn_rect.x + s->inspector_btn_rect.w * 0.5f;
        float cy = s->inspector_btn_rect.y + s->inspector_btn_rect.h * 0.5f;
        g_d2d.draw_line(cx - 2.0f, cy - 4.0f, cx - 6.0f, cy, 1.5f, fg);
        g_d2d.draw_line(cx - 6.0f, cy, cx - 2.0f, cy + 4.0f, 1.5f, fg);
        g_d2d.draw_line(cx + 2.0f, cy - 4.0f, cx + 6.0f, cy, 1.5f, fg);
        g_d2d.draw_line(cx + 6.0f, cy, cx + 2.0f, cy + 4.0f, 1.5f, fg);
    }
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
            char mono = 'B';
            for (size_t k = 0; k < bm.label_len && bm.label_ptr; ++k) {
                char c = bm.label_ptr[k];
                if ((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9')) {
                    mono = (c >= 'a' && c <= 'z') ? static_cast<char>(c - 32) : c;
                    break;
                }
            }
            float bcx = bm.rect.x + 11.0f;
            float bcy = bm.rect.y + bm.rect.h * 0.5f;
            AnvilUiColor badge_col = (i % 2 == 0) ? s->accent : s->accent_cyan;
            g_d2d.brush->SetColor(to_d2d_color(badge_col));
            g_d2d.dc_target->FillEllipse(D2D1::Ellipse(D2D1::Point2F(bcx, bcy), 6.2f, 6.2f), g_d2d.brush);
            AnvilUiRect mono_rc = { bm.rect.x + 4.8f, bm.rect.y, 12.4f, bm.rect.h };
            g_d2d.draw_utf8_text(&mono, 1, mono_rc, g_d2d.fmt_ui_badge, s->strip_bg, DWRITE_TEXT_ALIGNMENT_CENTER);
            AnvilUiRect bm_txt = { bm.rect.x + 21.0f, bm.rect.y, bm.rect.w - 27.0f, bm.rect.h };
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
        || s->select_dropdown_open
        || s->site_popover_open
        || s->tab_preview_open
        || s->tab_search_open
        || s->downloads_popover_open;
    if (!any_overlay) return 1;

    if (!g_d2d.ensure_surface(width, height)) return 0;

    uint32_t y_min = (s->tab_preview_open || s->tab_search_open)
        ? static_cast<uint32_t>(s->tab_strip_height)
        : static_cast<uint32_t>(s->top_bar_height);
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
        AnvilUiRect field = { s->find_bar_rect.x + 6.0f, s->find_bar_rect.y + 4.0f, 158.0f, s->find_bar_rect.h - 8.0f };
        AnvilUiColor error_bg = { 75, 30, 30, 255 }, error_fg = { 255, 160, 150, 255 };
        g_d2d.fill_rounded(field, 4.0f, 1.0f, s->find_no_matches ? error_bg : s->field_bg,
            s->find_no_matches ? error_fg : s->accent);
        AnvilUiRect q_rc = { field.x + 6.0f, field.y, field.w - 12.0f, field.h };
        g_d2d.draw_utf8_text(s->find_query_ptr, s->find_query_len, q_rc, g_d2d.fmt_ui_regular, s->ink_primary);

        AnvilUiRect c_rc = { s->find_bar_rect.x + 166.0f, s->find_bar_rect.y, 58.0f, s->find_bar_rect.h };
        g_d2d.draw_utf8_text(s->find_count_ptr, s->find_count_len, c_rc, g_d2d.fmt_ui_small, s->accent, DWRITE_TEXT_ALIGNMENT_TRAILING);

        auto draw_find_btn = [&](AnvilUiRect r, uint8_t hov, const char* g) {
            g_d2d.fill_rounded(r, 6.0f, 1.0f, hov ? s->control_hover_bg : s->control_bg, hov ? s->accent : s->divider);
            g_d2d.draw_utf8_text(g, 1, r, g_d2d.fmt_ui_semibold, s->ink_primary, DWRITE_TEXT_ALIGNMENT_CENTER);
        };
        if (s->find_case_rect.w > 0.0f) {
            g_d2d.fill_rounded(
                s->find_case_rect,
                6.0f,
                1.0f,
                s->find_case_sensitive ? s->control_active_bg : (s->find_case_hovered ? s->control_hover_bg : s->control_bg),
                (s->find_case_sensitive || s->find_case_hovered) ? s->accent : s->divider
            );
            g_d2d.draw_utf8_text(
                "Aa",
                2,
                s->find_case_rect,
                g_d2d.fmt_ui_badge,
                s->find_case_sensitive ? s->accent : s->ink_primary,
                DWRITE_TEXT_ALIGNMENT_CENTER
            );
        }
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
            // Small local icons: home, bookmark, clock, search.
            float cx = sug.rect.x + 16.0f, cy = sug.rect.y + 12.0f;
            if (sug.kind == 0) {
                g_d2d.draw_line(cx-6, cy, cx, cy-6, 1.5f, s->accent_cyan);
                g_d2d.draw_line(cx, cy-6, cx+6, cy, 1.5f, s->accent_cyan);
                g_d2d.draw_line(cx-4, cy, cx-4, cy+6, 1.5f, s->accent_cyan);
                g_d2d.draw_line(cx+4, cy, cx+4, cy+6, 1.5f, s->accent_cyan);
            } else if (sug.kind == 1) {
                g_d2d.draw_line(cx, cy-6, cx, cy+6, 1.5f, s->accent_cyan);
                g_d2d.draw_line(cx-6, cy, cx+6, cy, 1.5f, s->accent_cyan);
                g_d2d.draw_line(cx-4, cy-4, cx+4, cy+4, 1.5f, s->accent_cyan);
                g_d2d.draw_line(cx+4, cy-4, cx-4, cy+4, 1.5f, s->accent_cyan);
            } else {
                AnvilUiRect circle = {cx-6, cy-6, 12, 12};
                g_d2d.fill_rounded(circle, 6, 1.5f, s->overlay_bg, s->accent_cyan);
                if (sug.kind == 2) {
                    g_d2d.draw_line(cx, cy-4, cx, cy, 1.5f, s->accent_cyan);
                    g_d2d.draw_line(cx, cy, cx+3, cy, 1.5f, s->accent_cyan);
                } else { g_d2d.draw_line(cx+4, cy+4, cx+8, cy+8, 1.5f, s->accent_cyan); }
            }
            float tx = sug.rect.x + 32.0f;
            float rem = sug.rect.w - 40.0f;
            AnvilUiColor col = (sug.is_selected || sug.is_hovered) ? s->accent : s->ink_primary;
            size_t start = sug.match_start <= sug.title_len ? sug.match_start : 0;
            size_t len = sug.match_len <= sug.title_len - start ? sug.match_len : 0;
            const size_t offsets[3] = {0, start, start + len};
            const size_t lengths[3] = {start, len, sug.title_len - start - len};
            for (int part = 0; part < 3 && rem > 0.0f; ++part) {
                if (!lengths[part]) continue;
                float advance = g_d2d.measure_and_draw_segment(sug.title_ptr + offsets[part], lengths[part],
                    tx, sug.rect.y, rem, sug.rect.h, part == 1 ? g_d2d.fmt_ui_semibold : g_d2d.fmt_ui_regular, col);
                tx += advance; rem -= advance;
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

    // 7. Floating Site Information, Shields & Permissions Popover Card
    if (s->site_popover_open) {
        g_d2d.draw_shadow_card(s->site_popover_rect, 10.0f, s->overlay_bg, s->accent);
        AnvilUiRect title_rc = { s->site_popover_rect.x + 14.0f, s->site_popover_rect.y + 6.0f, s->site_popover_rect.w - 28.0f, 20.0f };
        g_d2d.draw_utf8_text(s->site_popover_title_ptr, s->site_popover_title_len, title_rc, g_d2d.fmt_ui_semibold, s->ink_primary);
        AnvilUiRect sub_rc = { s->site_popover_rect.x + 14.0f, s->site_popover_rect.y + 24.0f, s->site_popover_rect.w - 28.0f, 18.0f };
        g_d2d.draw_utf8_text(s->site_popover_sub_ptr, s->site_popover_sub_len, sub_rc, g_d2d.fmt_ui_badge, s->sec_fg);
        g_d2d.draw_line(
            s->site_popover_rect.x + 8.0f,
            s->site_popover_rect.y + 46.0f,
            s->site_popover_rect.x + s->site_popover_rect.w - 8.0f,
            s->site_popover_rect.y + 46.0f,
            1.0f,
            s->divider
        );
        for (size_t i = 0; i < s->site_popover_row_count && i < ANVIL_UI_MAX_MENU_ROWS; ++i) {
            const AnvilUiMenuRowInfo& row = s->site_popover_rows[i];
            if (row.has_divider_above) {
                g_d2d.draw_line(row.rect.x + 6.0f, row.rect.y - 1.0f, row.rect.x + row.rect.w - 6.0f, row.rect.y - 1.0f, 1.0f, s->divider);
            }
            if (row.is_hovered) {
                g_d2d.fill_rounded(row.rect, 6.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 92.0f, row.rect.h };
            g_d2d.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                g_d2d.fmt_ui_regular,
                row.is_hovered ? s->accent : s->ink_primary
            );
            AnvilUiRect src = { row.rect.x + row.rect.w - 84.0f, row.rect.y, 74.0f, row.rect.h };
            g_d2d.draw_utf8_text(
                row.shortcut_ptr,
                row.shortcut_len,
                src,
                g_d2d.fmt_ui_badge,
                row.is_hovered ? s->accent : s->accent_cyan,
                DWRITE_TEXT_ALIGNMENT_TRAILING
            );
        }
    }

    // 8. Floating Tab Hover Preview Card
    if (s->tab_preview_open) {
        g_d2d.draw_shadow_card(s->tab_preview_rect, 9.0f, s->overlay_bg, s->divider);
        AnvilUiRect bar = { s->tab_preview_rect.x + 10.0f, s->tab_preview_rect.y + 1.0f, s->tab_preview_rect.w - 20.0f, 2.5f };
        g_d2d.fill_rounded(bar, 1.25f, 0.0f, s->tab_preview_dot_color, s->tab_preview_dot_color);
        AnvilUiRect t_rc = { s->tab_preview_rect.x + 12.0f, s->tab_preview_rect.y + 6.0f, s->tab_preview_rect.w - 24.0f, 20.0f };
        g_d2d.draw_utf8_text(s->tab_preview_title_ptr, s->tab_preview_title_len, t_rc, g_d2d.fmt_ui_semibold, s->ink_primary);
        float dcx = s->tab_preview_rect.x + 16.0f;
        float dcy = s->tab_preview_rect.y + 35.0f;
        g_d2d.brush->SetColor(to_d2d_color(s->tab_preview_dot_color));
        g_d2d.dc_target->FillEllipse(D2D1::Ellipse(D2D1::Point2F(dcx, dcy), 3.2f, 3.2f), g_d2d.brush);
        AnvilUiRect u_rc = { s->tab_preview_rect.x + 24.0f, s->tab_preview_rect.y + 25.0f, s->tab_preview_rect.w - 36.0f, 18.0f };
        g_d2d.draw_utf8_text(s->tab_preview_url_ptr, s->tab_preview_url_len, u_rc, g_d2d.fmt_ui_small, s->ink_muted);
        AnvilUiRect m_rc = { s->tab_preview_rect.x + 12.0f, s->tab_preview_rect.y + 44.0f, s->tab_preview_rect.w - 24.0f, 18.0f };
        g_d2d.draw_utf8_text(s->tab_preview_meta_ptr, s->tab_preview_meta_len, m_rc, g_d2d.fmt_ui_badge, s->accent_cyan);
    }

    // 9. Floating Quick-Filter Tab Search & Switcher Card
    if (s->tab_search_open) {
        g_d2d.draw_shadow_card(s->tab_search_rect, 10.0f, s->overlay_bg, s->accent);
        AnvilUiRect hdr_rc = { s->tab_search_rect.x + 14.0f, s->tab_search_rect.y + 6.0f, s->tab_search_rect.w - 28.0f, 18.0f };
        g_d2d.draw_utf8_text("SEARCH OPEN TABS", 16, hdr_rc, g_d2d.fmt_ui_badge, s->accent);
        AnvilUiRect q_box = { s->tab_search_rect.x + 10.0f, s->tab_search_rect.y + 24.0f, s->tab_search_rect.w - 20.0f, 22.0f };
        g_d2d.fill_rounded(q_box, 6.0f, 1.0f, s->control_bg, s->accent);
        AnvilUiRect q_txt = { q_box.x + 8.0f, q_box.y, q_box.w - 16.0f, q_box.h };
        g_d2d.draw_utf8_text(s->tab_search_query_ptr, s->tab_search_query_len, q_txt, g_d2d.fmt_ui_small, s->ink_primary);
        g_d2d.draw_line(
            s->tab_search_rect.x + 8.0f,
            s->tab_search_rect.y + 50.0f,
            s->tab_search_rect.x + s->tab_search_rect.w - 8.0f,
            s->tab_search_rect.y + 50.0f,
            1.0f,
            s->divider
        );
        for (size_t i = 0; i < s->tab_search_row_count && i < ANVIL_UI_MAX_MENU_ROWS; ++i) {
            const AnvilUiMenuRowInfo& row = s->tab_search_rows[i];
            if (row.is_hovered) {
                g_d2d.fill_rounded(row.rect, 6.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 104.0f, row.rect.h };
            g_d2d.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                g_d2d.fmt_ui_regular,
                row.is_hovered ? s->accent : s->ink_primary
            );
            AnvilUiRect src = { row.rect.x + row.rect.w - 96.0f, row.rect.y, 86.0f, row.rect.h };
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

    // 10. Floating Recent Downloads Shelf Popover Card
    if (s->downloads_popover_open) {
        g_d2d.draw_shadow_card(s->downloads_popover_rect, 10.0f, s->overlay_bg, s->accent);
        AnvilUiRect title_rc = { s->downloads_popover_rect.x + 14.0f, s->downloads_popover_rect.y + 6.0f, s->downloads_popover_rect.w - 28.0f, 20.0f };
        g_d2d.draw_utf8_text("Downloads & SHA-256 Vault", 25, title_rc, g_d2d.fmt_ui_semibold, s->ink_primary);
        AnvilUiRect sub_rc = { s->downloads_popover_rect.x + 14.0f, s->downloads_popover_rect.y + 24.0f, s->downloads_popover_rect.w - 28.0f, 18.0f };
        g_d2d.draw_utf8_text(s->downloads_popover_sub_ptr, s->downloads_popover_sub_len, sub_rc, g_d2d.fmt_ui_badge, s->accent_cyan);
        g_d2d.draw_line(
            s->downloads_popover_rect.x + 8.0f,
            s->downloads_popover_rect.y + 46.0f,
            s->downloads_popover_rect.x + s->downloads_popover_rect.w - 8.0f,
            s->downloads_popover_rect.y + 46.0f,
            1.0f,
            s->divider
        );
        for (size_t i = 0; i < s->downloads_popover_row_count && i < ANVIL_UI_MAX_MENU_ROWS; ++i) {
            const AnvilUiMenuRowInfo& row = s->downloads_popover_rows[i];
            if (row.has_divider_above) {
                g_d2d.draw_line(row.rect.x + 6.0f, row.rect.y - 1.0f, row.rect.x + row.rect.w - 6.0f, row.rect.y - 1.0f, 1.0f, s->divider);
            }
            if (row.is_hovered) {
                g_d2d.fill_rounded(row.rect, 6.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 110.0f, row.rect.h };
            g_d2d.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                g_d2d.fmt_ui_regular,
                row.is_hovered ? s->accent : s->ink_primary
            );
            AnvilUiRect src = { row.rect.x + row.rect.w - 102.0f, row.rect.y, 92.0f, row.rect.h };
            g_d2d.draw_utf8_text(
                row.shortcut_ptr,
                row.shortcut_len,
                src,
                g_d2d.fmt_ui_badge,
                row.is_hovered ? s->accent : s->accent_cyan,
                DWRITE_TEXT_ALIGNMENT_TRAILING
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

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>

namespace {

enum TextAlign {
    ALIGN_LEADING = 0,
    ALIGN_TRAILING = 1,
    ALIGN_CENTER = 2
};

struct TextStyle {
    float font_px;
    bool bold;
};

static inline float clampf(float v, float lo, float hi) {
    return v < lo ? lo : (v > hi ? hi : v);
}

static inline uint16_t read_u16_be(const uint8_t* p) {
    return static_cast<uint16_t>((static_cast<uint16_t>(p[0]) << 8) | static_cast<uint16_t>(p[1]));
}

static inline int16_t read_i16_be(const uint8_t* p) {
    return static_cast<int16_t>(read_u16_be(p));
}

static inline uint32_t read_u32_be(const uint8_t* p) {
    return (static_cast<uint32_t>(p[0]) << 24)
        | (static_cast<uint32_t>(p[1]) << 16)
        | (static_cast<uint32_t>(p[2]) << 8)
        | static_cast<uint32_t>(p[3]);
}

static uint32_t decode_utf8(const char* src, size_t len, size_t* idx) {
    if (*idx >= len) return 0;
    uint8_t b0 = static_cast<uint8_t>(src[(*idx)++]);
    if (b0 < 0x80) return b0;
    if ((b0 & 0xE0) == 0xC0 && *idx < len) {
        uint8_t b1 = static_cast<uint8_t>(src[(*idx)++]);
        return ((static_cast<uint32_t>(b0 & 0x1F) << 6) | static_cast<uint32_t>(b1 & 0x3F));
    }
    if ((b0 & 0xF0) == 0xE0 && *idx + 1 < len) {
        uint8_t b1 = static_cast<uint8_t>(src[(*idx)++]);
        uint8_t b2 = static_cast<uint8_t>(src[(*idx)++]);
        return ((static_cast<uint32_t>(b0 & 0x0F) << 12)
            | (static_cast<uint32_t>(b1 & 0x3F) << 6)
            | static_cast<uint32_t>(b2 & 0x3F));
    }
    if ((b0 & 0xF8) == 0xF0 && *idx + 2 < len) {
        uint8_t b1 = static_cast<uint8_t>(src[(*idx)++]);
        uint8_t b2 = static_cast<uint8_t>(src[(*idx)++]);
        uint8_t b3 = static_cast<uint8_t>(src[(*idx)++]);
        return ((static_cast<uint32_t>(b0 & 0x07) << 18)
            | (static_cast<uint32_t>(b1 & 0x3F) << 12)
            | (static_cast<uint32_t>(b2 & 0x3F) << 6)
            | static_cast<uint32_t>(b3 & 0x3F));
    }
    return '?';
}

struct EdgeSeg {
    float x0, y0, x1, y1;
};

struct CachedGlyph {
    bool valid = false;
    uint32_t key = 0;
    int width = 0;
    int height = 0;
    int bearing_x = 0;
    int bearing_y = 0;
    float advance_px = 0.0f;
    uint8_t coverage[48 * 48];
};

struct LinuxTtfFont {
    uint8_t* data = nullptr;
    size_t size = 0;
    bool loaded = false;

    uint16_t units_per_em = 1000;
    int16_t ascender = 800;
    int16_t descender = -200;
    int16_t index_to_loc_format = 0;
    uint16_t num_glyphs = 0;
    uint16_t num_h_metrics = 0;

    uint32_t cmap_off = 0;
    uint32_t cmap_len = 0;
    uint32_t loca_off = 0;
    uint32_t loca_len = 0;
    uint32_t glyf_off = 0;
    uint32_t glyf_len = 0;
    uint32_t hmtx_off = 0;
    uint32_t hmtx_len = 0;

    uint32_t cmap_sub_off = 0;
    uint16_t cmap_format = 0;

    static constexpr size_t CACHE_SIZE = 256;
    CachedGlyph cache[CACHE_SIZE];

    bool load_from_candidates(const char* const* paths, size_t count) {
        for (size_t i = 0; i < count; ++i) {
            if (load_file(paths[i])) return true;
        }
        return false;
    }

    bool load_file(const char* path) {
        FILE* f = std::fopen(path, "rb");
        if (!f) return false;
        if (std::fseek(f, 0, SEEK_END) != 0) {
            std::fclose(f);
            return false;
        }
        long fsz = std::ftell(f);
        if (fsz <= 64 || fsz > 32 * 1024 * 1024) {
            std::fclose(f);
            return false;
        }
        std::fseek(f, 0, SEEK_SET);
        uint8_t* buf = static_cast<uint8_t*>(std::malloc(static_cast<size_t>(fsz)));
        if (!buf) {
            std::fclose(f);
            return false;
        }
        if (std::fread(buf, 1, static_cast<size_t>(fsz), f) != static_cast<size_t>(fsz)) {
            std::free(buf);
            std::fclose(f);
            return false;
        }
        std::fclose(f);
        data = buf;
        size = static_cast<size_t>(fsz);
        if (!parse_tables()) {
            std::free(data);
            data = nullptr;
            size = 0;
            return false;
        }
        loaded = true;
        return true;
    }

    bool parse_tables() {
        if (size < 12) return false;
        uint16_t num_tables = read_u16_be(data + 4);
        if (size < 12u + static_cast<size_t>(num_tables) * 16u) return false;

        uint32_t head_off = 0, head_len = 0;
        uint32_t hhea_off = 0, hhea_len = 0;
        uint32_t maxp_off = 0, maxp_len = 0;

        for (uint16_t i = 0; i < num_tables; ++i) {
            const uint8_t* rec = data + 12 + static_cast<size_t>(i) * 16;
            uint32_t tag = read_u32_be(rec);
            uint32_t off = read_u32_be(rec + 8);
            uint32_t len = read_u32_be(rec + 12);
            if (static_cast<size_t>(off) + static_cast<size_t>(len) > size) continue;
            if (tag == 0x68656164u) { head_off = off; head_len = len; } // 'head'
            else if (tag == 0x68686561u) { hhea_off = off; hhea_len = len; } // 'hhea'
            else if (tag == 0x6D617870u) { maxp_off = off; maxp_len = len; } // 'maxp'
            else if (tag == 0x636D6170u) { cmap_off = off; cmap_len = len; } // 'cmap'
            else if (tag == 0x6C6F6361u) { loca_off = off; loca_len = len; } // 'loca'
            else if (tag == 0x676C7966u) { glyf_off = off; glyf_len = len; } // 'glyf'
            else if (tag == 0x686D7478u) { hmtx_off = off; hmtx_len = len; } // 'hmtx'
        }

        if (head_len < 54 || hhea_len < 36 || maxp_len < 6 || cmap_len < 4 || loca_len < 4 || glyf_len < 4 || hmtx_len < 4) {
            return false;
        }
        units_per_em = read_u16_be(data + head_off + 18);
        if (units_per_em == 0) return false;
        index_to_loc_format = read_i16_be(data + head_off + 50);
        ascender = read_i16_be(data + hhea_off + 4);
        descender = read_i16_be(data + hhea_off + 6);
        num_h_metrics = read_u16_be(data + hhea_off + 34);
        num_glyphs = read_u16_be(data + maxp_off + 4);

        uint16_t cmap_tables = read_u16_be(data + cmap_off + 2);
        if (cmap_len < 4u + static_cast<uint32_t>(cmap_tables) * 8u) return false;
        uint32_t best_sub = 0;
        uint16_t best_fmt = 0;
        for (uint16_t i = 0; i < cmap_tables; ++i) {
            const uint8_t* rec = data + cmap_off + 4 + static_cast<size_t>(i) * 8;
            uint16_t pid = read_u16_be(rec);
            uint16_t eid = read_u16_be(rec + 2);
            uint32_t sub_rel = read_u32_be(rec + 4);
            if (sub_rel + 4 > cmap_len) continue;
            uint32_t abs_off = cmap_off + sub_rel;
            uint16_t fmt = read_u16_be(data + abs_off);
            if ((pid == 0 || (pid == 3 && (eid == 1 || eid == 10))) && (fmt == 12 || fmt == 4)) {
                if (fmt == 12 || best_fmt == 0) {
                    best_sub = abs_off;
                    best_fmt = fmt;
                }
            }
        }
        if (best_sub == 0) return false;
        cmap_sub_off = best_sub;
        cmap_format = best_fmt;
        return true;
    }

    uint16_t glyph_id(uint32_t cp) const {
        if (!loaded || cmap_sub_off == 0) return 0;
        if (cmap_format == 12) {
            if (cmap_sub_off + 16 > size) return 0;
            uint32_t n_groups = read_u32_be(data + cmap_sub_off + 12);
            if (cmap_sub_off + 16u + n_groups * 12u > size) return 0;
            const uint8_t* groups = data + cmap_sub_off + 16;
            uint32_t lo = 0, hi = n_groups;
            while (lo < hi) {
                uint32_t mid = lo + (hi - lo) / 2;
                const uint8_t* g = groups + mid * 12;
                uint32_t start_cp = read_u32_be(g);
                uint32_t end_cp = read_u32_be(g + 4);
                if (cp < start_cp) {
                    hi = mid;
                } else if (cp > end_cp) {
                    lo = mid + 1;
                } else {
                    uint32_t start_gid = read_u32_be(g + 8);
                    return static_cast<uint16_t>(start_gid + (cp - start_cp));
                }
            }
            return 0;
        }
        if (cmap_format == 4 && cp <= 0xFFFFu) {
            if (cmap_sub_off + 14 > size) return 0;
            uint16_t seg_count_x2 = read_u16_be(data + cmap_sub_off + 6);
            uint16_t seg_count = seg_count_x2 / 2;
            uint32_t end_codes = cmap_sub_off + 14;
            uint32_t start_codes = end_codes + seg_count_x2 + 2;
            uint32_t id_deltas = start_codes + seg_count_x2;
            uint32_t id_range_offsets = id_deltas + seg_count_x2;
            if (id_range_offsets + seg_count_x2 > size) return 0;
            for (uint16_t i = 0; i < seg_count; ++i) {
                uint16_t end_c = read_u16_be(data + end_codes + i * 2);
                if (cp > end_c) continue;
                uint16_t start_c = read_u16_be(data + start_codes + i * 2);
                if (cp < start_c) return 0;
                int16_t delta = read_i16_be(data + id_deltas + i * 2);
                uint16_t ro = read_u16_be(data + id_range_offsets + i * 2);
                if (ro == 0) {
                    return static_cast<uint16_t>((static_cast<int32_t>(cp) + delta) & 0xFFFF);
                }
                uint32_t glyph_idx_off = id_range_offsets + i * 2 + ro + (cp - start_c) * 2;
                if (glyph_idx_off + 2 > size) return 0;
                uint16_t gid = read_u16_be(data + glyph_idx_off);
                if (gid == 0) return 0;
                return static_cast<uint16_t>((static_cast<int32_t>(gid) + delta) & 0xFFFF);
            }
        }
        return 0;
    }

    uint16_t advance_units(uint16_t gid) const {
        if (!loaded || num_h_metrics == 0) return units_per_em / 2;
        uint16_t idx = gid < num_h_metrics ? gid : (num_h_metrics - 1);
        if (static_cast<size_t>(idx) * 4u + 2u > hmtx_len) return units_per_em / 2;
        return read_u16_be(data + hmtx_off + static_cast<size_t>(idx) * 4u);
    }

    bool glyph_slice(uint16_t gid, uint32_t* out_off, uint32_t* out_len) const {
        if (!loaded || gid >= num_glyphs) return false;
        uint32_t g0 = 0, g1 = 0;
        if (index_to_loc_format == 0) {
            if (static_cast<size_t>(gid + 1) * 2u + 2u > loca_len) return false;
            g0 = static_cast<uint32_t>(read_u16_be(data + loca_off + gid * 2u)) * 2u;
            g1 = static_cast<uint32_t>(read_u16_be(data + loca_off + (gid + 1) * 2u)) * 2u;
        } else {
            if (static_cast<size_t>(gid + 1) * 4u + 4u > loca_len) return false;
            g0 = read_u32_be(data + loca_off + gid * 4u);
            g1 = read_u32_be(data + loca_off + (gid + 1) * 4u);
        }
        if (g1 <= g0 || g1 > glyf_len) return false;
        *out_off = glyf_off + g0;
        *out_len = g1 - g0;
        return true;
    }

    void append_contour_edges(
        const float* xs,
        const float* ys,
        const uint8_t* on_curve,
        int n,
        EdgeSeg* edges,
        int* edge_count,
        int max_edges
    ) const {
        if (n < 2) return;
        auto push_line = [&](float x0, float y0, float x1, float y1) {
            if (*edge_count < max_edges && std::fabs(y1 - y0) > 1e-4f) {
                edges[(*edge_count)++] = { x0, y0, x1, y1 };
            }
        };
        auto push_quad = [&](float x0, float y0, float cx, float cy, float x1, float y1) {
            const int steps = 5;
            float px = x0, py = y0;
            for (int s = 1; s <= steps; ++s) {
                float t = static_cast<float>(s) / static_cast<float>(steps);
                float inv = 1.0f - t;
                float nx = inv * inv * x0 + 2.0f * inv * t * cx + t * t * x1;
                float ny = inv * inv * y0 + 2.0f * inv * t * cy + t * t * y1;
                push_line(px, py, nx, ny);
                px = nx;
                py = ny;
            }
        };

        float start_x = 0.0f, start_y = 0.0f;
        int first_idx = 0;
        if (on_curve[0]) {
            start_x = xs[0];
            start_y = ys[0];
            first_idx = 1;
        } else if (on_curve[n - 1]) {
            start_x = xs[n - 1];
            start_y = ys[n - 1];
            first_idx = 0;
        } else {
            start_x = 0.5f * (xs[n - 1] + xs[0]);
            start_y = 0.5f * (ys[n - 1] + ys[0]);
            first_idx = 0;
        }

        float cur_x = start_x;
        float cur_y = start_y;
        int i = first_idx;
        while (i < n) {
            if (on_curve[i]) {
                push_line(cur_x, cur_y, xs[i], ys[i]);
                cur_x = xs[i];
                cur_y = ys[i];
                ++i;
            } else {
                float cx = xs[i];
                float cy = ys[i];
                int next = (i + 1) % n;
                if (i + 1 < n && on_curve[next]) {
                    push_quad(cur_x, cur_y, cx, cy, xs[next], ys[next]);
                    cur_x = xs[next];
                    cur_y = ys[next];
                    i += 2;
                } else if (i + 1 < n) {
                    float mx = 0.5f * (cx + xs[next]);
                    float my = 0.5f * (cy + ys[next]);
                    push_quad(cur_x, cur_y, cx, cy, mx, my);
                    cur_x = mx;
                    cur_y = my;
                    ++i;
                } else {
                    push_quad(cur_x, cur_y, cx, cy, start_x, start_y);
                    cur_x = start_x;
                    cur_y = start_y;
                    ++i;
                }
            }
        }
        push_line(cur_x, cur_y, start_x, start_y);
    }

    void collect_glyph_edges(
        uint16_t gid,
        float scale,
        float dx,
        float dy,
        int depth,
        EdgeSeg* edges,
        int* edge_count,
        int max_edges,
        float* min_x,
        float* min_y,
        float* max_x,
        float* max_y
    ) const {
        if (depth > 4) return;
        uint32_t goff = 0, glen = 0;
        if (!glyph_slice(gid, &goff, &glen) || glen < 10) return;
        const uint8_t* g = data + goff;
        int16_t num_contours = read_i16_be(g);
        if (num_contours > 0 && num_contours <= 64) {
            uint32_t pos = 10;
            if (pos + static_cast<uint32_t>(num_contours) * 2u + 2u > glen) return;
            uint16_t end_pts[64];
            for (int16_t c = 0; c < num_contours; ++c) {
                end_pts[c] = read_u16_be(g + pos);
                pos += 2;
            }
            uint16_t num_pts = end_pts[num_contours - 1] + 1;
            if (num_pts == 0 || num_pts > 256) return;
            uint16_t insn_len = read_u16_be(g + pos);
            pos += 2 + insn_len;
            if (pos >= glen) return;

            uint8_t flags[256];
            uint16_t p = 0;
            while (p < num_pts && pos < glen) {
                uint8_t fl = g[pos++];
                flags[p++] = fl;
                if (fl & 0x08) {
                    if (pos >= glen) return;
                    uint8_t rep = g[pos++];
                    while (rep-- > 0 && p < num_pts) {
                        flags[p++] = fl;
                    }
                }
            }
            if (p < num_pts) return;

            int32_t raw_x[256];
            int32_t cur = 0;
            for (uint16_t i = 0; i < num_pts; ++i) {
                uint8_t fl = flags[i];
                if (fl & 0x02) {
                    if (pos >= glen) return;
                    int32_t d = g[pos++];
                    cur += (fl & 0x10) ? d : -d;
                } else if (!(fl & 0x10)) {
                    if (pos + 2 > glen) return;
                    cur += read_i16_be(g + pos);
                    pos += 2;
                }
                raw_x[i] = cur;
            }

            int32_t raw_y[256];
            cur = 0;
            for (uint16_t i = 0; i < num_pts; ++i) {
                uint8_t fl = flags[i];
                if (fl & 0x04) {
                    if (pos >= glen) return;
                    int32_t d = g[pos++];
                    cur += (fl & 0x20) ? d : -d;
                } else if (!(fl & 0x20)) {
                    if (pos + 2 > glen) return;
                    cur += read_i16_be(g + pos);
                    pos += 2;
                }
                raw_y[i] = cur;
            }

            float px[256];
            float py[256];
            uint8_t on_c[256];
            for (uint16_t i = 0; i < num_pts; ++i) {
                px[i] = static_cast<float>(raw_x[i]) * scale + dx;
                py[i] = -static_cast<float>(raw_y[i]) * scale + dy;
                on_c[i] = (flags[i] & 0x01) ? 1 : 0;
                if (px[i] < *min_x) *min_x = px[i];
                if (px[i] > *max_x) *max_x = px[i];
                if (py[i] < *min_y) *min_y = py[i];
                if (py[i] > *max_y) *max_y = py[i];
            }

            uint16_t start_pt = 0;
            for (int16_t c = 0; c < num_contours; ++c) {
                uint16_t end_pt = end_pts[c];
                if (end_pt >= start_pt && end_pt < num_pts) {
                    int cnt = static_cast<int>(end_pt - start_pt + 1);
                    append_contour_edges(px + start_pt, py + start_pt, on_c + start_pt, cnt, edges, edge_count, max_edges);
                }
                start_pt = end_pt + 1;
            }
        } else if (num_contours < 0) {
            uint32_t pos = 10;
            while (pos + 4 <= glen) {
                uint16_t cflags = read_u16_be(g + pos);
                uint16_t sub_gid = read_u16_be(g + pos + 2);
                pos += 4;
                float sub_dx = 0.0f, sub_dy = 0.0f;
                if (cflags & 0x0001) {
                    if (pos + 4 > glen) break;
                    if (cflags & 0x0002) {
                        sub_dx = static_cast<float>(read_i16_be(g + pos)) * scale;
                        sub_dy = -static_cast<float>(read_i16_be(g + pos + 2)) * scale;
                    }
                    pos += 4;
                } else {
                    if (pos + 2 > glen) break;
                    if (cflags & 0x0002) {
                        sub_dx = static_cast<float>(static_cast<int8_t>(g[pos])) * scale;
                        sub_dy = -static_cast<float>(static_cast<int8_t>(g[pos + 1])) * scale;
                    }
                    pos += 2;
                }
                if (cflags & 0x0008) pos += 2;
                else if (cflags & 0x0040) pos += 4;
                else if (cflags & 0x0080) pos += 8;

                collect_glyph_edges(
                    sub_gid,
                    scale,
                    dx + sub_dx,
                    dy + sub_dy,
                    depth + 1,
                    edges,
                    edge_count,
                    max_edges,
                    min_x,
                    min_y,
                    max_x,
                    max_y
                );
                if (!(cflags & 0x0020)) break;
            }
        }
    }

    const CachedGlyph* rasterize_glyph(uint32_t cp, float font_px) {
        if (!loaded || units_per_em == 0) return nullptr;
        uint32_t px_q = static_cast<uint32_t>(font_px * 2.0f + 0.5f);
        uint32_t key = (cp << 8) ^ px_q;
        size_t slot_idx = static_cast<size_t>((key * 2654435761u) % CACHE_SIZE);
        CachedGlyph& slot = cache[slot_idx];
        if (slot.valid && slot.key == key) {
            return &slot;
        }

        uint16_t gid = glyph_id(cp);
        float scale = font_px / static_cast<float>(units_per_em);
        slot.valid = true;
        slot.key = key;
        slot.advance_px = static_cast<float>(advance_units(gid)) * scale;
        slot.width = 0;
        slot.height = 0;
        slot.bearing_x = 0;
        slot.bearing_y = 0;
        std::memset(slot.coverage, 0, sizeof(slot.coverage));

        if (cp == ' ' || gid == 0) {
            return &slot;
        }

        EdgeSeg edges[512];
        int edge_count = 0;
        float min_x = 1e6f, min_y = 1e6f, max_x = -1e6f, max_y = -1e6f;
        collect_glyph_edges(gid, scale, 0.0f, 0.0f, 0, edges, &edge_count, 512, &min_x, &min_y, &max_x, &max_y);
        if (edge_count == 0 || max_x <= min_x || max_y <= min_y) {
            return &slot;
        }

        int x0 = static_cast<int>(std::floor(min_x));
        int y0 = static_cast<int>(std::floor(min_y));
        int x1 = static_cast<int>(std::ceil(max_x));
        int y1 = static_cast<int>(std::ceil(max_y));
        int gw = x1 - x0;
        int gh = y1 - y0;
        if (gw <= 0 || gh <= 0) return &slot;
        if (gw > 48) gw = 48;
        if (gh > 48) gh = 48;

        slot.width = gw;
        slot.height = gh;
        slot.bearing_x = x0;
        slot.bearing_y = -y0;

        static constexpr float SUB_OFFS[4] = { 0.125f, 0.375f, 0.625f, 0.875f };
        for (int py = 0; py < gh; ++py) {
            uint16_t row_hits[48] = {};
            for (int sy = 0; sy < 4; ++sy) {
                float sample_y = static_cast<float>(y0 + py) + SUB_OFFS[sy];
                float x_isects[64];
                int n_isects = 0;
                for (int e = 0; e < edge_count && n_isects < 64; ++e) {
                    const EdgeSeg& seg = edges[e];
                    if ((seg.y0 <= sample_y && seg.y1 > sample_y) || (seg.y1 <= sample_y && seg.y0 > sample_y)) {
                        float t = (sample_y - seg.y0) / (seg.y1 - seg.y0);
                        x_isects[n_isects++] = seg.x0 + t * (seg.x1 - seg.x0);
                    }
                }
                for (int i = 1; i < n_isects; ++i) {
                    float v = x_isects[i];
                    int j = i - 1;
                    while (j >= 0 && x_isects[j] > v) {
                        x_isects[j + 1] = x_isects[j];
                        --j;
                    }
                    x_isects[j + 1] = v;
                }
                for (int i = 0; i + 1 < n_isects; i += 2) {
                    float span_l = x_isects[i] - static_cast<float>(x0);
                    float span_r = x_isects[i + 1] - static_cast<float>(x0);
                    int px_start = static_cast<int>(std::floor(span_l));
                    int px_end = static_cast<int>(std::ceil(span_r));
                    if (px_start < 0) px_start = 0;
                    if (px_end > gw) px_end = gw;
                    for (int px = px_start; px < px_end; ++px) {
                        for (int sx = 0; sx < 4; ++sx) {
                            float sample_x = static_cast<float>(px) + SUB_OFFS[sx];
                            if (sample_x >= span_l && sample_x < span_r) {
                                row_hits[px] += 1;
                            }
                        }
                    }
                }
            }
            for (int px = 0; px < gw; ++px) {
                uint16_t h = row_hits[px];
                if (h > 16) h = 16;
                slot.coverage[py * gw + px] = static_cast<uint8_t>((h * 255u + 8u) / 16u);
            }
        }
        return &slot;
    }
};

struct LinuxSdfChromeRenderer {
    LinuxTtfFont font_regular;
    LinuxTtfFont font_bold;
    bool initialized = false;

    TextStyle fmt_ui_regular = { 13.0f, false };
    TextStyle fmt_ui_semibold = { 13.0f, true };
    TextStyle fmt_ui_small = { 11.5f, false };
    TextStyle fmt_ui_badge = { 10.5f, true };

    uint8_t* target_rgba = nullptr;
    uint32_t target_w = 0;
    uint32_t target_h = 0;

    void ensure_init() {
        if (initialized) return;
        initialized = true;
        static const char* const REG_PATHS[] = {
            "/usr/share/fonts/truetype/noto-sans-ttf/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/urw-base35/NimbusSans-Regular.ttf"
        };
        static const char* const BOLD_PATHS[] = {
            "/usr/share/fonts/truetype/noto-sans-ttf/NotoSans-SemiBold.ttf",
            "/usr/share/fonts/truetype/noto-sans-ttf/NotoSans-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/truetype/urw-base35/NimbusSans-Bold.ttf"
        };
        font_regular.load_from_candidates(REG_PATHS, sizeof(REG_PATHS) / sizeof(REG_PATHS[0]));
        if (!font_bold.load_from_candidates(BOLD_PATHS, sizeof(BOLD_PATHS) / sizeof(BOLD_PATHS[0]))) {
            font_bold.load_from_candidates(REG_PATHS, sizeof(REG_PATHS) / sizeof(REG_PATHS[0]));
        }
    }

    inline void blend_pixel(int x, int y, AnvilUiColor col, float cov = 1.0f) {
        if (x < 0 || y < 0 || static_cast<uint32_t>(x) >= target_w || static_cast<uint32_t>(y) >= target_h) return;
        uint32_t a = static_cast<uint32_t>(static_cast<float>(col.a) * clampf(cov, 0.0f, 1.0f) + 0.5f);
        if (a == 0) return;
        size_t off = (static_cast<size_t>(y) * static_cast<size_t>(target_w) + static_cast<size_t>(x)) * 4u;
        if (a >= 255) {
            target_rgba[off + 0] = col.r;
            target_rgba[off + 1] = col.g;
            target_rgba[off + 2] = col.b;
            target_rgba[off + 3] = 255;
        } else {
            uint32_t inv = 255u - a;
            target_rgba[off + 0] = static_cast<uint8_t>((col.r * a + target_rgba[off + 0] * inv + 127u) / 255u);
            target_rgba[off + 1] = static_cast<uint8_t>((col.g * a + target_rgba[off + 1] * inv + 127u) / 255u);
            target_rgba[off + 2] = static_cast<uint8_t>((col.b * a + target_rgba[off + 2] * inv + 127u) / 255u);
            target_rgba[off + 3] = 255;
        }
    }

    void fill_rect(AnvilUiRect r, AnvilUiColor col) {
        int x0 = static_cast<int>(std::floor(r.x));
        int y0 = static_cast<int>(std::floor(r.y));
        int x1 = static_cast<int>(std::ceil(r.x + r.w));
        int y1 = static_cast<int>(std::ceil(r.y + r.h));
        if (x0 < 0) x0 = 0;
        if (y0 < 0) y0 = 0;
        if (x1 > static_cast<int>(target_w)) x1 = static_cast<int>(target_w);
        if (y1 > static_cast<int>(target_h)) y1 = static_cast<int>(target_h);
        for (int y = y0; y < y1; ++y) {
            for (int x = x0; x < x1; ++x) {
                blend_pixel(x, y, col, 1.0f);
            }
        }
    }

    static inline float sdf_rounded_box(float px, float py, float hw, float hh, float rad) {
        float cr = clampf(rad, 0.0f, hw < hh ? hw : hh);
        float qx = std::fabs(px) - (hw - cr);
        float qy = std::fabs(py) - (hh - cr);
        float mx = qx > 0.0f ? qx : 0.0f;
        float my = qy > 0.0f ? qy : 0.0f;
        float out_d = std::sqrt(mx * mx + my * my);
        float in_d = (qx > qy ? qx : qy);
        if (in_d > 0.0f) in_d = 0.0f;
        return out_d + in_d - cr;
    }

    void fill_rounded(AnvilUiRect r, float radius, float border_w, AnvilUiColor fill, AnvilUiColor border) {
        if (r.w <= 0.0f || r.h <= 0.0f) return;
        int x0 = static_cast<int>(std::floor(r.x - 1.0f));
        int y0 = static_cast<int>(std::floor(r.y - 1.0f));
        int x1 = static_cast<int>(std::ceil(r.x + r.w + 1.0f));
        int y1 = static_cast<int>(std::ceil(r.y + r.h + 1.0f));
        if (x0 < 0) x0 = 0;
        if (y0 < 0) y0 = 0;
        if (x1 > static_cast<int>(target_w)) x1 = static_cast<int>(target_w);
        if (y1 > static_cast<int>(target_h)) y1 = static_cast<int>(target_h);

        float hw = r.w * 0.5f;
        float hh = r.h * 0.5f;
        float cx = r.x + hw;
        float cy = r.y + hh;

        for (int py = y0; py < y1; ++py) {
            float sy = static_cast<float>(py) + 0.5f - cy;
            for (int px = x0; px < x1; ++px) {
                float sx = static_cast<float>(px) + 0.5f - cx;
                float d = sdf_rounded_box(sx, sy, hw, hh, radius);
                float outer_cov = clampf(0.5f - d, 0.0f, 1.0f);
                if (outer_cov <= 0.0f) continue;
                AnvilUiColor col = fill;
                if (border_w > 0.0f) {
                    float bmix = clampf(0.5f + (d + border_w), 0.0f, 1.0f);
                    float inv = 1.0f - bmix;
                    col.r = static_cast<uint8_t>(fill.r * inv + border.r * bmix + 0.5f);
                    col.g = static_cast<uint8_t>(fill.g * inv + border.g * bmix + 0.5f);
                    col.b = static_cast<uint8_t>(fill.b * inv + border.b * bmix + 0.5f);
                    col.a = static_cast<uint8_t>(fill.a * inv + border.a * bmix + 0.5f);
                }
                blend_pixel(px, py, col, outer_cov);
            }
        }
    }

    void draw_rect_outline(float x0, float y0, float x1, float y1, float stroke, AnvilUiColor col) {
        AnvilUiRect r = { x0, y0, x1 - x0, y1 - y0 };
        AnvilUiColor transparent = { col.r, col.g, col.b, 0 };
        fill_rounded(r, 1.0f, stroke, transparent, col);
    }

    void fill_circle(float cx, float cy, float radius, AnvilUiColor col) {
        int x0 = static_cast<int>(std::floor(cx - radius - 1.0f));
        int y0 = static_cast<int>(std::floor(cy - radius - 1.0f));
        int x1 = static_cast<int>(std::ceil(cx + radius + 1.0f));
        int y1 = static_cast<int>(std::ceil(cy + radius + 1.0f));
        for (int py = y0; py <= y1; ++py) {
            for (int px = x0; px <= x1; ++px) {
                float dx = (static_cast<float>(px) + 0.5f) - cx;
                float dy = (static_cast<float>(py) + 0.5f) - cy;
                float d = std::sqrt(dx * dx + dy * dy) - radius;
                float cov = clampf(0.5f - d, 0.0f, 1.0f);
                if (cov > 0.0f) blend_pixel(px, py, col, cov);
            }
        }
    }

    void stroke_circle(float cx, float cy, float radius, float stroke, AnvilUiColor col) {
        float half_s = stroke * 0.5f;
        int x0 = static_cast<int>(std::floor(cx - radius - half_s - 1.0f));
        int y0 = static_cast<int>(std::floor(cy - radius - half_s - 1.0f));
        int x1 = static_cast<int>(std::ceil(cx + radius + half_s + 1.0f));
        int y1 = static_cast<int>(std::ceil(cy + radius + half_s + 1.0f));
        for (int py = y0; py <= y1; ++py) {
            for (int px = x0; px <= x1; ++px) {
                float dx = (static_cast<float>(px) + 0.5f) - cx;
                float dy = (static_cast<float>(py) + 0.5f) - cy;
                float d = std::fabs(std::sqrt(dx * dx + dy * dy) - radius) - half_s;
                float cov = clampf(0.5f - d, 0.0f, 1.0f);
                if (cov > 0.0f) blend_pixel(px, py, col, cov);
            }
        }
    }

    void draw_line(float x1, float y1, float x2, float y2, float stroke, AnvilUiColor col) {
        float half_s = stroke * 0.5f;
        float min_x = (x1 < x2 ? x1 : x2) - half_s - 1.0f;
        float min_y = (y1 < y2 ? y1 : y2) - half_s - 1.0f;
        float max_x = (x1 > x2 ? x1 : x2) + half_s + 1.0f;
        float max_y = (y1 > y2 ? y1 : y2) + half_s + 1.0f;
        int ix0 = static_cast<int>(std::floor(min_x));
        int iy0 = static_cast<int>(std::floor(min_y));
        int ix1 = static_cast<int>(std::ceil(max_x));
        int iy1 = static_cast<int>(std::ceil(max_y));
        float vx = x2 - x1;
        float vy = y2 - y1;
        float len2 = vx * vx + vy * vy;
        for (int py = iy0; py <= iy1; ++py) {
            for (int px = ix0; px <= ix1; ++px) {
                float sx = static_cast<float>(px) + 0.5f;
                float sy = static_cast<float>(py) + 0.5f;
                float t = len2 > 1e-5f ? clampf(((sx - x1) * vx + (sy - y1) * vy) / len2, 0.0f, 1.0f) : 0.0f;
                float dx = sx - (x1 + t * vx);
                float dy = sy - (y1 + t * vy);
                float d = std::sqrt(dx * dx + dy * dy) - half_s;
                float cov = clampf(0.5f - d, 0.0f, 1.0f);
                if (cov > 0.0f) blend_pixel(px, py, col, cov);
            }
        }
    }

    void draw_shadow_card(AnvilUiRect r, float radius, AnvilUiColor fill, AnvilUiColor border) {
        for (int i = 4; i >= 1; --i) {
            float expand = static_cast<float>(i) * 2.2f;
            uint8_t alpha = static_cast<uint8_t>(24 * (5 - i));
            AnvilUiRect sr = {
                r.x - expand * 0.5f,
                r.y + static_cast<float>(i) * 1.6f,
                r.w + expand,
                r.h + expand
            };
            AnvilUiColor scol = { 0, 0, 0, alpha };
            fill_rounded(sr, radius + expand * 0.4f, 0.0f, scol, scol);
        }
        fill_rounded(r, radius, 1.0f, fill, border);
    }

    float measure_utf8(const char* text, size_t len, TextStyle style) {
        if (!text || len == 0) return 0.0f;
        LinuxTtfFont& fnt = (style.bold && font_bold.loaded) ? font_bold : font_regular;
        if (!fnt.loaded) {
            return static_cast<float>(len) * (style.font_px * 0.56f);
        }
        float w = 0.0f;
        size_t idx = 0;
        while (idx < len) {
            uint32_t cp = decode_utf8(text, len, &idx);
            if (cp == 0) break;
            if (const CachedGlyph* cg = fnt.rasterize_glyph(cp, style.font_px)) {
                w += cg->advance_px;
            }
        }
        return w;
    }

    void draw_utf8_text(
        const char* text,
        size_t len,
        AnvilUiRect r,
        TextStyle style,
        AnvilUiColor col,
        TextAlign align = ALIGN_LEADING
    ) {
        if (!text || len == 0 || r.w <= 1.0f) return;
        LinuxTtfFont& fnt = (style.bold && font_bold.loaded) ? font_bold : font_regular;
        if (!fnt.loaded) return;

        float total_w = measure_utf8(text, len, style);
        float start_x = r.x;
        if (align == ALIGN_CENTER && total_w < r.w) {
            start_x = r.x + (r.w - total_w) * 0.5f;
        } else if (align == ALIGN_TRAILING && total_w < r.w) {
            start_x = r.x + (r.w - total_w);
        }

        float scale = style.font_px / static_cast<float>(fnt.units_per_em);
        float asc_px = static_cast<float>(fnt.ascender) * scale;
        float desc_px = static_cast<float>(fnt.descender) * scale;
        float baseline_y = r.y + (r.h + asc_px + desc_px) * 0.5f;

        int clip_x0 = static_cast<int>(std::floor(r.x));
        int clip_y0 = static_cast<int>(std::floor(r.y));
        int clip_x1 = static_cast<int>(std::ceil(r.x + r.w));
        int clip_y1 = static_cast<int>(std::ceil(r.y + r.h));

        float pen_x = start_x;
        size_t idx = 0;
        while (idx < len) {
            uint32_t cp = decode_utf8(text, len, &idx);
            if (cp == 0) break;
            const CachedGlyph* cg = fnt.rasterize_glyph(cp, style.font_px);
            if (!cg) continue;
            if (pen_x + cg->advance_px > r.x + r.w + 2.0f) break;

            int gx0 = static_cast<int>(std::floor(pen_x + 0.5f)) + cg->bearing_x;
            int gy0 = static_cast<int>(std::floor(baseline_y + 0.5f)) - cg->bearing_y;
            for (int py = 0; py < cg->height; ++py) {
                int dy = gy0 + py;
                if (dy < clip_y0 || dy >= clip_y1) continue;
                for (int px = 0; px < cg->width; ++px) {
                    int dx = gx0 + px;
                    if (dx < clip_x0 || dx >= clip_x1) continue;
                    uint8_t cov = cg->coverage[py * cg->width + px];
                    if (cov > 0) {
                        blend_pixel(dx, dy, col, static_cast<float>(cov) / 255.0f);
                    }
                }
            }
            pen_x += cg->advance_px;
        }
    }

    float measure_and_draw_segment(
        const char* text,
        size_t len,
        float x,
        float y,
        float max_w,
        float h,
        TextStyle style,
        AnvilUiColor col
    ) {
        if (!text || len == 0 || max_w <= 1.0f) return 0.0f;
        AnvilUiRect rc = { x, y, max_w, h };
        draw_utf8_text(text, len, rc, style, col, ALIGN_LEADING);
        float w = measure_utf8(text, len, style);
        return w < max_w ? w : max_w;
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

static LinuxSdfChromeRenderer g_linux_ui;

} // namespace

extern "C" void anvil_ui_dwm_configure_window(void*, int, int) {}

extern "C" int anvil_ui_render_top_chrome_d2d(
    uint8_t* rgba_pixels,
    uint32_t width,
    uint32_t height,
    const AnvilUiChromeState* s
) {
    if (!rgba_pixels || !s || width == 0 || height == 0) return 0;
    g_linux_ui.ensure_init();
    g_linux_ui.target_rgba = rgba_pixels;
    g_linux_ui.target_w = width;
    g_linux_ui.target_h = height;

    // 1. Seamless Tab Strip Background
    g_linux_ui.fill_rect({ 0.0f, 0.0f, s->window_width, s->tab_strip_height }, s->strip_bg);

    // 2. Modern Curved Tabs
    for (size_t i = 0; i < s->tab_count && i < ANVIL_UI_MAX_TABS; ++i) {
        const AnvilUiTabInfo& tab = s->tabs[i];
        if (tab.rect.w <= 0.0f) continue;
        AnvilUiColor bg = tab.is_active ? s->nav_top_bg : (tab.is_hovered ? s->control_bg : s->tab_idle_bg);
        AnvilUiColor fg = (tab.is_active || tab.is_hovered) ? s->ink_primary : s->ink_muted;
        AnvilUiColor border = tab.is_active ? s->divider : (tab.is_hovered ? s->divider : s->strip_bg);

        if (tab.is_active) {
            AnvilUiRect r_ext = { tab.rect.x, tab.rect.y, tab.rect.w, tab.rect.h + 4.0f };
            g_linux_ui.fill_rounded(r_ext, 8.0f, 1.0f, bg, border);
            g_linux_ui.fill_rect(
                { tab.rect.x + 1.0f, s->tab_strip_height - 2.0f, tab.rect.w - 2.0f, 4.0f },
                s->nav_top_bg
            );
            AnvilUiRect bar = { tab.rect.x + 12.0f, tab.rect.y + 1.0f, (tab.rect.w - 24.0f > 8.0f ? tab.rect.w - 24.0f : 8.0f), 2.5f };
            g_linux_ui.fill_rounded(bar, 1.25f, 0.0f, s->accent, s->accent);
        } else {
            AnvilUiRect r_idle = { tab.rect.x, tab.rect.y + 2.0f, tab.rect.w, tab.rect.h - 4.0f };
            g_linux_ui.fill_rounded(r_idle, 7.0f, 0.0f, bg, border);
            if (i + 1 < s->tab_count && !s->tabs[i + 1].is_active) {
                float sep_x = tab.rect.x + tab.rect.w + 2.0f;
                g_linux_ui.draw_line(sep_x, tab.rect.y + 8.0f, sep_x, tab.rect.y + tab.rect.h - 8.0f, 1.0f, s->divider);
            }
        }

        float dot_cx = tab.rect.x + 13.0f;
        float dot_cy = tab.rect.y + tab.rect.h * 0.5f;
        g_linux_ui.fill_circle(dot_cx, dot_cy, 3.2f, tab.dot_color);
        if (tab.is_pinned) {
            g_linux_ui.stroke_circle(dot_cx, dot_cy, 5.4f, 1.2f, s->accent);
        }

        float text_right_pad = tab.can_close ? 28.0f : 10.0f;
        AnvilUiRect text_rc = {
            tab.rect.x + 22.0f,
            tab.rect.y + 1.0f,
            (tab.rect.w - 22.0f - text_right_pad > 8.0f) ? (tab.rect.w - 22.0f - text_right_pad) : 8.0f,
            tab.rect.h - 2.0f
        };
        g_linux_ui.draw_utf8_text(
            tab.title_ptr,
            tab.title_len,
            text_rc,
            tab.is_active ? g_linux_ui.fmt_ui_semibold : g_linux_ui.fmt_ui_regular,
            fg
        );

        if (tab.can_close) {
            if (tab.is_close_hovered) {
                g_linux_ui.fill_rounded(tab.close_rect, 4.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            g_linux_ui.draw_close_x(
                tab.close_rect,
                3.5f,
                1.4f,
                tab.is_close_hovered ? s->ink_primary : s->ink_muted
            );
        }
    }

    if (s->new_tab_hovered) {
        g_linux_ui.fill_rounded(s->new_tab_rect, s->new_tab_rect.h * 0.5f, 0.0f, s->control_hover_bg, s->divider);
    }
    g_linux_ui.draw_plus(s->new_tab_rect, 5.0f, 1.5f, s->new_tab_hovered ? s->ink_primary : s->ink_muted);

    if (s->tab_search_btn_rect.w > 0.0f) {
        if (s->tab_search_btn_hovered || s->tab_search_active) {
            g_linux_ui.fill_rounded(
                s->tab_search_btn_rect,
                s->tab_search_btn_rect.h * 0.5f,
                s->tab_search_active ? 1.0f : 0.0f,
                s->tab_search_active ? s->control_active_bg : s->control_hover_bg,
                s->tab_search_active ? s->accent : s->divider
            );
        }
        float scx = s->tab_search_btn_rect.x + s->tab_search_btn_rect.w * 0.5f;
        float scy = s->tab_search_btn_rect.y + s->tab_search_btn_rect.h * 0.5f;
        AnvilUiColor sfg = s->tab_search_active ? s->accent : (s->tab_search_btn_hovered ? s->ink_primary : s->ink_muted);
        g_linux_ui.draw_line(scx - 4.0f, scy - 1.5f, scx, scy + 2.5f, 1.5f, sfg);
        g_linux_ui.draw_line(scx + 4.0f, scy - 1.5f, scx, scy + 2.5f, 1.5f, sfg);
    }

    // 3. Top-Right Caption Buttons (Minimize, Maximize/Restore, Close)
    if (s->win_min_rect.w > 0.0f) {
        if (s->win_min_hovered) {
            g_linux_ui.fill_rect(s->win_min_rect, s->control_hover_bg);
        }
        float mcx = s->win_min_rect.x + s->win_min_rect.w * 0.5f;
        float mcy = s->win_min_rect.y + s->win_min_rect.h * 0.5f;
        g_linux_ui.draw_line(mcx - 5.0f, mcy, mcx + 5.0f, mcy, 1.2f, s->ink_primary);

        if (s->win_max_hovered) {
            g_linux_ui.fill_rect(s->win_max_rect, s->control_hover_bg);
        }
        float xcx = s->win_max_rect.x + s->win_max_rect.w * 0.5f;
        float xcy = s->win_max_rect.y + s->win_max_rect.h * 0.5f;
        g_linux_ui.draw_rect_outline(xcx - 4.5f, xcy - 4.5f, xcx + 4.5f, xcy + 4.5f, 1.2f, s->ink_primary);

        AnvilUiColor close_fg = s->ink_primary;
        if (s->win_close_hovered) {
            AnvilUiColor crimson = { 232, 17, 35, 255 };
            g_linux_ui.fill_rect(s->win_close_rect, crimson);
            close_fg = { 255, 255, 255, 255 };
        }
        g_linux_ui.draw_close_x(s->win_close_rect, 4.5f, 1.3f, close_fg);
    }

    // 4. Navigation Bar Background
    g_linux_ui.fill_rect(
        { 0.0f, s->tab_strip_height, s->window_width, s->top_bar_height - s->tab_strip_height },
        s->nav_bg
    );
    g_linux_ui.fill_rect(
        { 0.0f, s->top_bar_height - 1.0f, s->window_width, 1.0f },
        s->divider
    );
    if (s->is_loading) {
        g_linux_ui.fill_rect(
            { 0.0f, s->top_bar_height - 2.5f, s->window_width * 0.68f, 2.5f },
            s->accent
        );
        g_linux_ui.fill_rect(
            { s->window_width * 0.68f, s->top_bar_height - 2.5f, s->window_width * 0.32f, 2.5f },
            s->accent_cyan
        );
    }

    AnvilUiColor fg = s->ink_primary;
    g_linux_ui.draw_nav_button_shell(s->back_rect, s->can_go_back, s->back_hovered, 0, s, &fg);
    {
        float cx = s->back_rect.x + s->back_rect.w * 0.5f;
        float cy = s->back_rect.y + s->back_rect.h * 0.5f;
        g_linux_ui.draw_line(cx - 5.5f, cy, cx + 5.5f, cy, 1.6f, fg);
        g_linux_ui.draw_line(cx - 5.5f, cy, cx - 1.0f, cy - 4.5f, 1.6f, fg);
        g_linux_ui.draw_line(cx - 5.5f, cy, cx - 1.0f, cy + 4.5f, 1.6f, fg);
    }

    g_linux_ui.draw_nav_button_shell(s->forward_rect, s->can_go_forward, s->forward_hovered, 0, s, &fg);
    {
        float cx = s->forward_rect.x + s->forward_rect.w * 0.5f;
        float cy = s->forward_rect.y + s->forward_rect.h * 0.5f;
        g_linux_ui.draw_line(cx - 5.5f, cy, cx + 5.5f, cy, 1.6f, fg);
        g_linux_ui.draw_line(cx + 5.5f, cy, cx + 1.0f, cy - 4.5f, 1.6f, fg);
        g_linux_ui.draw_line(cx + 5.5f, cy, cx + 1.0f, cy + 4.5f, 1.6f, fg);
    }

    g_linux_ui.draw_nav_button_shell(s->reload_rect, s->has_current_page || s->is_loading, s->reload_hovered, 0, s, &fg);
    if (s->is_loading) {
        g_linux_ui.draw_close_x(s->reload_rect, 4.5f, 1.6f, s->accent);
    } else {
        float cx = s->reload_rect.x + s->reload_rect.w * 0.5f;
        float cy = s->reload_rect.y + s->reload_rect.h * 0.5f;
        g_linux_ui.stroke_circle(cx, cy, 5.2f, 1.6f, fg);
        g_linux_ui.draw_line(cx + 2.5f, cy - 5.8f, cx + 5.8f, cy - 2.5f, 1.6f, fg);
        g_linux_ui.draw_line(cx + 1.5f, cy - 2.5f, cx + 5.8f, cy - 2.5f, 1.6f, fg);
    }

    if (s->home_rect.w > 0.0f) {
    g_linux_ui.draw_nav_button_shell(s->home_rect, 1, s->home_hovered, 0, s, &fg);
    {
        float cx = s->home_rect.x + s->home_rect.w * 0.5f;
        float cy = s->home_rect.y + s->home_rect.h * 0.5f;
        g_linux_ui.draw_line(cx - 6.0f, cy - 0.5f, cx, cy - 6.0f, 1.6f, fg);
        g_linux_ui.draw_line(cx, cy - 6.0f, cx + 6.0f, cy - 0.5f, 1.6f, fg);
        g_linux_ui.draw_rect_outline(cx - 4.0f, cy - 0.5f, cx + 4.0f, cy + 5.5f, 1.5f, fg);
    }
    }

    // 5. Full-Capsule Pill Omnibox
    AnvilUiColor omni_border = s->is_editing_address ? s->accent : (s->omnibox_hovered ? s->accent_cyan : s->divider);
    float omni_border_w = s->is_editing_address ? 1.5f : 1.0f;
    g_linux_ui.fill_rounded(s->omnibox_rect, s->omnibox_rect.h * 0.5f, omni_border_w, s->field_bg, omni_border);

    AnvilUiRect sec_inner = {
        s->security_badge_rect.x + 3.0f,
        s->security_badge_rect.y + 3.0f,
        s->security_badge_rect.w - 6.0f,
        s->security_badge_rect.h - 6.0f
    };
    g_linux_ui.fill_rounded(
        sec_inner,
        sec_inner.h * 0.5f,
        1.0f,
        s->sec_bg,
        s->sec_hovered ? s->accent : s->sec_fg
    );
    g_linux_ui.draw_utf8_text(
        s->sec_label_ptr,
        s->sec_label_len,
        sec_inner,
        g_linux_ui.fmt_ui_badge,
        s->sec_fg,
        ALIGN_CENTER
    );

    float text_x = s->address_rect.x + 8.0f;
    float right_reserve = (s->show_clear_btn ? 32.0f : 12.0f) + (s->show_reader_btn ? 30.0f : 0.0f);
    float text_max_w = s->address_rect.w - right_reserve;
    if (text_max_w < 16.0f) text_max_w = 16.0f;

    if (s->is_editing_address && s->is_input_select_all && s->selection_char_count > 0) {
        float sel_w = static_cast<float>(s->selection_char_count) * 7.2f + 8.0f;
        if (sel_w > text_max_w) sel_w = text_max_w;
        AnvilUiRect sel_rc = { text_x - 2.0f, s->address_rect.y + 4.0f, sel_w, s->address_rect.h - 8.0f };
        g_linux_ui.fill_rounded(sel_rc, 5.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
    }

    if (s->use_domain_highlight && s->url_domain_len > 0) {
        float cur_x = text_x;
        float rem_w = text_max_w;
        if (s->url_scheme_len > 0 && rem_w > 8.0f) {
            float w1 = g_linux_ui.measure_and_draw_segment(
                s->url_scheme_ptr,
                s->url_scheme_len,
                cur_x,
                s->address_rect.y,
                rem_w,
                s->address_rect.h,
                g_linux_ui.fmt_ui_regular,
                s->ink_muted
            );
            cur_x += w1;
            rem_w -= w1;
        }
        if (rem_w > 8.0f) {
            float w2 = g_linux_ui.measure_and_draw_segment(
                s->url_domain_ptr,
                s->url_domain_len,
                cur_x,
                s->address_rect.y,
                rem_w,
                s->address_rect.h,
                g_linux_ui.fmt_ui_semibold,
                s->ink_primary
            );
            cur_x += w2;
            rem_w -= w2;
        }
        if (s->url_path_len > 0 && rem_w > 8.0f) {
            AnvilUiRect path_rc = { cur_x, s->address_rect.y, rem_w, s->address_rect.h };
            g_linux_ui.draw_utf8_text(
                s->url_path_ptr,
                s->url_path_len,
                path_rc,
                g_linux_ui.fmt_ui_regular,
                s->ink_muted
            );
        }
    } else {
        AnvilUiRect addr_text_rc = { text_x, s->address_rect.y, text_max_w, s->address_rect.h };
        g_linux_ui.draw_utf8_text(
            s->display_text_ptr,
            s->display_text_len,
            addr_text_rc,
            g_linux_ui.fmt_ui_regular,
            s->display_text_muted ? s->ink_muted : s->ink_primary
        );
    }

    if (s->show_clear_btn) {
        if (s->clear_hovered) {
            g_linux_ui.fill_rounded(s->clear_address_rect, s->clear_address_rect.h * 0.5f, 0.0f, s->control_hover_bg, s->control_hover_bg);
        }
        g_linux_ui.draw_close_x(s->clear_address_rect, 3.5f, 1.4f, s->clear_hovered ? s->accent : s->ink_muted);
    }

    if (s->show_reader_btn && s->reader_btn_rect.w > 0.0f) {
        AnvilUiRect r_inner = {
            s->reader_btn_rect.x + 2.0f,
            s->reader_btn_rect.y + 2.0f,
            s->reader_btn_rect.w - 4.0f,
            s->reader_btn_rect.h - 4.0f
        };
        if (s->reader_active || s->reader_hovered) {
            g_linux_ui.fill_rounded(
                r_inner,
                r_inner.h * 0.5f,
                s->reader_active ? 1.0f : 0.0f,
                s->reader_active ? s->control_active_bg : s->control_hover_bg,
                s->accent
            );
        }
        AnvilUiColor r_col = (s->reader_active || s->reader_hovered) ? s->accent : s->ink_muted;
        float rx = s->reader_btn_rect.x + 7.5f;
        float ry = s->reader_btn_rect.y + 9.0f;
        g_linux_ui.draw_line(rx, ry, rx + 13.0f, ry, 1.4f, r_col);
        g_linux_ui.draw_line(rx, ry + 3.5f, rx + 13.0f, ry + 3.5f, 1.4f, r_col);
        g_linux_ui.draw_line(rx, ry + 7.0f, rx + 9.5f, ry + 7.0f, 1.4f, r_col);
        g_linux_ui.draw_line(rx, ry + 10.5f, rx + 11.5f, ry + 10.5f, 1.4f, r_col);
    }

    if (s->star_hovered || s->is_bookmarked) {
        AnvilUiRect star_inner = {
            s->star_rect.x + 3.0f,
            s->star_rect.y + 3.0f,
            s->star_rect.w - 6.0f,
            s->star_rect.h - 6.0f
        };
        if (s->star_hovered) {
            g_linux_ui.fill_rounded(star_inner, star_inner.h * 0.5f, 0.0f, s->control_hover_bg, s->accent);
        }
    }
    {
        AnvilUiColor star_col = (s->is_bookmarked || s->star_hovered) ? s->accent : s->ink_muted;
        float cx = s->star_rect.x + s->star_rect.w * 0.5f;
        float cy = s->star_rect.y + s->star_rect.h * 0.5f;
        g_linux_ui.draw_line(cx, cy - 5.5f, cx, cy + 5.5f, 1.5f, star_col);
        g_linux_ui.draw_line(cx - 5.5f, cy, cx + 5.5f, cy, 1.5f, star_col);
        g_linux_ui.draw_line(cx - 3.5f, cy - 3.5f, cx + 3.5f, cy + 3.5f, 1.4f, star_col);
        g_linux_ui.draw_line(cx + 3.5f, cy - 3.5f, cx - 3.5f, cy + 3.5f, 1.4f, star_col);
    }

    // 6. Right Action Cluster
    if (s->shields_pill_rect.w > 0.0f) {
        AnvilUiColor sh_bg = s->shields_active ? s->control_active_bg : (s->shields_hovered ? s->control_hover_bg : s->control_bg);
        AnvilUiColor sh_border = (s->shields_active || s->shields_hovered) ? s->accent : s->divider;
        g_linux_ui.fill_rounded(s->shields_pill_rect, s->shields_pill_rect.h * 0.5f, 1.0f, sh_bg, sh_border);
        g_linux_ui.draw_utf8_text(
            s->shields_text_ptr,
            s->shields_text_len,
            s->shields_pill_rect,
            g_linux_ui.fmt_ui_badge,
            s->accent,
            ALIGN_CENTER
        );
    }

    {
        AnvilUiColor zm_bg = s->zoom_hovered ? s->control_hover_bg : s->control_bg;
        AnvilUiColor zm_border = (s->zoom_non_default || s->zoom_hovered) ? s->accent : s->divider;
        g_linux_ui.fill_rounded(s->zoom_pill_rect, s->zoom_pill_rect.h * 0.5f, 1.0f, zm_bg, zm_border);
        g_linux_ui.draw_utf8_text(
            s->zoom_text_ptr,
            s->zoom_text_len,
            s->zoom_pill_rect,
            g_linux_ui.fmt_ui_small,
            (s->zoom_non_default || s->zoom_hovered) ? s->accent : s->ink_primary,
            ALIGN_CENTER
        );
    }

    if (s->settings_btn_rect.w > 0.0f) {
    g_linux_ui.draw_nav_button_shell(s->settings_btn_rect, 1, s->settings_hovered, s->settings_active, s, &fg);
    {
        float x0 = s->settings_btn_rect.x + 7.0f;
        float y0 = s->settings_btn_rect.y + 9.0f;
        g_linux_ui.draw_line(x0, y0, x0 + 14.0f, y0, 1.5f, fg);
        g_linux_ui.draw_line(x0, y0 + 5.0f, x0 + 14.0f, y0 + 5.0f, 1.5f, fg);
        g_linux_ui.draw_line(x0, y0 + 10.0f, x0 + 14.0f, y0 + 10.0f, 1.5f, fg);
        g_linux_ui.fill_circle(x0 + 4.0f, y0, 2.2f, s->accent);
        g_linux_ui.fill_circle(x0 + 10.0f, y0 + 5.0f, 2.2f, s->accent);
        g_linux_ui.fill_circle(x0 + 6.0f, y0 + 10.0f, 2.2f, s->accent);
    }
    }

    g_linux_ui.draw_nav_button_shell(s->downloads_btn_rect, 1, s->downloads_hovered, s->downloads_active, s, &fg);
    {
        float cx = s->downloads_btn_rect.x + s->downloads_btn_rect.w * 0.5f;
        float cy = s->downloads_btn_rect.y + s->downloads_btn_rect.h * 0.5f;
        g_linux_ui.draw_line(cx, cy - 5.5f, cx, cy + 2.5f, 1.6f, fg);
        g_linux_ui.draw_line(cx - 3.5f, cy - 1.0f, cx, cy + 2.5f, 1.6f, fg);
        g_linux_ui.draw_line(cx + 3.5f, cy - 1.0f, cx, cy + 2.5f, 1.6f, fg);
        g_linux_ui.draw_line(cx - 5.5f, cy + 5.0f, cx + 5.5f, cy + 5.0f, 1.6f, fg);
        if (s->downloads_has_entries) {
            AnvilUiColor emerald = { 110, 231, 183, 255 };
            g_linux_ui.fill_circle(s->downloads_btn_rect.x + s->downloads_btn_rect.w - 5.5f, s->downloads_btn_rect.y + 5.5f, 2.8f, emerald);
        }
    }

    if (s->inspector_btn_rect.w > 0.0f) {
    g_linux_ui.draw_nav_button_shell(s->inspector_btn_rect, 1, s->inspector_hovered, s->inspector_active, s, &fg);
    {
        float cx = s->inspector_btn_rect.x + s->inspector_btn_rect.w * 0.5f;
        float cy = s->inspector_btn_rect.y + s->inspector_btn_rect.h * 0.5f;
        g_linux_ui.draw_line(cx - 2.0f, cy - 4.0f, cx - 6.0f, cy, 1.5f, fg);
        g_linux_ui.draw_line(cx - 6.0f, cy, cx - 2.0f, cy + 4.0f, 1.5f, fg);
        g_linux_ui.draw_line(cx + 2.0f, cy - 4.0f, cx + 6.0f, cy, 1.5f, fg);
        g_linux_ui.draw_line(cx + 6.0f, cy, cx + 2.0f, cy + 4.0f, 1.5f, fg);
    }
    }

    g_linux_ui.draw_nav_button_shell(s->app_menu_btn_rect, 1, s->app_menu_hovered, s->app_menu_active, s, &fg);
    {
        float x0 = s->app_menu_btn_rect.x + 7.0f;
        float y0 = s->app_menu_btn_rect.y + 9.0f;
        g_linux_ui.draw_line(x0, y0, x0 + 14.0f, y0, 1.6f, fg);
        g_linux_ui.draw_line(x0, y0 + 5.0f, x0 + 14.0f, y0 + 5.0f, 1.6f, fg);
        g_linux_ui.draw_line(x0, y0 + 10.0f, x0 + 14.0f, y0 + 10.0f, 1.6f, fg);
    }

    // 7. Bookmarks Bar
    if (s->bookmark_count > 0 && s->top_header_height > s->top_bar_height) {
        g_linux_ui.fill_rect(
            { 0.0f, s->top_bar_height, s->window_width, s->top_header_height - s->top_bar_height },
            s->nav_bg
        );
        g_linux_ui.fill_rect(
            { 0.0f, s->top_header_height - 1.0f, s->window_width, 1.0f },
            s->divider
        );
        for (size_t i = 0; i < s->bookmark_count && i < ANVIL_UI_MAX_BOOKMARKS; ++i) {
            const AnvilUiBookmarkInfo& bm = s->bookmarks[i];
            if (bm.rect.x + bm.rect.w > s->window_width - 8.0f) break;
            g_linux_ui.fill_rounded(
                bm.rect,
                bm.rect.h * 0.5f,
                1.0f,
                bm.is_hovered ? s->control_hover_bg : s->control_bg,
                bm.is_hovered ? s->accent : s->divider
            );
            char mono = 'B';
            for (size_t k = 0; k < bm.label_len && bm.label_ptr; ++k) {
                char c = bm.label_ptr[k];
                if ((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9')) {
                    mono = (c >= 'a' && c <= 'z') ? static_cast<char>(c - 32) : c;
                    break;
                }
            }
            float bcx = bm.rect.x + 11.0f;
            float bcy = bm.rect.y + bm.rect.h * 0.5f;
            AnvilUiColor badge_col = (i % 2 == 0) ? s->accent : s->accent_cyan;
            g_linux_ui.fill_circle(bcx, bcy, 6.2f, badge_col);
            AnvilUiRect mono_rc = { bm.rect.x + 4.8f, bm.rect.y, 12.4f, bm.rect.h };
            g_linux_ui.draw_utf8_text(&mono, 1, mono_rc, g_linux_ui.fmt_ui_badge, s->strip_bg, ALIGN_CENTER);
            AnvilUiRect bm_txt = { bm.rect.x + 21.0f, bm.rect.y, bm.rect.w - 27.0f, bm.rect.h };
            g_linux_ui.draw_utf8_text(bm.label_ptr, bm.label_len, bm_txt, g_linux_ui.fmt_ui_small, s->ink_primary);
        }
    }

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
        || s->select_dropdown_open
        || s->site_popover_open
        || s->tab_preview_open
        || s->tab_search_open
        || s->downloads_popover_open;
    if (!any_overlay) return 1;

    g_linux_ui.ensure_init();
    g_linux_ui.target_rgba = rgba_pixels;
    g_linux_ui.target_w = width;
    g_linux_ui.target_h = height;

    // 1. Floating Bottom-Left Link-Hover / Loading Status Pill
    if (s->show_floating_status_pill && s->status_pill_len > 0) {
        float max_pill_w = s->window_width * 0.62f;
        if (max_pill_w < 180.0f) max_pill_w = 180.0f;
        float est_w = static_cast<float>(s->status_pill_len) * 6.8f + 24.0f;
        if (est_w > max_pill_w) est_w = max_pill_w;
        if (est_w < 120.0f) est_w = 120.0f;
        AnvilUiRect pill = { 12.0f, s->window_height - 34.0f, est_w, 24.0f };
        g_linux_ui.draw_shadow_card(pill, 12.0f, s->overlay_bg, s->divider);
        AnvilUiRect txt_rc = { pill.x + 11.0f, pill.y, pill.w - 22.0f, pill.h };
        g_linux_ui.draw_utf8_text(s->status_pill_ptr, s->status_pill_len, txt_rc, g_linux_ui.fmt_ui_small, s->ink_muted);
    }

    // 2. Floating Find-in-Page Bar
    if (s->find_open) {
        g_linux_ui.draw_shadow_card(s->find_bar_rect, 10.0f, s->overlay_bg, s->accent);
        AnvilUiRect field = { s->find_bar_rect.x + 6.0f, s->find_bar_rect.y + 4.0f, 158.0f, s->find_bar_rect.h - 8.0f };
        AnvilUiColor error_bg = { 75, 30, 30, 255 }, error_fg = { 255, 160, 150, 255 };
        g_linux_ui.fill_rounded(field, 4.0f, 1.0f, s->find_no_matches ? error_bg : s->field_bg,
            s->find_no_matches ? error_fg : s->accent);
        AnvilUiRect q_rc = { field.x + 6.0f, field.y, field.w - 12.0f, field.h };
        g_linux_ui.draw_utf8_text(s->find_query_ptr, s->find_query_len, q_rc, g_linux_ui.fmt_ui_regular, s->ink_primary);

        AnvilUiRect c_rc = { s->find_bar_rect.x + 166.0f, s->find_bar_rect.y, 58.0f, s->find_bar_rect.h };
        g_linux_ui.draw_utf8_text(s->find_count_ptr, s->find_count_len, c_rc, g_linux_ui.fmt_ui_small, s->accent, ALIGN_TRAILING);

        auto draw_find_btn = [&](AnvilUiRect r, uint8_t hov, const char* g) {
            g_linux_ui.fill_rounded(r, 6.0f, 1.0f, hov ? s->control_hover_bg : s->control_bg, hov ? s->accent : s->divider);
            g_linux_ui.draw_utf8_text(g, 1, r, g_linux_ui.fmt_ui_semibold, s->ink_primary, ALIGN_CENTER);
        };
        if (s->find_case_rect.w > 0.0f) {
            g_linux_ui.fill_rounded(
                s->find_case_rect,
                6.0f,
                1.0f,
                s->find_case_sensitive ? s->control_active_bg : (s->find_case_hovered ? s->control_hover_bg : s->control_bg),
                (s->find_case_sensitive || s->find_case_hovered) ? s->accent : s->divider
            );
            g_linux_ui.draw_utf8_text(
                "Aa",
                2,
                s->find_case_rect,
                g_linux_ui.fmt_ui_badge,
                s->find_case_sensitive ? s->accent : s->ink_primary,
                ALIGN_CENTER
            );
        }
        draw_find_btn(s->find_prev_rect, s->find_prev_hovered, "<");
        draw_find_btn(s->find_next_rect, s->find_next_hovered, ">");
        g_linux_ui.fill_rounded(
            s->find_close_rect,
            6.0f,
            1.0f,
            s->find_close_hovered ? s->control_hover_bg : s->control_bg,
            s->find_close_hovered ? s->accent : s->divider
        );
        g_linux_ui.draw_close_x(s->find_close_rect, 3.5f, 1.4f, s->ink_primary);
    }

    // 3. Floating Omnibox Autocomplete Suggestion Card
    if (s->suggestion_count > 0) {
        g_linux_ui.draw_shadow_card(s->suggestions_rect, 10.0f, s->overlay_bg, s->accent);
        for (size_t i = 0; i < s->suggestion_count && i < ANVIL_UI_MAX_SUGGESTIONS; ++i) {
            const AnvilUiSuggestionInfo& sug = s->suggestions[i];
            if (sug.is_selected || sug.is_hovered) {
                g_linux_ui.fill_rounded(
                    sug.rect,
                    6.0f,
                    0.0f,
                    sug.is_selected ? s->control_active_bg : s->control_hover_bg,
                    s->accent
                );
            }
            // Small local icons: home, bookmark, clock, search.
            float cx = sug.rect.x + 16.0f, cy = sug.rect.y + 12.0f;
            if (sug.kind == 0) {
                g_linux_ui.draw_line(cx-6, cy, cx, cy-6, 1.5f, s->accent_cyan);
                g_linux_ui.draw_line(cx, cy-6, cx+6, cy, 1.5f, s->accent_cyan);
                g_linux_ui.draw_line(cx-4, cy, cx-4, cy+6, 1.5f, s->accent_cyan);
                g_linux_ui.draw_line(cx+4, cy, cx+4, cy+6, 1.5f, s->accent_cyan);
            } else if (sug.kind == 1) {
                g_linux_ui.draw_line(cx, cy-6, cx, cy+6, 1.5f, s->accent_cyan);
                g_linux_ui.draw_line(cx-6, cy, cx+6, cy, 1.5f, s->accent_cyan);
                g_linux_ui.draw_line(cx-4, cy-4, cx+4, cy+4, 1.5f, s->accent_cyan);
                g_linux_ui.draw_line(cx+4, cy-4, cx-4, cy+4, 1.5f, s->accent_cyan);
            } else {
                AnvilUiRect circle = {cx-6, cy-6, 12, 12};
                g_linux_ui.fill_rounded(circle, 6, 1.5f, s->overlay_bg, s->accent_cyan);
                if (sug.kind == 2) {
                    g_linux_ui.draw_line(cx, cy-4, cx, cy, 1.5f, s->accent_cyan);
                    g_linux_ui.draw_line(cx, cy, cx+3, cy, 1.5f, s->accent_cyan);
                } else { g_linux_ui.draw_line(cx+4, cy+4, cx+8, cy+8, 1.5f, s->accent_cyan); }
            }
            float tx = sug.rect.x + 32.0f;
            float rem = sug.rect.w - 40.0f;
            AnvilUiColor col = (sug.is_selected || sug.is_hovered) ? s->accent : s->ink_primary;
            size_t start = sug.match_start <= sug.title_len ? sug.match_start : 0;
            size_t len = sug.match_len <= sug.title_len - start ? sug.match_len : 0;
            const size_t offsets[3] = {0, start, start + len};
            const size_t lengths[3] = {start, len, sug.title_len - start - len};
            for (int part = 0; part < 3 && rem > 0.0f; ++part) {
                if (!lengths[part]) continue;
                float advance = g_linux_ui.measure_and_draw_segment(sug.title_ptr + offsets[part], lengths[part],
                    tx, sug.rect.y, rem, sug.rect.h, part == 1 ? g_linux_ui.fmt_ui_semibold : g_linux_ui.fmt_ui_regular, col);
                tx += advance; rem -= advance;
            }
        }
    }

    // 4. Floating Hamburger App Menu Card
    if (s->app_menu_open) {
        g_linux_ui.draw_shadow_card(s->app_menu_rect, 10.0f, s->overlay_bg, s->divider);
        AnvilUiRect hdr_rc = { s->app_menu_rect.x + 14.0f, s->app_menu_rect.y + 2.0f, s->app_menu_rect.w - 28.0f, 26.0f };
        g_linux_ui.draw_utf8_text("ANVIL BROWSER", 13, hdr_rc, g_linux_ui.fmt_ui_badge, s->accent);
        g_linux_ui.draw_line(
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
                g_linux_ui.draw_line(row.rect.x + 6.0f, row.rect.y - 1.0f, row.rect.x + row.rect.w - 6.0f, row.rect.y - 1.0f, 1.0f, s->divider);
            }
            if (row.is_hovered) {
                g_linux_ui.fill_rounded(row.rect, 6.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 76.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                g_linux_ui.fmt_ui_regular,
                row.is_hovered ? s->accent : s->ink_primary
            );
            AnvilUiRect src = { row.rect.x + row.rect.w - 68.0f, row.rect.y, 58.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.shortcut_ptr,
                row.shortcut_len,
                src,
                g_linux_ui.fmt_ui_small,
                row.is_hovered ? s->accent_cyan : s->ink_muted,
                ALIGN_TRAILING
            );
        }
    }

    // 5. Floating Right-Click Context Menu Card
    if (s->context_menu_open) {
        g_linux_ui.draw_shadow_card(s->context_menu_rect, 10.0f, s->overlay_bg, s->divider);
        for (size_t i = 0; i < s->context_menu_row_count && i < ANVIL_UI_MAX_MENU_ROWS; ++i) {
            const AnvilUiMenuRowInfo& row = s->context_menu_rows[i];
            if (row.has_divider_above) {
                g_linux_ui.draw_line(row.rect.x + 6.0f, row.rect.y - 1.0f, row.rect.x + row.rect.w - 6.0f, row.rect.y - 1.0f, 1.0f, s->divider);
            }
            if (row.is_hovered) {
                g_linux_ui.fill_rounded(row.rect, 6.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 78.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                g_linux_ui.fmt_ui_regular,
                row.is_hovered ? s->accent : s->ink_primary
            );
            AnvilUiRect src = { row.rect.x + row.rect.w - 72.0f, row.rect.y, 62.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.shortcut_ptr,
                row.shortcut_len,
                src,
                g_linux_ui.fmt_ui_small,
                row.is_hovered ? s->accent_cyan : s->ink_muted,
                ALIGN_TRAILING
            );
        }
    }

    // 6. Floating <select> Dropdown Card
    if (s->select_dropdown_open) {
        g_linux_ui.draw_shadow_card(s->select_dropdown_rect, 8.0f, s->overlay_bg, s->accent);
        for (size_t i = 0; i < s->select_dropdown_row_count && i < ANVIL_UI_MAX_SELECT_ROWS; ++i) {
            const AnvilUiSelectRowInfo& row = s->select_dropdown_rows[i];
            if (row.is_hovered || row.is_selected) {
                g_linux_ui.fill_rounded(
                    row.rect,
                    5.0f,
                    0.0f,
                    row.is_hovered ? s->control_hover_bg : s->control_bg,
                    s->control_hover_bg
                );
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 20.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                row.is_selected ? g_linux_ui.fmt_ui_semibold : g_linux_ui.fmt_ui_regular,
                (row.is_hovered || row.is_selected) ? s->accent : s->ink_primary
            );
        }
    }

    // 7. Floating Site Information, Shields & Permissions Popover Card
    if (s->site_popover_open) {
        g_linux_ui.draw_shadow_card(s->site_popover_rect, 10.0f, s->overlay_bg, s->accent);
        AnvilUiRect title_rc = { s->site_popover_rect.x + 14.0f, s->site_popover_rect.y + 6.0f, s->site_popover_rect.w - 28.0f, 20.0f };
        g_linux_ui.draw_utf8_text(s->site_popover_title_ptr, s->site_popover_title_len, title_rc, g_linux_ui.fmt_ui_semibold, s->ink_primary);
        AnvilUiRect sub_rc = { s->site_popover_rect.x + 14.0f, s->site_popover_rect.y + 24.0f, s->site_popover_rect.w - 28.0f, 18.0f };
        g_linux_ui.draw_utf8_text(s->site_popover_sub_ptr, s->site_popover_sub_len, sub_rc, g_linux_ui.fmt_ui_badge, s->sec_fg);
        g_linux_ui.draw_line(
            s->site_popover_rect.x + 8.0f,
            s->site_popover_rect.y + 46.0f,
            s->site_popover_rect.x + s->site_popover_rect.w - 8.0f,
            s->site_popover_rect.y + 46.0f,
            1.0f,
            s->divider
        );
        for (size_t i = 0; i < s->site_popover_row_count && i < ANVIL_UI_MAX_MENU_ROWS; ++i) {
            const AnvilUiMenuRowInfo& row = s->site_popover_rows[i];
            if (row.has_divider_above) {
                g_linux_ui.draw_line(row.rect.x + 6.0f, row.rect.y - 1.0f, row.rect.x + row.rect.w - 6.0f, row.rect.y - 1.0f, 1.0f, s->divider);
            }
            if (row.is_hovered) {
                g_linux_ui.fill_rounded(row.rect, 6.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 92.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                g_linux_ui.fmt_ui_regular,
                row.is_hovered ? s->accent : s->ink_primary
            );
            AnvilUiRect src = { row.rect.x + row.rect.w - 84.0f, row.rect.y, 74.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.shortcut_ptr,
                row.shortcut_len,
                src,
                g_linux_ui.fmt_ui_badge,
                row.is_hovered ? s->accent : s->accent_cyan,
                ALIGN_TRAILING
            );
        }
    }

    // 8. Floating Tab Hover Preview Card
    if (s->tab_preview_open) {
        g_linux_ui.draw_shadow_card(s->tab_preview_rect, 9.0f, s->overlay_bg, s->divider);
        AnvilUiRect bar = { s->tab_preview_rect.x + 10.0f, s->tab_preview_rect.y + 1.0f, s->tab_preview_rect.w - 20.0f, 2.5f };
        g_linux_ui.fill_rounded(bar, 1.25f, 0.0f, s->tab_preview_dot_color, s->tab_preview_dot_color);
        AnvilUiRect t_rc = { s->tab_preview_rect.x + 12.0f, s->tab_preview_rect.y + 6.0f, s->tab_preview_rect.w - 24.0f, 20.0f };
        g_linux_ui.draw_utf8_text(s->tab_preview_title_ptr, s->tab_preview_title_len, t_rc, g_linux_ui.fmt_ui_semibold, s->ink_primary);
        float dcx = s->tab_preview_rect.x + 16.0f;
        float dcy = s->tab_preview_rect.y + 35.0f;
        g_linux_ui.fill_circle(dcx, dcy, 3.2f, s->tab_preview_dot_color);
        AnvilUiRect u_rc = { s->tab_preview_rect.x + 24.0f, s->tab_preview_rect.y + 25.0f, s->tab_preview_rect.w - 36.0f, 18.0f };
        g_linux_ui.draw_utf8_text(s->tab_preview_url_ptr, s->tab_preview_url_len, u_rc, g_linux_ui.fmt_ui_small, s->ink_muted);
        AnvilUiRect m_rc = { s->tab_preview_rect.x + 12.0f, s->tab_preview_rect.y + 44.0f, s->tab_preview_rect.w - 24.0f, 18.0f };
        g_linux_ui.draw_utf8_text(s->tab_preview_meta_ptr, s->tab_preview_meta_len, m_rc, g_linux_ui.fmt_ui_badge, s->accent_cyan);
    }

    // 9. Floating Quick-Filter Tab Search & Switcher Card
    if (s->tab_search_open) {
        g_linux_ui.draw_shadow_card(s->tab_search_rect, 10.0f, s->overlay_bg, s->accent);
        AnvilUiRect hdr_rc = { s->tab_search_rect.x + 14.0f, s->tab_search_rect.y + 6.0f, s->tab_search_rect.w - 28.0f, 18.0f };
        g_linux_ui.draw_utf8_text("SEARCH OPEN TABS", 16, hdr_rc, g_linux_ui.fmt_ui_badge, s->accent);
        AnvilUiRect q_box = { s->tab_search_rect.x + 10.0f, s->tab_search_rect.y + 24.0f, s->tab_search_rect.w - 20.0f, 22.0f };
        g_linux_ui.fill_rounded(q_box, 6.0f, 1.0f, s->control_bg, s->accent);
        AnvilUiRect q_txt = { q_box.x + 8.0f, q_box.y, q_box.w - 16.0f, q_box.h };
        g_linux_ui.draw_utf8_text(s->tab_search_query_ptr, s->tab_search_query_len, q_txt, g_linux_ui.fmt_ui_small, s->ink_primary);
        g_linux_ui.draw_line(
            s->tab_search_rect.x + 8.0f,
            s->tab_search_rect.y + 50.0f,
            s->tab_search_rect.x + s->tab_search_rect.w - 8.0f,
            s->tab_search_rect.y + 50.0f,
            1.0f,
            s->divider
        );
        for (size_t i = 0; i < s->tab_search_row_count && i < ANVIL_UI_MAX_MENU_ROWS; ++i) {
            const AnvilUiMenuRowInfo& row = s->tab_search_rows[i];
            if (row.is_hovered) {
                g_linux_ui.fill_rounded(row.rect, 6.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 104.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                g_linux_ui.fmt_ui_regular,
                row.is_hovered ? s->accent : s->ink_primary
            );
            AnvilUiRect src = { row.rect.x + row.rect.w - 96.0f, row.rect.y, 86.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.shortcut_ptr,
                row.shortcut_len,
                src,
                g_linux_ui.fmt_ui_small,
                row.is_hovered ? s->accent_cyan : s->ink_muted,
                ALIGN_TRAILING
            );
        }
    }

    // 10. Floating Recent Downloads Shelf Popover Card
    if (s->downloads_popover_open) {
        g_linux_ui.draw_shadow_card(s->downloads_popover_rect, 10.0f, s->overlay_bg, s->accent);
        AnvilUiRect title_rc = { s->downloads_popover_rect.x + 14.0f, s->downloads_popover_rect.y + 6.0f, s->downloads_popover_rect.w - 28.0f, 20.0f };
        g_linux_ui.draw_utf8_text("Downloads & SHA-256 Vault", 25, title_rc, g_linux_ui.fmt_ui_semibold, s->ink_primary);
        AnvilUiRect sub_rc = { s->downloads_popover_rect.x + 14.0f, s->downloads_popover_rect.y + 24.0f, s->downloads_popover_rect.w - 28.0f, 18.0f };
        g_linux_ui.draw_utf8_text(s->downloads_popover_sub_ptr, s->downloads_popover_sub_len, sub_rc, g_linux_ui.fmt_ui_badge, s->accent_cyan);
        g_linux_ui.draw_line(
            s->downloads_popover_rect.x + 8.0f,
            s->downloads_popover_rect.y + 46.0f,
            s->downloads_popover_rect.x + s->downloads_popover_rect.w - 8.0f,
            s->downloads_popover_rect.y + 46.0f,
            1.0f,
            s->divider
        );
        for (size_t i = 0; i < s->downloads_popover_row_count && i < ANVIL_UI_MAX_MENU_ROWS; ++i) {
            const AnvilUiMenuRowInfo& row = s->downloads_popover_rows[i];
            if (row.has_divider_above) {
                g_linux_ui.draw_line(row.rect.x + 6.0f, row.rect.y - 1.0f, row.rect.x + row.rect.w - 6.0f, row.rect.y - 1.0f, 1.0f, s->divider);
            }
            if (row.is_hovered) {
                g_linux_ui.fill_rounded(row.rect, 6.0f, 0.0f, s->control_hover_bg, s->control_hover_bg);
            }
            AnvilUiRect lrc = { row.rect.x + 10.0f, row.rect.y, row.rect.w - 110.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.label_ptr,
                row.label_len,
                lrc,
                g_linux_ui.fmt_ui_regular,
                row.is_hovered ? s->accent : s->ink_primary
            );
            AnvilUiRect src = { row.rect.x + row.rect.w - 102.0f, row.rect.y, 92.0f, row.rect.h };
            g_linux_ui.draw_utf8_text(
                row.shortcut_ptr,
                row.shortcut_len,
                src,
                g_linux_ui.fmt_ui_badge,
                row.is_hovered ? s->accent : s->accent_cyan,
                ALIGN_TRAILING
            );
        }
    }

    return 1;
}

#endif
