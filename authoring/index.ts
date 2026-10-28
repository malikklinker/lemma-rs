/** Scene construction runs once on load. Tracks are evaluated by Rust per sample. */
export type Vec2 = [number, number];
export type Vec3 = [number, number, number];
export type Color = [number, number, number, number];
export type Ease = "linear" | "smooth" | "cubic-out" | "hold";
export interface Keyframe<T> { time: number; value: T; ease?: Ease }
export type Animated<T> = T | { keyframes: Keyframe<T>[] };
export type Rate = number | { numerator: number; denominator: number };
export type FieldSource =
  | { kind: "synthetic"; resolution: Vec2; bounds: [number, number, number, number] }
  | { kind: "saved"; manifest: string };
export interface Palette {
  stops: [Color, Color, Color, Color, Color];
  range: Vec2;
  shift?: Animated<number>;
}
export interface CameraSpec {
  eye: Animated<Vec3>; target: Animated<Vec3>; up: Vec3;
  fov_y_degrees: number; near: number; far: number;
  viewport?: [number, number, number, number];
}
/** One transform for world geometry and anchors. Screen content stays fixed.
 * Defaults are zero translation/rotation and scale 1. Scale must stay positive.
 * XYZ Euler degrees use Rx * Ry * Rz; column vectors rotate Z, then Y, then X.
 */
export interface TransformSpec {
  translation?: Animated<Vec3>;
  rotation_degrees?: Animated<Vec3>;
  scale?: Animated<number>;
}
export interface SceneSpec {
  version: 1; size: Vec2; fps: { numerator: number; denominator: number };
  duration: number; background: Color; camera: CameraSpec;
  model?: TransformSpec;
  assets: { id: string; source: FieldSource }[]; nodes: NodeSpec[];
}
export type Anchor =
  | { kind: "world"; position: Animated<Vec3> }
  | { kind: "field"; field: string; point: Vec2; height: Animated<number>; lift?: number };
export interface SurfaceProps { id: string; field: string; height: Animated<number>; palette: Palette; grid?: number }
export interface TraceProps { id: string; field: string; points: Vec2[]; height: Animated<number>; lift?: number; width: number; color: Color; reveal?: Animated<number> }
export interface PolylineProps { id: string; points: Vec3[]; width: number; color: Color; reveal?: Animated<number> }
export interface MarkersProps { id: string; field: string; points: Vec2[]; height: Animated<number>; lift?: number; radius: number; colors: Color[] }
export interface TextProps { id: string; text?: string; children?: string | number; position: Vec2; font_size: number; color?: Color; opacity?: Animated<number> }
export interface LabelProps { id: string; text: string; anchor: Anchor; offset: Vec2; font_size: number; color?: Color; occlusion?: "always" | "hide" }
export interface LegendProps { id: string; position: Vec2; size: Vec2; palette: Palette }
export interface GraphicsPlaneProps { id: string; text: string; position: Animated<Vec3>; right: Vec3; up: Vec3; width: number; font_size: number; color?: Color }
export type NodeSpec =
  | ({ kind: "surface" } & SurfaceProps) | ({ kind: "trace" } & TraceProps)
  | ({ kind: "polyline" } & PolylineProps) | ({ kind: "markers" } & MarkersProps)
  | ({ kind: "text"; text: string } & Omit<TextProps, "text" | "children">)
  | ({ kind: "label" } & LabelProps) | ({ kind: "legend" } & LegendProps)
  | ({ kind: "graphics-plane" } & GraphicsPlaneProps);
export type AuthorElement =
  | { category: "camera"; spec: CameraSpec }
  | { category: "asset"; spec: { id: string; source: FieldSource } }
  | { category: "node"; spec: NodeSpec };
export type Child = AuthorElement | SceneSpec | Child[] | null | undefined | false;
export interface CompositionProps { width?: number; height?: number; fps?: Rate; duration: number; background?: Color; model?: TransformSpec; children?: Child }

