import { beforeAll, afterAll, vi } from "vitest";

// pdfjs-dist v5가 모듈 로드 시 DOMMatrix를 참조하는데 jsdom에 없음
if (typeof globalThis.DOMMatrix === "undefined") {
  globalThis.DOMMatrix = class DOMMatrix {
    constructor(init) {
      this.a = 1;
      this.b = 0;
      this.c = 0;
      this.d = 1;
      this.e = 0;
      this.f = 0;
      this.m11 = 1;
      this.m12 = 0;
      this.m13 = 0;
      this.m14 = 0;
      this.m21 = 0;
      this.m22 = 1;
      this.m23 = 0;
      this.m24 = 0;
      this.m31 = 0;
      this.m32 = 0;
      this.m33 = 1;
      this.m34 = 0;
      this.m41 = 0;
      this.m42 = 0;
      this.m43 = 0;
      this.m44 = 1;
      this.is2D = true;
      this.isIdentity = true;
    }
    multiply() {
      return new DOMMatrix();
    }
    translate(tx = 0, ty = 0) {
      return new DOMMatrix();
    }
    scale(sx = 1, sy = sx) {
      return new DOMMatrix();
    }
    rotate(angle = 0) {
      return new DOMMatrix();
    }
    inverse() {
      return new DOMMatrix();
    }
    transformPoint(p) {
      return p || { x: 0, y: 0, z: 0, w: 1 };
    }
    toFloat32Array() {
      return new Float32Array(16);
    }
    toFloat64Array() {
      return new Float64Array(16);
    }
    toString() {
      return "matrix(1, 0, 0, 1, 0, 0)";
    }
    static fromMatrix(m) {
      return new DOMMatrix();
    }
    static fromFloat32Array(a) {
      return new DOMMatrix();
    }
    static fromFloat64Array(a) {
      return new DOMMatrix();
    }
  };
}

const originalConsoleLog = console.log;
const originalConsoleError = console.error;
const originalConsoleWarn = console.warn;
let originalGetComputedStyle;

beforeAll(() => {
  console.log = vi.fn();
  console.error = vi.fn();
  console.warn = vi.fn();
  if (typeof window !== "undefined" && window.getComputedStyle) {
    originalGetComputedStyle = window.getComputedStyle;
    window.getComputedStyle = (element, pseudoElt) => {
      const style = originalGetComputedStyle(element, pseudoElt);
      return new Proxy(style, {
        get(target, prop) {
          if (prop === "transitionDuration" || prop === "transitionDelay") {
            const value = target[prop];
            return value && value !== "" ? value : "0s";
          }
          if (prop === "getPropertyValue") {
            return (name) => {
              const value = target.getPropertyValue(name);
              if (
                (name === "transition-duration" ||
                  name === "transition-delay") &&
                (!value || value === "")
              ) {
                return "0s";
              }
              return value;
            };
          }
          const value = target[prop];
          return typeof value === "function" ? value.bind(target) : value;
        },
      });
    };
  }
});

afterAll(() => {
  console.log = originalConsoleLog;
  console.error = originalConsoleError;
  console.warn = originalConsoleWarn;
  if (typeof window !== "undefined" && originalGetComputedStyle) {
    window.getComputedStyle = originalGetComputedStyle;
  }
});
