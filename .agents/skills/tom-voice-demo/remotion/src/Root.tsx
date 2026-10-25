import { Composition } from "remotion";

import { NarratedDemo, type DemoProps } from "./NarratedDemo";

const FPS = 30;

const empty: DemoProps = { title: "", scenes: [] };

export const RemotionRoot: React.FC = () => (
  <Composition
    id="NarratedDemo"
    component={NarratedDemo}
    fps={FPS}
    width={1920}
    height={1080}
    defaultProps={empty}
    durationInFrames={1}
    calculateMetadata={({ props }) => ({
      // Scene durations come from the narration WAVs, never from guesses.
      durationInFrames: Math.max(
        1,
        (props as DemoProps).scenes.reduce(
          (total, scene) => total + scene.durationInFrames,
          0,
        ),
      ),
    })}
  />
);
