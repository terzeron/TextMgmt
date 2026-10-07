// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

const { mockJsonPostReq, mockJsonPutReq, mockRawJsonGetReq } = vi.hoisted(() => ({
  mockJsonPostReq: vi.fn(),
  mockJsonPutReq: vi.fn(),
  mockRawJsonGetReq: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  jsonPostReq: mockJsonPostReq,
  jsonPutReq: mockJsonPutReq,
  rawJsonGetReq: mockRawJsonGetReq,
}));

vi.mock("../src/Actions", () => ({
  default: ({
    onPreviousDirectory,
    onNextDirectory,
    moveToUpperButtonClicked,
    moveToDirectoryButtonClicked,
    selectDirectoryButtonClicked,
    toNextEntryClicked,
    toPrevEntryClicked,
  }) => (
    <div>
      <button onClick={onPreviousDirectory}>이전 디렉토리</button>
      <button onClick={onNextDirectory}>다음 디렉토리</button>
      <button onClick={moveToUpperButtonClicked}>상위로 이동</button>
      <button onClick={moveToDirectoryButtonClicked}>이동</button>
      <button onClick={selectDirectoryButtonClicked}>카테고리 선택</button>
      <button onClick={toNextEntryClicked}>다음 항목</button>
      <button onClick={toPrevEntryClicked}>이전 항목</button>
    </div>
  ),
}));

import DirectoryEditPanel from "../src/DirectoryEditPanel";

afterEach(cleanup);

const props = (overrides = {}) => ({
  directory: { category: "comics/series", name: "series" },
  apiPrefix: "/comics",
  showDirectoryNavigation: true,
  selectedCategory: "comics",
  otherCategoryList: ["comics", "other"],
  previousDirectoryDisabled: false,
  nextDirectoryDisabled: false,
  onPreviousDirectory: vi.fn(),
  onNextDirectory: vi.fn(),
  isProcessing: false,
  onSelectCategory: vi.fn(),
  onMove: vi.fn(),
  onComplete: vi.fn(),
  onError: vi.fn(),
  ...overrides,
});

