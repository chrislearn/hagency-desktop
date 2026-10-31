#!/usr/bin/env python3
"""Hidden native Makepad/Metal review: real widgets, input and local draft storage.

No live Matrix session, publication, account credentials or provider call. Screenshots
and native bounds are evidence; this script does not invent a visual quality score.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request


class Review:
    def __init__(self, binary, output, width, language):
        self.output = output.resolve()
        self.output.mkdir(parents=True, exist_ok=True)
        self.profile = self.output / "profile"
        self.profile.mkdir(exist_ok=True)
        (self.profile / ".article-review-profile").touch()
        (self.profile / "ui-language.json").write_text(json.dumps(language))
        self.trace = []
        self.checks = []
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            self.port = sock.getsockname()[1]
        env = dict(os.environ, HAGENCY_DESKTOP_DATA_DIR=str(self.profile),
                   MAKEPAD_REMOTE=str(self.port), MAKEPAD_HIDE_WINDOWS="1", MAKEPAD_NO_FOCUS="1",
                   ARTICLE_REVIEW_WIDTH=str(width), ARTICLE_REVIEW_HEIGHT="820")
        env.pop("MAKEPAD_FOCUS", None)
        self.log = (self.output / "native.log").open("w")
        self.process = subprocess.Popen([str(binary.resolve())], env=env,
                                        stdout=self.log, stderr=subprocess.STDOUT)
        until = time.monotonic() + 40
        while time.monotonic() < until:
            if self.process.poll() is not None:
                raise RuntimeError("Review process exited; inspect native.log")
            try:
                if self.request("/s")["pid"] == self.process.pid:
                    break
            except OSError:
                time.sleep(.2)
        else:
            raise RuntimeError("Native window did not become ready")
        self.request("/event", data="article:open")
        time.sleep(.25)

    def request(self, path, **params):
        self.trace.append({"path": path, "params": params, "at": time.time()})
        url = f"http://127.0.0.1:{self.port}{path}?" + urllib.parse.urlencode(params)
        with urllib.request.urlopen(url, timeout=10) as response:
            return json.load(response)

    def snap(self):
        return self.request("/snap", w=0)["s"]

    def widget(self, ident):
        return next(w for w in self.snap() if w.get("i") == ident and w.get("r", [0, 0, 0, 0])[2] > 0)

    def click(self, ident):
        w = self.widget(ident)
        assert w.get("enabled", True), ident
        x, y, width, height = w["r"]
        self.request("/click", w=0, x=x+width/2, y=y+height/2, wait=1)
        time.sleep(.18)

    def fill(self, ident, value):
        self.click(ident)
        self.request("/k", w=0, c="A", cmd=1, wait=1)
        self.request("/t", w=0, t=value, wait=1)

    def capture(self, name):
        time.sleep(.15)
        image = self.request("/g", w=0, scale=.75)
        assert image.get("capture_kind") == "pixels"
        shutil.copy2(image["png"], self.output / (name + ".png"))
        (self.output / (name + ".json")).write_text(json.dumps(self.snap(), ensure_ascii=False, indent=2))

    def mark(self, name):
        self.checks.append(name)

    def consent(self):
        self.click("article_continue")
        self.click("article_allow")

    def stop(self, passed):
        try:
            self.capture("final" if passed else "failure")
        except Exception as error:
            self.trace.append({"captureError": str(error)})
            passed = False
        finally:
            try:
                self.request("/quit")
            except OSError:
                pass
            try:
                self.process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                self.process.terminate()
                try:
                    self.process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    self.process.kill()  # Only this owned offline fixture.
                    self.process.wait(timeout=8)
            self.log.close()
            log = (self.output / "native.log").read_text(errors="replace")
            fatal = any(s in log for s in ["panicked at", "ScriptError", "shader compilation failed"])
            (self.output / "report.json").write_text(json.dumps({
                "passed": passed and not fatal, "checks": self.checks, "fatalErrors": fatal,
                "renderer": "hidden native Makepad / Metal", "trace": self.trace,
            }, ensure_ascii=False, indent=2))
            assert not fatal, "Native runtime errors; inspect native.log"


def run(binary, output, width, language):
    app = Review(binary, output, width, language)
    passed = False
    try:
        app.consent()
        app.capture("01-library")
        app.click("article_new")
        title = "把周末留给自己" if language == "zh-CN" else "A little room to breathe"
        title_id = "write_title" if width >= 960 else "write_title_small"
        app.fill(title_id, title)
        app.click("write_mode_source")
        source = "## 慢一点，也很好\n\n清晨，给自己留一杯咖啡的时间。把手机放下，看阳光慢慢走过窗台。\n\n### 今天的小计划\n\n- 去河边散步\n- 读完喜欢的一章\n- 写下一个新的想法\n\n> 留白，是生活的一部分。\n\n" + "愿你在忙碌之间，也能找到属于自己的节奏。\n\n" * 12
        app.fill("write_source", source)
        before = {i: app.widget(i)["r"] for i in ["write_source", "write_mode_source", "write_publish"]}
        time.sleep(1.1)
        after = {i: app.widget(i)["r"] for i in before}
        assert before == after, ("Autosave moved controls", before, after)
        app.mark("autosave_does_not_move_source_or_navigation")
        library_path = next(app.profile.glob("mini-apps/**/library-v2.json"))
        library = json.loads(library_path.read_text())
        assert any(d["title"] == title for d in library["documents"])
        app.mark("native_input_and_local_draft_save")
        app.click("write_source")
        app.request("/k", w=0, c="A", cmd=1, wait=1)
        app.click("wt_bold" if width >= 960 else "wb_bold")
        formatted = app.widget("write_source").get("val")
        assert formatted != source and "**" in formatted, "Formatting lost the text selection"
        app.request("/k", w=0, c="Z", cmd=1, wait=1)
        assert app.widget("write_source").get("val") == source, "Undo did not restore source"
        app.request("/k", w=0, c="Z", cmd=1, shift=1, wait=1)
        assert app.widget("write_source").get("val") == formatted, "Redo did not restore formatting"
        app.request("/k", w=0, c="Z", cmd=1, wait=1)
        app.mark("formatting_preserves_selection_and_keyboard_undo_redo")
        app.click("write_source")
        app.request("/k", w=0, c="Home", cmd=1, wait=1)
        app.capture("02-editor")
        for ident in ["article_back", "write_mode_source", "write_mode_preview", "write_style", "write_publish"]:
            w = app.widget(ident)
            x, y, bw, bh = w["r"]
            assert bw >= 44 and bh >= 44, (ident, w["r"])
            assert x >= 0 and x + bw <= width + .5 and y + bh <= 820, (ident, w["r"])
        app.mark("primary_controls_are_visible_with_44_point_targets")
        app.click("write_mode_preview")
        app.capture("03-preview")
        app.click("write_style")
        app.capture("04-styles")
        if width < 960:
            previous = json.loads(library_path.read_text())["documents"][0]["theme"]
            app.click("s1")
        app.click("write_themes_close" if width >= 960 else "theme_sheet_close")
        if width < 960:
            time.sleep(.9)
            assert json.loads(library_path.read_text())["documents"][0]["theme"] == previous
            app.mark("cancelling_style_preview_restores_saved_theme")
        app.click("write_mode_source")
        value = app.widget("write_source").get("val")
        assert value == source, ("Mode switch changed source", value)
        app.mark("preview_and_style_navigation_preserve_source")
        app.request("/event", data="article:dark")
        time.sleep(.3)
        assert app.widget("write_source").get("val") == source
        app.capture("05-dark-editor")
        app.click("write_mode_preview")
        app.capture("06-dark-preview")
        app.click("write_style")
        app.capture("07-dark-styles")
        app.click("write_themes_close" if width >= 960 else "theme_sheet_close")
        app.mark("theme_switch_preserves_draft_and_current_page")
        if width >= 960:
            app.click("write_mode_split")
            app.click("write_style")
            paper = app.widget("write_paper")["r"]
            pane = app.widget("write_preview_pane")["r"]
            assert paper[0] >= pane[0] and paper[0]+paper[2] <= pane[0]+pane[2]+.5
            app.capture("08-split-with-styles")
            app.click("write_themes_close")
            app.mark("split_preview_stays_inside_pane_with_inspector_open")
        app.request("/event", data="article:light")
        app.request("/event", data="article:large-text")
        app.click("write_mode_source")
        app.capture("09-large-text")
        for ident in ["write_mode_source", "write_mode_preview", "write_publish"]:
            x, y, bw, bh = app.widget(ident)["r"]
            assert x >= 0 and x+bw <= width+.5, ("Large text overflow", ident, x, bw)
        style_id = "wb_style" if width < 448 else "write_style"
        assert app.widget(style_id)["r"][2] >= 44
        app.mark("large_text_keeps_primary_actions_reachable")
        app.click("write_publish")
        app.capture("10-publication-review")
        if width >= 960:
            app.widget("sheet_no_rooms")
            assert not app.widget("sheet_publish").get("enabled", True)
            app.click("sheet_cancel")
        else:
            assert app.widget("cover_done").get("t") in ("Next step", "下一步")
            assert not any(w.get("i") == "cover_x" and w.get("r", [0, 0, 0, 0])[2] > 0 for w in app.snap())
            app.click("cover_done")
            app.widget("rooms_empty")
            assert not app.widget("rooms_publish").get("enabled", True)
            app.capture("10b-publication-destination")
            app.click("article_back")
            app.click("article_back")
        app.mark("empty_destination_is_explained_and_cannot_publish")
        app.widget("write_source")
        app.mark("publication_stops_at_review_and_returns_to_draft")
        app.click("article_back")
        app.capture("11-saved-library")
        assert any(w.get("t") == title for w in app.snap()), "Saved draft is missing from library"
        app.mark("saved_draft_visible_after_return_to_library")
        app.widget("summary")
        app.mark("wrapped_library_title_keeps_status_visible")
        app.click("title")
        app.click("write_mode_source")
        assert app.widget("write_source").get("val") == source
        assert app.widget(title_id).get("val") == title
        app.capture("12-reopened-draft")
        app.mark("reopening_saved_draft_restores_title_and_source")
        passed = True
    finally:
        app.stop(passed)
    print(json.dumps({"passed": passed, "output": str(output), "checks": app.checks}), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--width", type=int, default=390)
    parser.add_argument("--language", choices=["zh-CN", "en"], default="zh-CN")
    args = parser.parse_args()
    run(args.binary, args.output, args.width, args.language)
