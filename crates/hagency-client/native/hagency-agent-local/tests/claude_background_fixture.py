#!/usr/bin/env python3
"""Disposable localhost Anthropic SSE fixture for one background shell command.

The model starts `sleep 2` in the background, answers, and after the CLI's own
task notification turn reports the background result. No credentials or
outside calls. Each request's stage is appended to `stages.jsonl`.
"""
import http.server
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
TOOL_ID = "background-native"
FOREGROUND = "Foreground answer"
BACKGROUND = "Background result"


def said(messages, text):
    return any(message.get("role") == "assistant" and isinstance(message.get("content"), list)
               and any(block.get("type") == "text" and block.get("text") == text
                       for block in message["content"] if isinstance(block, dict))
               for message in messages)


def stage(data):
    messages = data.get("messages", [])
    if "Bash" not in [tool.get("name") for tool in data.get("tools", [])]:
        return "side"
    if said(messages, FOREGROUND):
        return "notification"
    started = any(isinstance(message.get("content"), list) and any(
        isinstance(block, dict) and block.get("type") == "tool_result" and block.get("tool_use_id") == TOOL_ID
        for block in message["content"]) for message in messages)
    return "foreground" if started else "start"


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
        with (root / "stages.jsonl").open("a") as log:
            log.write(json.dumps({"stage": current, "messages": len(data.get("messages", []))}) + "\n")
        if current == "start":
            blocks = [{"type": "tool_use", "id": TOOL_ID, "name": "Bash",
                       "input": {"command": "sleep 2", "description": "Wait two seconds",
                                 "run_in_background": True}}]
        else:
            text = {"foreground": FOREGROUND, "notification": BACKGROUND}.get(current, "ok")
            blocks = [{"type": "text", "text": text}]
        stop = "tool_use" if current == "start" else "end_turn"
        message = {"id": f"msg_{current}_{len(data.get('messages', []))}", "type": "message", "role": "assistant",
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
