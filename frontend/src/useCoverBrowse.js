/**
 * 책·만화 탭 오른쪽 컬럼의 커버 목록 데이터를 서버에서 커서로 한 페이지씩 이어받는 훅.
 *
 * 전체 책은 수만 건이라 한 번에 받을 수 없다. 제목 사전순으로 정렬된 페이지를
 * 스크롤에 맞춰 이어붙인다. 서버는 epub·pdf만 보내고, 디렉토리에 직접 속한 책은 한 권씩,
 * 그 아래 작품 디렉토리는 대표 한 권(display_title에 디렉토리 이름)으로 묶어 보낸다.
 * 디렉토리를 고르면 같은 규칙을 그 디렉토리 기준으로 적용한다.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import { rawJsonGetReq } from "./Common";

// 한 페이지 크기. 커버 썸네일은 처음 보는 책마다 서버가 파일에서 만들므로 크게 잡지 않는다.
export const BROWSE_PAGE_SIZE = 60;

const LOAD_ERROR_MESSAGE = "책 목록을 불러오지 못했습니다.";

/**
 * category가 없으면 전체, 있으면 그 디렉토리를 기준으로 한 커버 목록 URL을 만든다.
 * uncovered가 true면 정식 표지가 없는 책만 받는다.
 */
export function buildBrowseUrl(apiPrefix, category, cursor, uncovered = false) {
  const params = [`limit=${BROWSE_PAGE_SIZE}`];
  if (category) params.push(`category=${encodeURIComponent(category)}`);
  if (cursor) params.push(`cursor=${encodeURIComponent(cursor)}`);
  if (uncovered) params.push("uncovered=true");
  return `${apiPrefix}/browse?${params.join("&")}`;
}

/**
 * @param {object} options
 * @param {string} options.apiPrefix - "" (book) 또는 "/comics"
 * @param {string} [options.category] - 비우면 전체 책
 * @param {boolean} [options.uncovered] - true면 정식 표지가 없는 책만. 서버가 전체 개수를 몰라 total은 null이다
 */
export function useCoverBrowse({ apiPrefix, category = "", uncovered = false }) {
  const [items, setItems] = useState([]);
  const [total, setTotal] = useState(null);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  // 응답이 늦게 도착해도 카테고리가 바뀐 뒤의 목록을 덮어쓰지 않게 세대 번호로 거른다.
  const generationRef = useRef(0);
  const cursorRef = useRef("");
  const hasMoreRef = useRef(false);
  const loadingRef = useRef(false);

  const fetchPage = useCallback(
    (generation) => {
      loadingRef.current = true;
      setLoading(true);
      setError("");
      const stop = () => {
        hasMoreRef.current = false;
        setHasMore(false);
        setError(LOAD_ERROR_MESSAGE);
      };
      rawJsonGetReq(
        buildBrowseUrl(apiPrefix, category, cursorRef.current, uncovered),
        (data) => {
          if (generation !== generationRef.current) return;
          if (!data || data["status"] !== "success") {
            // 실패한 채 계속 다음 페이지를 부르면 요청이 끝없이 반복되므로 멈추고 재시도를 맡긴다.
            stop();
            return;
          }
          const nextCursor = data["next_cursor"] || "";
          cursorRef.current = nextCursor;
          hasMoreRef.current = Boolean(nextCursor);
          setHasMore(Boolean(nextCursor));
          setTotal(data["total"] ?? null);
          setItems((prev) => prev.concat(data["result"] || []));
        },
        () => {
          if (generation !== generationRef.current) return;
          stop();
        },
        () => {
          if (generation !== generationRef.current) return;
          loadingRef.current = false;
          setLoading(false);
        },
      );
    },
    [apiPrefix, category, uncovered],
  );

  const reload = useCallback(() => {
    generationRef.current += 1;
    cursorRef.current = "";
    hasMoreRef.current = false;
    loadingRef.current = false;
    setItems([]);
    setTotal(null);
    setHasMore(false);
    fetchPage(generationRef.current);
  }, [fetchPage]);

  useEffect(() => {
    reload();
    return () => {
      // 언마운트 뒤에 도착하는 응답이 상태를 건드리지 않게 세대를 넘겨 둔다.
      generationRef.current += 1;
    };
  }, [reload]);

  const loadMore = useCallback(() => {
    if (loadingRef.current || !hasMoreRef.current) return;
    fetchPage(generationRef.current);
  }, [fetchPage]);

  return { items, total, hasMore, loading, error, loadMore, reload };
}
