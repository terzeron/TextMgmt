/* eslint-disable react-refresh/only-export-components --
   서점 카테고리 파싱 유틸이 컴포넌트와 co-located. HMR 힌트일 뿐 런타임 영향 없음. */
import { useEffect, useRef, useState } from "react";
import PropTypes from "prop-types";

import "./Edit.css";
import "bootstrap/dist/css/bootstrap.min.css";

import { Button, Spinner, Card } from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faSpinner } from "@fortawesome/free-solid-svg-icons";
import { rawJsonGetReq } from "./Common";

// 카테고리에서 최하위 + 바로 상위 두 단계를 추출 (공백으로 연결)
// 예: "소설/시/희곡 > SF > 한국SF" → "SF 한국SF"
// 예: "소설/시/희곡 > 중국소설" → "소설/시/희곡 중국소설"
// 예: "한국SF" → "한국SF"
export const getTwoLevelCategory = (category) => {
  if (!category) return "";
  const parts = category
    .split(">")
    .map((s) => s.trim())
    .filter(Boolean);
  if (parts.length <= 1) return parts[0] || "";
  return `${parts[parts.length - 2]} ${parts[parts.length - 1]}`;
};

// 다중 카테고리 경로(" || "로 구분)에서 각 경로의 마지막 두 단계를 추출
// 예: "소설 > 한국소설 || 소설 > 추리/미스터리/스릴러" → ["소설 한국소설", "소설 추리/미스터리/스릴러"]
export const extractMultiPathCategories = (category) => {
  if (!category) return [];
  return category
    .split("||")
    .map((p) => getTwoLevelCategory(p.trim()))
    .filter(Boolean);
};

// 만화 디렉터리 이름 뒤에 붙는 괄호·권수·화수·완결 표기를 잘라 작품명만 남긴다.
// 서점은 이 꼬리가 붙은 키워드를 0건으로 돌려준다.
// 첫 괄호 앞에서 먼저 끊고, 괄호 없는 꼬리는 표기 패턴으로 끊는다.
// 예: "개구리 인간 (1-34화)" → "개구리 인간"
// 예: "개구리 인간 1~47화[완결]" → "개구리 인간"
const COMIC_META_START =
  /(?:\s+|\s*[([])(?=\d{1,4}\s*(?:[-~]\s*\d{1,4})?\s*(?:권|화|편|장)|(?:완결|미완|연재|누락|잘림|미수록))/;
export const stripComicDirectoryMeta = (name) => {
  if (!name) return "";
  const trimmed = name.trim();
  // 이름이 괄호나 표기로 시작하면 자르면 빈 문자열이 된다. 그때는 원본을 쓴다.
  const parenIndex = trimmed.search(/[(（]/);
  const beforeParen = parenIndex > 0 ? trimmed.slice(0, parenIndex) : trimmed;
  const match = COMIC_META_START.exec(beforeParen);
  const stripped = (match ? beforeParen.slice(0, match.index) : beforeParen).trim();
  return stripped || trimmed;
};

// 만화 디렉터리 이름에 붙은 저자 표기를 떼고 작품명만 남긴다.
// "[저자] 제목", "(저자) 제목", "제목 [저자]", "제목 @ 저자"를 처리한다.
// "-"와 "_"는 구분자로 보지 않는다. "제목 1-34화", "1_장르"를 잘못 자르기 때문이다.
// 떼고 나면 비는 이름은 원본을 쓴다.
export const stripComicDirectoryAuthor = (name) => {
  if (!name) return "";
  const trimmed = name.trim();
  const stripped = trimmed
    .replace(/^\[[^\]]*\]\s*/, "")
    .replace(/^\([^)]*\)\s*/, "")
    .replace(/\s*\[[^\]]*\]$/, "")
    .replace(/\s*@.*$/, "")
    .trim();
  return stripped || trimmed;
};

const toComicSearchTitle = (name) =>
  stripComicDirectoryMeta(stripComicDirectoryAuthor(name));

// 검색 결과에서 카테고리를 수집하여 categories 객체에 추가
const collectStoreCategories = (storeData, storeKey, categories, searchTitle, comic) => {
  if (storeData?.status === "success" && storeData?.result?.length > 0) {
    storeData.result.forEach((item, idx) => {
      if (storeKey === "naverwebtoon" || storeKey === "kakaowebtoon") {
        const title = searchTitle?.trim();
        if (title && item.title?.includes(title)) {
          categories[`${storeKey}_${idx}_0`] = item.category?.includes("로맨스") ? "여성향" : "웹툰";
        }
        return;
      }
      if (comic) return;
      const cats = extractMultiPathCategories(item.category);
      cats.forEach((cat, pathIdx) => {
        categories[`${storeKey}_${idx}_${pathIdx}`] = cat;
      });
    });
  }
};

