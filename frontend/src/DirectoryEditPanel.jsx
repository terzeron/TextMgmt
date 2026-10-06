import { useEffect, useRef, useState } from "react";
import PropTypes from "prop-types";
import { Button, Card, Form, InputGroup } from "react-bootstrap";
import { jsonPostReq, jsonPutReq, rawJsonGetReq } from "./Common";
import Actions from "./Actions";
import { CATEGORY_PAGE_SIZE } from "./useCategoryTree";

const COMPLETION_VALUES = new Set(["완결", "미완", "외포완", "완외", "완", "完"]);
const COMPLETION_PATTERN = String.raw`(?:완결|미완|외포완|완외|완|完)`;
const CONTENTS_VALUE = String.raw`\d+(?:(?:\s*[-~]\s*|\s+)\d+\s*[화회권]?|\s*[화회권])`;
const PAREN_CONTENTS_PATTERN = new RegExp(
  String.raw`\(\s*(${CONTENTS_VALUE})\s*(${COMPLETION_PATTERN})\s*\)\s*$`,
);
const PLAIN_CONTENTS_PATTERN = new RegExp(
  String.raw`(?:^|\s)(${CONTENTS_VALUE})\s*(${COMPLETION_PATTERN})(?:\s+캡)?\s*$`,
);
const SPLIT_CONTENTS_PATTERN = new RegExp(
  String.raw`(?:\(\s*(${CONTENTS_VALUE})\s*\)|(${CONTENTS_VALUE}))\s*(?:\[\s*(${COMPLETION_PATTERN})\s*\]|\(\s*(${COMPLETION_PATTERN})\s*\))\s*$`,
);
const BARE_CONTENTS_PATTERN = new RegExp(
  String.raw`(?:^|\s)(?:\(\s*(${CONTENTS_VALUE})\s*\)|(${CONTENTS_VALUE}))\s*$`,
);
const EXTRA_CONTENTS_PATTERN = new RegExp(
  String.raw`(?:^|\s|\()\s*(${CONTENTS_VALUE})\s*\)?(.*)$`,
);
const EDITION_PATTERN =
  /(^|\s)(?:\(\s*)?(정식한국어판|정식판|한국어판|애장판|완전판)(?:\s*\))?(?=\s|$)/;

function buildContents(value, completion, fileCount, contentsUnit) {
  let normalizedValue = value
    .replace(/(\d)\s+(?=\d)/g, "$1-")
    .replace(/\s+/g, "");
  if (/\d$/.test(normalizedValue) && contentsUnit) {
    normalizedValue += contentsUnit;
  }
  const singleEpisodeMatch = /^(\d+)화$/.exec(normalizedValue);
  const episodeCount = Number(singleEpisodeMatch?.[1]);
  const joinedRangeMatch = /^1(\d+)화$/.exec(normalizedValue);
  const rangeEndCount = Number(joinedRangeMatch?.[1]);
  if (
    singleEpisodeMatch &&
    episodeCount > 1 &&
    episodeCount === Number(fileCount)
  ) {
    normalizedValue = `1~${episodeCount}화`;
  } else if (
    joinedRangeMatch &&
    rangeEndCount > 1 &&
    rangeEndCount === Number(fileCount)
  ) {
    normalizedValue = `1-${rangeEndCount}화`;
  }
  const normalizedCompletion = completion === "외포완" ? "완외"
    : completion === "완결" || completion === "完" ? "완" : completion;
  return [normalizedValue, normalizedCompletion].filter(Boolean).join(" ");
}

