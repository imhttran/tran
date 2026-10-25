#!/usr/bin/env node
// Stage 2 of the tom-voice-demo skill: turn a narration manifest into a 1080p
// MP4 with captions that follow the audio.
//
//   node render_demo.mjs --manifest demo/narration/manifest.json \
//        --out demo/demo-v2.mp4 --title "Tran Family archive"
//
// Scene length comes from the WAV duration in the manifest. Captions use word
// timestamps when the manifest has them, otherwise short segments distributed
// over the scene audio. Vietnamese text is never normalized or ASCII-folded.

import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REMOTION_DIR = path.resolve(HERE, "..", "remotion");
const FPS = 30;

function parseArgs(argv) {
  const args = {
    manifest: null,
    out: null,
    title: "",
    background: null,
    pause: 0.6,
    lead: 0,
    force: false,
    install: true,
    accent: "#996d21",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      const value = argv[i + 1];
      if (value === undefined) fail(`${arg} needs a value`);
      i += 1;
      return value;
    };
    switch (arg) {
      case "--manifest": args.manifest = next(); break;
      case "--out": args.out = next(); break;
      case "--title": args.title = next(); break;
      case "--background": args.background = next(); break;
      case "--pause": args.pause = Number(next()); break;
      case "--lead": args.lead = Number(next()); break;
      case "--accent": args.accent = next(); break;
      case "--force": args.force = true; break;
      case "--no-install": args.install = false; break;
      default: fail(`unknown argument: ${arg}`);
    }
  }
  if (!args.manifest) fail("--manifest is required");
  if (!args.out) fail("--out is required");
  if (!Number.isFinite(args.pause) || args.pause < 0) fail("--pause must be >= 0");
  if (!Number.isFinite(args.lead) || args.lead < 0) fail("--lead must be >= 0");
  return args;
}

function fail(message, code = 1) {
  console.error(`render: ${message}`);
  process.exit(code);
}

/** Split text into short caption segments at word boundaries. */
export function segmentText(text, maxChars = 48, maxWords = 9) {
  const words = text.trim().split(/\s+/).filter(Boolean);
  const segments = [];
  let current = [];
  for (const word of words) {
    const candidate = [...current, word];
    const length = candidate.join(" ").length;
    if (current.length && (length > maxChars || candidate.length > maxWords)) {
      segments.push(current.join(" "));
      current = [word];
    } else {
      current = candidate;
    }
  }
  if (current.length) segments.push(current.join(" "));
  return segments;
}

/** Captions from word timestamps (seconds, relative to the scene audio). */
function wordCaptions(words, duration) {
  const clean = words
    .filter((word) => word && typeof word.word === "string")
    .map((word) => ({
      word: word.word,
      start: Math.max(0, Number(word.start) || 0),
      end: Math.min(duration, Number(word.end) || 0),
    }))
    .filter((word) => word.end > word.start);
  const captions = [];
  let group = [];
  const flush = () => {
    if (!group.length) return;
    captions.push({
      text: group.map((word) => word.word).join(" "),
      from: group[0].start,
      to: Math.max(group[group.length - 1].end, group[0].start + 0.4),
    });
    group = [];
  };
  for (const word of clean) {
    const chars = [...group, word].map((item) => item.word).join(" ").length;
    if (group.length && (chars > 48 || group.length >= 9)) flush();
    group.push(word);
  }
  flush();
  return captions;
}

/** Captions by proportional character timing when no word data exists. */
function timedCaptions(text, duration) {
  const segments = segmentText(text);
  if (!segments.length) return [];
  const weights = segments.map((segment) => Math.max(1, segment.length));
  const total = weights.reduce((sum, weight) => sum + weight, 0);
  const start = Math.min(0.15, duration / 4);
  const end = Math.max(start, duration - Math.min(0.2, duration / 4));
  const span = end - start;
  let cursor = start;
  return segments.map((segment, index) => {
    const slice = (span * weights[index]) / total;
    const from = cursor;
    cursor += slice;
    return {
      text: segment,
      from: Number(from.toFixed(3)),
      to: Number((index === segments.length - 1 ? end : cursor).toFixed(3)),
    };
  });
}

