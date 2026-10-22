"""Exercise the shipped server's DPI dispatch without taking X11 ownership."""

import importlib
import json

# Xpra belongs to the qualified image, not the host test environment. Load it
# there, and configure its real factory before importing the concrete server.
features = importlib.import_module("xpra.server.features")

# Match the server-only surface: no GTK client, tray, sound, or forwarding.
for feature in ("gtk", "tray", "pulseaudio", "audio", "printer", "dbus",
                "webcam", "notification", "ssh", "mmap", "file"):
    setattr(features, feature, False)

ServerBase = importlib.import_module("xpra.server.base").ServerBase
XSettingsServer = importlib.import_module("xpra.x11.subsystem.xsettings").XSettingsServer


class SettingsSink(XSettingsServer):
    """Capture the X11 write boundary, leaving the real dispatch intact."""

    def __init__(self):
        super().__init__()
        self.updates = []
        self.dpi = 192

    def do_update_server_settings(self, settings, reset=False, dpi=0, *_args):
        self.updates.append({"dpi": dpi, "reset": reset, "settings": sorted(settings)})


sink = SettingsSink()
ServerBase.update_all_server_settings(sink, True)
expected = [{"dpi": 192, "reset": True, "settings": ["resource-manager", "xsettings-blob"]}]
assert sink.updates == expected, sink.updates
print(json.dumps(sink.updates))
