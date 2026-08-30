/**
 * 카테고리 폴더 트리의 데이터 로딩을 담당하는 공용 훅.
 *
 * Edit.jsx와 View.jsx는 같은 폴더 트리를 쓰지만 로딩 로직을 각자 복제해
 * 갖고 있었다. 그 결과 커서 페이지네이션과 책 단건 조회 폴백이 View.jsx에만
 * 적용되고 Edit.jsx는 누락되어, 10000건이 넘는 카테고리의 책이 편집 화면에서
 * 열리지 않는 장애가 발생했다. 로딩 경로를 여기 한 곳으로 모아 두 화면이
 * 다시 갈라지지 않게 한다.
 *
 * 책 선택 시의 side effect(제목 분해, 뷰어 URL 조립, 히스토리 갱신 등)는
 * 두 화면이 실제로 다르므로 각 컴포넌트에 남겨 둔다.
 */
import { useCallback, useEffect, useState } from "react";

import { jsonGetReq, rawJsonGetReq } from "./Common";
import {
  findCommonPrefix,
  buildFolderHierarchy,
  findFolderInTree,
  updateFolderInTree,
  MORE_ENTRY_FILE_TYPE,
  MORE_ENTRY_SUFFIX,
} from "./folderUtils";

// 카테고리 책 목록의 페이지 크기. 카테고리가 ES max_result_window(10000)를
// 넘어도 커서로 이어받아 전체에 도달할 수 있게 한다. 한 페이지 size 자체는
// max_result_window 이내여야 하므로 그 범위에서 크게 잡는다.
export const CATEGORY_PAGE_SIZE = 5000;

/** 책 목록 응답을 트리 노드로 변환한다. entryId는 "{category}/{book_id}" 형식. */
function toBookEntries(bookList, category) {
  return bookList.map((book) => ({
    id: category + "/" + book["book_id"].toString(),
    label: book["title"] + "." + book["file_type"],
    fileType: book["file_type"],
    children: [],
    book: book,
  }));
}

/**
 * @param {object}   options
 * @param {string}   options.apiPrefix - "" (book) 또는 "/comics"
 * @param {string}   [options.role] - "viewer"인 경우 비노출 카테고리를 걸러낸다
 * @param {function} [options.onError] - 카테고리 목록 로드 실패 시 호출
 */
