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

  it.each([
    "1-65화 외전 포함 미완",
    "1-65화 특별편 포함",
    "1-65화 후기 포함 완전판",
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
});
