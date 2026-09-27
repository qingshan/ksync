#!/usr/bin/env python3
"""Encode a Kindle demo storyboard produced by tests/e2e/kindle_demo.py."""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
STORYBOARD = ROOT / "target/demo/frames.json"
FRAMES = ROOT / "target/demo/frames"
OUT = ROOT / "dist/demo"
COMPOSED = ROOT / "target/demo/composed"


def compose(frame, index, total):
    source = FRAMES / frame["file"]
    if hashlib.sha256(source.read_bytes()).hexdigest() != frame["sha256"]:
        raise ValueError("Frame changed since verification: " + frame["file"])
    image = Image.open(source).convert("RGB")
    if image.size != (1264, 1680):
        raise ValueError("Unexpected framebuffer size: " + frame["file"])
    draw = ImageDraw.Draw(image)
    caption = "{}/{}  {}".format(index + 1, total, frame["caption"])
    font_path = ROOT / "kpm/waf/fonts/JetBrainsMonoNerdFontMono-Regular.ttf"
    size = 40
    while True:
        face = ImageFont.truetype(str(font_path), size)
        if draw.textlength(caption, font=face) <= 1200 or size <= 20:
            break
        size -= 2
    # Only the system status bar is replaced; the captured application stays intact.
    draw.rectangle((0, 0, 1263, 100), fill="white")
    draw.text((32, 50), caption, font=face, fill="black", anchor="lm")
    target = COMPOSED / frame["file"]
    image.save(target)
    return target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--readme", action="store_true", help="also update the README GIF and screenshot in docs/")
    args = parser.parse_args()
    if not STORYBOARD.exists():
        sys.exit("error: run `just demo` on a connected Kindle first")
    if not shutil.which("ffmpeg"):
        sys.exit("error: ffmpeg is required to encode the demo")
    data = json.loads(STORYBOARD.read_text())
    frames = data["frames"]
    if not frames:
        sys.exit("error: storyboard has no frames")
    if data.get("source") != "Kindle framebuffer":
        sys.exit("error: record a verified Kindle storyboard with the current recorder")
    OUT.mkdir(parents=True, exist_ok=True)
    COMPOSED.mkdir(parents=True, exist_ok=True)
    composed = [compose(frame, index, len(frames)) for index, frame in enumerate(frames)]
    concat = OUT / "frames.txt"
    with concat.open("w") as handle:
        handle.write("ffconcat version 1.0\n")
        for frame, image in zip(frames, composed):
            handle.write("file '{}'\nduration {:.3f}\n".format(str(image).replace("'", "'\\''"), frame["dwell_ms"] / 1000))
        handle.write("file '{}'\n".format(str(composed[-1]).replace("'", "'\\''")))
    stem = data.get("app", ROOT.name) + "-demo"
    mp4 = OUT / (stem + ".mp4")
    gif = OUT / (stem + ".gif")
    base = ["ffmpeg", "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", str(concat)]
    subprocess.run(base + ["-pix_fmt", "yuv420p", "-c:v", "libx264", "-crf", "20", "-movflags", "+faststart", str(mp4)], check=True)
    subprocess.run(base + ["-vf", "fps=12,scale=632:-1:flags=lanczos", "-loop", "0", str(gif)], check=True)
    shutil.copyfile(composed[0], OUT / (stem + "-poster.png"))
    if args.readme:
        shutil.copyfile(gif, ROOT / "docs/ksync-demo.gif")
        shutil.copyfile(FRAMES / frames[0]["file"], ROOT / "docs/ksync.png")
    concat.unlink()
    print(mp4)
    print(gif)


if __name__ == "__main__":
    main()