export function useCategoryTree({ apiPrefix, role, onError }) {
  const [folderData, setFolderData] = useState([]);
  const [categoryList, setCategoryList] = useState([]);
  const [hiddenCategories, setHiddenCategories] = useState(new Set());

  // 카테고리 목록을 받아 2단계 폴더 계층을 만든다.
  useEffect(() => {
    const categoryListUrl = apiPrefix + "/categories";
    jsonGetReq(
      categoryListUrl,
      null,
      (categoryCounts) => {
        // categoryCounts: {"_epub": 5, "_pdf": 3, "_root": 2, ...}
        const counts = categoryCounts || {};
        const allCategories = Object.keys(counts);

        // _root 카테고리(최상위 파일) 분리
        const hasRootFiles = allCategories.includes("_root");
        const nonEmptyCategories = allCategories.filter((c) => c !== "_root");

        const buildAndSetFolderData = (filteredCategories) => {
          const commonPrefix = findCommonPrefix(filteredCategories);
          const data = buildFolderHierarchy(
            filteredCategories.sort((a, b) => (a || "").localeCompare(b || "")),
            commonPrefix,
            counts,
          );

          // 최상위 파일이 있으면 가져와서 추가
          if (hasRootFiles) {
            jsonGetReq(
              apiPrefix + "/categories/_root",
              null,
              (bookList) => {
                const rootFiles = (Array.isArray(bookList) ? bookList : [])
                  .sort((a, b) =>
                    (a?.["title"] || "").localeCompare(b?.["title"] || ""),
                  )
                  .map((book) => ({
                    id: "/" + book["book_id"]?.toString(),
                    label: (book["title"] || "") + "." + (book["file_type"] || ""),
                    fileType: book["file_type"],
                    children: [],
                    book: book,
                  }));
                setFolderData([...data, ...rootFiles]);
              },
              () => {
                setFolderData(data);
              },
            );
          } else {
            setFolderData(data);
          }
        };

        // viewer인 경우 비노출 카테고리 필터링
        if (role === "viewer") {
          const contentType = apiPrefix === "" ? "book" : "comic";
          jsonGetReq(
            `/hidden-categories?content_type=${contentType}`,
            null,
            (hiddenList) => {
              const hiddenSet = new Set(hiddenList || []);
              setHiddenCategories(hiddenSet);
              buildAndSetFolderData(
                nonEmptyCategories.filter((cat) => {
                  for (const hidden of hiddenSet) {
                    if (cat === hidden || cat.startsWith(hidden + "/")) {
                      return false;
                    }
                  }
                  return true;
                }),
              );
            },
            () => {
              // hidden 목록 로드 실패 시 전체 카테고리 표시
              buildAndSetFolderData(nonEmptyCategories);
            },
          );
        } else {
          buildAndSetFolderData(nonEmptyCategories);
        }

        setCategoryList(allCategories);
      },
      (error) => {
        if (onError) onError(error);
      },
    );

    return () => {
      setFolderData([]);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- onError는 매 렌더 새 참조라 deps에서 제외 (의도된 누락)
  }, [apiPrefix, role]);

  /**
   * 카테고리의 책 목록을 커서 기반으로 한 페이지씩 이어 불러온다.
   * 서버가 표시 순서(제목)대로 정렬해 주므로 페이지를 그대로 이어붙이면 된다.
   * 다음 페이지가 남아 있으면 '더 보기' 합성 노드를 children 끝에 둔다.
   */
  const loadCategoryPage = useCallback(
    (category) => {
      const folder = findFolderInTree(folderData, category);
      if (!folder || folder.loadingBooks) {
        return;
      }
      const cursor = folder.booksCursor || "";
      const url =
        apiPrefix +
        "/categories/" +
        category +
        `?limit=${CATEGORY_PAGE_SIZE}` +
        (cursor ? `&cursor=${encodeURIComponent(cursor)}` : "");

      setFolderData((prev) =>
        updateFolderInTree(prev, category, (item) => ({
          ...item,
          loadingBooks: true,
        })),
      );

      const stopLoading = () =>
        setFolderData((prev) =>
          updateFolderInTree(prev, category, (item) => ({
            ...item,
            loadingBooks: false,
          })),
        );

      rawJsonGetReq(
        url,
        (data) => {
          if (!data || data["status"] !== "success") {
            stopLoading();
            return;
          }
          const bookList = data["result"] || [];
          const nextCursor = data["next_cursor"] || "";
          const total = data["total"] || 0;
          const bookEntries = toBookEntries(bookList, category);

          setFolderData((prev) =>
            updateFolderInTree(prev, category, (item) => {
              const children = item.children || [];
              // 하위 폴더는 앞에, 이미 불러온 책은 순서대로 유지하고 뒤에 이어붙인다.
              const subfolders = children.filter(
                (c) => c.fileType === "folder",
              );
              const loadedBooks = children.filter(
                (c) =>
                  c.fileType !== "folder" &&
                  c.fileType !== MORE_ENTRY_FILE_TYPE,
              );
              const books = [...loadedBooks, ...bookEntries];
              const moreEntry = nextCursor
                ? [
                    {
                      id: category + MORE_ENTRY_SUFFIX,
                      label: `더 보기 (${books.length}/${total})`,
                      fileType: MORE_ENTRY_FILE_TYPE,
                      children: [],
                    },
                  ]
                : [];
              return {
                ...item,
                booksLoaded: true,
                loadingBooks: false,
                booksCursor: nextCursor,
                children: [...subfolders, ...books, ...moreEntry],
              };
            }),
          );
        },
        stopLoading,
      );
    },
    [folderData, apiPrefix],
  );

  /**
   * 책 한 권을 폴더 트리와 무관하게 /books/{id}로 직접 조회한다.
   * 카테고리 목록(/categories/{category})은 ES max_result_window 때문에
   * 10000건에서 잘리므로, 목록에 의존하면 그 뒤의 책은 열 수 없다.
   */
  const loadBookById = useCallback(
    (bookId, onSuccess, onFailure) => {
      jsonGetReq(
        apiPrefix + "/books/" + bookId,
        null,
        (book) => {
          // viewer인 경우 hidden 카테고리 접근 차단
          if (role === "viewer" && hiddenCategories.size > 0) {
            const bookCat = book["category"] || "";
            for (const hidden of hiddenCategories) {
              if (bookCat === hidden || bookCat.startsWith(hidden + "/")) {
                if (onFailure) onFailure("접근 권한이 없는 카테고리입니다.");
                return;
              }
            }
          }
          if (onSuccess) onSuccess(book);
        },
        (error) => {
          if (onFailure) onFailure(`${error}`);
        },
      );
    },
    [apiPrefix, role, hiddenCategories],
  );

  return {
    folderData,
    setFolderData,
    categoryList,
    hiddenCategories,
    loadCategoryPage,
    loadBookById,
  };
}