function parseDirectoryName(name, fileCount, contentsUnit) {
  let remainder = name.trim();
  if (/^만화_|\d+[화회권]_(?:완결|미완|완외|완|完)$/.test(remainder)) {
    remainder = remainder
      .replace(/^만화_/, "")
      .replace(/(\d+)_(\d+[화회권])(?=_|$)/g, "$1~$2")
      .replace(/_/g, " ")
      .trim();
  }
  let author = "";
  const leadingAuthorMatch = /^\[([^\]]+)]\s*/.exec(remainder);
  if (
    leadingAuthorMatch &&
    !COMPLETION_VALUES.has(leadingAuthorMatch[1].trim())
  ) {
    author = leadingAuthorMatch[1].trim();
    remainder = remainder.slice(leadingAuthorMatch[0].length).trim();
  }

  const trailingAuthorMatch = /\s*\[([^\]]+)]\s*$/.exec(remainder);
  if (
    !author &&
    trailingAuthorMatch &&
    !COMPLETION_VALUES.has(trailingAuthorMatch[1].trim())
  ) {
    author = trailingAuthorMatch[1].trim();
    remainder = remainder.slice(0, trailingAuthorMatch.index).trim();
  }

  let edition = "";
  let contents = "";
  const inlineContentsMatch =
    PAREN_CONTENTS_PATTERN.exec(remainder) ||
    PLAIN_CONTENTS_PATTERN.exec(remainder);
  const splitContentsMatch = SPLIT_CONTENTS_PATTERN.exec(remainder);
  const extraContentsMatch = EXTRA_CONTENTS_PATTERN.exec(remainder);
  const hasCompletedExtras =
    extraContentsMatch &&
    /외전|특별편|후기|외포완/.test(extraContentsMatch[2]) &&
    /(?:^|[^가-힣\w])(?:완결|외포완|완|完)(?=$|[^가-힣\w])/.test(extraContentsMatch[2]);
  const contentsMatch = hasCompletedExtras
    ? extraContentsMatch
    : inlineContentsMatch || splitContentsMatch;
  if (contentsMatch) {
    const value = contentsMatch[1] || contentsMatch[2];
    const completion = hasCompletedExtras
      ? "완외"
      : inlineContentsMatch
        ? inlineContentsMatch[2]
        : splitContentsMatch[3] || splitContentsMatch[4];
    contents = buildContents(value, completion, fileCount, contentsUnit);
    remainder = remainder.slice(0, contentsMatch.index).trim();
  } else {
    const bareContentsMatch = BARE_CONTENTS_PATTERN.exec(remainder);
    const value = bareContentsMatch
      ? buildContents(bareContentsMatch[1] || bareContentsMatch[2], "", fileCount, contentsUnit)
      : "";
    const rangeMatch = /^1[-~](\d+)[화회권]$/.exec(value);
    if (rangeMatch && Number(rangeMatch[1]) > 0 && Number(rangeMatch[1]) === Number(fileCount)) {
      contents = value;
      remainder = remainder.slice(0, bareContentsMatch.index).trim();
    }
  }

  const editionMatch = EDITION_PATTERN.exec(remainder);
  if (editionMatch) {
    edition = editionMatch[2];
    remainder = `${remainder.slice(0, editionMatch.index)} ${remainder.slice(
      editionMatch.index + editionMatch[0].length,
    )}`.trim();
  }

  return {
    directoryAuthor: author,
    directoryTitle: remainder.replace(/\s+/g, " "),
    directoryEdition: edition,
    directoryContents: contents,
  };
}

function buildDirectoryName({
  directoryAuthor,
  directoryTitle,
  directoryEdition,
  directoryContents,
}) {
  const parts = [];
  if (directoryAuthor.trim()) parts.push(`[${directoryAuthor.trim()}]`);
  if (directoryTitle.trim()) parts.push(directoryTitle.trim());
  if (directoryEdition.trim()) parts.push(`(${directoryEdition.trim()})`);
  if (directoryContents.trim()) parts.push(`(${directoryContents.trim()})`);
  return parts.join(" ");
}

function splitLeadingText(value) {
  const match = /^(\S+)\s+(.+)$/.exec(value.trim());
  return match ? [match[1], match[2]] : null;
}

