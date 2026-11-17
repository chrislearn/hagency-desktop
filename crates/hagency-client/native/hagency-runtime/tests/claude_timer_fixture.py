#!/usr/bin/env python3
"""Disposable localhost Anthropic SSE fixture for one timer the session sets.

The model schedules a wake-up one minute ahead and answers. When the CLI
starts the wake-up turn by itself the model reports, and a later prompt gets
one more answer. No credentials or outside calls. Each request's stage is
appended to `stages.jsonl`.
"""
import http.server
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
TOOL_ID = "toolu_wake_1"
WAKE_PROMPT = "Report the timer result"
counts = {}


def stage(data):
    if "ScheduleWakeup" not in [tool.get("name") for tool in data.get("tools", [])]:
        return "side"
    messages = data.get("messages", [])
    if not any(message.get("role") == "assistant" for message in messages):
        return "start"
    last = messages[-1] if messages else {}
    content = last.get("content")
    if last.get("role") == "user" and isinstance(content, list) and any(
            isinstance(block, dict) and block.get("type") == "tool_result" for block in content):
        return "foreground"
    return "later"


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        data = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
        if "count_tokens" in self.path:
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"input_tokens":17}')
            return
        current = stage(data)
        counts[current] = counts.get(current, 0) + 1
        with (root / "stages.jsonl").open("a") as log:
            log.write(json.dumps({"stage": current, "n": counts[current],
                                  "messages": len(data.get("messages", []))}) + "\n")
        if current == "start":
            blocks = [{"type": "tool_use", "id": TOOL_ID, "name": "ScheduleWakeup",
                       "input": {"delaySeconds": 60, "reason": "offline contract: report in one minute",
                                 "prompt": WAKE_PROMPT}}]
        elif current == "foreground":
            blocks = [{"type": "text", "text": "Foreground answer"}]
        elif current == "later":
            blocks = [{"type": "text", "text": "Timer report" if counts[current] == 1 else "Next answer"}]
        else:
            blocks = [{"type": "text", "text": "ok"}]
        stop = "tool_use" if current == "start" else "end_turn"
        message = {"id": f"msg_{current}_{counts[current]}", "type": "message", "role": "assistant",
                   "model": data.get("model", "claude-sonnet-4-5"), "content": blocks,
                   "stop_reason": stop, "stop_sequence": None,
                   "usage": {"input_tokens": 17, "output_tokens": 9, "cache_read_input_tokens": 0,
                             "cache_creation_input_tokens": 0}}
        self.send_response(200)
        if not data.get("stream"):
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps(message).encode())
            return
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()

        def event(kind, value):
            self.wfile.write(f'event: {kind}\ndata: {json.dumps({"type": kind, **value})}\n\n'.encode())
            self.wfile.flush()
        event("message_start", {"message": dict(message, content=[], stop_reason=None)})
        for index, block in enumerate(blocks):
            if block["type"] == "tool_use":
                event("content_block_start", {"index": index, "content_block": dict(block, input={})})
                event("content_block_delta", {"index": index, "delta": {"type": "input_json_delta",
                                                                         "partial_json": json.dumps(block["input"])}})
            else:
                event("content_block_start", {"index": index, "content_block": {"type": "text", "text": ""}})
                event("content_block_delta", {"index": index, "delta": {"type": "text_delta", "text": block["text"]}})
            event("content_block_stop", {"index": index})
        event("message_delta", {"delta": {"stop_reason": stop, "stop_sequence": None}, "usage": {"output_tokens": 9}})
        event("message_stop", {})


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
