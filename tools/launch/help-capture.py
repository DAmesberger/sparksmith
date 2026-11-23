#!/usr/bin/env python3
"""Regenerate the help stills inside the macOS guest and keep provenance for review."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
PROJECT = Path("examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith")
SCENES = [
    ("help-search", "panel/help", "The Help panel lists finding and rule matches for annular ring."),
    ("help-shortcuts", "panel/keyboard-shortcuts",
     "The Keyboard shortcuts panel shows the active binding for Undo."),
]


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def require_guest():
    if platform.system() == "Darwin":
        guest = subprocess.run(["/usr/sbin/sysctl", "-n", "kern.hv_vmm_present"],
                               capture_output=True, text=True)
        if guest.returncode == 0 and guest.stdout.strip() == "1":
            return
    raise SystemExit("Run the capture in the macOS virtual machine.")


def version():
    manifest = (REPO / "Cargo.toml").read_text()
    section = manifest.split("[workspace.package]", 1)[1].split("\n[", 1)[0]
    match = re.search(r'^version\s*=\s*"([\w.+-]+)"', section, re.MULTILINE)
    if not match:
        raise SystemExit("The checkout does not declare its workspace package version.")
    return match[1]


def capture(executable):
    require_guest()
    executable = executable.resolve(strict=True)
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise SystemExit(f"Desktop executable required: {executable}")
    captures = REPO / "target" / "help-capture"
    captures.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix=version() + "-", dir=captures))
    kit = captures / "launchkit"
    code = HERE / "launchkit.swift"
    if not kit.exists() or kit.stat().st_mtime_ns < code.stat().st_mtime_ns:
        run("swiftc", "-O", "-o", kit, code)
    windows = {}
    with tempfile.TemporaryDirectory(dir=output) as temp:
        work = Path(temp)
        project = work / PROJECT.name
        shutil.copyfile(REPO / PROJECT, project)
        for scene, _, _ in SCENES:
            state = work / scene
            dirs = {key: state / key for key in ("config", "recovery", "cache", "home")}
            for path in dirs.values():
                path.mkdir(parents=True)
            env = dict(os.environ, HOME=str(dirs["home"]),
                       SPARKSMITH_LIBRARY_DIR=str(REPO / "library"),
                       SPARKSMITH_CONFIG_DIR=str(dirs["config"]),
                       SPARKSMITH_CACHE_DIR=str(dirs["cache"]),
                       SPARKSMITH_RECOVERY_DIR=str(dirs["recovery"]))
            with (output / f"{scene}.log").open("wb") as log:
                app = subprocess.Popen([executable, project], env=env, stdout=log, stderr=log)
                try:
                    pid = str(app.pid)
                    until = time.monotonic() + 30
                    while time.monotonic() < until:
                        if app.poll() is not None:
                            raise RuntimeError(f"Desktop exited. See {log.name}")
                        window = subprocess.run([kit, "window", pid], capture_output=True)
                        if window.returncode == 0:
                            break
                        time.sleep(0.25)
                    else:
                        raise RuntimeError(f"Desktop opened no window. See {log.name}")
                    # A tiled guest may keep its own window size. These shots use keys only.
                    subprocess.run([kit, "resize", pid, "1380", "900"], check=False)
                    run(kit, "play", pid, HERE / "shots" / f"{scene}.shot",
                        output / f"{scene}-keys.json")
                    frame = json.loads(run(kit, "window", pid, capture_output=True).stdout)
                    windows[scene] = frame
                    time.sleep(0.3)  # allow a compositor frame after accessibility changed
                    run("/usr/sbin/screencapture", "-x", "-o", "-l", str(frame["id"]),
                        output / f"{scene}.png")
                finally:
                    if app.poll() is None:
                        app.terminate()
                        try:
                            app.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            app.kill()
                            app.wait()
    records = [{"topic": topic, "file": f"{scene}.png", "alt": alt,
                "window": windows[scene], "sha256": digest(output / f"{scene}.png")}
               for scene, topic, alt in SCENES]
    record = {"version": version(), "binary_sha256": digest(executable),
              "project": PROJECT.as_posix(), "project_sha256": digest(REPO / PROJECT),
              "images": records}
    (output / "capture.json").write_text(json.dumps(record, ensure_ascii=False, indent=2) + "\n")
    print(output)


def install(output):
    """Only install the reviewed image set. Never invoke the GUI on the host."""
    record = json.loads((output / "capture.json").read_text())
    if (record["version"] != version() or record["project"] != PROJECT.as_posix()
            or record["project_sha256"] != digest(REPO / PROJECT)):
        raise SystemExit("Capture version or example differs from this checkout.")
    expected = {f"{scene}.png": (topic, alt) for scene, topic, alt in SCENES}
    if len(record["images"]) != len(expected) or {x["file"] for x in record["images"]} != set(expected):
        raise SystemExit("The capture record does not cover the expected images.")
    changes = []
    for image in record["images"]:
        name = image["file"]
        topic, alt = expected[name]
        file = output / name
        if image["topic"] != topic or image["alt"] != alt or digest(file) != image["sha256"]:
            raise SystemExit(f"The capture record differs for {name}")
        with file.open("rb") as stream:
            if stream.read(8) != b"\x89PNG\r\n\x1a\n":
                raise SystemExit(f"Expected a PNG window capture in {name}")
        page = REPO / "library/help/topics/panels" / (topic.split("/")[1] + ".md")
        source = page.read_text()
        tag = f'![{alt}]({name} "{version()}")'
        lines = [line for line in source.splitlines() if not (
            line.startswith("![") and re.search(r"\]\(" + re.escape(name) + r'( |\))', line))]
        changes.append((page, "\n".join(lines).rstrip() + "\n\n" + tag + "\n", file))
    for page, source, image in changes:
        shutil.copyfile(image, REPO / "library/help/media" / image.name)
        page.write_text(source)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--desktop", type=Path, help="guest-local Sparksmith desktop executable")
    mode.add_argument("--install", type=Path, help="reviewed folder containing capture.json and PNGs")
    mode.add_argument("--list", action="store_true", help="list the scripted screenshots")
    args = parser.parse_args()
    if args.desktop:
        capture(args.desktop)
    elif args.install:
        install(args.install)
    else:
        for scene, topic, alt in SCENES:
            print(f"{topic}: {scene}.png — {alt}")