const combineSearchResults = (titleResult, authorTitleResult) => {
  const candidates = [
    ...(titleResult?.result || []).slice(0, 2),
    ...(authorTitleResult?.result || []).slice(0, 2),
  ];
  const seen = new Set();
  const results = candidates.filter((book) => {
    const key = book.book_url || `${book.title || ""}_${book.author || ""}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  }).slice(0, 4);
  if (!results.length && titleResult?.error && (authorTitleResult?.error || !authorTitleResult)) {
    return titleResult;
  }
  return {
    ...authorTitleResult,
    ...titleResult,
    status: results.length ? "success" : "not_found",
    result: results,
  };
};

const requestSearch = (store, search) => {
  const params = new URLSearchParams(search);
  return new Promise((resolve) => {
    rawJsonGetReq(
      `/search/bookstore/${store}?${params.toString()}`,
      resolve,
      (error) => {
        console.error(error);
        resolve({ error: true, message: "검색 중 오류가 발생했습니다." });
      },
    );
  });
};

const buildStoreSearchUrl = (store, title, author) => {
  const encodedTitle = encodeURIComponent(title || "");
  switch (store) {
    case "munpia": {
      const keyword = [author, title].filter(Boolean).join(" ");
      return keyword
        ? `https://novel.munpia.com/page/hd.platinum/view/search/keyword/${encodeURIComponent(keyword)}/order/search_result`
        : "";
    }
    case "naverseries": {
      const keyword = [title, author].filter(Boolean).join(" ");
      return keyword
        ? `https://series.naver.com/search/search.series?t=all&q=${encodeURIComponent(keyword)}`
        : "";
    }
    case "joara":
      return title
        ? `https://www.joara.com/search?target=subject&word=${encodedTitle}&search=`
        : "";
    case "naverwebtoon":
      return title
        ? `https://comic.naver.com/search?keyword=${encodedTitle}`
        : "";
    case "kakaowebtoon":
      return title
        ? `https://webtoon.kakao.com/search?keyword=${encodedTitle}`
        : "";
    default:
      return "";
  }
};

// 카테고리 유사도 판정에 참여하는 서점 목록
// 교보는 "국내도서 > 소설 > 한국소설" 처럼 다른 두 곳과 같은 모양의 분류 경로를 준다.
const CATEGORY_STORES = ["yes24", "aladin", "kyobo", "naver", "naverwebtoon", "kakaowebtoon"];

const collectSearchCategories = (results, title, comic) => {
  const categories = {};
  for (const store of CATEGORY_STORES) {
    collectStoreCategories(results[store], store, categories, title, comic);
  }
  if (Object.values(categories).includes("여성향")) {
    for (const key of Object.keys(categories)) {
      if (categories[key] === "웹툰") delete categories[key];
    }
  }
  return categories;
};

// 책 모드의 자동 검색 대상 서점 목록. 나머지 서점은 버튼을 눌러야 검색한다.
// 웹툰은 책 모드에서 수동 검색할 때만 추천에 참여한다.
const AUTO_SEARCH_STORES = ["yes24", "aladin", "kyobo", "naver"];

// 만화 모드의 자동 검색 대상 서점 목록. 제목이 일치한 웹툰만 추천에 참여한다.
const COMIC_AUTO_SEARCH_STORES = [
  "yes24",
  "aladin",
  "naverwebtoon",
  "kakaowebtoon",
];

// 만화 모드에서 보여주지 않는 서점
const COMIC_HIDDEN_STORES = ["naver", "munpia", "naverseries", "joara"];

// 서점 목록 정의 (supportsIsbn: ISBN 검색 지원 여부, hideIsbn: ISBN 버튼 숨김)
const STORES = [
  { key: "yes24", label: "Yes24", supportsIsbn: true },
  { key: "aladin", label: "알라딘", supportsIsbn: true },
  { key: "kyobo", label: "교보문고", supportsIsbn: true },
  { key: "naver", label: "네이버쇼핑", supportsIsbn: false },
  { key: "ridi", label: "RIDI", supportsIsbn: false },
  { key: "munpia", label: "문피아", supportsIsbn: false },
  { key: "naverseries", label: "시리즈", supportsIsbn: false },
  // 조아라는 웹소설 연재처라 ISBN 이 없다. 제목으로만 찾는다.
  { key: "joara", label: "조아라", supportsIsbn: false },
  // 웹툰도 ISBN 이 없다. 제목으로만 찾으므로 ISBN 버튼을 숨긴다.
  { key: "naverwebtoon", label: "네이버웹툰", supportsIsbn: false, hideIsbn: true },
  { key: "kakaowebtoon", label: "카카오웹툰", supportsIsbn: false, hideIsbn: true },
];

