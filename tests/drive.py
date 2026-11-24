"""Drives an isolated Signet (SIGNET_TEST) with posted messages. Never moves the real mouse."""
import ctypes as C
import ctypes.wintypes as W
import os
import shutil
import subprocess
import sys
import time

u = C.windll.user32
u.SetProcessDpiAwarenessContext(C.c_void_p(-4))
u.PostMessageW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM]
u.FindWindowW.restype = W.HWND
MOVE, DOWN, UP, RUP, WHEEL, KEY, CHAR, TIMER = 0x200, 0x201, 0x202, 0x205, 0x20A, 0x100, 0x102, 0x113
RUN = 0x8000 + 2
MODES = ["Region", "Window", "Screen", "All", "Last", "Scroll", "Delay", "Interval", "Text", "Color"]


class App:
    def __init__(self, exe, data, fresh=True):
        if fresh:
            shutil.rmtree(data, ignore_errors=True)
        os.makedirs(data, exist_ok=True)
        self.data, self.log = data, os.path.join(data, "log.txt")
        env = dict(os.environ, SIGNET_TEST="1", SIGNET_DIR=data, SIGNET_LOG=self.log)
        self.p = subprocess.Popen([exe], env=env)
        for _ in range(100):
            self.h = self.find("SignetNotch", "Signet sensor")
            if self.h:
                break
            time.sleep(0.05)
        time.sleep(0.8)
        self.s = u.GetDpiForWindow(self.h) / 96

    def find(self, cls, title=None, nth=0):
        out = []

        def each(h, _):
            b = C.create_unicode_buffer(64)
            u.GetClassNameW(h, b, 64)
            t = C.create_unicode_buffer(64)
            u.GetWindowTextW(h, t, 64)
            pid = W.DWORD()
            u.GetWindowThreadProcessId(h, C.byref(pid))
            if pid.value == self.p.pid and b.value == cls and (title is None or t.value == title):
                out.append(h)
            return True

        u.EnumWindows(C.WINFUNCTYPE(C.c_bool, W.HWND, W.LPARAM)(each), 0)
        return out[nth] if len(out) > nth else None

    def rect(self, h=None):
        r = W.RECT()
        u.GetWindowRect(h or self.h, C.byref(r))
        return r.left, r.top, r.right, r.bottom

    @staticmethod
    def lp(x, y):
        return ((int(y) & 0xFFFF) << 16) | (int(x) & 0xFFFF)

    def post(self, msg, x=0, y=0, wp=0, h=None, scale=True):
        k = self.s if scale and h is None else 1
        u.PostMessageW(h or self.h, msg, wp, self.lp(x * k, y * k))

    def move(self, x, y):
        self.post(MOVE, x, y)
        time.sleep(0.05)

    def click(self, x, y, wait=0.35):
        self.move(x, y)
        self.post(DOWN, x, y)
        time.sleep(0.04)
        self.post(UP, x, y)
        time.sleep(wait)

    def drag(self, a, b, wait=0.4):
        self.move(*a)
        self.post(DOWN, *a)
        for k in range(1, 11):
            self.move(a[0] + (b[0] - a[0]) * k / 10, a[1] + (b[1] - a[1]) * k / 10)
        self.post(UP, *b)
        time.sleep(wait)

    def peek(self):
        self.move(105, 4)
        time.sleep(0.9)

    def mode(self, name):
        self.post(RUN, wp=MODES.index(name))

    def size(self):
        l, t, r, b = self.rect()
        return round((r - l) / self.s), round((b - t) / self.s)

    def shot(self, path, h=None, pad=40, box=None):
        from PIL import ImageGrab

        l, t, r, b = box or self.rect(h)
        ImageGrab.grab((l - pad, max(t, 0), r + pad, b + pad), all_screens=True).save(path)

    def memory(self):
        import psutil

        m = psutil.Process(self.p.pid).memory_full_info()
        return f"working {m.rss / 2**20:.1f} MB, private {m.private / 2**20:.1f} MB, unique {m.uss / 2**20:.1f} MB"

    def alive(self):
        return self.p.poll() is None

    def quit(self):
        u.PostMessageW(self.h, 0x10, 0, 0)
        try:
            self.p.wait(3)
        except subprocess.TimeoutExpired:
            self.p.kill()
