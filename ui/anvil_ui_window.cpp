// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// Native C++20 Win32 & DWM Frameless Window Controls, Snap Layouts, and Resize Hit-Testing.

#include "anvil_core_abi.h"

#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

extern "C" int anvil_ui_is_maximized(void* hwnd_ptr) {
    HWND hwnd = static_cast<HWND>(hwnd_ptr);
    if (!hwnd) return 0;
    return IsZoomed(hwnd) ? 1 : 0;
}

extern "C" void anvil_ui_window_action(void* hwnd_ptr, int action) {
    HWND hwnd = static_cast<HWND>(hwnd_ptr);
    if (!hwnd) return;
    switch (action) {
        case 1: // Minimize
            ShowWindow(hwnd, SW_MINIMIZE);
            break;
        case 2: // Toggle Maximize / Restore
            if (IsZoomed(hwnd)) {
                ShowWindow(hwnd, SW_RESTORE);
            } else {
                ShowWindow(hwnd, SW_MAXIMIZE);
            }
            break;
        case 3: // Close
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            break;
        case 4: // Begin native window caption drag / Aero Snap
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION, 0);
            break;
        default:
            break;
    }
}

extern "C" intptr_t anvil_ui_handle_nchittest(
    void* hwnd_ptr,
    int screen_x,
    int screen_y,
    int on_draggable_strip,
    int on_max_button
) {
    HWND hwnd = static_cast<HWND>(hwnd_ptr);
    if (!hwnd) return 1; // HTCLIENT

    POINT pt = { screen_x, screen_y };
    ScreenToClient(hwnd, &pt);

    RECT rc = {};
    GetClientRect(hwnd, &rc);
    int w = rc.right - rc.left;
    int h = rc.bottom - rc.top;

    if (!IsZoomed(hwnd)) {
        const UINT dpi = GetDpiForWindow(hwnd);
        const int border = (dpi > 0) ? static_cast<int>((6u * dpi + 48u) / 96u) : 6;
        bool left = pt.x >= 0 && pt.x < border;
        bool right = pt.x < w && pt.x >= w - border;
        bool top = pt.y >= 0 && pt.y < border;
        bool bottom = pt.y < h && pt.y >= h - border;

        if (top && left) return HTTOPLEFT;
        if (top && right) return HTTOPRIGHT;
        if (bottom && left) return HTBOTTOMLEFT;
        if (bottom && right) return HTBOTTOMRIGHT;
        if (left) return HTLEFT;
        if (right) return HTRIGHT;
        if (top) return HTTOP;
        if (bottom) return HTBOTTOM;
    }

    if (on_max_button) {
        return HTMAXBUTTON;
    }
    if (on_draggable_strip) {
        return HTCAPTION;
    }
    return HTCLIENT;
}

#else

namespace {
struct LinuxWindowState {
    int maximized = 0;
    int pending_action = 0;
    int width = 1280;
    int height = 800;
};
static LinuxWindowState g_linux_win;

constexpr intptr_t ANVIL_HTCLIENT = 1;
constexpr intptr_t ANVIL_HTCAPTION = 2;
constexpr intptr_t ANVIL_HTMAXBUTTON = 9;
constexpr intptr_t ANVIL_HTLEFT = 10;
constexpr intptr_t ANVIL_HTRIGHT = 11;
constexpr intptr_t ANVIL_HTTOP = 12;
constexpr intptr_t ANVIL_HTTOPLEFT = 13;
constexpr intptr_t ANVIL_HTTOPRIGHT = 14;
constexpr intptr_t ANVIL_HTBOTTOM = 15;
constexpr intptr_t ANVIL_HTBOTTOMLEFT = 16;
constexpr intptr_t ANVIL_HTBOTTOMRIGHT = 17;
} // namespace

extern "C" int anvil_ui_is_maximized(void* hwnd_ptr) {
    if (hwnd_ptr) {
        uintptr_t packed = reinterpret_cast<uintptr_t>(hwnd_ptr);
        if ((packed & 0xFFFFu) > 0 && ((packed >> 16) & 0xFFFFu) > 0) {
            g_linux_win.width = static_cast<int>((packed >> 16) & 0xFFFFu);
            g_linux_win.height = static_cast<int>(packed & 0xFFFFu);
        }
    }
    return g_linux_win.maximized;
}

extern "C" void anvil_ui_window_action(void*, int action) {
    if (action == 2) {
        g_linux_win.maximized = g_linux_win.maximized ? 0 : 1;
    } else if (action == 100) {
        g_linux_win.maximized = 0;
    } else if (action == 101) {
        g_linux_win.maximized = 1;
    }
    g_linux_win.pending_action = action;
}

extern "C" intptr_t anvil_ui_handle_nchittest(
    void* hwnd_ptr,
    int client_x,
    int client_y,
    int on_draggable_strip,
    int on_max_button
) {
    int w = g_linux_win.width;
    int h = g_linux_win.height;
    if (hwnd_ptr) {
        uintptr_t packed = reinterpret_cast<uintptr_t>(hwnd_ptr);
        int pw = static_cast<int>((packed >> 16) & 0xFFFFu);
        int ph = static_cast<int>(packed & 0xFFFFu);
        if (pw > 0 && ph > 0) {
            w = pw;
            h = ph;
            g_linux_win.width = pw;
            g_linux_win.height = ph;
        }
    }

    if (!g_linux_win.maximized && w > 32 && h > 32) {
        const int border = 6;
        bool left = client_x >= 0 && client_x < border;
        bool right = client_x < w && client_x >= w - border;
        bool top = client_y >= 0 && client_y < border;
        bool bottom = client_y < h && client_y >= h - border;

        if (top && left) return ANVIL_HTTOPLEFT;
        if (top && right) return ANVIL_HTTOPRIGHT;
        if (bottom && left) return ANVIL_HTBOTTOMLEFT;
        if (bottom && right) return ANVIL_HTBOTTOMRIGHT;
        if (left) return ANVIL_HTLEFT;
        if (right) return ANVIL_HTRIGHT;
        if (top) return ANVIL_HTTOP;
        if (bottom) return ANVIL_HTBOTTOM;
    }

    if (on_max_button) {
        return ANVIL_HTMAXBUTTON;
    }
    if (on_draggable_strip) {
        return ANVIL_HTCAPTION;
    }
    return ANVIL_HTCLIENT;
}

#endif
