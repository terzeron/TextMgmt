/**
 * PDF 뷰어 진단 기록.
 * 모바일 브라우저가 메모리 부족으로 탭을 죽이면 그 순간에는 서버로 아무것도 보낼 수 없다.
 * 그래서 마지막 상태를 localStorage 에 계속 덮어 쓰고, 다음에 뷰어가 열릴 때
 * "정상 종료(clean)가 아니었던" 기록이 남아 있으면 그때 서버로 보낸다.
 */
import { reportClientError } from "./clientLogger";

const STORAGE_KEY = "tm.pdfTrace";
// 죽기 직전의 흐름을 보려는 것이라 최근 이벤트만 남긴다. 서버가 stack 을 5000자에서 자르므로 그 안에 들어가게 잡는다.
const MAX_EVENTS = 24;

let record = null;

function readStored() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

function writeStored() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(record));
  } catch {
    // 저장소를 못 쓰는 환경(사생활 보호 모드 등)에서는 기록만 포기한다
  }
}

function formatStats(stats) {
  return Object.entries(stats)
    .map(([key, value]) => `${key}=${value}`)
    .join(" ");
}

/**
 * 뷰어가 열릴 때 호출한다. 이전 뷰어가 정상 종료하지 않았다면 그 기록을 서버로 보낸 뒤 새 기록을 시작한다.
 * @returns {boolean} 이전 비정상 종료 기록을 보냈는지 여부
 */
export function startPdfTrace({ bookId }) {
  const previous = readStored();
  const unclean = Boolean(previous) && previous.clean === false;
  if (unclean) {
    reportClientError({
      errorType: "CUSTOM_ERROR",
      message: `[PDF_TRACE] 이전 PDF 뷰어가 정상 종료되지 않음: book=${previous.bookId} last=${previous.last ?? ""}`,
      stack: (previous.events || []).join("\n"),
    });
  }
  record = { bookId, started: Date.now(), clean: false, last: "", events: [] };
  writeStored();
  return unclean;
}

/** 이벤트와 그 시점의 상태를 기록한다. 매번 localStorage 에 써서 탭이 죽어도 마지막 값이 남는다. */
export function tracePdf(event, stats = {}) {
  if (!record) return;
  const visibility =
    typeof document !== "undefined" && document.visibilityState === "hidden"
      ? "hidden"
      : "visible";
  const line = `+${Date.now() - record.started}ms ${event} ${formatStats({ ...stats, vis: visibility })}`;
  record.last = line;
  record.events.push(line);
  if (record.events.length > MAX_EVENTS) record.events.shift();
  writeStored();
}

/** 뷰어가 정상적으로 닫힐 때(언마운트, pagehide) 호출한다. */
export function endPdfTrace() {
  if (!record) return;
  record.clean = true;
  writeStored();
  record = null;
}
