"""抓取行为生态缸窗口所在屏幕区域，用于人工验收。

用法:
    python tools/capture.py [输出路径]

窗口是分层/透明窗口，PrintWindow 通常会得到黑底，
所以这里直接抓屏幕上的合成结果。
"""

import ctypes
import os
import sys
import time
from ctypes import wintypes

from PIL import ImageGrab

user32 = ctypes.windll.user32
user32.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
user32.FindWindowW.restype = wintypes.HWND
user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
user32.GetWindowRect.restype = wintypes.BOOL
user32.IsWindowVisible.argtypes = [wintypes.HWND]
user32.IsWindowVisible.restype = wintypes.BOOL

TITLE = "行为生态缸"


def find() -> int:
    hwnd = user32.FindWindowW(None, TITLE)
    if not hwnd:
        raise SystemExit(f"找不到窗口: {TITLE}")
    return hwnd


def rect(hwnd):
    r = wintypes.RECT()
    if not user32.GetWindowRect(hwnd, ctypes.byref(r)):
        raise SystemExit("GetWindowRect failed")
    return r.left, r.top, r.right, r.bottom


def main() -> None:
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join("shots", "window.png")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)

    hwnd = find()
    if not user32.IsWindowVisible(hwnd):
        raise SystemExit("窗口不可见")
    time.sleep(0.4)
    l, t, r, b = rect(hwnd)
    # 多抓 24px 边距，方便看透明边缘与桌面叠合效果。
    pad = 24
    img = ImageGrab.grab(bbox=(l - pad, t - pad, r + pad, b + pad), all_screens=True)
    img.save(out)
    print(f"saved {out}  {img.size[0]}x{img.size[1]}  rect=({l},{t},{r},{b})")


if __name__ == "__main__":
    main()
