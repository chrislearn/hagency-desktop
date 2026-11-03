#!/usr/bin/env python3
"""Real cross-process dashboard restore/ack journey using synthetic UI data.

Starts only the supplied fixture executables in private temporary profiles.
The production Bridge/store and both host restore/ack paths run unchanged;
its explicit fixture launcher skips LaunchServices because peers already run.
No restore-view injection, daemon enrollment, account login or live app control.
"""
from __future__ import annotations
import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
from urllib.parse import urlencode
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[1]


def wait_for(predicate, timeout=20):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        result = predicate()
        if result:
            return result
        time.sleep(0.1)
    raise AssertionError("Timed out waiting for fixture evidence")


class Peer:
    def __init__(self, binary, kind, name, home, output):
        self.kind, self.name, self.output = kind, name, output
        self.out = output / name
        self.out.mkdir()
        profile = home / (name + "-profile")
        profile.mkdir()
        (profile / ".article-review-profile").touch()
        (profile / "ui-language.json").write_text('"en"')
        sock = socket.socket()
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
        sock.close()
        self.base = f"http://127.0.0.1:{port}"
        env = dict(os.environ, MAKEPAD_REMOTE=str(port), MAKEPAD_HIDE_WINDOWS="1", MAKEPAD_NO_FOCUS="1",
                   HAGENCY_OWNER_HANDOVER_FIXTURE_HOME=str(home), HAGENCY_OWNER_DAEMON_HOME=str(home / "unused-daemon"),
                   HAGENCY_DESKTOP_DATA_DIR=str(profile), HAGENCY_DOCK_OFFLINE_ACCOUNT="1",
                   HAGENCY_OWNER_REVIEW_WIDTH="760", HAGENCY_DOCK_REVIEW_SIZE="1200x850")
        env.pop("MAKEPAD_FOCUS", None)
        command = [str(binary.resolve())]
        if kind == "standalone":
            command += ["--fixture=running", "--locale=en", "--appearance=light", "--size=760x720"]
        self.logfile = (self.out / "fixture.log").open("w")
        self.process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=self.logfile, stderr=subprocess.STDOUT)
        def ready():
            if self.process.poll() is not None:
                raise AssertionError(f"{name} exited during startup")
            try:
                return bool(self.req("/s").get("w"))
            except OSError:
                return False
        try:
            wait_for(ready)
            self.req("/g", w=0)
            time.sleep(0.4)
        except Exception:
            self.close()
            raise

    def req(self, path, **params):
        return json.load(urlopen(self.base + path + ("?" + urlencode(params) if params else ""), timeout=10))

    def event(self, value):
        self.req("/event", data=value, wait=1)
        self.req("/g", w=0)

    def rows(self):
        return self.req("/snap", w=0)["s"]

    def find(self, identity):
        return next((row for row in self.rows() if row.get("i") == identity and row["r"][2] > 0 and row["r"][3] > 0), None)

    def click(self, identity):
        if identity == "od_open_other" and not self.find(identity):
            self.click("od_owner_menu")
        row = self.find(identity)
        assert row, (self.name, "missing control", identity)
        x, y, w, h = row["r"]
        try:
            self.req("/click", x=x+w/2, y=y+h/2, w=0, wait=1)
        except OSError:
            # A successful handover can close the source HTTP server before
            # its click reply arrives. Only accept the expected normal exit;
            # bridge completion and destination acknowledgment are checked
            # separately below.
            if identity != "od_open_other":
                raise
            try:
                status = self.process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                raise
            if status != 0:
                raise
        # Open-in-Hagency may close after a fast acknowledgment; callers wait
        # on the process rather than assuming a second HTTP request succeeds.
        if identity != "od_open_other":
            self.req("/g", w=0)

    def view(self):
        marker = "od-embedded-view " if self.kind == "embedded" else "od-state "
        self.event("dock:owner-dump" if self.kind == "embedded" else "dump")
        lines = self.req("/log", n=200)["l"]
        rows = [json.loads(line.split(marker, 1)[1]) for line in lines if marker in line]
        assert rows, (self.name, "missing view dump")
        return rows[-1]["view"]

    def capture(self, name):
        self.req("/g", w=0)
        image = self.req("/g", w=0)
        shutil.copyfile(image["png"], self.out / (name + ".png"))
        rows = self.rows()
        (self.out / (name + ".json")).write_text(json.dumps(rows, indent=2, ensure_ascii=False))
        (self.out / (name + "-view.json")).write_text(json.dumps(self.view(), indent=2, ensure_ascii=False))
        return {"window": self.req("/s")["w"], "ownerHost": [r["r"] for r in rows if r.get("i") in ["owner_host", "dashboard"]]}

    def bridge_events(self):
        self.logfile.flush()
        result = []
        for line in (self.out / "fixture.log").read_text().splitlines():
            if "od-handover-fixture " in line:
                result.append(json.loads(line.split("od-handover-fixture ", 1)[1]))
        return result

    def close(self):
        if self.process.poll() is None:
            try:
                self.req("/quit")
                self.process.wait(timeout=5)
            except (OSError, subprocess.TimeoutExpired):
                self.process.terminate()
                self.process.wait(timeout=5)
        self.logfile.close()