describe("DirectoryEditPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    mockRawJsonGetReq.mockImplementation((_url, success) =>
      success({
        status: "success",
        result: { file_count: 3, page_count: 120, total_file_size: 5000 },
      }),
    );
  });

  it.each(["통계 먼저", "단위 먼저"])("%s 완료돼도 자동 분할 전체가 끝나야 검색용 메타데이터를 알린다", async (order) => {
    const pending = {};
    const onMetadataReady = vi.fn();
    mockRawJsonGetReq.mockImplementation((url, success) => {
      pending[url.includes("category-pdf-stats") ? "stats" : "unit"] = success;
    });
    render(<DirectoryEditPanel {...props({
      directory: { category: "comics/series", name: "[저자] 작품명 (애장판) 1-35 완 + 후기 캡" },
      onMetadataReady,
    })} />);
    expect(onMetadataReady).not.toHaveBeenCalled();
    const completeStats = () => pending.stats({ status: "success", result: { file_count: 35, page_count: 100, total_file_size: 5000 } });
    const completeUnit = () => pending.unit({ status: "success", result: [{ file_path: "comics/series/01화.pdf" }] });
    await act(async () => (order === "통계 먼저" ? completeStats : completeUnit)());
    expect(onMetadataReady).not.toHaveBeenCalled();
    await act(async () => (order === "통계 먼저" ? completeUnit : completeStats)());
    await waitFor(() => expect(onMetadataReady).toHaveBeenCalledOnce());
    expect(screen.getByLabelText("목차").value).toBe("1-35화 완외");
    expect(onMetadataReady).toHaveBeenCalledWith({ category: "comics/series", sourceName: "[저자] 작품명 (애장판) 1-35 완 + 후기 캡", title: "작품명", author: "저자" });
    fireEvent.change(screen.getByLabelText("제목"), { target: { value: "수정한 제목" } });
    expect(onMetadataReady).toHaveBeenCalledOnce();
  });

  it("메타데이터 조회가 실패해도 기본 분할 결과로 검색을 시작할 수 있다", async () => {
    const onMetadataReady = vi.fn();
    mockRawJsonGetReq.mockImplementation((_url, _success, failure) => failure());
    render(<DirectoryEditPanel {...props({ directory: { category: "comics/series", name: "작품명 1-35 완" }, onMetadataReady })} />);

    await waitFor(() => expect(onMetadataReady).toHaveBeenCalledOnce());
    expect(onMetadataReady).toHaveBeenCalledWith({ category: "comics/series", sourceName: "작품명 1-35 완", title: "작품명", author: "" });
  });

  it.each([
    ["땅 보고 걷는 아이 1 70 외포완 + 후기포함 캡", "땅 보고 걷는 아이", "1-70화 완외", "화"],
    ["라일락 200% 1-87 외포완 + 후기포함 캡", "라일락 200%", "1-87화 완외", "화"],
    ["맘마미안 1-102 완 캡", "맘마미안", "1-102화 완", "화"],
    ["바른연애 길잡이 1-159 외포완 + 후기 캡", "바른연애 길잡이", "1-159화 완외", "화"],
    ["작품명 1-35 완 캡", "작품명", "1-35권 완", "권"],
    ["작품명 1-35 외포완", "작품명", "1-35화 완외", "화"],
  ])("단위 없는 회차 %s를 분할한 뒤 파일명에서 단위를 추론한다", async (sourceName, title, contents, unit) => {
    const onMetadataReady = vi.fn();
    mockRawJsonGetReq.mockImplementation((url, success) => success({
      status: "success",
      result: url.includes("category-pdf-stats")
        ? { file_count: 35, page_count: 100, total_file_size: 5000 }
        : [{ file_path: `comics/series/01${unit}.pdf` }],
    }));
    const values = props({ directory: { category: "comics/series", name: sourceName }, onMetadataReady });
    render(<DirectoryEditPanel {...values} />);

    await waitFor(() => expect(screen.getByLabelText("목차").value).toBe(contents));
    expect(screen.getByLabelText("제목").value).toBe(title);
    expect(screen.getByLabelText("신규 이름").value).toBe(`${title} (${contents})`);
    expect(onMetadataReady).toHaveBeenCalledOnce();
    expect(onMetadataReady).toHaveBeenCalledWith({ category: "comics/series", sourceName, title, author: "" });
    fireEvent.click(screen.getByText("이동"));
    expect(values.onMove).toHaveBeenCalledWith(`${title} (${contents})`);
  });

  it("통계와 디렉토리 이름 구성 요소를 input field로 표시한다", async () => {
    const values = props({
      directory: {
        category: "comics/series",
        name: "[홍길동] 테스트 만화 (애장판) (1-10권 완)",
      },
    });
    render(<DirectoryEditPanel {...values} />);

    expect((await screen.findByLabelText("파일 수")).value).toBe("3ea");
    expect(screen.getByLabelText("페이지 수").value).toBe("120p");
    expect(screen.getByLabelText("파일 크기").value).toBe("5MB");
    expect(screen.getByLabelText("저자").value).toBe("홍길동");
    const titleInput = screen.getByLabelText("제목");
    const editionInput = screen.getByLabelText("판본");
    expect(titleInput.value).toBe("테스트 만화");
    expect(editionInput.value).toBe("애장판");
    expect(titleInput.classList.contains("directory-title-control")).toBe(true);
    expect(editionInput.classList.contains("directory-edition-control")).toBe(true);
    expect(screen.getByLabelText("목차").value).toBe("1-10권 완");
    expect(screen.getByLabelText("신규 이름").value).toBe(
      "[홍길동] 테스트 만화 (애장판) (1-10권 완)",
    );
  });

  it("디렉토리 탐색 액션을 전달한다", () => {
    const values = props();
    render(<DirectoryEditPanel {...values} />);

    fireEvent.click(screen.getByText("이전 디렉토리"));
    fireEvent.click(screen.getByText("다음 디렉토리"));
    fireEvent.click(screen.getByText("상위로 이동"));
    fireEvent.click(screen.getByText("이동"));
    fireEvent.click(screen.getByText("카테고리 선택"));
    fireEvent.click(screen.getByText("다음 항목"));
    fireEvent.click(screen.getByText("이전 항목"));
    expect(values.onPreviousDirectory).toHaveBeenCalledOnce();
    expect(values.onNextDirectory).toHaveBeenCalledOnce();
    expect(values.onMove).toHaveBeenCalledWith("series");
    expect(values.onSelectCategory).toHaveBeenCalledOnce();
  });

  it("입력한 구성 요소로 신규 이름을 조합한다", () => {
    render(<DirectoryEditPanel {...props()} />);

    fireEvent.change(screen.getByLabelText("저자"), {
      target: { value: "홍길동" },
    });
    fireEvent.change(screen.getByLabelText("제목"), {
      target: { value: "테스트 만화" },
    });
    fireEvent.change(screen.getByLabelText("판본"), {
      target: { value: "애장판" },
    });
    fireEvent.change(screen.getByLabelText("목차"), {
      target: { value: "1-10권 완" },
    });

    expect(screen.getByLabelText("신규 이름").value).toBe(
      "[홍길동] 테스트 만화 (애장판) (1-10권 완)",
    );
  });

  it.each([
    ["XXX / YYY", "XXX / YYY"],
    ["XXX/YYY / ZZZ", "XXX/YYY / ZZZ"],
    ["XXX  /  YYY", "XXX  /  YYY"],
    ["XXX, YYY", "XXX, YYY"],
  ])("제목 입력 %s를 자동 치환하지 않고 신규 이름에 반영한다", (input, expected) => {
    const values = props({ directory: { category: "comics/series", name: "[저자] 기존 제목 (애장판) (1-10권 완)" } });
    render(<DirectoryEditPanel {...values} />);

    fireEvent.change(screen.getByLabelText("제목"), { target: { value: input } });
    fireEvent.blur(screen.getByLabelText("제목"));

    expect(screen.getByLabelText("제목").value).toBe(expected);
    expect(screen.getByLabelText("신규 이름").value).toBe(`[저자] ${expected} (애장판) (1-10권 완)`);
    fireEvent.click(screen.getByText("이동"));
    expect(values.onMove).toHaveBeenCalledWith(`[저자] ${expected} (애장판) (1-10권 완)`);
  });

  it.each([
    "봉이 / 갈피 / 오윤",
    "XXX/YYY / ZZZ",
  ])("제목 %s에서 포커스가 빠져도 슬래시와 수동 신규 이름을 유지한다", (input) => {
    const values = props({ directory: { category: "comics/series", name: "[저자] 기존 제목 (애장판) (1-10권 완)" } });
    render(<DirectoryEditPanel {...values} />);
    const titleInput = screen.getByLabelText("제목");
    fireEvent.change(titleInput, { target: { value: input } });
    fireEvent.change(screen.getByLabelText("신규 이름"), { target: { value: "사용자 지정 이름" } });
    fireEvent.focus(titleInput);
    fireEvent.blur(titleInput);

    expect(titleInput.value).toBe(input);
    expect(screen.getByLabelText("신규 이름").value).toBe("사용자 지정 이름");
    fireEvent.click(screen.getByText("이동"));
    expect(values.onMove).toHaveBeenCalledWith("사용자 지정 이름");
  });

  it("슬래시 없는 제목에서 포커스가 빠져도 수동으로 수정한 신규 이름을 유지한다", () => {
    render(<DirectoryEditPanel {...props()} />);
    fireEvent.change(screen.getByLabelText("신규 이름"), { target: { value: "사용자 지정 이름" } });
    const titleInput = screen.getByLabelText("제목");
    fireEvent.focus(titleInput);
    fireEvent.blur(titleInput);
    expect(screen.getByLabelText("신규 이름").value).toBe("사용자 지정 이름");
  });

  it("분할, 교환, 복원 버튼으로 이름 구성 요소를 편집한다", () => {
    render(
      <DirectoryEditPanel
        {...props({
          directory: {
            category: "comics/series",
            name: "[홍길동] 테스트 만화 (애장판) (1-10권 완)",
          },
        })}
      />,
    );

    fireEvent.change(screen.getByLabelText("제목"), {
      target: { value: "김작가 새제목" },
    });
    fireEvent.click(screen.getAllByRole("button", { name: "분할" })[1]);
    expect(screen.getByLabelText("저자").value).toBe("김작가");
    expect(screen.getByLabelText("제목").value).toBe("새제목");

    fireEvent.click(screen.getByRole("button", { name: "교환" }));
    expect(screen.getByLabelText("저자").value).toBe("새제목");
    expect(screen.getByLabelText("제목").value).toBe("김작가");

    fireEvent.click(screen.getByRole("button", { name: "복원" }));
    expect(screen.getByLabelText("저자").value).toBe("홍길동");
    expect(screen.getByLabelText("제목").value).toBe("테스트 만화");
    expect(screen.getByLabelText("신규 이름").value).toBe(
      "[홍길동] 테스트 만화 (애장판) (1-10권 완)",
    );
  });

  it("분리된 목차 범위와 완결 표기를 표준 형식으로 조합한다", () => {
    render(
      <DirectoryEditPanel
        {...props({
          directory: {
            category: "comics/series",
            name: "[홍길동] 테스트 만화 (애장판) (1-10권) [완결]",
          },
        })}
      />,
    );

    expect(screen.getByLabelText("저자").value).toBe("홍길동");
    expect(screen.getByLabelText("제목").value).toBe("테스트 만화");
    expect(screen.getByLabelText("판본").value).toBe("애장판");
    expect(screen.getByLabelText("목차").value).toBe("1-10권 완");
    expect(screen.getByLabelText("신규 이름").value).toBe(
      "[홍길동] 테스트 만화 (애장판) (1-10권 완)",
    );
  });

  it("맨 뒤 저자와 괄호 없는 판본을 분리한다", () => {
    render(
      <DirectoryEditPanel
        {...props({
          directory: {
            category: "comics/series",
            name: "테스트 만화 정식한국어판 (1~20화) (完) [김작가]",
          },
        })}
      />,
    );

    expect(screen.getByLabelText("저자").value).toBe("김작가");
    expect(screen.getByLabelText("제목").value).toBe("테스트 만화");
    expect(screen.getByLabelText("판본").value).toBe("정식한국어판");
    expect(screen.getByLabelText("목차").value).toBe("1~20화 완");
    expect(screen.getByLabelText("신규 이름").value).toBe(
      "[김작가] 테스트 만화 (정식한국어판) (1~20화 완)",
    );
  });

  it("완결 표기를 맨 뒤 저자로 오인하지 않는다", () => {
    render(
      <DirectoryEditPanel
        {...props({
          directory: {
            category: "comics/series",
            name: "테스트 만화 (1-10회) [완]",
          },
        })}
      />,
    );

    expect(screen.getByLabelText("저자").value).toBe("");
    expect(screen.getByLabelText("제목").value).toBe("테스트 만화");
    expect(screen.getByLabelText("목차").value).toBe("1-10회 완");
  });

  it.each(["1 7권", "1-7권", "(1-7권)"])("완결 표기가 없는 %s는 파일 7개를 확인한 뒤 분리한다", async (suffix) => {
    let completeStats;
    const onMetadataReady = vi.fn();
    const sourceName = `터무니 없는 스킬로 이세계 방랑밥 ${suffix}`;
    mockRawJsonGetReq.mockImplementation((_url, success) => { completeStats = success; });
    const values = props({ directory: { category: "comics/series", name: sourceName }, onMetadataReady });
    render(<DirectoryEditPanel {...values} />);
    expect(onMetadataReady).not.toHaveBeenCalled();

    act(() => completeStats({ status: "success", result: { file_count: 7, page_count: 100, total_file_size: 5000 } }));

    await waitFor(() => expect(screen.getByLabelText("목차").value).toBe("1-7권"));
    expect(screen.getByLabelText("제목").value).toBe("터무니 없는 스킬로 이세계 방랑밥");
    expect(screen.getByLabelText("신규 이름").value).toBe("터무니 없는 스킬로 이세계 방랑밥 (1-7권)");
    expect(onMetadataReady).toHaveBeenCalledExactlyOnceWith({ category: "comics/series", sourceName, title: "터무니 없는 스킬로 이세계 방랑밥", author: "" });
    fireEvent.click(screen.getByText("이동"));
    expect(values.onMove).toHaveBeenCalledWith("터무니 없는 스킬로 이세계 방랑밥 (1-7권)");
  });

  it.each(["1-44화", "1~44화", "(1-44화)", "1 - 44 화", "2-44권", "1-44회"])(
    "완결 없는 명시적 범위 %s는 파일 수와 무관하게 분리한다",
    async (suffix) => {
      let completeStats;
      mockRawJsonGetReq.mockImplementation((_url, success) => { completeStats = success; });
      const sourceName = `악녀는 모래시계를 되돌린다 ${suffix}`;
      const expectedContents = suffix.replace(/[()\s]/g, "");
      const onMetadataReady = vi.fn();
      const values = props({ directory: { category: "comics/series", name: sourceName }, onMetadataReady });
      render(<DirectoryEditPanel {...values} />);

      expect(screen.getByLabelText("제목").value).toBe("악녀는 모래시계를 되돌린다");
      expect(screen.getByLabelText("목차").value).toBe(expectedContents);
      expect(onMetadataReady).not.toHaveBeenCalled();
      act(() => completeStats({ status: "success", result: { file_count: 45, page_count: 100, total_file_size: 5000 } }));
      await waitFor(() => expect(onMetadataReady).toHaveBeenCalledExactlyOnceWith({
        category: "comics/series", sourceName, title: "악녀는 모래시계를 되돌린다", author: "",
      }));
      expect(screen.getByLabelText("신규 이름").value).toBe(`악녀는 모래시계를 되돌린다 (${expectedContents})`);
      fireEvent.change(screen.getByLabelText("목차"), { target: { value: "수정" } });
      fireEvent.click(screen.getByRole("button", { name: "복원" }));
      expect(screen.getByLabelText("목차").value).toBe(expectedContents);
      fireEvent.click(screen.getByText("이동"));
      expect(values.onMove).toHaveBeenCalledWith(`악녀는 모래시계를 되돌린다 (${expectedContents})`);
    },
  );

  it.each(["API 오류", "전송 오류"])("명시적 범위는 통계 조회 %s에도 분리한다", async (failure) => {
    mockRawJsonGetReq.mockImplementation((_url, success, error) => failure === "API 오류"
      ? success({ status: "error" }) : error());
    const onMetadataReady = vi.fn();
    const sourceName = "악녀는 모래시계를 되돌린다 1-44화";
    render(<DirectoryEditPanel {...props({ directory: { category: "comics/series", name: sourceName }, onMetadataReady })} />);

    await waitFor(() => expect(onMetadataReady).toHaveBeenCalledExactlyOnceWith({
      category: "comics/series", sourceName, title: "악녀는 모래시계를 되돌린다", author: "",
    }));
    expect(screen.getByLabelText("목차").value).toBe("1-44화");
  });

  it.each(["1 7권", "1-44", "44화"])("완결 없는 불명확한 범위 %s와 파일 수가 다르면 제목을 유지한다", (suffix) => {
    const sourceName = `터무니 없는 스킬로 이세계 방랑밥 ${suffix}`;
    render(<DirectoryEditPanel {...props({ directory: { category: "comics/series", name: sourceName } })} />);
    expect(screen.getByLabelText("제목").value).toBe(sourceName);
    expect(screen.getByLabelText("목차").value).toBe("");
  });

  it.each(["(01-05)", "(1-5)", "(1~5)", "( 01 - 05 )"])(
    "단위 없는 괄호 범위 %s를 목차로 분리한다",
    (suffix) => {
      const sourceName = `미녀는 야수 ${suffix}`;
      const expectedContents = suffix.replace(/[()\s]/g, "");
      const values = props({ directory: { category: "comics/series", name: sourceName } });
      render(<DirectoryEditPanel {...values} />);

      expect(screen.getByLabelText("제목").value).toBe("미녀는 야수");
      expect(screen.getByLabelText("목차").value).toBe(expectedContents);
      expect(screen.getByLabelText("신규 이름").value).toBe(`미녀는 야수 (${expectedContents})`);
    },
  );

  it.each(["(2020-2021)", "(05-01)", "(5-5)", "(1234-1240)"])(
    "연도 범위나 순서가 맞지 않는 괄호 숫자 %s는 제목으로 유지한다",
    (suffix) => {
      const sourceName = `미녀는 야수 ${suffix}`;
      render(<DirectoryEditPanel {...props({ directory: { category: "comics/series", name: sourceName } })} />);
      expect(screen.getByLabelText("제목").value).toBe(sourceName);
      expect(screen.getByLabelText("목차").value).toBe("");
    },
  );

  it.each(["제목", "목차", "신규 이름"])("완결 표기 없는 범위를 보정해도 편집한 %s를 보존한다", async (label) => {
    let completeStats;
    mockRawJsonGetReq.mockImplementation((_url, success) => { completeStats = success; });
    render(<DirectoryEditPanel {...props({ directory: { category: "comics/series", name: "터무니 없는 스킬로 이세계 방랑밥 1 7권" } })} />);
    fireEvent.change(screen.getByLabelText(label), { target: { value: "사용자 편집" } });
    act(() => completeStats({ status: "success", result: { file_count: 7, page_count: 100, total_file_size: 5000 } }));
    await waitFor(() => expect(screen.getByLabelText(label).value).toBe("사용자 편집"));
    if (label === "제목") expect(screen.getByLabelText("목차").value).toBe("1-7권");
  });

  it("단일 화수와 완결 표기를 목차로 분리한다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, success) =>
      success({
        status: "success",
        result: { file_count: 122, page_count: 120, total_file_size: 5000 },
      }),
    );
    render(
      <DirectoryEditPanel
        {...props({
          directory: {
            category: "comics/series",
            name: "[홍길동] 테스트 만화 123화 완결",
          },
        })}
      />,
    );

    await waitFor(() => {
      expect(screen.getByLabelText("제목").value).toBe("테스트 만화");
      expect(screen.getByLabelText("목차").value).toBe("123화 완");
    });
  });

  it("단일 화수가 파일 수와 같으면 1부터의 범위로 보정한다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, success) =>
      success({
        status: "success",
        result: { file_count: 123, page_count: 120, total_file_size: 5000 },
      }),
    );
    render(
      <DirectoryEditPanel
        {...props({
          directory: {
            category: "comics/series",
            name: "[홍길동] 테스트 만화 123화 완결",
          },
        })}
      />,
    );

    await waitFor(() => {
      expect(screen.getByLabelText("목차").value).toBe("1~123화 완");
      expect(screen.getByLabelText("신규 이름").value).toBe(
        "[홍길동] 테스트 만화 (1~123화 완)",
      );
    });
  });

  it.each([
    ["1102화 완결", 102, "1-102화 완"],
    ["1102화 미완", 102, "1-102화 미완"],
    ["1102화 完", 102, "1-102화 완"],
    ["1102화 완결", 101, "1102화 완"],
    ["2102화 완결", 102, "2102화 완"],
    ["1102화 완결", 1102, "1~1102화 완"],
  ])("붙은 화수 %s와 파일 수 %i를 목차 %s로 처리한다", async (originalContents, fileCount, expectedContents) => {
    let resolveStats;
    mockRawJsonGetReq.mockImplementation((_url, success) => {
      resolveStats = success;
    });
    const title = "대위님! 이번 전쟁터는 이곳인가요？";
    const values = props({
      directory: {
        category: "comics/series",
        name: `${title} ${originalContents}`,
      },
    });
    render(<DirectoryEditPanel {...values} />);

    expect(screen.getByLabelText("목차").value).toBe(originalContents.replace(/(?:완결|完)$/, "완"));
    act(() => resolveStats({
      status: "success",
      result: { file_count: fileCount, page_count: 120, total_file_size: 5000 },
    }));

    await waitFor(() => {
      expect(screen.getByLabelText("제목").value).toBe(title);
      expect(screen.getByLabelText("목차").value).toBe(expectedContents);
      expect(screen.getByLabelText("신규 이름").value).toBe(
        `${title} (${expectedContents})`,
      );
    });
    fireEvent.change(screen.getByLabelText("목차"), { target: { value: "수정" } });
    fireEvent.click(screen.getByRole("button", { name: "복원" }));
    expect(screen.getByLabelText("목차").value).toBe(expectedContents);
    fireEvent.click(screen.getByText("이동"));
    expect(values.onMove).toHaveBeenCalledWith(`${title} (${expectedContents})`);
  });

  it.each([
    ["1 65화 외전, 특별편, 후기 포함 완 캡", "1-65화 완외"],
    ["1-65화 완결 외전 포함", "1-65화 완외"],
    ["1~65회 후기 포함 完", "1~65회 완외"],
    ["(1-65권 완 특별편 포함)", "1-65권 완외"],
    ["1-65화 [완] (외전 포함)", "1-65화 완외"],
    ["(1-65화 완외)", "1-65화 완외"],
    ["(1-65화) [완외]", "1-65화 완외"],
    ["165화 외전 포함 완", "1-65화 완외"],
  ])("완결과 추가 회차 정보 %s를 %s로 통일한다", async (originalContents, expectedContents) => {
    mockRawJsonGetReq.mockImplementation((_url, success) => success({
      status: "success",
      result: { file_count: 65, page_count: 120, total_file_size: 5000 },
    }));
    const values = props({
      directory: {
        category: "comics/series",
        name: `동트는 로맨스 ${originalContents}`,
      },
    });
    render(<DirectoryEditPanel {...values} />);

    await waitFor(() => {
      expect(screen.getByLabelText("저자").value).toBe("");
      expect(screen.getByLabelText("제목").value).toBe("동트는 로맨스");
      expect(screen.getByLabelText("목차").value).toBe(expectedContents);
      expect(screen.getByLabelText("신규 이름").value).toBe(
        `동트는 로맨스 (${expectedContents})`,
      );
    });
    fireEvent.change(screen.getByLabelText("목차"), { target: { value: "수정" } });
    fireEvent.click(screen.getByRole("button", { name: "복원" }));
    expect(screen.getByLabelText("목차").value).toBe(expectedContents);
    fireEvent.click(screen.getByText("이동"));
    expect(values.onMove).toHaveBeenCalledWith(`동트는 로맨스 (${expectedContents})`);
  });

  it.each(["외포 완 툰", "외포  완 툰", "외포완 툰"])("외전 완결 표기 %s를 완외로 분리한다", async (suffix) => {
    const sourceName = `써클 트랩 1-22화 ${suffix}`;
    const onMetadataReady = vi.fn();
    const values = props({
      directory: { category: "comics/series", name: sourceName },
      onMetadataReady,
    });
    render(<DirectoryEditPanel {...values} />);

    expect(screen.getByLabelText("제목").value).toBe("써클 트랩");
    expect(screen.getByLabelText("목차").value).toBe("1-22화 완외");
    expect(screen.getByLabelText("신규 이름").value).toBe("써클 트랩 (1-22화 완외)");
    await waitFor(() => expect(onMetadataReady).toHaveBeenCalledExactlyOnceWith({
      category: "comics/series", sourceName, title: "써클 트랩", author: "",
    }));
    fireEvent.change(screen.getByLabelText("목차"), { target: { value: "수정" } });
    fireEvent.click(screen.getByRole("button", { name: "복원" }));
    expect(screen.getByLabelText("목차").value).toBe("1-22화 완외");
    fireEvent.click(screen.getByText("이동"));
    expect(values.onMove).toHaveBeenCalledWith("써클 트랩 (1-22화 완외)");
  });

  it.each([
    "1-65화 외전 포함 미완",
    "1-65화 특별편 포함",
    "1-65화 후기 포함 완전판",
    "1-65화 외포 미완 툰",
    "1-65화 외포 툰",
  ])("완결 표기가 없는 %s를 완외로 오인하지 않는다", (suffix) => {
    render(<DirectoryEditPanel {...props({
      directory: { category: "comics/series", name: `동트는 로맨스 ${suffix}` },
    })} />);

    expect(screen.getByLabelText("목차").value).toBe("");
    expect(screen.getByLabelText("신규 이름").value).not.toContain("완외");
  });

  it("제목의 외전 표기만으로 목차를 완외로 바꾸지 않는다", () => {
    render(<DirectoryEditPanel {...props({
      directory: { category: "comics/series", name: "외전 로맨스 (1-65화 완)" },
    })} />);

    expect(screen.getByLabelText("제목").value).toBe("외전 로맨스");
    expect(screen.getByLabelText("목차").value).toBe("1-65화 완");
  });

  it.each([
    [["comic/series/두근두근 연극부 01화.pdf", "comic/series/후기.pdf"], "1-35화 완외"],
    [["comic/series/두근두근 연극부 01권.pdf"], "1-35권 완외"],
    [["comic/35권/두근두근 연극부 01화.pdf"], "1-35화 완외"],
    [["comic/series/두근두근 연극부 01.pdf"], "1-35 완외"],
    [["comic/series/01화.pdf", "comic/series/01권.pdf"], "1-35 완외"],
    [["comic/series/01권 01화.pdf"], "1-35 완외"],
  ])("단위 없는 범위는 내부 파일 이름 %j로 %s를 결정한다", async (filePaths, expectedContents) => {
    mockRawJsonGetReq.mockImplementation((url, success) => success({
      status: "success",
      result: url.includes("category-pdf-stats")
        ? { file_count: 35, page_count: 120, total_file_size: 5000 }
        : filePaths.map((file_path) => ({ file_path, title: "파일 이름 대신 쓰면 안 되는 01권" })),
    }));
    const values = props({
      directory: {
        category: "comics/series",
        name: "두근두근 연극부 1-35 완 + 후기 캡",
      },
    });
    render(<DirectoryEditPanel {...values} />);

    await waitFor(() => {
      expect(screen.getByLabelText("제목").value).toBe("두근두근 연극부");
      expect(screen.getByLabelText("목차").value).toBe(expectedContents);
      expect(screen.getByLabelText("신규 이름").value).toBe(
        `두근두근 연극부 (${expectedContents})`,
      );
    });
    fireEvent.change(screen.getByLabelText("목차"), { target: { value: "수정" } });
    fireEvent.click(screen.getByRole("button", { name: "복원" }));
    expect(screen.getByLabelText("목차").value).toBe(expectedContents);
    fireEvent.click(screen.getByText("이동"));
    expect(values.onMove).toHaveBeenCalledWith(`두근두근 연극부 (${expectedContents})`);
  });

  it("다음 페이지까지 확인하고 누락된 단위를 결정한다", async () => {
    mockRawJsonGetReq.mockImplementation((url, success) => {
      if (url.includes("category-pdf-stats")) {
        success({ status: "success", result: { file_count: 35, page_count: 120, total_file_size: 5000 } });
      } else if (url.includes("cursor=")) {
        success({ status: "success", result: [{ file_path: "comics/series/02화.pdf" }] });
      } else {
        success({ status: "success", result: [{ file_path: "comics/series/01.pdf" }], next_cursor: "page/2+=" });
      }
    });
    render(<DirectoryEditPanel {...props({
      directory: { category: "comics/series", name: "두근두근 연극부 (1-35 완)" },
    })} />);

    await waitFor(() => expect(screen.getByLabelText("목차").value).toBe("1-35화 완"));
    expect(mockRawJsonGetReq.mock.calls.some(([url]) => url.includes("cursor=page%2F2%2B%3D"))).toBe(true);
  });

  it("본편 파일에 단위가 없으면 다음 페이지의 외전 파일로 단위를 결정한다", async () => {
    const sourceName = "바른연애 길잡이 1-159 외포완 + 후기 캡";
    const category = `9_temp/${sourceName}`;
    const onMetadataReady = vi.fn();
    let completeExtras;
    mockRawJsonGetReq.mockImplementation((url, success) => {
      if (url.includes("category-pdf-stats")) {
        success({ status: "success", result: { file_count: 159, page_count: 100, total_file_size: 5000 } });
      } else if (url.includes("cursor=")) {
        completeExtras = success;
      } else {
        success({
          status: "success",
          result: [{ file_path: `${category}/[0001] 0001.pdf` }],
          next_cursor: "extras/2+=",
        });
      }
    });
    const values = props({ directory: { category, name: sourceName }, onMetadataReady });
    render(<DirectoryEditPanel {...values} />);

    expect(screen.getByLabelText("제목").value).toBe("바른연애 길잡이");
    expect(screen.getByLabelText("목차").value).toBe("1-159 완외");
    expect(onMetadataReady).not.toHaveBeenCalled();
    expect(mockRawJsonGetReq).toHaveBeenCalledWith(
      `/comics/categories/9_temp/${encodeURIComponent(sourceName)}?limit=5000&cursor=extras%2F2%2B%3D`,
      expect.any(Function), expect.any(Function),
    );

    act(() => completeExtras({
      status: "success",
      result: [
        { file_path: `${category}/[0153] 외전 0001화.pdf` },
        { file_path: `${category}/[0159] 후기.pdf` },
      ],
    }));

    await waitFor(() => expect(screen.getByLabelText("목차").value).toBe("1-159화 완외"));
    expect(screen.getByLabelText("신규 이름").value).toBe("바른연애 길잡이 (1-159화 완외)");
    expect(onMetadataReady).toHaveBeenCalledExactlyOnceWith({ category, sourceName, title: "바른연애 길잡이", author: "" });
    fireEvent.click(screen.getByText("이동"));
    expect(values.onMove).toHaveBeenCalledWith("바른연애 길잡이 (1-159화 완외)");
  });

  it.each(["제목", "목차", "신규 이름"])("단위 조회가 늦어도 편집한 %s를 보존한다", async (label) => {
    let resolveFiles;
    mockRawJsonGetReq.mockImplementation((url, success) => {
      if (url.includes("category-pdf-stats")) {
        success({ status: "success", result: { file_count: 35, page_count: 120, total_file_size: 5000 } });
      } else {
        resolveFiles = success;
      }
    });
    render(<DirectoryEditPanel {...props({
      directory: { category: "comics/series", name: "두근두근 연극부 1-35 완 + 후기 캡" },
    })} />);
    fireEvent.change(screen.getByLabelText(label), { target: { value: "사용자 편집" } });
    act(() => resolveFiles({ status: "success", result: [{ file_path: "comics/series/01화.pdf" }] }));

    await waitFor(() => expect(screen.getByLabelText(label).value).toBe("사용자 편집"));
    if (label === "제목") {
      expect(screen.getByLabelText("목차").value).toBe("1-35화 완외");
      expect(screen.getByLabelText("신규 이름").value).toBe("사용자 편집 (1-35화 완외)");
    }
  });

  it("이미 단위가 있는 범위는 파일 목록을 조회하지 않는다", () => {
    render(<DirectoryEditPanel {...props({
      directory: { category: "comics/series", name: "두근두근 연극부 (1-35권 완외)" },
    })} />);

    expect(screen.getByLabelText("목차").value).toBe("1-35권 완외");
    expect(mockRawJsonGetReq).toHaveBeenCalledTimes(1);
  });

  it.each(["API 오류", "전송 오류"])("단위 조회 %s에서는 단위를 추측하지 않는다", async (failureType) => {
    mockRawJsonGetReq.mockImplementation((url, success, failure) => {
      if (url.includes("category-pdf-stats")) {
        success({ status: "success", result: { file_count: 35, page_count: 120, total_file_size: 5000 } });
      } else if (failureType === "API 오류") {
        success({ status: "failure", error: "조회 오류" });
      } else {
        failure();
      }
    });
    render(<DirectoryEditPanel {...props({
      directory: { category: "comics/series", name: "두근두근 연극부 1-35 완 + 후기 캡" },
    })} />);

    await waitFor(() => expect(screen.getByLabelText("목차").value).toBe("1-35 완외"));
  });

  it.each(["1-8권完 캡", "1-8권완 캡", "1-8권완결 캡", "(1-8권完)"])(
    "단위에 붙은 완결 표기 %s를 목차로 분리한다",
    (suffix) => {
      const values = props({ directory: { category: "comics/series", name: `사상 최강의 3류 만화가 ${suffix}` } });
      render(<DirectoryEditPanel {...values} />);

      expect(screen.getByLabelText("제목").value).toBe("사상 최강의 3류 만화가");
      expect(screen.getByLabelText("목차").value).toBe("1-8권 완");
      expect(screen.getByLabelText("신규 이름").value).toBe("사상 최강의 3류 만화가 (1-8권 완)");
      fireEvent.click(screen.getByText("이동"));
      expect(values.onMove).toHaveBeenCalledWith("사상 최강의 3류 만화가 (1-8권 완)");
    },
  );

  it.each(["1~10화[완결]", "1~10화[完]", "(1~10화 완결)", "1~10화 완결"])(
    "완결 표기 %s를 완으로 통일한다",
    (suffix) => {
      const values = props({
        directory: { category: "comics/series", name: `라크리모사 ${suffix}` },
      });
      render(<DirectoryEditPanel {...values} />);

      expect(screen.getByLabelText("저자").value).toBe("");
      expect(screen.getByLabelText("제목").value).toBe("라크리모사");
      expect(screen.getByLabelText("목차").value).toBe("1~10화 완");
      expect(screen.getByLabelText("신규 이름").value).toBe("라크리모사 (1~10화 완)");
      fireEvent.click(screen.getByText("이동"));
      expect(values.onMove).toHaveBeenCalledWith("라크리모사 (1~10화 완)");
    },
  );

  it.each(["1_fiction", "2_science", "일반_디렉토리"])(
    "목차 정보가 없는 디렉토리 %s의 밑줄을 보존한다",
    (originalName) => {
      render(<DirectoryEditPanel {...props({
        directory: { category: "comics/series", name: originalName },
      })} />);

      expect(screen.getByLabelText("제목").value).toBe(originalName);
      expect(screen.getByLabelText("신규 이름").value).toBe(originalName);
    },
  );

  it.each([
    ["만화_정구미_노란구미의_돈까스_취업_1_2권_完", "", "정구미 노란구미의 돈까스 취업", "1~2권 완"],
    ["정구미_노란구미의_돈까스_취업_1_2권_完", "", "정구미 노란구미의 돈까스 취업", "1~2권 완"],
    ["만화_[정구미]_노란구미의_돈까스_취업_1_2권_完", "정구미", "노란구미의 돈까스 취업", "1~2권 완"],
    ["만화의_시작_1_2권_완", "", "만화의 시작", "1~2권 완"],
    ["정구미_만화_취업_1_2권_完", "", "정구미 만화 취업", "1~2권 완"],
    ["만화_정구미_노란구미의_돈까스_취업_1-2권_完", "", "정구미 노란구미의 돈까스 취업", "1-2권 완"],
  ])("밑줄 형식 %s에서 접두어와 이름 구성 요소를 정리한다", (originalName, author, title, contents) => {
    const values = props({
      directory: { category: "comics/series", name: originalName },
    });
    render(<DirectoryEditPanel {...values} />);
    const newName = `${author ? `[${author}] ` : ""}${title} (${contents})`;

    expect(screen.getByLabelText("저자").value).toBe(author);
    expect(screen.getByLabelText("제목").value).toBe(title);
    expect(screen.getByLabelText("목차").value).toBe(contents);
    expect(screen.getByLabelText("신규 이름").value).toBe(newName);
    fireEvent.change(screen.getByLabelText("제목"), { target: { value: "수정" } });
    fireEvent.click(screen.getByRole("button", { name: "복원" }));
    expect(screen.getByLabelText("신규 이름").value).toBe(newName);
    fireEvent.click(screen.getByText("이동"));
    expect(values.onMove).toHaveBeenCalledWith(newName);
  });

  it("변경한 구성 요소로 조합한 이름을 이동 대상에 전달한다", () => {
    const values = props();
    render(<DirectoryEditPanel {...values} />);

    fireEvent.change(screen.getByLabelText("저자"), {
      target: { value: "김작가" },
    });
    fireEvent.change(screen.getByLabelText("제목"), {
      target: { value: "새 만화" },
    });
    fireEvent.change(screen.getByLabelText("판본"), {
      target: { value: "완전판" },
    });
    fireEvent.change(screen.getByLabelText("목차"), {
      target: { value: "1-20권 완" },
    });
    fireEvent.click(screen.getByText("이동"));

    expect(values.onMove).toHaveBeenCalledWith(
      "[김작가] 새 만화 (완전판) (1-20권 완)",
    );
  });

  it("권·화 정보만 있는 괄호를 목차로 분리한다", () => {
    render(
      <DirectoryEditPanel
        {...props({
          directory: {
            category: "comics/series",
            name: "[홍길동] 테스트 만화 (1-10권 완)",
          },
        })}
      />,
    );

    expect(screen.getByLabelText("판본").value).toBe("");
    expect(screen.getByLabelText("목차").value).toBe("1-10권 완");
  });

  it("단일 판본 정보를 목차로 오인하지 않는다", () => {
    render(
      <DirectoryEditPanel
        {...props({
          directory: {
            category: "comics/series",
            name: "[홍길동] 테스트 만화 (완전판)",
          },
        })}
      />,
    );

    expect(screen.getByLabelText("판본").value).toBe("완전판");
    expect(screen.getByLabelText("목차").value).toBe("");
  });

  it.each(["갠소", "개인소장", "공유금지", "공금"])(
    "비허용 판본 텍스트 %s는 제목에 남기고 판본을 비운다",
    (label) => {
      render(
        <DirectoryEditPanel
          {...props({
            directory: {
              category: "comics/series",
              name: `[홍길동] 테스트 만화 (${label}) (1-10권 완)`,
            },
          })}
        />,
      );

      expect(screen.getByLabelText("제목").value).toBe(`테스트 만화 (${label})`);
      expect(screen.getByLabelText("판본").value).toBe("");
      expect(screen.getByLabelText("목차").value).toBe("1-10권 완");
    },
  );

  it.each([
    [
      "API 오류 메시지",
      (_url, success) => success({ status: "error", error: "통계 오류" }),
      "통계 오류",
    ],
    [
      "메시지가 없는 API 오류",
      (_url, success) => success({ status: "error" }),
      "PDF 통계를 불러오지 못했습니다.",
    ],
    [
      "전송 오류",
      (_url, _success, failure) => failure(),
      "PDF 통계를 불러오지 못했습니다.",
    ],
  ])("통계 조회 %s를 표시한다", async (_label, implementation, message) => {
    mockRawJsonGetReq.mockImplementation(implementation);
    render(<DirectoryEditPanel {...props()} />);
    expect(await screen.findByText(message)).toBeTruthy();
  });

  it("이름이 비었거나 경로 구분자가 있으면 변경을 거부한다", () => {
    const values = props();
    render(<DirectoryEditPanel {...values} />);
    const name = screen.getByLabelText("신규 이름");
    fireEvent.change(name, { target: { value: " /invalid " } });
    fireEvent.click(screen.getByRole("button", { name: "변경" }));
    expect(values.onError).toHaveBeenCalledWith(
      "디렉토리 이름을 입력하세요. 이름에는 '/'를 사용할 수 없습니다.",
    );

    fireEvent.change(name, { target: { value: "   " } });
    expect(screen.getByRole("button", { name: "변경" }).disabled).toBe(true);
    expect(mockJsonPutReq).not.toHaveBeenCalled();
  });

  it("같은 이름은 요청하지 않고 유효한 이름은 부모 경로 안에서 변경한다", () => {
    const values = props();
    render(<DirectoryEditPanel {...values} />);
    fireEvent.click(screen.getByRole("button", { name: "변경" }));
    expect(mockJsonPutReq).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText("신규 이름"), {
      target: { value: "  volume  " },
    });
    mockJsonPutReq.mockImplementation((_url, _body, success, _failure, done) => {
      success();
      done();
    });
    fireEvent.click(screen.getByRole("button", { name: "변경" }));
    expect(mockJsonPutReq).toHaveBeenCalledWith(
      "/comics/categories/rename",
      { old_category: "comics/series", new_category: "comics/volume" },
      expect.any(Function),
      expect.any(Function),
      expect.any(Function),
    );
    expect(values.onComplete).toHaveBeenCalledWith(
      "디렉토리 이름을 변경했습니다.",
      { type: "rename", category: "comics/volume" },
    );
  });

  it("최상위 디렉토리 이름 변경 실패를 표시한다", () => {
    const values = props({ directory: { category: "series", name: "series" } });
    mockJsonPutReq.mockImplementation((_url, _body, _success, failure, done) => {
      failure("충돌");
      done();
    });
    render(<DirectoryEditPanel {...values} />);
    fireEvent.change(screen.getByLabelText("신규 이름"), {
      target: { value: "new-series" },
    });
    fireEvent.click(screen.getByRole("button", { name: "변경" }));
    expect(mockJsonPutReq).toHaveBeenCalledWith(
      "/comics/categories/rename",
      { old_category: "series", new_category: "new-series" },
      expect.any(Function),
      expect.any(Function),
      expect.any(Function),
    );
    expect(values.onError).toHaveBeenCalledWith(
      "디렉토리 변경에 실패했습니다. 충돌",
    );
  });

  it("이름 변경 요청 중에는 중복 제출을 막는다", () => {
    mockJsonPutReq.mockImplementation(() => {});
    render(<DirectoryEditPanel {...props()} />);
    fireEvent.change(screen.getByLabelText("신규 이름"), {
      target: { value: "new-series" },
    });
    fireEvent.click(screen.getByRole("button", { name: "변경" }));
    expect(screen.getByText("처리 중...").disabled).toBe(true);
    expect(screen.getByText("삭제").disabled).toBe(true);
  });

  it("삭제 취소 시 요청하지 않고 삭제 성공을 부모에게 알린다", () => {
    const values = props();
    render(<DirectoryEditPanel {...values} />);
    vi.mocked(window.confirm).mockReturnValueOnce(false);
    fireEvent.click(screen.getByText("삭제"));
    expect(mockJsonPostReq).not.toHaveBeenCalled();

    mockJsonPostReq.mockImplementation((_url, _body, success, _failure, done) => {
      success();
      done();
    });
    fireEvent.click(screen.getByText("삭제"));
    expect(mockJsonPostReq).toHaveBeenCalledWith(
      "/comics/categories/delete",
      { category: "comics/series", delete_files: true },
      expect.any(Function),
      expect.any(Function),
      expect.any(Function),
    );
    expect(values.onComplete).toHaveBeenCalledWith(
      "디렉토리와 하위 파일을 삭제했습니다.",
      { type: "delete", category: "comics/series" },
    );
  });

  it("삭제 실패와 bulk rename 실패를 표시하고 bulk 성공 실패 항목을 나열한다", async () => {
    const values = props();
    render(<DirectoryEditPanel {...values} />);
    mockJsonPostReq.mockImplementation((_url, _body, _success, failure, done) => {
      failure();
      done();
    });
    fireEvent.click(screen.getByText("삭제"));
    expect(values.onError).toHaveBeenCalledWith(
      "디렉토리 삭제에 실패했습니다. undefined",
    );

    fireEvent.change(screen.getByPlaceholderText("파일 이름에 적용할 패턴"), {
      target: { value: "old" },
    });
    fireEvent.change(screen.getByPlaceholderText("정규표현식 치환 문자열"), {
      target: { value: "new" },
    });
    fireEvent.click(screen.getByText("파일 이름 변경"));
    expect(await screen.findByText("오류: 파일 이름 변경 요청에 실패했습니다.")).toBeTruthy();

    mockJsonPostReq.mockImplementation((_url, _body, success, _failure, done) => {
      success({
        changed_count: 2,
        failed_count: 1,
        failures: [{ file: "bad.pdf", error: "잠김" }],
      });
      done();
    });
    fireEvent.click(screen.getByText("파일 이름 변경"));
    expect(await screen.findByText("변경된 파일 2개, 실패 1개")).toBeTruthy();
    expect(screen.getByText("bad.pdf: 잠김")).toBeTruthy();
  });

  it("실패 없는 bulk rename을 표시한다", async () => {
    mockJsonPostReq.mockImplementation((_url, _body, success, _failure, done) => {
      success({ changed_count: 0 });
      done();
    });
    render(<DirectoryEditPanel {...props()} />);
    fireEvent.change(screen.getByPlaceholderText("파일 이름에 적용할 패턴"), {
      target: { value: "old" },
    });
    fireEvent.click(screen.getByText("파일 이름 변경"));
    expect(await screen.findByText("변경된 파일 0개")).toBeTruthy();
    expect(screen.queryByRole("listitem")).toBeNull();
  });

  it("일괄 변경 요청 중에는 입력과 재실행을 막는다", () => {
    mockJsonPostReq.mockImplementation(() => {});
    render(<DirectoryEditPanel {...props()} />);
    fireEvent.change(screen.getByPlaceholderText("파일 이름에 적용할 패턴"), {
      target: { value: "old" },
    });
    fireEvent.click(screen.getByText("파일 이름 변경"));
    expect(screen.getByText("변경 중...").disabled).toBe(true);
    expect(screen.getByPlaceholderText("파일 이름에 적용할 패턴").disabled).toBe(true);
  });

  describe("커버리지 경계", () => {
    const rangeDirectory = { category: "comics/series", name: "작품명 1-35 완" };

    it("저자 칸의 분할 버튼은 공백 기준으로 저자와 제목을 나눈다", () => {
      render(<DirectoryEditPanel {...props()} />);

      fireEvent.change(screen.getByLabelText("저자"), { target: { value: "김작가 새제목" } });
      fireEvent.click(screen.getAllByRole("button", { name: "분할" })[0]);

      expect(screen.getByLabelText("저자").value).toBe("김작가");
      expect(screen.getByLabelText("제목").value).toBe("새제목");
    });

    it("공백이 없어 나눌 수 없으면 분할 버튼은 값을 바꾸지 않는다", () => {
      render(<DirectoryEditPanel {...props()} />);

      fireEvent.change(screen.getByLabelText("저자"), { target: { value: "한단어" } });
      fireEvent.change(screen.getByLabelText("제목"), { target: { value: "단어" } });
      fireEvent.click(screen.getAllByRole("button", { name: "분할" })[0]);
      fireEvent.click(screen.getAllByRole("button", { name: "분할" })[1]);

      expect(screen.getByLabelText("저자").value).toBe("한단어");
      expect(screen.getByLabelText("제목").value).toBe("단어");
    });

    it("제목을 비우면 신규 이름에서 제목을 뺀다", () => {
      render(<DirectoryEditPanel {...props({ directory: { category: "comics/series", name: "[저자] 작품명" } })} />);

      fireEvent.change(screen.getByLabelText("제목"), { target: { value: "  " } });

      expect(screen.getByLabelText("신규 이름").value).toBe("[저자]");
    });

    it.each(["완결", "完", "외포완"])("완결 표기 %s를 표준 표기로 바꾼다", (completion) => {
      render(<DirectoryEditPanel {...props({ directory: { category: "comics/series", name: `작품명 1-35 ${completion}` } })} />);

      expect(screen.getByLabelText("목차").value).toMatch(/^1-35.* 완외?$/);
    });

    it("언마운트 뒤 늦게 도착한 통계·파일 응답은 무시한다", () => {
      const callbacks = [];
      mockRawJsonGetReq.mockImplementation((url, success, failure) => {
        callbacks.push({ url, success, failure });
      });
      const onMetadataReady = vi.fn();
      const { unmount } = render(<DirectoryEditPanel {...props({ directory: rangeDirectory, onMetadataReady })} />);
      expect(callbacks.length).toBeGreaterThanOrEqual(2);
      unmount();

      for (const { success, failure } of callbacks) {
        success({ status: "success", result: [], next_cursor: "" });
        failure();
      }
      expect(onMetadataReady).not.toHaveBeenCalled();
    });

    it("파일 목록 응답이 실패 상태이거나 배열이 아니면 단위 추론을 마친다", async () => {
      const onMetadataReady = vi.fn();
      mockRawJsonGetReq.mockImplementation((url, success) => {
        if (url.includes("category-pdf-stats")) success({ status: "error" });
        else success({ status: "error" });
      });
      render(<DirectoryEditPanel {...props({ directory: rangeDirectory, onMetadataReady })} />);

      await waitFor(() => expect(onMetadataReady).toHaveBeenCalledOnce());
    });

    it("file_path가 없는 파일은 단위 추론에서 건너뛴다", async () => {
      const onMetadataReady = vi.fn();
      mockRawJsonGetReq.mockImplementation((url, success) => {
        if (url.includes("category-pdf-stats")) success({ status: "success", result: { file_count: 3, page_count: 30, total_file_size: 100 } });
        else success({ status: "success", result: [{}, { file_path: "a/01화.pdf" }] });
      });
      render(<DirectoryEditPanel {...props({ directory: rangeDirectory, onMetadataReady })} />);

      await waitFor(() => expect(onMetadataReady).toHaveBeenCalledOnce());
    });
  });
});
