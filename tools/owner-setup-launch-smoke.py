#!/usr/bin/env python3
"""Check setup navigation with Hosting already open, using an isolated bundle.

Does not enroll an owner, start a daemon, or modify installed applications.
Exercises LaunchServices argument delivery and the production form acknowledgment.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import socket
import subprocess
import tempfile
import time
from urllib.parse import urlencode
from urllib.request import urlopen
import uuid


def wait_for(predicate, timeout=20):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        try:
            value = predicate()
            if value:
                return value
        except (OSError, ValueError):
            pass
        time.sleep(0.1)
    raise AssertionError("Timed out waiting for setup window")


def request(port, path, **params):
    suffix = "?" + urlencode(params) if params else ""
    return json.load(urlopen(f"http://127.0.0.1:{port}{path}{suffix}", timeout=5))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    report = {"passed": False, "checks": [], "binarySha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              "limitations": ["Uses an isolated bundle identifier; installed bundle lookup is not exercised.",
                              "Only setup navigation is exercised; no enrollment or hosting is started."]}
    ports = []

    def check(name, passed):
        report["checks"].append({"check": name, "passed": bool(passed)})
        assert passed, name

    try:
        with tempfile.TemporaryDirectory(prefix="oset-", dir="/tmp") as tmp:
            root = Path(tmp)
            bundle = root / "Setup Review.app"
            executable = bundle / "Contents/MacOS/hagency-owner-controller"
            executable.parent.mkdir(parents=True)
            shutil.copy2(args.binary, executable)
            (bundle / "Contents/Info.plist").write_bytes(plistlib.dumps({
                "CFBundleIdentifier": "io.hagency.owner-controller.review." + uuid.uuid4().hex,
                "CFBundleName": "Hagency Setup Review", "CFBundleExecutable": executable.name,
                "CFBundlePackageType": "APPL", "NSHighResolutionCapable": True,
            }))
            subprocess.run(["codesign", "--force", "--sign", "-", str(bundle)], check=True, capture_output=True)
            home = root / "daemon"
            inbox = home / "setup-navigation"
            home.mkdir(mode=0o700)
            inbox.mkdir(mode=0o700)
            now = int(time.time() * 1000)
            token = uuid.uuid4().hex
            ticket = inbox / (token + ".json")
            envelope = {"version": 1, "id": token, "ownerId": "@review:example.test", "source": "embedded", "target": "standalone",
                        "createdAtMs": now, "expiresAtMs": now + 120000,
                        "view": {"kind": "setup", "server": "https://example.test", "adoptRoot": str(root / "owner profile")}}
            ticket.write_text(json.dumps(envelope))
            ticket.chmod(0o600)
            setup_args = ["--setup", "--owner", envelope["ownerId"], "--server", envelope["view"]["server"],
                          "--adopt-root", envelope["view"]["adoptRoot"], "--setup-request", str(ticket)]

            def launch(name, new, extra):
                with socket.socket() as s:
                    s.bind(("127.0.0.1", 0))
                    port = s.getsockname()[1]
                command = ["/usr/bin/open", "-g", "-j"] + (["-n"] if new else [])
                command += ["--env", "MAKEPAD_HIDE_WINDOWS=1", "--env", "MAKEPAD_NO_FOCUS=1", "--env", f"MAKEPAD_REMOTE={port}",
                            "-o", str(out / (name + ".log")), "--stderr", str(out / (name + "-stderr.log")),
                            str(bundle), "--args", "--daemon-home", str(home), *extra]
                subprocess.run(command, check=True)
                if new:
                    ports.append(port)
                    wait_for(lambda: request(port, "/s").get("w"))
                return port

            first = launch("existing", True, [])
            first_pid = request(first, "/s")["pid"]
            launch("old-path", False, setup_args)
            time.sleep(1)
            ack = inbox / (token + ".ack")
            check("ordinary reopen does not deliver setup to the existing process", not ack.exists())
            second = launch("setup", True, setup_args)
            wait_for(ack.exists)
            check("new setup instance acknowledges the exact requested form", json.loads(ack.read_text()) == envelope)
            check("existing dashboard stays open", request(first, "/s")["pid"] == first_pid)
            check("setup has its own controller process", request(second, "/s")["pid"] != first_pid)
            snap = request(second, "/snap", w=0)
            fields = {item["i"]: item.get("t", "") for item in snap["s"] if item.get("i") in ["setup_server", "setup_owner"]}
            check("setup shows the supplied server and owner", fields == {"setup_server": "https://example.test", "setup_owner": "@review:example.test"})
            request(second, "/g", w=0)
            time.sleep(0.4)
            capture = request(second, "/g", w=0)
            shutil.copyfile(capture["png"], out / "setup.png")
            check("setup navigation creates no enrollment or daemon socket", not (home / "enrollment.json").exists() and not (home / "control.sock").exists())
            report["passed"] = True
            for port in ports:
                request(port, "/quit")
            ports.clear()
    finally:
        for port in ports:
            try:
                request(port, "/quit")
            except OSError:
                pass
        (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "checks": len(report["checks"])}))


if __name__ == "__main__":
    main()