def same_destination(left, right):
    # Pixel scroll offset may clamp to the destination's different viewport.
    return all(left.get(key) == right.get(key) for key in (
        "ownerId", "tab", "selectedResource", "accountFilter", "resourceFilter", "modelFilter",
        "requesterFilter", "filtersOpen", "dateStart", "dateEnd", "period", "pages"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--standalone", type=Path, required=True)
    parser.add_argument("--embedded", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    report = {"passed": False, "syntheticOwner": "@owner:example.test", "checks": [],
              "binaries": {key: {"path": str(value.resolve()), "sha256": hashlib.sha256(value.read_bytes()).hexdigest()}
                           for key, value in [("standalone", args.standalone), ("embedded", args.embedded)]},
              "limitations": ["Peers are prestarted; this test does not exercise LaunchServices bundle lookup.",
                              "Usage data is synthetic; no daemon, provider credentials, or resource execution is used.",
                              "Concurrent controllers are independent observers; competing destination claims are not tested.",
                              "Scroll offsets can clamp to different viewport sizes; original and restored offsets are recorded."]}
    peers = []
    def check(label, condition):
        report["checks"].append({"check": label, "passed": bool(condition)})
        assert condition, label
    try:
        with tempfile.TemporaryDirectory(prefix="hagency-two-host-ui-") as temporary, ExitStack() as lifecycle:
            home = Path(temporary)
            (home / "owner-dashboard-ui-fixture").write_text("owner-dashboard-ui-fixture-v1\n")
            def start(binary, kind, name):
                peer = Peer(binary, kind, name, home, args.output)
                peers.append(peer)
                lifecycle.callback(peer.close)
                return peer
            embedded = start(args.embedded, "embedded", "embedded")
            source = start(args.standalone, "standalone", "standalone-source")
            # UI actions establish the source view; no fixture restore command.
            source.click("od_tab_usage")
            source.click("od_period_week")
            source.click("od_tab_resources")
            source.click("od_resource_open")
            original = source.view()
            check("source UI has a selected resource and week period", original["selectedResource"] is not None and original["period"]["kind"] == "week")
            report["sourceBounds"] = source.capture("before-handover")
            # Start outside Hosting. The production surface must select the
            # Hosting page before delivering Restore to its active receiver.
            embedded.click("hosting")
            check("destination Hosting page starts hidden", not embedded.find("od_tab_overview"))
            embedded.event("dock:handover-hold:true")
            source.click("od_open_other")
            wait_for(lambda: any(e["detail"]["event"] == "opening" for e in source.bridge_events()))
            time.sleep(0.7)
            check("source stays open until destination UI acknowledges", source.process.poll() is None)
            check("no completion is invented by launching a peer", not any(e["detail"]["event"] == "completed" for e in source.bridge_events()))
            embedded.event("dock:handover-hold:false")
            wait_for(lambda: source.process.poll() is not None)
            check("source exits through production completion path", source.process.returncode == 0)
            restored = embedded.view()
            check("embedded production host restores source view", same_destination(original, restored))
            acknowledged = [e for e in embedded.bridge_events() if e["detail"]["event"] == "uiAcknowledged" and e["detail"]["accepted"]]
            completed = [e for e in source.bridge_events() if e["detail"]["event"] == "completed"]
            check("embedded UI acknowledgment precedes source completion",
                  acknowledged and completed and acknowledged[0]["atMs"] <= completed[0]["atMs"])
            report["embeddedBounds"] = embedded.capture("after-handover")
            # Reverse path uses the embedded real Open separately action.
            receiver = start(args.standalone, "standalone", "standalone-receiver")
            if not embedded.find("od_open_other"):
                embedded.click("od_owner_menu")
            check("embedded adapter offers Open separately", embedded.find("od_open_other")["t"] == "Open separately")
            embedded.click("od_open_other")
            wait_for(lambda: same_destination(original, receiver.view()))
            wait_for(lambda: any(e["detail"]["event"] == "completed" for e in embedded.bridge_events()))
            check("reverse handover restores in standalone host", same_destination(original, receiver.view()))
            check("reverse handover is acknowledged by standalone UI", any(e["detail"]["event"] == "uiAcknowledged" and e["detail"]["accepted"] for e in receiver.bridge_events()))
            check("Open separately keeps embedded controller available", embedded.process.poll() is None)
            report["reverseBounds"] = receiver.capture("after-reverse")
            # A further controller sends to embedded while the first remains
            # open and keeps its own independent view/selection.
            concurrent = start(args.standalone, "standalone", "standalone-concurrent")
            concurrent.click("od_tab_activity")
            concurrent_view = concurrent.view()
            receiver_before = receiver.view()
            concurrent.click("od_open_other")
            wait_for(lambda: concurrent.process.poll() is not None)
            check("another controller can hand over independently", same_destination(concurrent_view, embedded.view()))
            check("simultaneous controller remains open with its own view", receiver.process.poll() is None and same_destination(receiver_before, receiver.view()))
            check("fixture handover creates no daemon enrollment", not (home / "unused-daemon").exists())
            embedded.capture("concurrent-destination")
            report["events"] = {peer.name: peer.bridge_events() for peer in peers}
            report["passed"] = True
    except Exception as error:
        report["error"] = repr(error)
        raise
    finally:
        for peer in reversed(peers):
            peer.close()
        (args.output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps({"passed": report["passed"], "checks": len(report["checks"]), "output": str(args.output)}))


if __name__ == "__main__":
    main()