function buildScenes(manifest, manifestDir, fps, pause) {
  if (!Array.isArray(manifest.scenes) || manifest.scenes.length === 0) {
    fail("the manifest has no scenes; run narrate.py first");
  }
  return manifest.scenes.map((scene) => {
    const wav = path.resolve(manifestDir, scene.audio ?? `${scene.id}.wav`);
    if (!existsSync(wav) || statSync(wav).size === 0) {
      fail(`missing or empty audio for scene ${scene.id}: ${wav}`);
    }
    const duration = Number(scene.duration_seconds);
    if (!Number.isFinite(duration) || duration <= 0) {
      fail(`scene ${scene.id} has no valid duration_seconds`);
    }
    const captions = Array.isArray(scene.words) && scene.words.length
      ? wordCaptions(scene.words, duration)
      : timedCaptions(scene.text ?? "", duration);
    return {
      id: scene.id,
      text: scene.text ?? "",
      audioSource: wav,
      captions,
      durationInFrames: Math.max(1, Math.round((duration + pause) * fps)),
    };
  });
}

function ensureDependencies(install) {
  const marker = path.join(REMOTION_DIR, "node_modules", "remotion", "package.json");
  if (existsSync(marker)) return;
  if (!install) {
    fail("Remotion dependencies are missing (run npm install in the skill's remotion/ directory)");
  }
  console.log("render: installing Remotion dependencies (first run)");
  execFileSync("npm", ["install", "--no-audit", "--no-fund"], {
    cwd: REMOTION_DIR,
    stdio: "inherit",
  });
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const manifestPath = path.resolve(args.manifest);
  if (!existsSync(manifestPath)) fail(`manifest not found: ${manifestPath}`);
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const manifestDir = path.dirname(manifestPath);

  const scenes = buildScenes(manifest, manifestDir, FPS, args.pause);
  if (!scenes.length) fail("nothing to render");

  const out = path.resolve(args.out);
  if (existsSync(out) && !args.force) {
    const base = path.basename(out);
    const hint = base === "demo.mp4" || base === "import.mp4"
      ? " Keep the original recording; write demo-v2.mp4 / import-v2.mp4 instead."
      : "";
    fail(`refusing to overwrite ${out}.${hint} Pass --force if you mean it.`, 2);
  }
  mkdirSync(path.dirname(out), { recursive: true });

  const publicDir = path.join(REMOTION_DIR, "public");
  const audioDir = path.join(publicDir, "audio");
  mkdirSync(audioDir, { recursive: true });

  const props = {
    title: args.title,
    accent: args.accent,
    background: null,
    scenes: scenes.map((scene) => {
      const audioName = `audio/${path.basename(scene.audioSource)}`;
      copyFileSync(scene.audioSource, path.join(publicDir, audioName));
      return {
        id: scene.id,
        audio: audioName,
        durationInFrames: scene.durationInFrames,
        captions: scene.captions,
        text: scene.text,
      };
    }),
  };

  if (args.lead > 0) {
    // A silent lead keeps captions aligned to the audio that follows it.
    props.scenes.unshift({
      id: "__lead",
      audio: null,
      durationInFrames: Math.max(1, Math.round(args.lead * FPS)),
      captions: [],
      text: "",
    });
  }

  if (args.background) {
    const source = path.resolve(args.background);
    if (!existsSync(source)) fail(`background not found: ${source}`);
    const mediaDir = path.join(publicDir, "media");
    mkdirSync(mediaDir, { recursive: true });
    const name = `media/${path.basename(source)}`;
    copyFileSync(source, path.join(publicDir, name));
    props.background = name;
  }

  ensureDependencies(args.install);

  const propsPath = path.join(REMOTION_DIR, `.props-${process.pid}.json`);
  writeFileSync(propsPath, JSON.stringify(props, null, 2), "utf8");

  const totalFrames = props.scenes.reduce((sum, scene) => sum + scene.durationInFrames, 0);
  console.log(
    `render: ${props.scenes.length} scenes, ${totalFrames} frames ` +
      `(${(totalFrames / FPS).toFixed(1)}s), ${props.scenes.reduce((n, s) => n + s.captions.length, 0)} captions`,
  );

  try {
    execFileSync(
      "npx",
      [
        "--yes",
        "remotion",
        "render",
        "src/index.ts",
        "NarratedDemo",
        out,
        `--props=${propsPath}`,
        "--codec=h264",
        "--crf=18",
        "--audio-bitrate=192k",
        ...(args.force ? ["--overwrite"] : []),
      ],
      { cwd: REMOTION_DIR, stdio: "inherit" },
    );
  } finally {
    try { rmSync(propsPath, { force: true }); } catch { /* ignore */ }
  }

  const size = statSync(out).size;
  console.log(`render: ${out} (${(size / 1024 / 1024).toFixed(1)} MiB)`);
  console.log(JSON.stringify({ out, bytes: size, frames: totalFrames, fps: FPS, scenes: props.scenes.length }));
}

main();
