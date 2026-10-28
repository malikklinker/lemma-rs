#!/usr/bin/env python3
"""Opt-in native X11 preview smoke test; requires Xvfb and libX11.

Run in the project development environment. This test uses an isolated software
Vulkan display, never a hardware-throughput claim or an existing desktop window.
"""
import argparse
import ctypes as c
import ctypes.util
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import struct
import time
import zlib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path(__file__).resolve().parent.parent
    parser.add_argument("--engine", type=Path, default=root / "target/debug/lemma")
    parser.add_argument("--output", type=Path, default=root / "out/preview-check")
    parser.add_argument("--xvfb", default=shutil.which("Xvfb"))
    parser.add_argument("--x11-library", default=ctypes.util.find_library("X11"))
    parser.add_argument("--adapter", default="llvmpipe", help="explicit software adapter-name substring")
    args = parser.parse_args()
    if not all([args.xvfb, args.x11_library]):
        parser.error("provide Xvfb and libX11 via PATH/library paths or explicit options")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    engine = args.engine.resolve()
    lib = c.CDLL(args.x11_library)
    signatures = [
        ("XOpenDisplay", c.c_void_p, [c.c_char_p]),
        ("XDefaultRootWindow", c.c_ulong, [c.c_void_p]),
        ("XQueryTree", c.c_int, [c.c_void_p, c.c_ulong, c.POINTER(c.c_ulong), c.POINTER(c.c_ulong), c.POINTER(c.POINTER(c.c_ulong)), c.POINTER(c.c_uint)]),
        ("XFetchName", c.c_int, [c.c_void_p, c.c_ulong, c.POINTER(c.c_void_p)]),
        ("XFree", c.c_int, [c.c_void_p]),
        ("XKeysymToKeycode", c.c_uint, [c.c_void_p, c.c_ulong]),
        ("XSendEvent", c.c_int, [c.c_void_p, c.c_ulong, c.c_int, c.c_long, c.c_void_p]),
        ("XFlush", c.c_int, [c.c_void_p]),
        ("XResizeWindow", c.c_int, [c.c_void_p, c.c_ulong, c.c_uint, c.c_uint]),
        ("XSetInputFocus", c.c_int, [c.c_void_p, c.c_ulong, c.c_int, c.c_ulong]),
        ("XCloseDisplay", c.c_int, [c.c_void_p]),
        ("XGetGeometry", c.c_int, [c.c_void_p, c.c_ulong, c.POINTER(c.c_ulong), c.POINTER(c.c_int), c.POINTER(c.c_int), c.POINTER(c.c_uint), c.POINTER(c.c_uint), c.POINTER(c.c_uint), c.POINTER(c.c_uint)]),
        ("XGetImage", c.c_void_p, [c.c_void_p, c.c_ulong, c.c_int, c.c_int, c.c_uint, c.c_uint, c.c_ulong, c.c_int]),
        ("XGetPixel", c.c_ulong, [c.c_void_p, c.c_int, c.c_int]),
        ("XDestroyImage", c.c_int, [c.c_void_p]),
    ]
    for name, result, parameters in signatures:
        function = getattr(lib, name)
        function.restype, function.argtypes = result, parameters

    class Key(c.Structure):
        _fields_ = [(name, kind) for name, kind in [
            ("type", c.c_int), ("serial", c.c_ulong), ("send_event", c.c_int),
            ("display", c.c_void_p), ("window", c.c_ulong), ("root", c.c_ulong),
            ("subwindow", c.c_ulong), ("time", c.c_ulong), ("x", c.c_int),
            ("y", c.c_int), ("x_root", c.c_int), ("y_root", c.c_int),
            ("state", c.c_uint), ("keycode", c.c_uint), ("same_screen", c.c_int),
        ]]

    class Event(c.Union):
        _fields_ = [("key", Key), ("pad", c.c_long * 24)]

    class Motion(c.Structure):
        _fields_ = Key._fields_[:-2] + [("is_hint", c.c_char), ("same_screen", c.c_int)]

    class XImage(c.Structure):
        _fields_ = [("width", c.c_int), ("height", c.c_int), ("xoffset", c.c_int),
                    ("format", c.c_int), ("data", c.c_void_p), ("byte_order", c.c_int),
                    ("bitmap_unit", c.c_int), ("bitmap_bit_order", c.c_int),
                    ("bitmap_pad", c.c_int), ("depth", c.c_int), ("bytes_per_line", c.c_int),
                    ("bits_per_pixel", c.c_int), ("red_mask", c.c_ulong),
                    ("green_mask", c.c_ulong), ("blue_mask", c.c_ulong)]

    server = app = display_handle = None
    read_fd, write_fd = os.pipe()
    try:
        with (output / "xvfb.log").open("w") as server_log:
            server = subprocess.Popen(
                [args.xvfb, "-displayfd", str(write_fd), "-screen", "0", "1280x800x24", "-nolisten", "tcp"],
                pass_fds=[write_fd], stdout=subprocess.DEVNULL, stderr=server_log,
            )
        os.close(write_fd)
        write_fd = None
        assert select.select([read_fd], [], [], 10)[0], "Xvfb startup timed out"
        number = os.read(read_fd, 100).decode().strip()
        assert number.isdigit(), "Xvfb failed; see xvfb.log"
        display = ":" + number
        env = os.environ.copy()
        env.pop("WAYLAND_DISPLAY", None)
        env["DISPLAY"] = display
        scene = output / "scene.json"
        subprocess.run([str(engine), "dump-scene", "--output", str(scene)], check=True, stdout=subprocess.DEVNULL)
        spec = json.loads(scene.read_text())
        captures = output / f"captures-{time.time_ns()}"
        with (output / "preview.log").open("w") as app_log:
            app = subprocess.Popen(
                [str(engine), "preview", "--scene", str(scene), "--adapter", args.adapter, "--allow-software", "--paused", "--frame", "240", "--size", "640x360", "--capture-dir", str(captures)],
                env=env, stdout=app_log, stderr=app_log,
            )
        display_handle = lib.XOpenDisplay(display.encode())
        assert display_handle, "cannot connect to isolated X11 display"
        window_root = lib.XDefaultRootWindow(display_handle)
        checks = []

        def title(window):
            pointer = c.c_void_p()
            if lib.XFetchName(display_handle, window, c.byref(pointer)) and pointer.value:
                text = c.string_at(pointer).decode(errors="replace")
                lib.XFree(pointer)
                return text
            return ""

        def windows():
            root_id, parent = c.c_ulong(), c.c_ulong()
            children, count = c.POINTER(c.c_ulong)(), c.c_uint()
            lib.XQueryTree(display_handle, window_root, c.byref(root_id), c.byref(parent), c.byref(children), c.byref(count))
            found = [(children[i], title(children[i])) for i in range(count.value)]
            if children:
                lib.XFree(children)
            return found

        def until(predicate, description, timeout=10):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                assert app.poll() is None, f"preview exited {app.returncode}; see preview.log"
                result = predicate()
                if result:
                    return result
                time.sleep(0.05)
            raise AssertionError(f"timeout: {description}; windows={windows()}")

        window = until(lambda: next((w for w, t in windows() if t.startswith("lemma |")), None), "window creation")

        def key(symbol):
            lib.XSetInputFocus(display_handle, window, 1, 0)
            for kind, mask in [(2, 1), (3, 2)]:
                event = Event()
                event.key = Key(type=kind, display=display_handle, window=window, root=window_root,
                                keycode=lib.XKeysymToKeycode(display_handle, symbol), same_screen=1)
                assert lib.XSendEvent(display_handle, window, 0, mask, c.byref(event))
            lib.XFlush(display_handle)

        def pointer(x, y, button=None):
            motion = Motion(type=6, display=display_handle, window=window, root=window_root,
                            x=x, y=y, same_screen=1, state=256 if button == "drag" else 0)
            assert lib.XSendEvent(display_handle, window, 0, 64, c.byref(motion))
            if button in ("press", "release"):
                event = Event()
                event.key = Key(type=4 if button == "press" else 5, display=display_handle,
                                window=window, root=window_root, x=x, y=y, keycode=1, same_screen=1)
                assert lib.XSendEvent(display_handle, window, 0, 4 if button == "press" else 8, c.byref(event))
            lib.XFlush(display_handle)

        def click(x, y):
            pointer(x, y, "press")
            pointer(x, y, "release")

        def geometry():
            root_id, x, y = c.c_ulong(), c.c_int(), c.c_int()
            width, height, border, depth = (c.c_uint() for _ in range(4))
            assert lib.XGetGeometry(display_handle, window, c.byref(root_id), c.byref(x), c.byref(y),
                                    c.byref(width), c.byref(height), c.byref(border), c.byref(depth))
            return width.value, height.value

        def screenshot(path):
            width, height = geometry()
            image = lib.XGetImage(display_handle, window, 0, 0, width, height, c.c_ulong(-1), 2)
            assert image, "cannot capture native window"
            try:
                metadata = c.cast(image, c.POINTER(XImage)).contents
                masks = [metadata.red_mask, metadata.green_mask, metadata.blue_mask]
                shifts = [(mask & -mask).bit_length() - 1 for mask in masks]
                pixels = bytearray()
                for y in range(height):
                    pixels.append(0)  # PNG's no-filter row prefix.
                    for x in range(width):
                        value = lib.XGetPixel(image, x, y)
                        pixels.extend(round(((value & mask) >> shift) * 255 / (mask >> shift))
                                      for mask, shift in zip(masks, shifts))
                def chunk(kind, data):
                    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
                path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                                 + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))
            finally:
                lib.XDestroyImage(image)

        def wait_capture(previous, size):
            files = until(lambda: sorted(captures.glob("*.png")) if len(list(captures.glob("*.png"))) > previous else None,
                          "scene-only PNG capture")
            path = max(files, key=lambda item: item.stat().st_mtime_ns)
            assert struct.unpack(">II", path.read_bytes()[16:24]) == size, "capture must exclude toolbar"
            return path

        def state(fragment):
            current = until(lambda: title(window) if fragment in title(window) else None, fragment)
            checks.append(current)
            return current

        state("| 240 / 479 ")
        assert geometry() == (640, 472), "--size specifies scene area, plus 112-pixel controls"
        click(168, 388)
        state("| 241 / 479 ")
        click(128, 388)
        state("| 240 / 479 ")
        click(224, 388)
        state("paused, once")
        click(224, 388)
        state("paused, loop")
        key(0xFF53)
        state("| 241 / 479 ")
        key(0xFF51)
        state("| 240 / 479 ")
        key(0xFF50)
        state("| 0 / 479 ")
        key(0xFF57)
        state("| 479 / 479 ")
        key(0x20)
        state("playing")
        until(lambda: "| 479 / 479 " not in title(window), "loop playback advances")
        key(0x20)
        state("paused")
        click(60, 388)
        state("playing")
        click(60, 388)
        state("paused")
        pointer(20, 424, "press")
        state("| 0 / 479 ")
        pointer(800, 424, "drag")
        state("| 479 / 479 ")
        pointer(620, 424, "release")
        click(320, 424)
        state("| 240 / 479 ")
        key(ord("s"))
        captured = [str(wait_capture(0, (640, 360)))]
        scene.write_text("{broken reload")
        state("RELOAD FAILED")
        screenshot(output / "reload-failed.png")
        spec["background"] = [0.04, 0.06, 0.09, 1]
        scene.write_text(json.dumps(spec))
        until(lambda: "RELOAD FAILED" not in title(window), "reload recovery")
        checks.append(title(window))
        lib.XResizeWindow(display_handle, window, 800, 562)
        lib.XFlush(display_handle)
        time.sleep(0.5)
        click(210, 514)
        state("| 120 / 479 ")
        click(650, 478)
        captured.append(str(wait_capture(1, (800, 450))))
        held = captures.with_name(captures.name + "-held")
        captures.rename(held)
        captures.write_text("capture directory intentionally unavailable")
        try:
            key(ord("s"))
            until(lambda: "preview capture failed" in (output / "preview.log").read_text(), "nonfatal capture failure")
            time.sleep(0.1)
            screenshot(output / "capture-failed.png")
        finally:
            captures.unlink()
            held.rename(captures)
        key(ord("s"))
        captured.append(str(wait_capture(2, (800, 450))))
        screenshot_path = output / "preview.png"
        screenshot(screenshot_path)
        key(0xFF1B)
        app.wait(timeout=10)
        assert app.returncode == 0
        report = {"checks": checks, "exit_code": app.returncode, "screenshot": str(screenshot_path), "captures": captured}
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report, indent=2))
    finally:
        if write_fd is not None:
            os.close(write_fd)
        os.close(read_fd)
        if app and app.poll() is None:
            app.terminate()
            app.wait(timeout=10)
        if display_handle:
            lib.XCloseDisplay(display_handle)
        if server:
            server.terminate()
            server.wait(timeout=10)


if __name__ == "__main__":
    main()
