---
name: tom-voice-demo
description: Produce narrated demo videos with Tom's cloned voice (MLX Audio + Qwen3-TTS on the Mac) and Remotion. Use when asked to narrate a demo, regenerate a scene's voice-over, add captions that follow narration, time scenes from audio, or rebuild demo.mp4/import.mp4 with the Tom voice.
---

# Tom voice demo

Narrated demo videos with Tom's cloned voice, generated locally with MLX Audio,
composited in Remotion. No paid services, and the voice reference never leaves
the machine.

Two stages, one contract:

```
1. narrate.py     scenes.json -> one WAV per scene + manifest.json   (Mac, MLX)
2. render_demo.mjs manifest.json -> 1080p MP4 with captions          (any OS, Remotion)
```

`scenes.json` is the only input either stage needs:

```json
[{ "id": "01-open", "text": "Welcome to the Tran Family archive." }]
```

`manifest.json` is stage 1's output and stage 2's input: per scene it records the
script, WAV path, real audio duration, and the hashes that decide whether the
audio can be reused.

## Prerequisites (Mac)

| Item | Default path |
| --- | --- |
| Demo project | `~/agentic-workspace/projects/tran-demo` |
| MLX Audio | `~/tools/mlx-audio` |
| Python | `~/tools/mlx-audio/.venv311/bin/python` |
| Voice reference | `~/tools/mlx-audio/reference.wav` |
| Accepted voice test | `~/tools/mlx-audio/audio_000.wav` |
| TTS model | `mlx-community/Qwen3-TTS-12Hz-0.6B-Base-bf16` |
| STT model | `mlx-community/whisper-large-v3-turbo-asr-fp16` |

Override any of them with flags; nothing is hard-coded into the scripts.

## Invocation

Automatic discovery: the skill is named `tom-voice-demo`; ask for it by name.

When discovery is unavailable (for example a Zed session that started before the
skill was installed), attach the instruction file explicitly:

```
@.agents/skills/tom-voice-demo/SKILL.md   narrate the checkout demo
```

From a shell, the two stages are directly callable — that is also the fallback
when the agent cannot run a skill:

```sh
python3 .agents/skills/tom-voice-demo/scripts/narrate.py \
  --scenes demo/scenes.json --out demo/narration

node .agents/skills/tom-voice-demo/scripts/render_demo.mjs \
  --manifest demo/narration/manifest.json --out demo/demo-v2.mp4
```

## Stage 1 — narration (Mac, MLX)

```sh
python3 .agents/skills/tom-voice-demo/scripts/narrate.py \
  --scenes demo/scenes.json \
  --out demo/narration
```

Each scene is generated **sequentially, in its own temporary directory**, then
moved to `<out>/<scene-id>.wav`. The underlying command is:

```sh
/Users/imhttran/tools/mlx-audio/.venv311/bin/python -m mlx_audio.tts.generate \
  --model mlx-community/Qwen3-TTS-12Hz-0.6B-Base-bf16 \
  --ref_audio /Users/imhttran/tools/mlx-audio/reference.wav \
  --stt_model mlx-community/whisper-large-v3-turbo-asr-fp16 \
  --text "<scene text>"
```

Text is passed as a subprocess argv element — never through a shell, never
interpolated.

Behaviour:

- **Reuse**: an existing WAV is reused when the script, model, and reference
  audio hash are unchanged. The manifest records the decision.
- **No silent overwrite**: a WAV whose inputs changed is reported and skipped
  unless `--force` is given.
- **One scene at a time**: `--only 03-ask` regenerates that scene without
  touching any other audio.
- **Durations**: read from the WAV header (byte rate), so no ffprobe is needed.
- **Reference transcript**: if `--ref-text-file <path>` is given, it is passed as
  `--ref_text` (an accurate transcript is better than transcription). Otherwise
  automatic transcription via `--stt_model` is used.
- **Word timestamps**: if a scene entry carries `words` (`[{"word","start","end"}]`,
  seconds), the manifest keeps them and the renderer uses them. Without words,
  captions are timed segments.

Before adding any new MLX flag, run `--help` on the Mac and confirm it exists:

```sh
~/tools/mlx-audio/.venv311/bin/python -m mlx_audio.tts.generate --help
```

### Testing without MLX

`--tts-json` replaces the MLX argv for tests or another engine. It is a JSON
array whose elements are substituted as whole arguments (`{text}`, `{out}`,
`{ref}`, `{model}`, `{stt_model}`, `{ref_text}` — no shell, no partial-string
interpolation):

```sh
python3 narrate.py --scenes scenes.json --out out \
  --tts-json '["/usr/bin/python3","fake_tts.py","{text}","{out}"]'
```

## Stage 2 — Remotion

The skill ships a minimal, self-contained Remotion project (`remotion/`). Use it
directly, or copy `src/NarratedDemo.tsx` into the demo project's composition and
keep the same props.

```sh
node .agents/skills/tom-voice-demo/scripts/render_demo.mjs \
  --manifest demo/narration/manifest.json \
  --out demo/demo-v2.mp4 \
  --title "Tran Family archive"
```

- 1920x1080, 30 fps, H.264 + AAC, readable 44 px captions.
- Scene length = audio duration + `--pause` (default 0.6 s) + `--lead` on the
  first scene, converted to frames from the manifest.
- Captions use `words` when present, otherwise short timed segments split at
  word boundaries and distributed by character count over the scene audio.
- Vietnamese is rendered exactly as written (no ASCII folding, no stripping).
- Captions sit in the lower safe area on a translucent plate; the centre of the
  frame stays clear for the application UI.
- `--background demo/raw.mp4` places a recording behind the captions, muted.
- Audio is copied into the project's `public/audio/` (Remotion `staticFile`) so
  the composition never references absolute paths.
- `--force` is required to overwrite an existing output.

### Existing recordings

`demo.mp4` and `import.mp4` are inputs, not outputs. Never rewrite them. When
both are rebuilt, write `demo-v2.mp4` and `import-v2.mp4`. The renderer refuses to
overwrite any existing file without `--force`.

### Regenerating one scene

1. `narrate.py --scenes … --out … --only 04-ask --force`
2. Re-render. Only that scene's audio changed; the other WAVs are reused.

## Content and quality rules

- Narrate only behaviour visible in the recording. No claims about features that
  are not shown.
- Use synthetic data. Never drive or modify live family records.
- Keep credentials, tokens, cookies, and private source text out of frame and
  out of narration.
- The voice-reference transcript is audio-calibration material, **not** genealogy
  evidence.
- Check pronunciation of Vietnamese names by listening when playback is
  available. If you could not listen, say so explicitly in the report.
- Keep voice samples, private transcripts, recordings, and generated media out of
  Git. Commit only the skill code and instructions.

## Self-check

One runnable check, no fixtures, no framework:

```sh
python3 .agents/skills/tom-voice-demo/scripts/selfcheck.py
```

It generates a WAV through the `--tts-json` seam, asserts duration parsing and
manifest reuse, and fails if any of that breaks. It does not need MLX.

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| `mlx_audio` import error | Wrong Python; use `.venv311/bin/python` from the MLX Audio tree |
| Empty or missing WAV | MLX wrote `audio_000.wav` elsewhere; the script runs it in a temp dir and looks for a non-empty result there |
| `refusing to overwrite` | Inputs changed; pass `--force` for that scene only |
| Captions drift | `manifest.json` durations are stale; re-run stage 1 |
| Remotion cannot find audio | Run `render_demo.mjs` from the skill; it copies audio into `public/audio/` |
| Vietnamese diacritics dropped | Something ASCII-folded the text; captions must use the script verbatim |