export default function Bookstore(props) {
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [isbn, setIsbn] = useState("");
  const [data, setData] = useState({});
  const categoryResults = useRef({});
  // 서점별로 마지막에 수행한 검색 방법 ("isbn" | "title_author"). 토글 버튼의 선택 상태가 된다.
  const [methods, setMethods] = useState({});
  const visibleStores = props.comic
    ? STORES.filter((s) => !COMIC_HIDDEN_STORES.includes(s.key))
    : STORES;
  const onCategoriesFound = props.onCategoriesFound;
  const reportStoreCategories = (store, result, searchTitle) => {
    categoryResults.current = { ...categoryResults.current, [store]: result };
    if (CATEGORY_STORES.includes(store) && onCategoriesFound) {
      onCategoriesFound(collectSearchCategories(categoryResults.current, searchTitle, props.comic));
    }
  };

  // bookInfo 변경 시 로컬 필드만 동기화 (검색은 트리거하지 않음)
  useEffect(() => {
    const rawTitle = props.bookInfo?.title || "";
    setTitle(props.comic && !props.titleParsed ? toComicSearchTitle(rawTitle) : rawTitle);
    // 만화 모드는 제목만으로 검색한다. 저자를 넣으면 서점이 0건을 돌려주는 경우가 있다.
    setAuthor(props.comic ? "" : props.bookInfo?.author || "");
    setIsbn(props.comic ? "" : props.bookInfo?.isbn || "");
  }, [props.bookInfo, props.comic, props.titleParsed]);

  // 책 정보 로딩 또는 이름 변경 시에만 자동 검색 실행
  useEffect(() => {
    if (!props.searchTrigger) return; // 초기 마운트 시 스킵
    let cancelled = false;

    // 탭 및 데이터 초기화
    setData({});
    categoryResults.current = {};
    setMethods({});
    if (onCategoriesFound) {
      onCategoriesFound({});
    }

    // 자동 검색: ISBN → 저자+제목 → 제목 순으로 시도
    const autoSearch = async (store) => {
      const currentIsbn = props.comic ? "" : props.bookInfo.isbn || "";
      const rawTitle = props.bookInfo.title || "";
      const currentTitle = props.comic && !props.titleParsed
        ? toComicSearchTitle(rawTitle)
        : rawTitle;
      const currentAuthor = props.comic ? "" : props.bookInfo.author || "";

      if (!currentIsbn && !currentTitle && !currentAuthor) return null;

      if (!currentIsbn && currentTitle) {
        setMethods((prev) => ({ ...prev, [store]: "title_author" }));
        setData((prev) => ({ ...prev, [store]: { loading: true } }));
        const [titleResult, authorTitleResult] = await Promise.all([
          requestSearch(store, { title: currentTitle }),
          currentAuthor
            ? requestSearch(store, {
                title: currentTitle,
                author: currentAuthor,
              })
            : null,
        ]);
        const result = combineSearchResults(titleResult, authorTitleResult);
        setData((prev) => ({ ...prev, [store]: result }));
        return result;
      }

      // 1. ISBN 검색 시도 (ISBN이 있는 경우)
      if (currentIsbn) {
        setMethods((prev) => ({ ...prev, [store]: "isbn" }));
        const result = await fetchWithMethodInternal(
          store,
          "isbn",
          currentIsbn,
          currentTitle,
          currentAuthor,
        );
        if (result?.status === "success" && result?.result?.length > 0) {
          return result;
        }
      }

      // 2. 저자+제목 검색 시도
      if (currentTitle || currentAuthor) {
        setMethods((prev) => ({ ...prev, [store]: "title_author" }));
        const result = await fetchWithMethodInternal(
          store,
          "title_author",
          currentIsbn,
          currentTitle,
          currentAuthor,
        );
        if (result?.status === "success" && result?.result?.length > 0) {
          return result;
        }
      }

      // 3. 제목만으로 검색 시도 (저자+제목으로 결과가 없는 경우)
      if (currentTitle) {
        return await fetchWithMethodInternal(
          store,
          "title_only",
          currentIsbn,
          currentTitle,
          currentAuthor,
        );
      }

      return null;
    };

    const runAutoSearch = async () => {
      // 서점 검색을 병렬 실행 (서점 간 의존성 없음)
      const autoStores = props.comic
        ? COMIC_AUTO_SEARCH_STORES
        : AUTO_SEARCH_STORES;
      const entries = await Promise.all(
        autoStores.map(async (storeKey) => [
          storeKey,
          await autoSearch(storeKey),
        ]),
      );
      const results = Object.fromEntries(entries);
      if (cancelled) return;
      categoryResults.current = results;

      // 카테고리 유사도 판정 서점의 결과만 수집하여 부모에게 전달
      if (onCategoriesFound) {
        const searchTitle = props.comic && !props.titleParsed
          ? toComicSearchTitle(props.bookInfo.title || "")
          : props.bookInfo.title;
        onCategoriesFound(collectSearchCategories(results, searchTitle, props.comic));
      }
    };

    runAutoSearch();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- searchTrigger 변경만 트리거로 사용 (다른 props는 의도적으로 deps에서 제외)
  }, [props.searchTrigger]);

  // 내부 검색 함수 (자동 검색용, 결과 반환)
  const fetchWithMethodInternal = (
    store,
    method,
    isbnVal,
    titleVal,
    authorVal,
  ) => {
    return new Promise((resolve) => {
      setData((prev) => ({ ...prev, [store]: { loading: true } }));

      const params = new URLSearchParams();

      switch (method) {
        case "isbn":
          /* v8 ignore next -- auto-search only selects ISBN when an ISBN exists. */
          if (isbnVal) params.append("isbn", isbnVal);
          break;
        case "title_author":
          if (titleVal) params.append("title", titleVal);
          if (authorVal) params.append("author", authorVal);
          break;
        case "title_only":
          /* v8 ignore next -- auto-search only selects title-only when a title exists. */
          if (titleVal) params.append("title", titleVal);
          break;
        /* v8 ignore next 4 -- method is constrained by UI/auto-search call sites. */
        default:
          if (isbnVal) params.append("isbn", isbnVal);
          if (titleVal) params.append("title", titleVal);
          if (authorVal) params.append("author", authorVal);
      }

      /* v8 ignore next 8 -- auto-search validates terms before calling this helper. */
      if (params.toString() === "") {
        setData((prev) => ({
          ...prev,
          [store]: { error: true, message: "검색어가 없습니다." },
        }));
        resolve(null);
        return;
      }

      rawJsonGetReq(
        `/search/bookstore/${store}?${params.toString()}`,
        (json) => {
          setData((prev) => ({ ...prev, [store]: json }));

          resolve(json);
        },
        (error) => {
          console.error(error);
          setData((prev) => ({
            ...prev,
            [store]: { error: true, message: "검색 중 오류가 발생했습니다." },
          }));
          resolve(null);
        },
      );
    });
  };

  // 특정 검색 방법으로 검색 수행 (버튼 클릭용)
  const fetchWithMethod = (store, method) => {
    // 검색어 결정 (default 절이 모든 경우를 덮으므로 초기값 불필요)
    let searchTerms;
    switch (method) {
      case "isbn":
        searchTerms = isbn;
        break;
      case "title_author":
        searchTerms = `${title}_${author}`;
        break;
      /* v8 ignore next 2 -- public buttons pass only explicit search methods. */
      default:
        searchTerms = `${isbn}_${title}_${author}`;
    }

    // 캐시 키에 검색어 포함
    const cacheKey = `${store}_${method}_${searchTerms}`;

    setMethods((prev) => ({ ...prev, [store]: method }));

    // 이미 해당 검색 결과가 있으면 재사용
    if (data[cacheKey] && !data[cacheKey].loading && !data[cacheKey].error) {
      setData((prev) => ({ ...prev, [store]: data[cacheKey] }));
      reportStoreCategories(store, data[cacheKey], title);
      return;
    }

    // 새 검색 시작 시 기존 결과 초기화
    setData((prev) => ({ ...prev, [store]: { loading: true } }));

    if (method === "title_author" && !isbn && title) {
      Promise.all([
        requestSearch(store, { title }),
        author ? requestSearch(store, { title, author }) : null,
      ]).then(([titleResult, authorTitleResult]) => {
        const result = combineSearchResults(titleResult, authorTitleResult);
        setData((prev) => ({ ...prev, [store]: result, [cacheKey]: result }));
        reportStoreCategories(store, result, title);
      });
      return;
    }

    const params = new URLSearchParams();

    switch (method) {
      case "isbn":
        if (isbn) params.append("isbn", isbn);
        break;
      case "title_author":
        if (title) params.append("title", title);
        if (author) params.append("author", author);
        break;
      /* v8 ignore next 4 -- public buttons pass only explicit search methods. */
      default:
        if (isbn) params.append("isbn", isbn);
        if (title) params.append("title", title);
        if (author) params.append("author", author);
    }

    rawJsonGetReq(
      `/search/bookstore/${store}?${params.toString()}`,
      (json) => {
        // 결과를 store와 cacheKey 둘 다에 저장
        setData((prev) => ({ ...prev, [store]: json, [cacheKey]: json }));
        reportStoreCategories(store, json, title);
      },
      (error) => {
        console.error(error);
        setData((prev) => ({
          ...prev,
          [store]: { error: true, message: "검색 중 오류가 발생했습니다." },
        }));
      },
    );
  };

  // 서점 한 줄 렌더링: 서점명 | 검색 버튼들 | 검색 결과
  const renderStoreRow = (storeInfo) => {
    const storeKey = storeInfo.key;
    const result = data[storeKey];
    const method = methods[storeKey];
    const searchUrl =
      result?.search_url || buildStoreSearchUrl(storeKey, title, author);
    const spinner = result?.loading && (
      <FontAwesomeIcon icon={faSpinner} spin className="ms-1" />
    );

    return (
      <div
        key={storeKey}
        className="p-2 border-bottom"
        style={{
          display: "grid",
          gridTemplateColumns: "5.5rem minmax(6rem, 13rem) 1fr",
          columnGap: "0.5rem",
          alignItems: "start",
        }}
      >
        <strong>{storeInfo.label}</strong>

        {/* 검색 버튼들: 폭이 부족하면 줄바꿈 */}
        <div className="d-flex flex-wrap gap-1">
          {!storeInfo.hideIsbn && (
            <Button
              variant={
                isbn && storeInfo.supportsIsbn
                  ? "outline-primary"
                  : "outline-secondary"
              }
              size="sm"
              active={method === "isbn"}
              aria-pressed={method === "isbn"}
              onClick={() => fetchWithMethod(storeKey, "isbn")}
              disabled={result?.loading || !isbn || !storeInfo.supportsIsbn}
              title={
                !isbn
                  ? "ISBN 정보 없음"
                  : !storeInfo.supportsIsbn
                    ? "이 서점은 ISBN 검색 미지원"
                    : ""
              }
            >
              ISBN
              {method === "isbn" && spinner}
            </Button>
          )}
          <Button
            variant={title || author ? "outline-primary" : "outline-secondary"}
            size="sm"
            active={method === "title_author"}
            aria-pressed={method === "title_author"}
            onClick={() => fetchWithMethod(storeKey, "title_author")}
            disabled={result?.loading || (!title && !author)}
          >
            도서명
            {method === "title_author" && spinner}
          </Button>
          {searchUrl && (
            <a href={searchUrl} target="_blank" rel="noreferrer">
              <Button variant="outline-secondary" size="sm">
                서점
              </Button>
            </a>
          )}
        </div>

        {/* 검색 결과 */}
        <div>
          {result?.loading && (
            <div className="text-center">
              <Spinner animation="border" size="sm" />
            </div>
          )}

          {result?.error && (
            <div className="text-danger">
              {result.message || "검색 중 오류가 발생했습니다."}
            </div>
          )}

          {result && !result.loading && !result.error && (
            <>
              {result.status === "success" && result.result.length > 0 ? (
                result.result.map((item, idx) => (
                  <div
                    key={item.book_url || idx}
                    className={idx > 0 ? "pt-1 mt-1 border-top" : ""}
                  >
                    <div>
                      <a href={item.book_url} target="_blank" rel="noreferrer">
                        <strong>{item.title}</strong>
                      </a>
                    </div>
                    <small className="text-muted">
                      {item.author && (
                        <span>
                          {storeKey === "naverwebtoon" || storeKey === "kakaowebtoon"
                            ? item.author.replaceAll(" / ", ", ")
                            : item.author}
                        </span>
                      )}
                      {item.category && <span> | {item.category}</span>}
                      {item.isbn && <span> | ISBN: {item.isbn}</span>}
                    </small>
                  </div>
                ))
              ) : (
                <span className="text-muted">검색 결과가 없습니다.</span>
              )}
            </>
          )}
        </div>
      </div>
    );
  };

  return (
    <Card>
      <Card.Header>서점 검색</Card.Header>

      <Card.Body className="p-0">{visibleStores.map(renderStoreRow)}</Card.Body>
    </Card>
  );
}

Bookstore.propTypes = {
  bookInfo: PropTypes.shape({
    author: PropTypes.string,
    title: PropTypes.string,
    isbn: PropTypes.string,
  }).isRequired,
  searchTrigger: PropTypes.number,
  onCategoriesFound: PropTypes.func,
  comic: PropTypes.bool,
  titleParsed: PropTypes.bool,
};
