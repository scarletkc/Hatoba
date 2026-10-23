# Temporary spike (#268): WebKitGTK under set_zoom_level, the call wry makes for Webview::set_zoom.
import json
import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2  # noqa: E402

SCRIPT = open(sys.argv[1], encoding="utf-8").read()
PAGE = '<!doctype html><body style="margin:0"><div id="box" style="width:100px;height:10px"></div></body>'
results = {}

win = Gtk.Window()
win.set_default_size(800, 600)
view = WebKit2.WebView()
win.add(view)
win.show_all()


def measure(label, then):
    def done(v, res):
        results[label] = json.loads(v.evaluate_javascript_finish(res).to_string())
        results[label]["widget"] = [view.get_allocated_width(), view.get_allocated_height()]
        results[label]["zoom_level"] = view.get_zoom_level()
        then()

    view.evaluate_javascript(SCRIPT, -1, None, None, None, done)


def zoomed():
    view.set_zoom_level(1.5)
    GLib.timeout_add(800, lambda: measure("zoom_1.5", finish) and False)


def finish():
    print(json.dumps(results, indent=2))
    Gtk.main_quit()


def loaded(v, event):
    if event == WebKit2.LoadEvent.FINISHED:
        GLib.timeout_add(500, lambda: measure("zoom_1", zoomed) and False)


view.connect("load-changed", loaded)
view.load_html(PAGE, "file:///")
GLib.timeout_add_seconds(30, lambda: (print("timeout", results), Gtk.main_quit()))
Gtk.main()
