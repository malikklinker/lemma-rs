import type { Child } from "./index";
export function jsx<P>(type: (props: P) => Child, props: P): Child {
  if (typeof type !== "function") throw new Error("Lemma JSX accepts scene components, not HTML elements");
  return type(props);
}
export const jsxs = jsx;
export const jsxDEV = jsx;
export function Fragment({ children }: { children?: Child }): Child { return children; }
export namespace JSX {
  export type Element = Child;
  export interface ElementChildrenAttribute { children: unknown }
  export interface IntrinsicAttributes { key?: string | number }
  export interface IntrinsicElements {}
}
