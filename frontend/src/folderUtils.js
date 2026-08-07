/**
 * 폴더 계층 구조 관련 공유 유틸리티
 */

/**
 * 카테고리의 다음 페이지를 불러오기 위한 합성 트리 노드 식별자.
 * 실제 책이 아니므로 이전/다음 이동 대상에서 제외한다.
 */
export const MORE_ENTRY_FILE_TYPE = "more";
export const MORE_ENTRY_SUFFIX = "/__more__";

/** 트리 children 중 실제 책 항목만 남긴다 (하위 폴더와 '더 보기' 노드 제외). */
function onlyBookEntries(children) {
  return children.filter(
    (item) =>
      item.fileType !== "folder" && item.fileType !== MORE_ENTRY_FILE_TYPE,
  );
}

/**
 * 카테고리 문자열 배열에서 공통 prefix를 찾는다.
 * @param {string[]} strings - 카테고리 문자열 배열
 * @returns {string} 공통 prefix (슬래시 포함)
 */
export function findCommonPrefix(strings) {
  if (!strings || strings.length === 0) return "";
  if (strings.length === 1) {
    const parts = strings[0].split("/");
    return parts.length > 1 ? parts.slice(0, -1).join("/") + "/" : "";
  }
  const parts = strings.map((s) => s.split("/"));
  const minLen = Math.min(...parts.map((p) => p.length));
  let commonParts = [];
  for (let i = 0; i < minLen - 1; i++) {
    const part = parts[0][i];
    if (parts.every((p) => p[i] === part)) {
      commonParts.push(part);
    } else {
      break;
    }
  }
  return commonParts.length > 0 ? commonParts.join("/") + "/" : "";
}

/**
 * 카테고리 문자열 배열을 2단계 트리 구조로 변환한다.
 * prefix 제거 후 첫 번째 '/' 기준으로 부모-자식 그룹핑.
 *
 * @param {string[]} categories - 정렬된 카테고리 문자열 배열
 * @param {string} commonPrefix - 공통 prefix
 * @param {Object} [categoryCounts] - 카테고리별 항목 수 (예: {"_epub": 5, "_pdf": 3})
 * @returns {Array} 트리 구조 배열
 */
export function buildFolderHierarchy(
  categories,
  commonPrefix,
  categoryCounts = {},
) {
  const groups = new Map(); // parentName -> { categoryIds: [], hasParentCategory: false }

  for (const category of categories) {
    const stripped = commonPrefix
      ? category.replace(commonPrefix, "")
      : category;
    const slashIndex = stripped.indexOf("/");

    if (slashIndex === -1) {
      // 슬래시 없음 → 1 depth 항목
      const parentName = stripped;
      if (!groups.has(parentName)) {
        groups.set(parentName, { categoryIds: [], hasParentCategory: false });
      }
      groups.get(parentName).hasParentCategory = true;
      groups.get(parentName).parentCategoryId = category;
    } else {
      // 슬래시 있음 → 2 depth 항목 (부모/자식)
      const parentName = stripped.substring(0, slashIndex);
      if (!groups.has(parentName)) {
        groups.set(parentName, { categoryIds: [], hasParentCategory: false });
      }
      groups.get(parentName).categoryIds.push(category);
    }
  }

  const result = [];

  for (const [parentName, group] of groups) {
    if (group.categoryIds.length === 0) {
      // 자식 없음 → 일반 1 depth 폴더
      result.push({
        id: group.parentCategoryId,
        label: parentName,
        fileType: "folder",
        booksLoaded: false,
        count: categoryCounts[group.parentCategoryId] || 0,
      });
    } else {
      // 자식이 있는 경우
      const children = group.categoryIds.map((catId) => {
        const stripped = commonPrefix ? catId.replace(commonPrefix, "") : catId;
        const childName = stripped.substring(stripped.indexOf("/") + 1);
        return {
          id: catId,
          label: childName,
          fileType: "folder",
          booksLoaded: false,
          count: categoryCounts[catId] || 0,
        };
      });

      const childrenTotal = children.reduce((sum, c) => sum + c.count, 0);

      if (group.hasParentCategory) {
        // 부모 카테고리도 존재 → 실제 부모 폴더
        const ownCount = categoryCounts[group.parentCategoryId] || 0;
        result.push({
          id: group.parentCategoryId,
          label: parentName,
          fileType: "folder",
          isVirtualParent: false,
          booksLoaded: false,
          count: ownCount + childrenTotal,
          children: children,
        });
      } else {
        // 부모 카테고리 없음 → 가상 부모 폴더
        result.push({
          id: "__virtual__" + parentName,
          label: parentName,
          fileType: "folder",
          isVirtualParent: true,
          booksLoaded: false,
          count: childrenTotal,
          children: children,
        });
      }
    }
  }

  return result;
}

/**
 * entry ID에서 카테고리와 bookId를 분리한다.
 * 카테고리에 '/'가 포함될 수 있으므로 lastIndexOf를 사용하여
 * 마지막 '/' 뒤의 숫자를 bookId로 판단한다.
 *
 * @param {string} entryId - entry ID (예: "Fiction/Novels/12345")
 * @returns {{ category: string, bookId: string } | null}
 */