export default function DirectoryEditPanel({
  directory,
  apiPrefix,
  showDirectoryNavigation,
  selectedCategory,
  otherCategoryList,
  suggestedCategories,
  previousDirectoryDisabled,
  nextDirectoryDisabled,
  onPreviousDirectory,
  onNextDirectory,
  isProcessing,
  onSelectCategory,
  onMove,
  onComplete,
  onError,
  onMetadataReady,
}) {
  const initialNameParts = parseDirectoryName(directory.name);
  const [author, setAuthor] = useState(initialNameParts.directoryAuthor);
  const [title, setTitle] = useState(initialNameParts.directoryTitle);
  const [edition, setEdition] = useState(initialNameParts.directoryEdition);
  const [contents, setContents] = useState(initialNameParts.directoryContents);
  const [name, setName] = useState(buildDirectoryName(initialNameParts));
  const [saving, setSaving] = useState(false);
  const [pdfStats, setPdfStats] = useState(null);
  const [contentsUnit, setContentsUnit] = useState("");
  const metadataKey = JSON.stringify([apiPrefix, directory.category, directory.name]);
  const [statsReadyKey, setStatsReadyKey] = useState("");
  const [unitReadyKey, setUnitReadyKey] = useState("");
  const reportedMetadataKey = useRef("");
  const [statsError, setStatsError] = useState("");
  const [bulkPattern, setBulkPattern] = useState("");
  const [bulkReplacement, setBulkReplacement] = useState("");
  const [bulkBusy, setBulkBusy] = useState(false);
  const [bulkInfo, setBulkInfo] = useState("");
  const [bulkFailures, setBulkFailures] = useState([]);

  const updateNameParts = (updates) => {
    const next = {
      directoryAuthor: author,
      directoryTitle: title,
      directoryEdition: edition,
      directoryContents: contents,
      ...updates,
    };
    setAuthor(next.directoryAuthor);
    setTitle(next.directoryTitle);
    setEdition(next.directoryEdition);
    setContents(next.directoryContents);
    setName(buildDirectoryName(next));
  };

  const splitAuthor = () => {
    const result = splitLeadingText(author);
    if (result) {
      updateNameParts({
        directoryAuthor: result[0],
        directoryTitle: result[1],
      });
    }
  };

  const splitTitle = () => {
    const result = splitLeadingText(title);
    if (result) {
      updateNameParts({
        directoryAuthor: result[0],
        directoryTitle: result[1],
      });
    }
  };

  const restoreName = () => {
    const original = parseDirectoryName(directory.name, pdfStats?.file_count, contentsUnit);
    updateNameParts(original);
  };

  useEffect(() => {
    let cancelled = false;
    setStatsReadyKey("");
    setPdfStats(null);
    setStatsError("");
    rawJsonGetReq(
      `${apiPrefix}/category-pdf-stats?category=${encodeURIComponent(directory.category)}`,
      (data) => {
        if (cancelled) return;
        if (data.status === "success") {
          setPdfStats(data.result);
        } else {
          setStatsError(data.error || "PDF 통계를 불러오지 못했습니다.");
        }
        setStatsReadyKey(metadataKey);
      },
      () => {
        if (cancelled) return;
        setStatsError("PDF 통계를 불러오지 못했습니다.");
        setStatsReadyKey(metadataKey);
      },
    );
    return () => { cancelled = true; };
  }, [apiPrefix, directory.category, directory.name, metadataKey]);

  useEffect(() => {
    setUnitReadyKey("");
    setContentsUnit("");
    const parsed = parseDirectoryName(directory.name);
    if (!/^\d+[-~]\d+\s/.test(parsed.directoryContents)) {
      setUnitReadyKey(metadataKey);
      return;
    }

    let cancelled = false;
    const units = new Set();
    const categoryPath = directory.category.split("/").map(encodeURIComponent).join("/");
    const loadFilePage = (cursor = "") => {
      rawJsonGetReq(
        `${apiPrefix}/categories/${categoryPath}?limit=${CATEGORY_PAGE_SIZE}${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`,
        (data) => {
          if (cancelled) return;
          if (data.status !== "success" || !Array.isArray(data.result)) {
            setUnitReadyKey(metadataKey);
            return;
          }
          for (const file of data.result) {
            const filename = (file.file_path || "").split("/").pop();
            for (const match of filename.matchAll(/\d+(권|화)/g)) units.add(match[1]);
          }
          if (units.size > 1) {
            setUnitReadyKey(metadataKey);
            return;
          }
          if (data.next_cursor) {
            loadFilePage(data.next_cursor);
          } else {
            if (units.size === 1) setContentsUnit([...units][0]);
            setUnitReadyKey(metadataKey);
          }
        },
        () => { if (!cancelled) setUnitReadyKey(metadataKey); },
      );
    };
    loadFilePage();
    return () => { cancelled = true; };
  }, [apiPrefix, directory.category, directory.name, metadataKey]);

  useEffect(() => {
    const parsed = parseDirectoryName(directory.name);
    const inferred = parseDirectoryName(directory.name, pdfStats?.file_count, contentsUnit);
    const nextTitle = title === parsed.directoryTitle ? inferred.directoryTitle : title;
    const nextContents = contents === parsed.directoryContents ? inferred.directoryContents : contents;
    if (title === nextTitle && contents === nextContents) return;

    const currentParts = {
      directoryAuthor: author,
      directoryTitle: title,
      directoryEdition: edition,
      directoryContents: contents,
    };
    setTitle(nextTitle);
    setContents(nextContents);
    setName((current) => current === buildDirectoryName(currentParts)
      ? buildDirectoryName({ ...currentParts, directoryTitle: nextTitle, directoryContents: nextContents })
      : current,
    );
  }, [directory.name, pdfStats?.file_count, contentsUnit, author, title, edition, contents]);

  useEffect(() => {
    if (!onMetadataReady || statsReadyKey !== metadataKey || unitReadyKey !== metadataKey || reportedMetadataKey.current === metadataKey) return;
    const parsed = parseDirectoryName(directory.name);
    const inferred = parseDirectoryName(directory.name, pdfStats?.file_count, contentsUnit);
    // 목차 보정 결과가 편집 필드에 반영된 다음 검색을 시작한다.
    if (contents === parsed.directoryContents && contents !== inferred.directoryContents) return;
    if (title === parsed.directoryTitle && title !== inferred.directoryTitle) return;
    reportedMetadataKey.current = metadataKey;
    onMetadataReady({ category: directory.category, sourceName: directory.name, title, author });
  }, [onMetadataReady, statsReadyKey, unitReadyKey, metadataKey, directory.category, directory.name, pdfStats?.file_count, contentsUnit, contents, title, author]);

  const submit = () => {
    const cleanName = name.trim();
    if (!cleanName || cleanName.includes("/") || cleanName === "." || cleanName === "..") {
      onError("디렉토리 이름을 입력하세요. 이름에는 '/'를 사용할 수 없습니다.");
      return;
    }
    const parent = directory.category.includes("/")
      ? directory.category.slice(0, directory.category.lastIndexOf("/"))
      : "";
    const newCategory = parent ? `${parent}/${cleanName}` : cleanName;
    if (newCategory === directory.category) return;
    setSaving(true);
    jsonPutReq(
      `${apiPrefix}/categories/rename`,
      { old_category: directory.category, new_category: newCategory },
      () => onComplete("디렉토리 이름을 변경했습니다.", { type: "rename", category: newCategory }),
      (error) => onError(`디렉토리 변경에 실패했습니다. ${error}`),
      () => setSaving(false),
    );
  };

  const deleteDirectory = () => {
    const confirmed = window.confirm(
      `"${directory.category}" 디렉토리와 하위 디렉토리의 모든 파일을 삭제할까요? 이 작업은 되돌릴 수 없습니다.`,
    );
    if (!confirmed) return;

    setSaving(true);
    jsonPostReq(
      `${apiPrefix}/categories/delete`,
      { category: directory.category, delete_files: true },
      () => onComplete("디렉토리와 하위 파일을 삭제했습니다.", { type: "delete", category: directory.category }),
      (error) => onError(`디렉토리 삭제에 실패했습니다. ${error}`),
      () => setSaving(false),
    );
  };

  const bulkRenameFiles = () => {
    setBulkBusy(true);
    setBulkInfo("이름 변경 중...");
    setBulkFailures([]);
    jsonPostReq(
      `${apiPrefix}/categories/bulk-rename-files`,
      { category: directory.category, pattern: bulkPattern, replacement: bulkReplacement },
      (result) => {
        const failures = result.failures || [];
        setBulkFailures(failures);
        const failureText = result.failed_count ? `, 실패 ${result.failed_count}개` : "";
        setBulkInfo(`변경된 파일 ${result.changed_count || 0}개${failureText}`);
      },
      (error) => setBulkInfo(`오류: ${error || "파일 이름 변경 요청에 실패했습니다."}`),
      () => setBulkBusy(false),
    );
  };

  return (
    <>
    <Card className="mb-3">
      <Card.Header>디렉토리 편집</Card.Header>
      <Card.Body>
        <InputGroup size="sm" className="directory-edit-row mb-2">
          <InputGroup.Text>파일 수</InputGroup.Text>
          <Form.Control
            aria-label="파일 수"
            value={pdfStats ? `${pdfStats.file_count.toLocaleString()}ea` : ""}
            readOnly
            disabled
          />
          <InputGroup.Text>페이지 수</InputGroup.Text>
          <Form.Control
            aria-label="페이지 수"
            value={pdfStats ? `${pdfStats.page_count.toLocaleString()}p` : ""}
            readOnly
            disabled
          />
          <InputGroup.Text>파일 크기</InputGroup.Text>
          <Form.Control
            aria-label="파일 크기"
            value={pdfStats ? `${Math.trunc(pdfStats.total_file_size / 1000).toLocaleString()}MB` : ""}
            readOnly
            disabled
          />
        </InputGroup>
        {!pdfStats && (
          <div className={statsError ? "text-danger mb-2" : "mb-2"} role="status">
            {statsError || "PDF 통계 불러오는 중..."}
          </div>
        )}
        <InputGroup size="sm" className="directory-edit-row mb-2">
          <InputGroup.Text>저자</InputGroup.Text>
          <Form.Control
            aria-label="저자"
            value={author}
            onChange={(event) =>
              updateNameParts({ directoryAuthor: event.target.value })
            }
            disabled={saving || isProcessing}
          />
          <InputGroup.Text>목차</InputGroup.Text>
          <Form.Control
            aria-label="목차"
            value={contents}
            onChange={(event) =>
              updateNameParts({ directoryContents: event.target.value })
            }
            disabled={saving || isProcessing}
          />
          <Button variant="outline-secondary" className="btn-xs" onClick={splitAuthor} disabled={saving || isProcessing}>
            분할
          </Button>
          <Button
            variant="outline-secondary"
            className="btn-xs"
            onClick={() =>
              updateNameParts({ directoryAuthor: title, directoryTitle: author })
            }
            disabled={saving || isProcessing}
          >
            교환
          </Button>
        </InputGroup>
        <InputGroup size="sm" className="directory-edit-row mb-2">
          <InputGroup.Text>제목</InputGroup.Text>
          <Form.Control
            aria-label="제목"
            className="directory-title-control"
            value={title}
            onChange={(event) =>
              updateNameParts({ directoryTitle: event.target.value.replace(/\s*\/\s*/g, ", ") })
            }
            onBlur={(event) => {
              if (event.target.value.includes("/")) {
                updateNameParts({ directoryTitle: event.target.value.replace(/\s*\/\s*/g, ", ") });
              }
            }}
            disabled={saving || isProcessing}
          />
          <InputGroup.Text>판본</InputGroup.Text>
          <Form.Control
            aria-label="판본"
            className="directory-edition-control"
            value={edition}
            onChange={(event) =>
              updateNameParts({ directoryEdition: event.target.value })
            }
            disabled={saving || isProcessing}
          />
          <Button variant="outline-secondary" className="btn-xs" onClick={splitTitle} disabled={saving || isProcessing}>
            분할
          </Button>
          <Button variant="outline-secondary" className="btn-xs" onClick={restoreName} disabled={saving || isProcessing}>
            복원
          </Button>
        </InputGroup>
        <InputGroup size="sm" className="directory-edit-row mb-3">
          <InputGroup.Text>신규 이름</InputGroup.Text>
          <Form.Control
            aria-label="신규 이름"
            value={name}
            onChange={(event) => setName(event.target.value)}
            disabled={saving || isProcessing}
          />
          <Button
            variant="outline-success"
            className="btn-xs"
            onClick={submit}
            disabled={saving || isProcessing || !name.trim()}
          >
            {saving ? "처리 중..." : "변경"}
          </Button>
          <Button
            variant="outline-danger"
            className="btn-xs"
            onClick={deleteDirectory}
            disabled={saving || isProcessing}
          >
            삭제
          </Button>
        </InputGroup>
        <hr />
        <Actions
          directoryMode
          showDirectoryNavigation={showDirectoryNavigation}
          selectedEntryId={directory.category}
          selectedCategory={selectedCategory}
          otherCategoryList={otherCategoryList}
          previousDirectoryDisabled={previousDirectoryDisabled}
          nextDirectoryDisabled={nextDirectoryDisabled}
          onPreviousDirectory={onPreviousDirectory}
          onNextDirectory={onNextDirectory}
          newFileName={name}
          moveToUpperButtonClicked={() => {}}
          moveToDirectoryButtonClicked={() => onMove(name.trim())}
          selectDirectoryButtonClicked={onSelectCategory}
          toNextEntryClicked={() => {}}
          toPrevEntryClicked={() => {}}
          suggestedCategories={suggestedCategories}
          isProcessing={isProcessing || saving}
        />
      </Card.Body>
    </Card>
    <Card className="mb-3">
        <Card.Header>파일 이름 일괄 편집</Card.Header>
        <Card.Body>
          <Form.Group className="mb-3">
            <Form.Label>Python 정규표현식</Form.Label>
            <Form.Control
              value={bulkPattern}
              onChange={(event) => setBulkPattern(event.target.value)}
              placeholder="파일 이름에 적용할 패턴"
              disabled={bulkBusy || saving || isProcessing}
            />
          </Form.Group>
          <Form.Group className="mb-3">
            <Form.Label>바꿀 문자열</Form.Label>
            <Form.Control
              value={bulkReplacement}
              onChange={(event) => setBulkReplacement(event.target.value)}
              placeholder="정규표현식 치환 문자열"
              disabled={bulkBusy || saving || isProcessing}
            />
          </Form.Group>
          <Button onClick={bulkRenameFiles} disabled={!bulkPattern || bulkBusy || saving || isProcessing}>
            {bulkBusy ? "변경 중..." : "파일 이름 변경"}
          </Button>
          <div className="info mt-3" role="status">
            {bulkInfo}
            {bulkFailures.length > 0 && (
              <ul className="mb-0 mt-2">
                {bulkFailures.map((failure, index) => (
                  <li key={`${failure.file}-${index}`}>
                    {failure.file}: {failure.error}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </Card.Body>
      </Card>
    </>
  );
}

DirectoryEditPanel.propTypes = {
  directory: PropTypes.shape({
    category: PropTypes.string.isRequired,
    name: PropTypes.string.isRequired,
  }).isRequired,
  apiPrefix: PropTypes.string.isRequired,
  showDirectoryNavigation: PropTypes.bool.isRequired,
  selectedCategory: PropTypes.string.isRequired,
  otherCategoryList: PropTypes.arrayOf(PropTypes.string).isRequired,
  suggestedCategories: PropTypes.object,
  previousDirectoryDisabled: PropTypes.bool.isRequired,
  nextDirectoryDisabled: PropTypes.bool.isRequired,
  onPreviousDirectory: PropTypes.func.isRequired,
  onNextDirectory: PropTypes.func.isRequired,
  isProcessing: PropTypes.bool.isRequired,
  onSelectCategory: PropTypes.func.isRequired,
  onMove: PropTypes.func.isRequired,
  onComplete: PropTypes.func.isRequired,
  onError: PropTypes.func.isRequired,
  onMetadataReady: PropTypes.func,
};
