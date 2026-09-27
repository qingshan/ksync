#!/usr/bin/env python3
"""Capture and verify the current Kindle catalog/dialog navigation tour."""
import csv
import hashlib
import io
import json
import os
import shutil
import subprocess
import time
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
APP = "ksync"
HOST = os.environ.get("KINDLE_E2E_HOST", "kindle")
WAF = "/var/local/mesquite/" + APP
FRAMES = ROOT / "target/demo/frames"
STORYBOARD = ROOT / "target/demo/frames.json"
XINPUT = "/tmp/kindle_xinput"


def ssh(command, text=True, input=None):
    return subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15", HOST, command],
                          check=True, text=text, input=input, stdout=subprocess.PIPE, timeout=30)


def status():
    return json.loads(ssh("cat " + WAF + "/status.json").stdout)


def wait_status(timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            return status()
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            time.sleep(1)
    raise TimeoutError("status.json did not become readable")


def screenshot():
    return ssh('/usr/sbin/screenshot -f /tmp/ksync-demo.png && cat /tmp/ksync-demo.png', text=False).stdout


def expect(*labels):
    deadline = time.monotonic() + 20
    while True:
        image = screenshot()
        with Image.open(io.BytesIO(image)) as source:
            if source.size != (1264, 1680):
                raise RuntimeError("This tour requires the Oasis 1264x1680 framebuffer")
        words = []
        # Sparse layout finds fields; block layout also finds inverted buttons.
        for mode in ("11", "6"):
            result = subprocess.run(["tesseract", "stdin", "stdout", "--psm", mode, "tsv"],
                                    input=image, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                    check=True, timeout=30)
            words.extend(row for row in csv.DictReader(io.StringIO(result.stdout.decode()), delimiter="\t")
                         if row["text"].strip() and int(row["top"]) > 215)
        text = " ".join(word["text"] for word in words).lower()
        if all(label.lower() in text for label in labels):
            return image, words
        if time.monotonic() >= deadline:
            (FRAMES / "failure.png").write_bytes(image)
            raise AssertionError("Expected " + repr(labels) + "; saw: " + text)
        time.sleep(0.5)


def tap(label):
    _, words = expect(label)
    wanted = label.lower().split()
    matches = []
    for index in range(len(words) - len(wanted) + 1):
        group = words[index:index + len(wanted)]
        if [word["text"].lower() for word in group] != wanted:
            continue
        if len({(word["block_num"], word["par_num"], word["line_num"]) for word in group}) != 1:
            continue
        left = min(int(word["left"]) for word in group)
        top = min(int(word["top"]) for word in group)
        right = max(int(word["left"]) + int(word["width"]) for word in group)
        bottom = max(int(word["top"]) + int(word["height"]) for word in group)
        matches.append(((left + right) // 2, (top + bottom) // 2))
    if not matches:
        raise AssertionError("Could not locate control: " + label)
    # Add catalog appears as a title and a button; choose the lower match.
    x, y = max(matches, key=lambda point: point[1])
    ssh("DISPLAY=:0 " + XINPUT, input=f"tap {x} {y}\nsleep 700\n")


def shot(label, caption, *expected):
    if not wait_status():
        raise RuntimeError("status.json is empty")
    out, _ = expect(*expected)
    path = FRAMES / (label + ".png")
    path.write_bytes(out)
    print("captured", label, flush=True)
    return {"label": label, "caption": caption, "dwell_ms": 3000, "file": path.name,
            "sha256": hashlib.sha256(out).hexdigest(), "expected": list(expected)}


def main():
    if not shutil.which("tesseract"):
        raise SystemExit("Install tesseract-ocr to verify and locate Kindle controls")
    FRAMES.mkdir(parents=True, exist_ok=True)
    STORYBOARD.unlink(missing_ok=True)
    ssh("test -x " + XINPUT, text=False)
    for name in ("index.html", "style.css", "ui.js", "form.js", "script.js"):
        installed = ssh("cat " + WAF + "/" + name, text=False).stdout
        if installed != (ROOT / "kpm/waf" / name).read_bytes():
            raise RuntimeError("Install the current package before recording: " + name + " differs")
    initial = wait_status()
    if initial["state"] in ("running", "stopping"):
        raise RuntimeError("Wait for the active sync to finish before recording")
    before = ssh("md5sum /mnt/us/ksync/var/catalogs.json").stdout
    ssh("setsid /var/local/kmc/bin/kpm launch {0} >/tmp/{0}-demo-launch.log 2>&1 </dev/null &".format(APP))
    time.sleep(2)
    ssh("DISPLAY=:0 " + XINPUT, input="key Escape\nsleep 700\n")
    frames = []
    try:
        frames.append(shot("01-catalogs", "Your catalogs. Ready to sync.", "Your reading", "Sync all", "Catalogs"))
        tap("Refresh")
        expect("Sync all", "Catalogs")
        tap("Add catalog")
        frames.append(shot("02-add", "Connect an OPDS catalog", "Add catalog", "OPDS URL", "Cancel"))
        tap("Add catalog")
        frames.append(shot("03-validation", "Check the details before saving", "Name and URL are required", "Cancel"))
        tap("Cancel")
        expect("Your reading", "Catalogs")
        if initial["catalogs"]:
            tap("Edit")
            frames.append(shot("04-edit", "Choose what Sync all includes", "Edit catalog", "Include in sync all", "Delete catalog"))
            tap("Delete catalog")
            frames.append(shot("05-confirm", "Catalog removal asks first", "Downloaded books stay", "Keep catalog"))
            tap("Keep catalog")
            tap("Cancel")
        tap("Settings")
        frames.append(shot("06-settings", "Organize your Kindle collections", "Collection prefix", "Rebuild collections", "Done"))
        tap("Done")
        frames.append(shot("07-ready", "Back to your daily reading", "Your reading", "Sync all", "Catalogs"))
    finally:
        ssh("DISPLAY=:0 " + XINPUT, input="key Escape\nsleep 500\n")
        if ssh("md5sum /mnt/us/ksync/var/catalogs.json").stdout != before:
            raise AssertionError("Catalog configuration changed during the navigation tour")
    final = status()
    if final["catalogs"] != initial["catalogs"] or final["state"] in ("running", "stopping"):
        raise AssertionError("Demo changed catalogs or started a sync")
    STORYBOARD.write_text(json.dumps({"app": APP, "source": "Kindle framebuffer",
                                     "screen": [1264, 1680], "frames": frames}, indent=2) + "\n")
    print("Verified", len(frames), "Kindle frames; catalog configuration unchanged")


if __name__ == "__main__":
    main()
