#!/usr/bin/env python3
"""One runnable check for the tom-voice-demo narration stage.

Does not need MLX, a voice reference, or network. It drives narrate.py through
the `--tts-json` seam with a fake engine and asserts the behaviour the workflow
depends on:

  * one WAV per scene, non-empty, with a readable duration
  * manifest records the script, duration, and path
  * a second run reuses the audio instead of regenerating it
  * changed text is refused without --force
  * --force regenerates it

    python3 .agents/skills/tom-voice-demo/scripts/selfcheck.py
"""

from __future__ import annotations

import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
NARRATE = os.path.join(HERE, "narrate.py")

FAKE_TTS = r'''
import math, struct, sys, wave

# argv: fake_tts.py <text> <out>
text = sys.argv[1] if len(sys.argv) > 1 else ""
seconds = max(0.6, min(3.0, 0.3 + len(text) * 0.012))
rate = 22050
frames = int(rate * seconds)
with wave.open("audio_000.wav", "w") as out:
    out.setnchannels(1)
    out.setsampwidth(2)
    out.setframerate(rate)
    data = bytearray()
    for index in range(frames):
        value = int(3000 * math.sin(2 * math.pi * 220 * index / rate))
        data += struct.pack("<h", value)
    out.writeframes(bytes(data))
'''


def run(args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, NARRATE, *args], capture_output=True, text=True)


def main() -> int:
    work = tempfile.mkdtemp(prefix="tom-voice-selfcheck-")
    try:
        fake = os.path.join(work, "fake_tts.py")
        with open(fake, "w", encoding="utf-8") as handle:
            handle.write(FAKE_TTS)

        scenes_path = os.path.join(work, "scenes.json")
        out_dir = os.path.join(work, "narration")
        scenes = [
            {"id": "01-open", "text": "Welcome to the Tran Family archive."},
            {"id": "02-vietnamese", "text": "Hồ sơ gia đình họ Trần, đời thứ mười hai."},
        ]
        with open(scenes_path, "w", encoding="utf-8") as handle:
            json.dump(scenes, handle, ensure_ascii=False)

        tts = json.dumps([sys.executable, fake, "{text}", "{out}"])
        common = ["--scenes", scenes_path, "--out", out_dir, "--tts-json", tts]

        first = run(common)
        if first.returncode != 0:
            print(first.stdout, first.stderr, sep="\n")
            raise SystemExit("selfcheck: first run failed")

        manifest_path = os.path.join(out_dir, "manifest.json")
        manifest = json.load(open(manifest_path, encoding="utf-8"))
        assert len(manifest["scenes"]) == 2, manifest
        for entry in manifest["scenes"]:
            wav = os.path.join(out_dir, entry["audio"])
            assert os.path.getsize(wav) > 1000, f"empty audio: {wav}"
            assert entry["duration_seconds"] > 0, entry
            assert entry["text"], entry
        # Vietnamese survives the round trip unchanged.
        assert manifest["scenes"][1]["text"] == scenes[1]["text"], manifest["scenes"][1]

        before = {name: os.stat(os.path.join(out_dir, name)).st_mtime_ns
                  for name in os.listdir(out_dir) if name.endswith(".wav")}

        second = run(common)
        if second.returncode != 0:
            print(second.stdout, second.stderr, sep="\n")
            raise SystemExit("selfcheck: second run failed")
        assert second.stdout.count("reuse") == 2, second.stdout
        after = {name: os.stat(os.path.join(out_dir, name)).st_mtime_ns
                 for name in os.listdir(out_dir) if name.endswith(".wav")}
        assert before == after, "unchanged inputs rewrote audio"
        manifest2 = json.load(open(manifest_path, encoding="utf-8"))
        assert all(entry["status"] == "reused" for entry in manifest2["scenes"]), manifest2

        scenes[1]["text"] = "Câu đã sửa."
        with open(scenes_path, "w", encoding="utf-8") as handle:
            json.dump(scenes, handle, ensure_ascii=False)
        refused = run(common)
        assert refused.returncode == 2, (refused.returncode, refused.stdout, refused.stderr)
        assert "refusing to overwrite" in refused.stderr, refused.stderr

        forced = run(common + ["--force"])
        assert forced.returncode == 0, forced.stderr
        assert forced.stdout.count("generate") == 2, forced.stdout

        # The WAV header parser is the only timing source; prove it reads a
        # hand-built file as well as the fake engine's output.
        probe = os.path.join(work, "probe.wav")
        with open(probe, "wb") as handle:
            data = b"\x00\x00" * 22050
            handle.write(b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVE")
            handle.write(b"fmt " + struct.pack("<IHHIIHH", 16, 1, 1, 22050, 44100, 2, 16))
            handle.write(b"data" + struct.pack("<I", len(data)) + data)

        sys.path.insert(0, HERE)
        import narrate  # noqa: E402

        assert abs(narrate.wav_duration(probe) - 1.0) < 0.01, narrate.wav_duration(probe)

        print("selfcheck: PASS (2 scenes, reuse, conflict, force, duration parser)")
        return 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
