#!/usr/bin/env python3
"""Generate one narration WAV per scene with Tom's cloned voice (MLX Audio).

Stage 1 of the tom-voice-demo skill. Local only: the reference recording is
passed to MLX on this machine and never uploaded.

    narrate.py --scenes demo/scenes.json --out demo/narration
    narrate.py --text-file script.txt --out demo/narration
    narrate.py --scenes scenes.json --out narration --only 03-ask --force

Writes <out>/<scene-id>.wav and <out>/manifest.json. The manifest records the
script, the audio duration, the output path, and the input hashes that decide
whether the audio can be reused.

No shell is involved anywhere: the text is one argv element.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time

HOME = os.path.expanduser("~")
DEFAULTS = {
    "python": os.path.join(HOME, "tools/mlx-audio/.venv311/bin/python"),
    "ref": os.path.join(HOME, "tools/mlx-audio/reference.wav"),
    "model": "mlx-community/Qwen3-TTS-12Hz-0.6B-Base-bf16",
    "stt_model": "mlx-community/whisper-large-v3-turbo-asr-fp16",
}

ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


def die(message: str, code: int = 1) -> "NoReturn":  # noqa: F821
    print(f"narrate: {message}", file=sys.stderr)
    raise SystemExit(code)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: str) -> str | None:
    try:
        with open(path, "rb") as handle:
            return sha256_bytes(handle.read())
    except OSError:
        return None


def wav_duration(path: str) -> float:
    """Duration of a RIFF/WAVE file in seconds, from its header.

    Works for PCM and IEEE-float WAVs without ffprobe: byte_rate is in the fmt
    chunk, and the data chunk length gives the sample bytes.
    """

    with open(path, "rb") as handle:
        header = handle.read(12)
        if len(header) < 12 or header[:4] != b"RIFF" or header[8:12] != b"WAVE":
            die(f"not a WAV file: {path}")
        byte_rate = None
        while True:
            chunk = handle.read(8)
            if len(chunk) < 8:
                break
            chunk_id, size = chunk[:4], struct.unpack("<I", chunk[4:])[0]
            if chunk_id == b"fmt ":
                fmt = handle.read(size)
                if size >= 12:
                    byte_rate = struct.unpack("<I", fmt[8:12])[0]
                if size % 2:
                    handle.seek(1, os.SEEK_CUR)
            elif chunk_id == b"data":
                if not byte_rate:
                    die(f"WAV has no fmt chunk before data: {path}")
                return size / float(byte_rate)
            else:
                handle.seek(size + (size % 2), os.SEEK_CUR)
    die(f"WAV has no data chunk: {path}")


def load_scenes(args: argparse.Namespace) -> list[dict]:
    if args.text_file:
        try:
            text = open(args.text_file, encoding="utf-8").read().strip()
        except OSError as err:
            die(f"cannot read {args.text_file}: {err}")
        if not text:
            die(f"{args.text_file} is empty")
        stem = os.path.splitext(os.path.basename(args.text_file))[0] or "scene"
        return [{"id": stem, "text": text}]

    try:
        with open(args.scenes, encoding="utf-8") as handle:
            data = json.load(handle)
    except OSError as err:
        die(f"cannot read {args.scenes}: {err}")
    except json.JSONDecodeError as err:
        die(f"{args.scenes} is not valid JSON: {err}")
    if isinstance(data, dict) and isinstance(data.get("scenes"), list):
        data = data["scenes"]
    if not isinstance(data, list) or not data:
        die("scenes.json must be a non-empty array of {id, text}")

    scenes, seen = [], set()
    for index, entry in enumerate(data, 1):
        if not isinstance(entry, dict):
            die(f"scene {index} is not an object")
        scene_id = entry.get("id")
        text = entry.get("text")
        if not isinstance(scene_id, str) or not ID_RE.match(scene_id):
            die(f"scene {index} has an unsafe id: {scene_id!r}")
        if scene_id in seen:
            die(f"duplicate scene id: {scene_id}")
        seen.add(scene_id)
        if not isinstance(text, str) or not text.strip():
            die(f"scene {scene_id} has no text")
        scene = {"id": scene_id, "text": text}
        if isinstance(entry.get("words"), list):
            scene["words"] = entry["words"]
        scenes.append(scene)
    return scenes


def build_argv(args: argparse.Namespace, text: str, out_wav: str, ref_text: str | None) -> list[str]:
    if args.tts_json:
        try:
            template = json.loads(args.tts_json)
        except json.JSONDecodeError as err:
            die(f"--tts-json is not valid JSON: {err}")
        if not isinstance(template, list) or not all(isinstance(part, str) for part in template):
            die("--tts-json must be a JSON array of strings")
        values = {
            "{text}": text,
            "{out}": out_wav,
            "{ref}": args.ref,
            "{model}": args.model,
            "{stt_model}": args.stt_model,
        }
        argv = []
        for part in template:
            if "{ref_text}" in part:
                if not ref_text:
                    die("--tts-json uses {ref_text} but no --ref-text-file was given")
                part = part.replace("{ref_text}", ref_text)
            for token, value in values.items():
                part = part.replace(token, value)
            argv.append(part)
        return argv

    argv = [
        args.python,
        "-m",
        "mlx_audio.tts.generate",
        "--model",
        args.model,
        "--ref_audio",
        args.ref,
        "--stt_model",
        args.stt_model,
        "--text",
        text,
    ]
    if ref_text:
        # Confirm against `mlx_audio.tts.generate --help` before relying on it.
        argv += ["--ref_text", ref_text]
    return argv


def cache_key(args: argparse.Namespace, text: str, ref_sha: str | None, ref_text_sha: str | None) -> str:
    payload = {
        "text": text,
        "model": args.model,
        "stt_model": args.stt_model,
        "ref_sha256": ref_sha,
        "ref_text_sha256": ref_text_sha,
    }
    return sha256_bytes(json.dumps(payload, sort_keys=True).encode("utf-8"))


def read_manifest(path: str) -> dict:
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.load(handle)
        if isinstance(data, dict) and isinstance(data.get("scenes"), list):
            return data
    except (OSError, json.JSONDecodeError):
        pass
    return {"version": 1, "scenes": []}


def write_json_atomic(path: str, data: dict) -> None:
    directory = os.path.dirname(os.path.abspath(path))
    os.makedirs(directory, exist_ok=True)
    handle, temp = tempfile.mkstemp(dir=directory, prefix=".manifest-", suffix=".json")
    try:
        with os.fdopen(handle, "w", encoding="utf-8") as out:
            json.dump(data, out, ensure_ascii=False, indent=2)
            out.write("\n")
        os.replace(temp, path)
    except BaseException:
        if os.path.exists(temp):
            os.unlink(temp)
        raise


def find_generated(directory: str) -> str | None:
    candidates = [
        os.path.join(directory, name)
        for name in os.listdir(directory)
        if name.lower().endswith(".wav")
    ]
    if not candidates:
        return None
    candidates.sort(key=lambda path: (os.path.basename(path) != "audio_000.wav", -os.path.getmtime(path)))
    return candidates[0]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--scenes", help="scenes.json: [{id, text}]")
    source.add_argument("--text-file", help="one UTF-8 text file, used as a single scene")
    parser.add_argument("--out", required=True, help="output directory for WAVs and manifest.json")
    parser.add_argument("--python", default=DEFAULTS["python"])
    parser.add_argument("--ref", default=DEFAULTS["ref"], help="voice reference WAV")
    parser.add_argument("--model", default=DEFAULTS["model"])
    parser.add_argument("--stt-model", default=DEFAULTS["stt_model"])
    parser.add_argument("--ref-text-file", help="accurate transcript of the reference audio")
    parser.add_argument("--tts-json", help="override the TTS argv (JSON array); for tests or another engine")
    parser.add_argument("--only", help="comma-separated scene ids to (re)generate")
    parser.add_argument("--force", action="store_true", help="regenerate even when inputs changed")
    parser.add_argument("--dry-run", action="store_true", help="print the plan, generate nothing")
    args = parser.parse_args()

    scenes = load_scenes(args)
    if args.only:
        wanted = {part.strip() for part in args.only.split(",") if part.strip()}
        unknown = wanted - {scene["id"] for scene in scenes}
        if unknown:
            die(f"--only names unknown scenes: {', '.join(sorted(unknown))}")
        scenes = [scene for scene in scenes if scene["id"] in wanted]

    ref_sha = sha256_file(args.ref)
    if args.tts_json is None:
        if not os.path.isfile(args.python):
            die(f"python not found: {args.python} (pass --python)")
        if ref_sha is None:
            die(f"voice reference not found: {args.ref} (pass --ref)")

    ref_text = None
    ref_text_sha = None
    if args.ref_text_file:
        try:
            ref_text = open(args.ref_text_file, encoding="utf-8").read().strip()
        except OSError as err:
            die(f"cannot read {args.ref_text_file}: {err}")
        if not ref_text:
            die(f"{args.ref_text_file} is empty")
        ref_text_sha = sha256_bytes(ref_text.encode("utf-8"))

    os.makedirs(args.out, exist_ok=True)
    manifest_path = os.path.join(args.out, "manifest.json")
    manifest = read_manifest(manifest_path)
    by_id = {entry.get("id"): entry for entry in manifest["scenes"]}

    # Preflight: decide every scene before touching the filesystem, so a conflict
    # never leaves a half-regenerated scene set.
    plan = []
    for scene in scenes:
        scene_id = scene["id"]
        text = scene["text"]
        out_wav = os.path.join(args.out, f"{scene_id}.wav")
        key = cache_key(args, text, ref_sha, ref_text_sha)
        existing = by_id.get(scene_id)
        exists = os.path.isfile(out_wav) and os.path.getsize(out_wav) > 0
        if args.force:
            plan.append((scene, out_wav, key, "generate"))
        elif exists and existing and existing.get("key") == key:
            plan.append((scene, out_wav, key, "reuse"))
        elif exists:
            plan.append((scene, out_wav, key, "conflict"))
        else:
            plan.append((scene, out_wav, key, "generate"))

    conflicts = [scene["id"] for scene, _, _, action in plan if action == "conflict"]
    if conflicts:
        die(
            "refusing to overwrite audio whose inputs changed: "
            + ", ".join(conflicts)
            + " (pass --force, or --only for just those scenes)",
            2,
        )

    results = []
    for scene, out_wav, key, action in plan:
        scene_id = scene["id"]
        if action == "reuse":
            duration = wav_duration(out_wav)
            entry = dict(by_id[scene_id])
            entry["status"] = "reused"
            results.append(entry)
            print(f"reuse    {scene_id}  {duration:.2f}s  {out_wav}")
            continue

        argv = build_argv(args, scene["text"], out_wav, ref_text)
        if args.dry_run:
            print(f"plan     {scene_id}  argv={json.dumps(argv, ensure_ascii=False)}")
            continue

        with tempfile.TemporaryDirectory(prefix=f"tts-{scene_id}-") as work:
            try:
                completed = subprocess.run(argv, cwd=work, capture_output=True, text=True)
            except OSError as err:
                die(f"{scene_id}: cannot run the TTS command: {err}")
            if completed.returncode != 0:
                tail = (completed.stderr or completed.stdout or "").strip()[-2000:]
                die(f"{scene_id}: TTS failed (exit {completed.returncode})\n{tail}")
            produced = find_generated(work)
            if not produced or os.path.getsize(produced) == 0:
                tail = (completed.stderr or completed.stdout or "").strip()[-2000:]
                die(f"{scene_id}: the TTS command produced no non-empty WAV\n{tail}")
            staging = out_wav + ".part"
            shutil.move(produced, staging)
            os.replace(staging, out_wav)

        duration = wav_duration(out_wav)
        if duration <= 0:
            die(f"{scene_id}: generated audio has zero duration")

        entry = {
            "id": scene_id,
            "text": scene["text"],
            "audio": os.path.relpath(out_wav, args.out),
            "duration_seconds": round(duration, 3),
            "key": key,
            "model": args.model,
            "ref_audio": args.ref,
            "ref_sha256": ref_sha,
            "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "status": "generated",
        }
        if "words" in scene:
            entry["words"] = scene["words"]
        results.append(entry)
        print(f"generate {scene_id}  {duration:.2f}s  {out_wav}")

    if args.dry_run:
        return 0

    # Keep manifest order aligned with scenes.json, preserving other entries.
    ordered = []
    result_by_id = {entry["id"]: entry for entry in results}
    for entry in manifest["scenes"]:
        if entry.get("id") in result_by_id:
            ordered.append(result_by_id.pop(entry["id"]))
        else:
            ordered.append(entry)
    ordered.extend(result_by_id.values())
    manifest["scenes"] = ordered
    manifest["voice"] = {
        "model": args.model,
        "stt_model": args.stt_model,
        "ref_audio": args.ref,
        "ref_sha256": ref_sha,
    }
    write_json_atomic(manifest_path, manifest)
    print(f"manifest {manifest_path}  scenes={len(ordered)}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
