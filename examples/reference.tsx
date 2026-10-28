import {
  Composition, Camera, Field, Surface, Trace, Markers, Text, Legend, Label, GraphicsPlane,
  ocean, tween,
} from "@lemma/authoring";
import type { Vec2, Vec3, Color } from "@lemma/authoring";

const height = tween(0, 1.55, 1.4, 5.8);
const points: Vec2[] = Array.from({ length: 241 }, (_, i) => {
  const t = i / 240;
  return [-2.8 + 5.6 * t, 0.9 * Math.sin(t * Math.PI * 2)];
});
const samples = [24, 68, 113, 158, 207].map(i => points[i]);
const bright: Color = [0.94, 0.97, 0.99, 1];
const muted: Color = [0.52, 0.62, 0.71, 1];

function Header() {
  return <>
    <Text id="eyebrow" position={[54, 30]} font_size={12} color={[0.37, 0.77, 0.74, 1]}>L E M M A   /   S T U D Y  0 0 1</Text>
    <Text id="title" position={[51, 57]} font_size={37} color={bright}>A field, given form</Text>
    <Text id="equation" position={[54, 108]} font_size={18} color={muted}>(x, z)  →  (x, h · f(x, z), z)</Text>
  </>;
}
export default function Reference() {
  return <Composition duration={8} model={{
    rotation_degrees: tween<Vec3>([0, 0, 0], [0, 12, 0], 2, 6.4),
    scale: tween(1, 0.96, 2, 6.4),
  }}>
    <Camera viewport={[0, 148, 1280, 452]} fov_y_degrees={50}
      eye={tween<Vec3>([0, 10.3, 0.12], [7.5, 6.3, 8.3], 1, 6.4)} target={[0.15, 0, 0.5]} />
    <Field id="field" source={{ kind: "synthetic", resolution: [129, 129], bounds: [-3, 3, -3, 3] }} />
    <Surface id="surface" field="field" height={height} palette={ocean} grid={0.17} />
    <Trace id="trace" field="field" points={points} height={height} lift={0.055} width={3.5}
      color={[0.94, 0.98, 1, 1]} reveal={tween(0, 1, 0.1, 3.8)} />
    <Markers id="samples" field="field" points={samples} height={height} lift={0.12} radius={0.075}
      colors={[[1, 0.53, 0.31, 1], [1, 0.76, 0.35, 1], [0.95, 0.97, 0.99, 1], [0.39, 0.86, 0.83, 1], [0.48, 0.69, 1, 1]]} />
    <Header />
    <Text id="section" position={[54, 616]} font_size={12} color={muted}>SAME SAMPLES. NEW PERSPECTIVE.</Text>
    <Text id="caption" position={[54, 641]} font_size={16} color={[0.76, 0.81, 0.85, 1]}>A flat scalar map becomes a surface as height and camera evolve.</Text>
    <Text id="legend-title" position={[1020, 56]} font_size={11} color={muted}>FIELD VALUE</Text>
    <Legend id="legend" position={[1020, 81]} size={[200, 9]} palette={ocean} />
    <Text id="legend-min" position={[1020, 101]} font_size={12} color={muted}>−0.85</Text>
    <Text id="legend-max" position={[1183, 101]} font_size={12} color={muted}>+0.95</Text>
    <Label id="probe-label" text="sample 03" anchor={{ kind: "field", field: "field", point: samples[2], height, lift: 0.24 }}
      offset={[14, -28]} font_size={15} color={bright} occlusion="hide" />
    <GraphicsPlane id="world-equation" text="h · f(x, z)" position={[0.8, 1.8, -2.6]}
      right={[1, 0, 0]} up={[0, 1, 0]} width={1.7} font_size={60} color={[0.57, 0.86, 0.83, 0.9]} />
  </Composition>;
}
