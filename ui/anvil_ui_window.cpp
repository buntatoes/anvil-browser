// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
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
        const int border = 6;
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

extern "C" int anvil_ui_is_maximized(void*) { return 0; }
extern "C" void anvil_ui_window_action(void*, int) {}
extern "C" intptr_t anvil_ui_handle_nchittest(void*, int, int, int, int) { return 1; }

#endif