export function parseEntryId(entryId) {
  if (!entryId) return null;

  // root file 처리 (id가 '/'로 시작)
  if (entryId.startsWith("/")) {
    return { category: "_root", bookId: entryId.substring(1) };
  }

  const lastSlashIndex = entryId.lastIndexOf("/");
  if (lastSlashIndex === -1) return null;

  const possibleBookId = entryId.substring(lastSlashIndex + 1);
  const possibleCategory = entryId.substring(0, lastSlashIndex);

  // bookId가 숫자인지 확인
  if (/^\d+$/.test(possibleBookId)) {
    return { category: possibleCategory, bookId: possibleBookId };
  }

  return null;
}

/**
 * 2단계 트리에서 카테고리 ID로 폴더를 검색한다.
 * 1단계(최상위)와 2단계(children) 모두 검색.
 *
 * @param {Array} folderData - 폴더 트리 데이터
 * @param {string} categoryId - 검색할 카테고리 ID
 * @returns {object|null} 찾은 폴더 객체 또는 null
 */
export function findFolderInTree(folderData, categoryId) {
  for (const item of folderData) {
    if (item.id === categoryId) return item;
    if (item.children) {
      for (const child of item.children) {
        if (child.id === categoryId) return child;
      }
    }
  }
  return null;
}

/**
 * 2단계 트리에서 카테고리 ID의 항목을 불변 업데이트한다.
 *
 * @param {Array} folderData - 폴더 트리 데이터
 * @param {string} categoryId - 업데이트할 카테고리 ID
 * @param {function} updater - 해당 항목을 받아 새 항목을 반환하는 함수
 * @returns {Array} 새 folderData
 */
export function updateFolderInTree(folderData, categoryId, updater) {
  return folderData.map((item) => {
    if (item.id === categoryId) {
      return updater(item);
    }
    if (item.children) {
      const childIndex = item.children.findIndex((c) => c.id === categoryId);
      if (childIndex !== -1) {
        const newChildren = [...item.children];
        newChildren[childIndex] = updater(newChildren[childIndex]);
        return { ...item, children: newChildren };
      }
    }
    return item;
  });
}

/**
 * 2단계 트리에서 카테고리 ID의 children을 불변 업데이트한다.
 *
 * @param {Array} folderData - 폴더 트리 데이터
 * @param {string} categoryId - 업데이트할 카테고리 ID
 * @param {function} updater - children 배열을 받아 새 배열을 반환하는 함수
 * @returns {Array} 새 folderData
 */
export function updateFolderChildren(folderData, categoryId, updater) {
  return updateFolderInTree(folderData, categoryId, (item) => ({
    ...item,
    children: updater(item.children || []),
  }));
}

/**
 * 선택된 항목의 다음 항목 ID를 결정한다.
 * root file과 폴더 내 파일 모두 처리.
 *
 * @param {Array} folderData - 폴더 트리 데이터
 * @param {string} selectedEntryId - 현재 선택된 항목 ID
 * @returns {string|null} 다음 항목 ID 또는 null
 */
export function determineNextEntryId(folderData, selectedEntryId) {
  // root file 처리 (id가 '/'로 시작하는 경우, 예: '/917518')
  if (selectedEntryId.startsWith("/")) {
    const rootFiles = folderData.filter((item) => item.fileType !== "folder");
    const index = rootFiles.findIndex((item) => item.id === selectedEntryId);
    if (index >= 0 && index < rootFiles.length - 1) {
      return rootFiles[index + 1].id;
    }
    return null;
  }

  // 폴더 내 파일 처리 - parseEntryId로 카테고리/bookId 분리
  const parsed = parseEntryId(selectedEntryId);
  if (parsed && parsed.bookId) {
    const folder = findFolderInTree(folderData, parsed.category);
    const children = folder?.children;
    if (children) {
      // children 중 책만 필터 (하위 폴더 제외)
      const bookChildren = onlyBookEntries(children);
      const index = bookChildren.findIndex(
        (item) => item.id === selectedEntryId,
      );
      if (0 <= index && index < bookChildren.length - 1) {
        return bookChildren[index + 1].id;
      }
    }
  }
  return null;
}

/**
 * 선택된 항목의 이전 항목 ID를 결정한다.
 * root file과 폴더 내 파일 모두 처리.
 *
 * @param {Array} folderData - 폴더 트리 데이터
 * @param {string} selectedEntryId - 현재 선택된 항목 ID
 * @returns {string|null} 이전 항목 ID 또는 null
 */
export function determinePrevEntryId(folderData, selectedEntryId) {
  // root file 처리
  if (selectedEntryId.startsWith("/")) {
    const rootFiles = folderData.filter((item) => item.fileType !== "folder");
    const index = rootFiles.findIndex((item) => item.id === selectedEntryId);
    if (index > 0) {
      return rootFiles[index - 1].id;
    }
    return null;
  }

  // 폴더 내 파일 처리
  const parsed = parseEntryId(selectedEntryId);
  if (parsed && parsed.bookId) {
    const folder = findFolderInTree(folderData, parsed.category);
    const children = folder?.children;
    if (children) {
      const bookChildren = onlyBookEntries(children);
      const index = bookChildren.findIndex(
        (item) => item.id === selectedEntryId,
      );
      if (index > 0) {
        return bookChildren[index - 1].id;
      }
    }
  }
  return null;
}
