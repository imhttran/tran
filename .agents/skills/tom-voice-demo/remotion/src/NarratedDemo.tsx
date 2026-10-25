import {
  AbsoluteFill,
  Audio,
  OffthreadVideo,
  Series,
  interpolate,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

export type Caption = { text: string; from: number; to: number };

export type Scene = {
  id: string;
  /** Relative to the Remotion public directory; null for a silent lead. */
  audio: string | null;
  durationInFrames: number;
  captions: Caption[];
  text: string;
};

export type DemoProps = {
  title: string;
  scenes: Scene[];
  /** Optional screen recording, relative to the Remotion public directory. */
  background?: string | null;
  /** Accent colour for the title plate. */
  accent?: string;
};

const TITLE_PLATE: React.CSSProperties = {
  position: "absolute",
  top: 48,
  left: 56,
  padding: "12px 20px",
  borderRadius: 10,
  background: "rgba(16, 24, 20, 0.72)",
  color: "#f4f1e8",
  fontFamily: "Georgia, 'Times New Roman', serif",
  fontSize: 30,
  letterSpacing: 0.4,
};

const CAPTION_PLATE: React.CSSProperties = {
  position: "absolute",
  left: "50%",
  bottom: 72,
  transform: "translateX(-50%)",
  maxWidth: 1600,
  minWidth: 480,
  padding: "18px 28px",
  borderRadius: 14,
  background: "rgba(12, 18, 15, 0.82)",
  color: "#ffffff",
  fontFamily: "system-ui, -apple-system, 'Segoe UI', sans-serif",
  fontSize: 44,
  lineHeight: 1.3,
  textAlign: "center",
  // Captions must keep Vietnamese diacritics intact; no text-transform tricks.
  whiteSpace: "pre-wrap",
  textWrap: "balance",
};

/** The caption covering this frame, or null between segments. */
function captionAt(captions: Caption[], seconds: number): Caption | null {
  return (
    captions.find((caption) => seconds >= caption.from && seconds < caption.to) ??
    null
  );
}

const SceneView: React.FC<{ scene: Scene }> = ({ scene }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const seconds = frame / fps;
  const caption = captionAt(scene.captions, seconds);

  if (!caption) return null;

  // A short fade keeps the text readable without animating the layout.
  const local = seconds - caption.from;
  const remaining = caption.to - seconds;
  const opacity = Math.min(
    interpolate(local, [0, 0.18], [0, 1], { extrapolateRight: "clamp" }),
    interpolate(remaining, [0, 0.18], [0, 1], { extrapolateLeft: "clamp" }),
  );

  return (
    <div style={{ ...CAPTION_PLATE, opacity }} data-scene={scene.id}>
      {caption.text}
    </div>
  );
};

export const NarratedDemo: React.FC<DemoProps> = ({
  title,
  scenes,
  background,
  accent = "#996d21",
}) => (
  <AbsoluteFill style={{ backgroundColor: "#0e1512" }}>
    {background ? (
      <AbsoluteFill>
        <OffthreadVideo
          src={staticFile(background)}
          muted
          style={{ width: "100%", height: "100%", objectFit: "cover" }}
        />
      </AbsoluteFill>
    ) : (
      <AbsoluteFill
        style={{
          background:
            "radial-gradient(120% 120% at 20% 0%, #1c2a23 0%, #0e1512 60%, #0a0f0d 100%)",
        }}
      />
    )}

    <Series>
      {scenes.map((scene) => (
        <Series.Sequence key={scene.id} durationInFrames={scene.durationInFrames}>
          {scene.audio ? <Audio src={staticFile(scene.audio)} /> : null}
          <SceneView scene={scene} />
        </Series.Sequence>
      ))}
    </Series>

    {title ? (
      <div style={{ ...TITLE_PLATE, borderLeft: `6px solid ${accent}` }}>
        {title}
      </div>
    ) : null}
  </AbsoluteFill>
);
