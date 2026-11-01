const FAMILY = "TM PDF Noto Serif CJK";
let fontLoad;
const preparedContexts = new WeakSet();

// PDF.js는 비내장 글꼴을 canvas의 generic family로 대체한다. CSS의
// font-family는 canvas 텍스트에 적용되지 않으므로 이 context에만 fallback을 넣는다.
export async function preparePdfCanvasFonts(context) {
  if (!context || !document.fonts || typeof FontFace === "undefined") return;
  try {
    fontLoad ??= Promise.all([400, 700].map(async (weight) => {
      const url = `${import.meta.env.BASE_URL}fonts/noto-serif-kr-${weight}.woff2`;
      const face = new FontFace(FAMILY, `url("${url}")`, { weight: String(weight) });
      await face.load();
      document.fonts.add(face);
    }));
    await fontLoad;
  } catch (error) {
    fontLoad = undefined;
    console.warn("PDF 대체 글꼴을 불러오지 못했습니다:", error);
    return;
  }
  if (preparedContexts.has(context)) return;
  let prototype = Object.getPrototypeOf(context);
  let descriptor;
  while (prototype && !descriptor) {
    descriptor = Object.getOwnPropertyDescriptor(prototype, "font");
    prototype = Object.getPrototypeOf(prototype);
  }
  if (!descriptor?.get || !descriptor?.set) return;
  Object.defineProperty(context, "font", {
    configurable: true,
    get() { return descriptor.get.call(this); },
    set(value) {
      // PDF.js의 내장 글꼴 이름은 앞에 유지한다. 일반 family 직전에만 Noto를 넣는다.
      const font = value.includes(`"${FAMILY}"`) ? value : value.replace(
        /(^|[,\s])(serif|sans-serif|monospace)$/,
        `$1"${FAMILY}", $2`,
      );
      descriptor.set.call(this, font);
    },
  });
  preparedContexts.add(context);
}
