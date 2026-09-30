import { useState } from "react";

export const FONT_FAMILIES = [
  { label: "기본", value: "" },
  { label: "나눔고딕", value: "'Nanum Gothic', sans-serif" },
  {
    label: "나눔바른고딕",
    value: "'Nanum Barun Gothic', 'NanumBarunGothic', sans-serif",
  },
  {
    label: "나눔명조",
    value: "'Nanum Myeongjo', 'Noto Serif CJK KR', serif",
  },
  {
    label: "함초롬바탕",
    value: "'HCR Batang', '함초롬바탕', 'Noto Serif CJK KR', serif",
  },
  { label: "Noto Serif CJK", value: "'Noto Serif CJK KR', serif" },
  { label: "Noto Sans CJK", value: "'Noto Sans CJK KR', sans-serif" },
  { label: "KoPub 바탕체", value: "'KoPub Batang', 'KoPub바탕체', serif" },
];

const LEGACY_FONT_VALUES = new Map([
  [
    "'Nanum Myeongjo', serif",
    "'Nanum Myeongjo', 'Noto Serif CJK KR', serif",
  ],
  [
    "'HCR Batang', '함초롬바탕', serif",
    "'HCR Batang', '함초롬바탕', 'Noto Serif CJK KR', serif",
  ],
]);

export const readReaderFontFamily = () => {
  const stored =
    localStorage.getItem("reader_fontFamily") ??
    localStorage.getItem("epub_fontFamily") ??
    "";
  const saved = LEGACY_FONT_VALUES.get(stored) ?? stored;
  return FONT_FAMILIES.some((font) => font.value === saved) ? saved : "";
};

export const saveReaderFontFamily = (value) => {
  localStorage.setItem("reader_fontFamily", value);
  localStorage.setItem("epub_fontFamily", value);
};

export function useReaderFontFamily() {
  const [fontFamily, setFontFamily] = useState(readReaderFontFamily);

  const handleFontFamilyChange = (value) => {
    setFontFamily(value);
    saveReaderFontFamily(value);
  };

  return [fontFamily, handleFontFamilyChange];
}

const KOREAN_UNICODE_RANGE =
  "U+AC00-D7AF, U+1100-11FF, U+3130-318F, U+A960-A97F, U+D7B0-D7FF, " +
  "U+3000-303F, U+FF00-FFEF, U+25A0-25FF, U+203B, U+327E-327F";
const LATIN_UNICODE_RANGE =
  "U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, " +
  "U+02DC, U+2000-206F, U+2074, U+20AC, U+2122, U+2191, U+2193, " +
  "U+2212, U+2215, U+FEFF, U+FFFD";
const KOREAN_HANJA_UNICODE_RANGE =
  `${KOREAN_UNICODE_RANGE}, U+3400-4DBF, U+4E00-9FFF, U+F900-FAFF`;
const KOREAN_HANJA_LATIN_UNICODE_RANGE =
  `${KOREAN_HANJA_UNICODE_RANGE}, ${LATIN_UNICODE_RANGE}`;

const FONT_FACE_FAMILIES = [
  {
    family: "Nanum Gothic",
    file: "nanum-gothic",
    weights: [400, 700],
    subsets: [
      ["korean", KOREAN_UNICODE_RANGE],
      ["latin", LATIN_UNICODE_RANGE],
    ],
  },
  {
    family: "Nanum Myeongjo",
    file: "nanum-myeongjo",
    weights: [400, 700],
    subsets: [
      ["korean", KOREAN_UNICODE_RANGE],
      ["latin", LATIN_UNICODE_RANGE],
    ],
  },
  {
    family: "Noto Serif CJK KR",
    faces: [
      { file: "noto-serif-kr-400.woff2", weight: 400 },
      { file: "noto-serif-kr-700.woff2", weight: 700 },
    ],
    unicodeRange: KOREAN_HANJA_LATIN_UNICODE_RANGE,
  },
  {
    family: "Noto Sans CJK KR",
    faces: [
      { file: "noto-sans-kr-400.woff2", weight: 400 },
      { file: "noto-sans-kr-700.woff2", weight: 700 },
    ],
    unicodeRange: KOREAN_HANJA_LATIN_UNICODE_RANGE,
  },
  {
    family: "Nanum Barun Gothic",
    faces: [
      { file: "NanumBarunGothicUltraLightSubset.woff2", weight: 200 },
      { file: "NanumBarunGothicLightSubset.woff2", weight: 300 },
      { file: "NanumBarunGothicSubset.woff2", weight: 400 },
      { file: "NanumBarunGothicBoldSubset.woff2", weight: 700 },
    ],
    unicodeRange: KOREAN_HANJA_LATIN_UNICODE_RANGE,
  },
  {
    family: "KoPub Batang",
    faces: [{ file: "KoPubBatang-Medium.ttf", weight: 400 }],
    unicodeRange: KOREAN_HANJA_LATIN_UNICODE_RANGE,
  },
];

export function buildFontFaceCss(baseUrl) {
  const face = (family, source, weight, range) =>
    `@font-face { font-family: '${family}'; font-style: normal; ` +
    `font-weight: ${weight}; font-display: swap; src: ${source}; ` +
    `unicode-range: ${range}; }`;
  const rules = [];

  for (const font of FONT_FACE_FAMILIES) {
    if (font.subsets) {
      for (const weight of font.weights) {
        for (const [subset, range] of font.subsets) {
          rules.push(
            face(
              font.family,
              `url(${baseUrl}${font.file}-${subset}-${weight}.woff2) format('woff2')`,
              weight,
              range,
            ),
          );
        }
      }
      continue;
    }
    for (const entry of font.faces) {
      const format = entry.file.endsWith(".ttf") ? "truetype" : "woff2";
      rules.push(
        face(
          font.family,
          `url(${baseUrl}${entry.file}) format('${format}')`,
          entry.weight,
          font.unicodeRange,
        ),
      );
    }
  }
  return rules.join("\n");
}
