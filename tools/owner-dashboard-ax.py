#!/usr/bin/env python3
"""Own-process native AX and semantic/action regression journey.

Requires a fixture-enabled native-accessibility release build. This launches its
own hidden Makepad instance, uses only its loopback remote endpoint/content view,
and always closes that process. It does not enable VoiceOver or inspect live apps.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
from PIL import Image, ImageChops
from urllib.parse import urlencode
from urllib.request import urlopen


def nodes(tree):
    yield tree
    for child in tree.get("children", []):
        yield from nodes(child)


class Journey:
    def __init__(self, base, out, embedded):
        self.base, self.out, self.embedded = base, out, embedded
        self.checks = []
        self.sequence = 0

    def get(self, path, **params):
        response = urlopen(self.base + path + ("?" + urlencode(params) if params else ""), timeout=20).read()
        return json.loads(response)

    def event(self, command):
        result = self.get("/event", data=command, wait=1)
        # Remote Custom events and native AX actions enqueue UI work. Present
        # the owned drawable before reading the resulting tree.
        if not (command == "ax" or command.startswith("ax-native") or command.startswith("dock:owner-ax")):
            self.get("/g")
        return result

    def capture(self, command, marker, name):
        self.event(command)
        log = self.get("/log", since=self.sequence)
        self.sequence = log["n"]
        entries = [line.split(marker, 1)[1] for line in log["l"] if marker in line]
        assert entries, (command, log)
        value = json.loads(entries[-1])
        (self.out / (name + ".json")).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
        return value

    def semantic(self, name):
        return self.capture("dock:owner-ax" if self.embedded else "ax", "od-ax ", name)

    def native(self, name, action=""):
        prefix = "dock:owner-ax-native:" if self.embedded else "ax-native:"
        return self.capture(prefix + action, "od-ax-native ", name)

    def grab(self, name):
        self.get("/g")
        result = self.get("/g")
        shutil.copyfile(result["png"], self.out / (name + ".png"))
        (self.out / (name + "-capture.json")).write_text(json.dumps(result, indent=2)+"\n")

    def scenario(self, name):
        self.event(("dock:owner:" if self.embedded else "scenario:") + name)

    def native_nodes(self, tree):
        return [n for window in tree["windows"] for n in nodes(window["native"])]

    def find(self, tree, title):
        matches = [n for n in self.native_nodes(tree) if n.get("title") == title or (n.get("role") == "AXMenuItem" and n.get("value") == title)]
        assert matches, f"Native AX control missing: {title}"
        return matches[-1]

    def check(self, label, condition):
        assert condition, label
        self.checks.append(label)

    def press(self, title, name):
        tree = self.native(name + "-before")
        control = self.find(tree, title)
        result = self.native(name + "-press", control["id"])
        self.check(name + " native press accepted", any(n.get("pressed") for n in self.native_nodes(result)))
        self.get("/g")
        self.get("/g")
        return control

    def run(self):
        self.scenario("running")
        self.grab("01-overview")
        semantic = self.semantic("01-semantic")
        native = self.native("01-native")
        self.check("native AppKit tree exists", bool(native["windows"]))
        labels = self.native_nodes(native)
        if self.embedded:
            self.check("embedded Agent navigation is a native actionable control", any(n.get("title") == "Agent" and n.get("role") in ["AXButton","AXRadioButton"] for n in labels))
            self.check("embedded host exposes Open separately rather than standalone Dock back", any(n.get("title") == "Open separately" for n in labels) and not any(n.get("title") == "Dock back" for n in labels))
        self.check("native button and selected tab roles", any(n.get("role") == "AXButton" for n in labels) and any(n.get("role") == "AXRadioButton" and n.get("value") == "1" for n in labels))
        pause = self.find(native, "Pause hosting")
        sn = next(n for win in semantic["windows"] for uid,n in win["tree"]["nodes"] if str(uid) == pause["id"])
        b = sn["properties"]["bounds"]
        self.check("Retina physical-to-AppKit bounds", abs(pause["frame"][2] - (b["x1"] - b["x0"])) < 1 and abs(pause["frame"][3] - (b["y1"] - b["y0"])) < 1)
        self.event(("dock:owner-refresh:" if self.embedded else "refresh-step:") + "1")
        refreshed = self.native("02-refresh")
        self.check("refresh preserves native control identity", self.find(refreshed,"Pause hosting")["id"] == pause["id"])
        self.press("Resources", "03-resources")
        after = self.native("03-resources-after")
        self.check("native tab action selects resources", self.find(after,"Resources").get("value") == "1")
        self.press("Overview", "04-overview")
        self.press("Stop hosting", "05-stop")
        confirm = self.native("05-confirm")
        self.check("native Stop opens confirmation", any(n.get("title") == "Cancel" for n in self.native_nodes(confirm)))
        cancel = self.find(confirm, "Cancel")
        confirm_semantic = self.semantic("05-confirm-semantic")
        self.check("confirmation takes keyboard focus on Cancel", any(str(w["tree"]["focus"]) == cancel["id"] for w in confirm_semantic["windows"]))
        self.check("confirmation exposes labelled nonmodal dialog", any(n.get("role") == "dialog" and n.get("properties",{}).get("label") for w in confirm_semantic["windows"] for _,n in w["tree"]["nodes"]))
        self.press("Cancel", "06-cancel")
        canceled = self.native("06-cancel-after")
        self.check("native Cancel dismisses confirmation", not any(n.get("title") == "Cancel" for n in self.native_nodes(canceled)))
        stop = self.find(canceled, "Stop hosting")
        canceled_semantic = self.semantic("06-cancel-semantic")
        if not self.embedded: self.capture("dump", "od-state ", "06-cancel-widget-state")
        self.check("Cancel restores keyboard focus to Stop", any(str(w["tree"]["focus"]) == stop["id"] for w in canceled_semantic["windows"]))
        self.press("Pause hosting", "07-pause")
        semantic = self.semantic("07-pause-after")
        # Embedded fixtures deliberately return an uncertain offline outcome;
        # the standalone fixture records the unsent command as still pending.
        expected = "Pause hosting: outcome unknown" if self.embedded else "Pause hosting: sending"
        self.check("native Pause reaches the host fixture's command state", any(expected in str(n.get("properties",{}).get("label","")) for w in semantic["windows"] for _,n in w["tree"]["nodes"]))
        self.scenario("unknown")
        unknown = self.native("08-unknown")
        self.check("unknown quota remains explicit in native tree", any("not reported" in str(n.get("value","")).lower() or "unknown" in str(n.get("value","")).lower() for n in self.native_nodes(unknown)))
        if not self.embedded:
            self.scenario("connecting")
            self.grab("08-reduced-motion")
            self.check("reduced motion hides busy spinner", not self.get("/snap", q="LoadingSpinner")["s"])
            sequence = self.get("/gseq", n=3, every_ms=120)
            frames = [Image.open(path).convert("RGB") for path in sequence["png"]]
            self.check("reduced motion busy page stays visually static", all(ImageChops.difference(frames[0], frame).getbbox() is None for frame in frames[1:]))
            (self.out/"08-reduced-motion-frames.json").write_text(json.dumps(sequence,indent=2)+"\n")
        self.scenario("running")
        focus_tree = self.native("08-focus-before")
        target = self.find(focus_tree, "Details · Reviewer")
        self.native("08-focus-action", "focus:"+target["id"])
        self.get("/g")
        focused = self.semantic("08-focus-after")
        self.check("native focus reaches normal Makepad focus", any(str(w["tree"]["focus"]) == target["id"] for w in focused["windows"]))
        focus_window = next(w for w in focused["windows"] if str(w["tree"]["focus"]) == target["id"])
        bounds = next(n["properties"]["bounds"] for uid,n in focus_window["tree"]["nodes"] if str(uid) == target["id"])
        height = next(w["sz"][1] for w in self.get("/s")["w"] if w["i"] == focus_window["window"])
        self.check("native offscreen focus scrolls control into viewport", bounds["y0"] >= 0 and bounds["y1"] <= height+1)

        if not self.embedded:
            self.event("text-scale:2")
            self.press("Dashboard page", "08-menu-open")
            self.grab("08-menu")
            expanded = self.native("08-menu-options")
            option = self.find(expanded, "Resources")
            self.check("native menu exposes nonzero bounds", any(n.get("role") == "AXMenu" and n["frame"][2] > 0 and n["frame"][3] > 0 for n in self.native_nodes(expanded)))
            x,y,w,h = option["frame"]
            hit = self.native("08-menu-hit-test", f"hit:{x+w/2},{y+h/2}")
            self.check("native hit testing finds the popup option", any((window.get("hit") or {}).get("id") == option["id"] for window in hit["windows"]))
            self.native("08-menu-select", option["id"])
            self.get("/g")
            selected = self.native("08-menu-selected")
            self.check("native dropdown option selects dashboard page", self.find(selected,"Dashboard page").get("value") == "Resources")
            self.event("text-scale:1")
            self.scenario("running")
        before = self.native("09-owner-before")
        old = self.find(before,"Pause hosting")["id"]
        self.event(("dock:owner-scope:" if self.embedded else "owner-scope:") + "@other-owner:example.test")
        after = self.native("09-owner-after")
        self.check("owner switch retires old native node IDs", old not in {n.get("id") for n in self.native_nodes(after)})
        self.check("owner switch removes previous owner labels", not any("Edward" in str(n.get("value","")) for n in self.native_nodes(after)))
        stale = self.native("09-old-action", old)
        self.check("retired native target cannot press", not any(n.get("pressed") for n in self.native_nodes(stale)))
        if not self.embedded:
            self.event("setup")
            setup = self.native("10-setup")
            inputs = [n for n in self.native_nodes(setup) if n.get("role") == "AXTextField"]
            self.check("setup exposes labelled native text inputs", len(inputs) >= 2 and all(n.get("title") or n.get("label") for n in inputs))
            field = inputs[0]
            value = "A你e\u0301👩‍💻"
            self.native("10-value-set", "value:"+field["id"]+":"+value)
            self.native("10-selection-set", "selection:"+field["id"]+":1,1")
            text = self.native("10-text-after")
            self.check("native SetValue edits actual input", any(n.get("id") == field["id"] and n.get("value") == value for n in self.native_nodes(text)))
            sem = self.semantic("10-text-semantic")
            self.check("native UTF16 selection reaches grapheme selection", any(n.get("properties",{}).get("textSelection",{}).get("anchor",{}).get("characterIndex") == 1 for w in sem["windows"] for _,n in w["tree"]["nodes"]))
            self.native("11-ime-value", "value:"+field["id"]+":prefix ")
            self.get("/g")
            self.native("11-ime-focus", "focus:"+field["id"])
            self.native("11-ime-selection", "selection:"+field["id"]+":7,0")
            self.get("/g")
            mark = self.native("11-ime-mark", "ime-mark:ni")
            self.check("native NSTextInputClient marked text queued", any(w.get("imeQueued") for w in mark["windows"]))
            self.get("/g")
            marked = self.native("11-ime-marked")
            self.check("native view has active IME composition", any(w.get("hasMarkedText") for w in marked["windows"]))
            self.native("11-ime-commit", "ime-commit:你")
            self.get("/g")
            committed = self.native("11-ime-committed")
            self.grab("11-ime-committed")
            self.check("native non-Latin IME commit preserves existing text", any(n.get("id") == field["id"] and n.get("value") == "prefix 你" for n in self.native_nodes(committed)))
            self.check("native IME commit clears marked state", not any(w.get("hasMarkedText") for w in committed["windows"]))
            before_rect = next(n["r"] for n in self.get("/snap",q="setup_server")["s"] if n["ty"] == "TextInput")
            self.event("text-scale:2")
            self.grab("12-populated-input-200")
            after_rect = next(n["r"] for n in self.get("/snap",q="setup_server")["s"] if n["ty"] == "TextInput")
            dpi = self.get("/s")["w"][0]["dpi"]
            def ink_height(path, bounds, scale):
                pixels = Image.open(path).convert("RGB")
                x,y,w,h = bounds
                # First letters only, excluding the caret and the blue border.
                sample = pixels.crop((int((x+8)*dpi),int((y+4)*dpi),int((x+8+32*scale)*dpi),int((y+h-4)*dpi)))
                rows = [row for row in range(sample.height) if any(max(sample.getpixel((col,row))) < 120 for col in range(sample.width))]
                return max(rows)-min(rows)+1 if rows else 0
            normal = ink_height(self.out/"11-ime-committed.png",before_rect,1)
            enlarged = ink_height(self.out/"12-populated-input-200.png",after_rect,2)
            (self.out/"12-text-scale-metrics.json").write_text(json.dumps({"normalInkHeight":normal,"enlargedInkHeight":enlarged,"dpi":dpi},indent=2)+"\n")
            self.check("populated text rerenders at 200 percent", normal > 0 and enlarged >= normal*1.7)
            final_text = self.native("12-text-scale-native")
            self.check("font scaling preserves composed input text", any(n.get("id") == field["id"] and n.get("value") == "prefix 你" for n in self.native_nodes(final_text)))
            back = self.find(final_text, "Back")
            self.native("13-setup-focus-submit", "focus:"+back["id"])
            self.get("/g");self.get("/g")
            self.grab("13-setup-before-validation")
            before_validation = self.semantic("13-setup-before-validation")
            setup_rect = next(n["r"] for n in self.get("/snap",q="setup_page")["s"] if n.get("i")=="setup_page")
            before_bounds = next(n["properties"]["bounds"] for w in before_validation["windows"] for uid,n in w["tree"]["nodes"] if str(uid)==field["id"])
            self.check("setup invalid field starts above the viewport", before_bounds["y0"] < setup_rect[1])
            self.press("Enable hosting", "13-setup-invalid-submit")
            self.grab("13-setup-validation-reveal")
            validation = self.semantic("13-setup-validation-reveal")
            after_bounds = next(n["properties"]["bounds"] for w in validation["windows"] for uid,n in w["tree"]["nodes"] if str(uid)==field["id"])
            self.check("invalid setup focuses the actual server input", any(str(w["tree"]["focus"])==field["id"] for w in validation["windows"]))
            self.check("invalid setup reveals the field at 200 percent", after_bounds["y0"]>=setup_rect[1]-1 and after_bounds["y1"]<=setup_rect[1]+setup_rect[3]+1)


        return self.checks


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--embedded", action="store_true")
    args = p.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    logfile = args.output / "fixture.log"
    env = dict(os.environ, MAKEPAD_HIDE_WINDOWS="1")
    env.pop("MAKEPAD_FOCUS", None)
    if args.embedded:
        profile = (args.output / "profile").resolve()
        profile.mkdir(exist_ok=True)
        (profile / ".article-review-profile").touch()
        (profile / "ui-language.json").write_text(json.dumps("en"))
        env["HAGENCY_DESKTOP_DATA_DIR"] = str(profile)
        env["HAGENCY_DOCK_OFFLINE_ACCOUNT"] = "1"
    command = [str(args.binary.resolve()), "--remote"]
    if not args.embedded: command += ["--fixture=running", "--size=960x760", "--reduced-motion"]
    report = {"binary": str(args.binary.resolve()), "binarySha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(), "commands": command, "mode": "embedded" if args.embedded else "standalone",
              "evidence": "Own-process native AppKit NSAccessibility and normal UI actions; not a manual VoiceOver journey"}
    process = None
    journey = None
    with logfile.open("w") as output:
        try:
            process = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT, env=env)
            deadline = time.monotonic() + 30
            match = None
            while time.monotonic() < deadline:
                match = re.search(r"listening on 127.0.0.1:(\d+) pid=(\d+)", logfile.read_text())
                if match: break
                assert process.poll() is None, "fixture exited during startup"
                time.sleep(.1)
            assert match and int(match[2]) == process.pid, "no owned fixture remote endpoint"
            journey = Journey("http://127.0.0.1:"+match[1], args.output, args.embedded)
            deadline = time.monotonic() + 20
            while not journey.get("/s").get("w"):
                assert time.monotonic() < deadline, "fixture window was not created"
                time.sleep(.1)
            # Native hidden windows need a present to establish layout and allow
            # initial font uploads. Captures stay within the fixture drawable.
            journey.get("/g")
            report["checks"] = journey.run()
            report["passed"] = True
        except Exception as error:
            report["passed"] = False
            report["error"] = str(error)
            report["checks"] = journey.checks if journey else []
            raise
        finally:
            if process:
                if journey and process.poll() is None:
                    try: journey.get("/quit")
                    except Exception: pass
                try: process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.terminate()
                    process.wait(timeout=5)
                report["exitCode"] = process.returncode
            (args.output/"report.json").write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n")
    print(json.dumps(report,ensure_ascii=False))

if __name__ == "__main__": main()