export function tween<T extends number | Vec3 | Color>(from: T, to: T, start: number, end: number, ease: Ease = "smooth"): Animated<T> {
  if (!(Number.isFinite(start) && Number.isFinite(end) && start >= 0 && end > start)) throw new Error("tween requires 0 <= start < end");
  return { keyframes: [{ time: start, value: from, ease }, { time: end, value: to }] };
}
export function keyframes<T>(...keys: Keyframe<T>[]): Animated<T> { return { keyframes: keys }; }
export function flatten(children: Child): (AuthorElement | SceneSpec)[] {
  if (Array.isArray(children)) return children.flatMap(flatten);
  if (children === undefined || children === null || children === false) return [];
  if (typeof children !== "object") throw new Error("Composition children must be scene components; put text inside a Text component");
  return [children];
}
export function Composition({ width = 1280, height = 720, fps = 60, duration, background = [0.027, 0.047, 0.075, 1], model, children }: CompositionProps): SceneSpec {
  const items = flatten(children);
  for (const item of items) {
    if ("version" in item) throw new Error("Nested Compositions are not supported");
    const { category, spec } = item;
    if (!["camera", "asset", "node"].includes(category) || !spec || typeof spec !== "object" || Array.isArray(spec)) {
      const id = spec && typeof spec === "object" && "id" in spec ? `${spec.id}: ` : "";
      throw new Error(`${id}unsupported Composition child; return a scene component or Fragment`);
    }
  }
  const cameras = items.filter((x): x is Extract<AuthorElement, { category: "camera" }> => "category" in x && x.category === "camera");
  if (cameras.length !== 1) throw new Error(`Composition requires exactly one Camera; received ${cameras.length}`);
  return {
    version: 1, size: [width, height], fps: typeof fps === "number" ? { numerator: fps, denominator: 1 } : fps,
    duration, background, camera: cameras[0].spec, ...(model === undefined ? {} : { model }),
    assets: items.flatMap(x => "category" in x && x.category === "asset" ? [x.spec] : []),
    nodes: items.flatMap(x => "category" in x && x.category === "node" ? [x.spec] : []),
  };
}
export function Camera(props: Pick<CameraSpec, "eye" | "target"> & Partial<Omit<CameraSpec, "eye" | "target">>): AuthorElement {
  return { category: "camera", spec: { up: [0, 1, 0], fov_y_degrees: 39, near: 0.1, far: 100, ...props } };
}
export function Field(props: { id: string; source: FieldSource }): AuthorElement { return { category: "asset", spec: props }; }
export function Surface(props: SurfaceProps): AuthorElement { return { category: "node", spec: { kind: "surface", ...props } }; }
export function Trace(props: TraceProps): AuthorElement { return { category: "node", spec: { kind: "trace", ...props } }; }
export function Polyline(props: PolylineProps): AuthorElement { return { category: "node", spec: { kind: "polyline", ...props } }; }
export function Markers(props: MarkersProps): AuthorElement { return { category: "node", spec: { kind: "markers", ...props } }; }
export function Text({ children, text, ...props }: TextProps): AuthorElement {
  if (children !== undefined && text !== undefined) throw new Error(`${props.id}: provide text or children, not both`);
  if (text === undefined && typeof children !== "string" && typeof children !== "number") throw new Error(`${props.id}: Text needs a string`);
  return { category: "node", spec: { kind: "text", ...props, text: text ?? String(children) } };
}
export function Label(props: LabelProps): AuthorElement { return { category: "node", spec: { kind: "label", ...props } }; }
export function Legend(props: LegendProps): AuthorElement { return { category: "node", spec: { kind: "legend", ...props } }; }
export function GraphicsPlane(props: GraphicsPlaneProps): AuthorElement { return { category: "node", spec: { kind: "graphics-plane", ...props } }; }
export const ocean: Palette = {
  stops: [[0.08, 0.17, 0.31, 1], [0.12, 0.43, 0.56, 1], [0.18, 0.72, 0.69, 1], [0.75, 0.86, 0.49, 1], [1, 0.72, 0.34, 1]],
  range: [-0.85, 0.95],
};
