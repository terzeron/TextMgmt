import { useCallback, useEffect, useRef, useState } from "react";
import { requestInsertPreview } from "./insertPreview";

// 저장 전에 끼워 넣을 이미지들(키 → { url | label, blob, previewSrc })을 들고 있는다. PDF 쪽 삽입과 EPUB 삽화가 함께 쓴다.
// 서버가 정규화한 미리보기(blob URL)를 받아 오고, 쓰지 않게 되면(교체·취소·언마운트) blob URL 을 해제한다.
// onChange 는 목록이 바뀔 때마다 부르며, 호출 쪽이 이전 저장 오류를 지우는 데 쓴다.
export default function usePendingImages({ apiPrefix, bookId, onChange }) {
  const [entries, setEntries] = useState(() => new Map());
  const entriesRef = useRef(entries);
  useEffect(() => {
    entriesRef.current = entries;
  }, [entries]);
  const onChangeRef = useRef(onChange);
  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);
  // 화면을 닫은 뒤 도착한 미리보기 응답은 버린다(받으면 해제할 곳이 없는 blob URL 이 남는다).
  const unmountedRef = useRef(false);
  useEffect(() => {
    unmountedRef.current = false;
    return () => {
      unmountedRef.current = true;
      for (const entry of entriesRef.current.values()) {
        URL.revokeObjectURL(entry.previewSrc);
      }
    };
  }, []);

  // entry 가 null 이면 key 의 이미지를 뺀다.
  const set = useCallback((key, entry) => {
    onChangeRef.current?.();
    const previous = entriesRef.current.get(key);
    if (previous && previous.previewSrc !== entry?.previewSrc) {
      URL.revokeObjectURL(previous.previewSrc);
    }
    setEntries((prev) => {
      const next = new Map(prev);
      if (entry) next.set(key, entry);
      else next.delete(key);
      return next;
    });
  }, []);

  const clear = useCallback(() => {
    for (const entry of entriesRef.current.values()) {
      URL.revokeObjectURL(entry.previewSrc);
    }
    setEntries(new Map());
  }, []);

  // 서버가 이미지(URL 이면 내려받고, 이 기기의 파일이면 받아서)를 정규화한 미리보기를 받아 온다. 저장할 때 들어가는 것과 같은 이미지다.
  // 실패 사유(문자열)를 돌려주고, 성공하면 key 에 이미지를 표시한 뒤 null 을 돌려준다.
  const preview = useCallback(
    async (key, source, meta = {}) => {
      try {
        const blob = await requestInsertPreview(apiPrefix, bookId, source);
        /* v8 ignore next -- the viewer unmounts only while this request is in flight. */
        if (unmountedRef.current) return null;
        set(key, {
          ...(typeof source === "string"
            ? { url: source }
            : { label: source.name }),
          ...meta,
          blob,
          previewSrc: URL.createObjectURL(blob),
        });
        return null;
      } catch (err) {
        return err?.message || "이미지를 가져오지 못했습니다";
      }
    },
    [apiPrefix, bookId, set],
  );

  return { entries, set, clear, preview };
}
