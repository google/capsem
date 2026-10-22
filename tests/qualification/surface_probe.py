"""Guest-side public /proc and X11 probe; uses only libraries the image ships."""

import ctypes
import json
import os
import stat
from pathlib import Path


class WindowAttributes(ctypes.Structure):
    """Xlib's XWindowAttributes ABI; queried inside the Linux workload only."""

    _fields_ = [(name, ctypes.c_int) for name in ("x", "y", "width", "height", "border", "depth")] + [
        ("visual", ctypes.c_void_p), ("root", ctypes.c_ulong), ("window_class", ctypes.c_int),
        ("bit_gravity", ctypes.c_int), ("win_gravity", ctypes.c_int), ("backing_store", ctypes.c_int),
        ("backing_planes", ctypes.c_ulong), ("backing_pixel", ctypes.c_ulong), ("save_under", ctypes.c_int),
        ("colormap", ctypes.c_ulong), ("map_installed", ctypes.c_int), ("map_state", ctypes.c_int),
        ("all_events", ctypes.c_long), ("your_events", ctypes.c_long), ("propagate", ctypes.c_long),
        ("override_redirect", ctypes.c_int), ("screen", ctypes.c_void_p),
    ]


def application_processes():
    application = Path(os.environ["CAPSEM_GUI_APP"]).name
    rows = []
    display = {}
    for process in Path("/proc").glob("[0-9]*"):
        try:
            argv = process.joinpath("cmdline").read_bytes().rstrip(b"\0").split(b"\0")
            if not argv or Path(os.fsdecode(argv[0])).name != application:
                continue
            status = dict(line.split(":", 1) for line in process.joinpath("status").read_text().splitlines())
            kinds = [os.fsdecode(arg).removeprefix("--type=") for arg in argv if arg.startswith(b"--type=")]
            rows.append({"kind": kinds[0] if kinds else "main",
                         "nspid": status["NSpid"].split(), "seccomp": status["Seccomp"].strip(),
                         "no_new_privs": status["NoNewPrivs"].strip(),
                         "sandbox_disabled": b"--no-sandbox" in argv})
            if not kinds:
                environment = dict(entry.split(b"=", 1) for entry in process.joinpath("environ").read_bytes().split(b"\0") if b"=" in entry)
                display = {key: os.fsdecode(environment[os.fsencode(key)])
                           for key in ("DISPLAY", "XAUTHORITY") if os.fsencode(key) in environment}
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    return rows, display


def display_processes():
    """The display runs in the same unprivileged, confined workload."""
    rows = []
    for process in Path("/proc").glob("[0-9]*"):
        try:
            argv = process.joinpath("cmdline").read_bytes().split(b"\0")
            binary = Path(os.readlink(process / "exe"))
            # Xpra deliberately renames argv[0] (for example Xvfb-for-Xpra).
            # The executable inode identifies the actual display server.
            if not argv or binary.name not in {"Xorg", "Xvfb"}:
                continue
            status = dict(line.split(":", 1) for line in process.joinpath("status").read_text().splitlines())
            rows.append({"binary": str(binary), "uid": int(status["Uid"].split()[0]),
                         "setuid": bool(binary.stat().st_mode & stat.S_ISUID),
                         "seccomp": status["Seccomp"].strip(), "no_new_privs": status["NoNewPrivs"].strip()})
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    return rows


def display_dimensions(environment):
    """Physical X11 dimensions, independently of Xpra's requested DPI."""
    if "DISPLAY" not in environment:
        return None
    os.environ.update(environment)
    x11 = ctypes.CDLL("libX11.so.6")
    pointer = ctypes.c_void_p
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XOpenDisplay.restype = pointer
    x11.XDefaultScreen.argtypes = [pointer]
    x11.XDefaultScreen.restype = ctypes.c_int
    x11.XCloseDisplay.argtypes = [pointer]
    for name in ("XDisplayWidth", "XDisplayHeight", "XDisplayWidthMM", "XDisplayHeightMM", "XDefaultDepth"):
        getattr(x11, name).argtypes = [pointer, ctypes.c_int]
        getattr(x11, name).restype = ctypes.c_int
    display = x11.XOpenDisplay(os.fsencode(environment["DISPLAY"]))
    if not display:
        return None
    try:
        screen = x11.XDefaultScreen(display)
        pixels = [x11.XDisplayWidth(display, screen), x11.XDisplayHeight(display, screen)]
        millimeters = [x11.XDisplayWidthMM(display, screen), x11.XDisplayHeightMM(display, screen)]
        return {"pixels": pixels, "millimeters": millimeters, "depth": x11.XDefaultDepth(display, screen),
                "dpi": [round(px * 25.4 / mm, 2) if mm > 0 else None
                        for px, mm in zip(pixels, millimeters, strict=True)]}
    finally:
        x11.XCloseDisplay(display)


