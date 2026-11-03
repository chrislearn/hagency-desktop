#!/usr/bin/env python3
"""Review expired pool authorization in the shipped native UI, with offline data."""
import argparse
import hashlib
import json
from pathlib import Path
from native_local_agents import Host, ROOT


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/examples/hagency_visual")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    reports = []
    for language, label in [("en", "Paused · authorization ended"), ("zh-CN", "已暂停 · 本期授权结束")]:
        output = args.output / language
        host = Host(args.binary.resolve(), output.resolve(), language, True, pool={
            "blockingReason": "agent_period_ended",
            # Deliberately stale readiness must not override current authority.
            "runtimeState": "qualified",
            "periodKey": "2026-10",
            "expiresAtMs": 1793491200000,
            "createdAtMs": 1793404800000,
        })
        try:
            host.click("agents", content=False)
            host.wait(lambda: host.row("pool_agent_open"), "pool agent list")
            host.reveal("pool_agent_open")
            host.wait(lambda: any(w.get("t") == label for w in host.snap()), "paused list label")
            host.capture("list")
            host.click("pool_agent_open")
            host.wait(lambda: any(w.get("t") == label for w in host.snap()), "paused detail label")
            host.wait(lambda: host.row("pool_authority_detail"), "reason for pause")
            reason = host.row("pool_authority_detail")["t"]
            assert ("rooms and history" if language == "en" else "房间和历史记录") in reason
            assert not any(w.get("i") == "pool_agent_start" and w["r"][2] > 0 for w in host.snap())
            host.capture("detail")
            rows = [w for w in host.snap() if w.get("ty") != "Splash"]
            (output / "detail.json").write_text(json.dumps(rows, ensure_ascii=False, indent=2))
            visible = [w.get("t", "") for w in rows if w.get("r", [0, 0, 0, 0])[2] > 0]
            assert not any(t in ("进程已启动 · 最近记录", "Process started · last reported") for t in visible)
            reports.append({"language": language, "passed": True})
        finally:
            host.close()
    report = {
        "cases": reports,
        "sourceSha256": hashlib.sha256((ROOT / "apps/hagency/bundle/main.splash").read_bytes()).hexdigest(),
        "binarySha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "offline": True,
    }
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(reports))


if __name__ == "__main__":
    main()