def window_titles(environment):
    """Query actual named X11 windows, without a desktop client or test package."""
    if "DISPLAY" not in environment:
        return []
    os.environ.update(environment)
    x11 = ctypes.CDLL("libX11.so.6")
    window = ctypes.c_ulong
    pointer = ctypes.c_void_p
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XOpenDisplay.restype = pointer
    x11.XDefaultRootWindow.argtypes = [pointer]
    x11.XDefaultRootWindow.restype = window
    x11.XQueryTree.argtypes = [pointer, window, ctypes.POINTER(window), ctypes.POINTER(window),
                              ctypes.POINTER(ctypes.POINTER(window)), ctypes.POINTER(ctypes.c_uint)]
    x11.XFetchName.argtypes = [pointer, window, ctypes.POINTER(ctypes.c_char_p)]
    x11.XGetWindowAttributes.argtypes = [pointer, window, ctypes.POINTER(WindowAttributes)]
    x11.XInternAtom.argtypes = [pointer, ctypes.c_char_p, ctypes.c_int]
    x11.XInternAtom.restype = ctypes.c_ulong
    x11.XGetWindowProperty.argtypes = [pointer, window, ctypes.c_ulong, ctypes.c_long, ctypes.c_long,
                                      ctypes.c_int, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong),
                                      ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong),
                                      ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(pointer)]
    x11.XFree.argtypes = [pointer]
    x11.XCloseDisplay.argtypes = [pointer]
    display = x11.XOpenDisplay(os.fsencode(environment["DISPLAY"]))
    if not display:
        return []
    try:
        pending = [(x11.XDefaultRootWindow(display), 0)]
        names = []
        title_atom = x11.XInternAtom(display, b"_NET_WM_NAME", 1)
        visited = 0
        while pending:
            wid, depth = pending.pop()
            visited += 1
            if visited > 256:
                raise RuntimeError("graphical fixture exceeded 256 X11 windows")
            name = ctypes.c_char_p()
            attributes = WindowAttributes()
            # Xpra unmaps application windows while no viewer is attached.
            # The headless image gate proves creation and nontrivial geometry;
            # the integrated viewer acceptance owns visible rendering/input.
            application_window = (x11.XGetWindowAttributes(display, wid, ctypes.byref(attributes))
                                  and attributes.width > 1 and attributes.height > 1)
            if application_window and x11.XFetchName(display, wid, ctypes.byref(name)) and name.value:
                names.append(name.value.decode("utf-8", errors="replace"))
            if name:
                x11.XFree(name)
            if application_window and title_atom:
                kind, size, remaining = ctypes.c_ulong(), ctypes.c_ulong(), ctypes.c_ulong()
                form = ctypes.c_int()
                data = pointer()
                result = x11.XGetWindowProperty(display, wid, title_atom, 0, 1024, 0, 0,
                                                ctypes.byref(kind), ctypes.byref(form), ctypes.byref(size),
                                                ctypes.byref(remaining), ctypes.byref(data))
                try:
                    if result == 0 and form.value == 8 and 0 < size.value <= 4096 and data:
                        names.append(ctypes.string_at(data, size.value).decode("utf-8", errors="replace"))
                finally:
                    if data:
                        x11.XFree(data)
            if depth >= 3:
                continue
            root, parent = window(), window()
            children = ctypes.POINTER(window)()
            count = ctypes.c_uint()
            if x11.XQueryTree(display, wid, ctypes.byref(root), ctypes.byref(parent),
                              ctypes.byref(children), ctypes.byref(count)):
                try:
                    if count.value > 256:
                        raise RuntimeError("graphical fixture exceeded 256 child windows")
                    pending.extend((children[index], depth + 1) for index in range(count.value))
                finally:
                    if children:
                        x11.XFree(children)
        return names
    finally:
        x11.XCloseDisplay(display)


if __name__ == "__main__":
    processes, environment = application_processes()
    print(json.dumps({"processes": processes, "display_processes": display_processes(),
                      "display_dimensions": display_dimensions(environment),
                      "window_titles": window_titles(environment)}))
