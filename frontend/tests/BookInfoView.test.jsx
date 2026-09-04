// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup, fireEvent } from "@testing-library/react";

afterEach(cleanup);

import BookInfoView from "../src/BookInfoView";

const defaultBookInfo = {
  title: "테스트 제목",
  author: "테스트 저자",
  file_type: "pdf",
  file_path: "/books/test.pdf",
  file_size: 1024000,
  line_count: 0,
  page_count: 150,
  isbn: "978-1234567890",
};

describe("BookInfoView", () => {
  it("기본 정보를 읽기 전용으로 표시한다", () => {
    render(<BookInfoView bookInfo={defaultBookInfo} />);
    expect(screen.getByDisplayValue("/books/test.pdf")).toBeTruthy();
    expect(screen.getByDisplayValue("pdf")).toBeTruthy();
    expect(screen.getByDisplayValue("978-1234567890")).toBeTruthy();
    expect(screen.getByDisplayValue("1,024,000")).toBeTruthy();
    expect(screen.getByDisplayValue("150쪽")).toBeTruthy();
  });

  it("pageCount가 0이고 lineCount가 있으면 행 수를 표시한다", () => {
    render(
      <BookInfoView
        bookInfo={{ ...defaultBookInfo, page_count: 0, line_count: 500 }}
      />,
    );
    expect(screen.getByDisplayValue("500행")).toBeTruthy();
  });

  it("pageCount와 lineCount가 모두 0이면 -를 표시한다", () => {
    render(
      <BookInfoView
        bookInfo={{ ...defaultBookInfo, page_count: 0, line_count: 0 }}
      />,
    );
    expect(screen.getByDisplayValue("-")).toBeTruthy();
  });

  it("isbn이 없으면 -를 표시한다", () => {
    render(<BookInfoView bookInfo={{ ...defaultBookInfo, isbn: "" }} />);
    const isbnField = screen.getAllByDisplayValue("-");
    expect(isbnField.length).toBeGreaterThanOrEqual(1);
  });

  it("bookInfo가 없으면 빈 객체로 대체하여 기본값으로 안전하게 렌더링한다", () => {
    render(<BookInfoView bookInfo={null} />);
    // '{}' 대체 후 title/author/fileType/filePath/fileSize 모두 fallback 값 사용
    expect(screen.getByDisplayValue("0")).toBeTruthy(); // fileSize 기본값
    const dashFields = screen.getAllByDisplayValue("-");
    expect(dashFields.length).toBeGreaterThanOrEqual(1); // isbn 및 분량 기본값
  });

  it("isEditEnabled=false일 때 저자/제목 입력 필드를 숨긴다", () => {
    render(<BookInfoView bookInfo={defaultBookInfo} isEditEnabled={false} />);
    expect(screen.queryByText("저자")).toBeNull();
    expect(screen.queryByText("제목")).toBeNull();
  });

  it("isEditEnabled=true일 때 저자/제목 입력 필드를 표시한다", () => {
    render(
      <BookInfoView
        bookInfo={defaultBookInfo}
        isEditEnabled={true}
        onAuthorChange={vi.fn()}
        onTitleChange={vi.fn()}
      />,
    );
    expect(screen.getByText("저자")).toBeTruthy();
    expect(screen.getByText("제목")).toBeTruthy();
    expect(screen.getByDisplayValue("테스트 저자")).toBeTruthy();
    expect(screen.getByDisplayValue("테스트 제목")).toBeTruthy();
  });

  it("저자 입력 변경 시 콜백을 호출한다", () => {
    const onAuthorChange = vi.fn();
    render(
      <BookInfoView
        bookInfo={defaultBookInfo}
        isEditEnabled={true}
        onAuthorChange={onAuthorChange}
        onTitleChange={vi.fn()}
      />,
    );
    fireEvent.change(screen.getByDisplayValue("테스트 저자"), {
      target: { value: "새 저자" },
    });
    expect(onAuthorChange).toHaveBeenCalled();
  });

  it("제목 입력 변경 시 콜백을 호출한다", () => {
    const onTitleChange = vi.fn();
    render(
      <BookInfoView
        bookInfo={defaultBookInfo}
        isEditEnabled={true}
        onAuthorChange={vi.fn()}
        onTitleChange={onTitleChange}
      />,
    );
    fireEvent.change(screen.getByDisplayValue("테스트 제목"), {
      target: { value: "새 제목" },
    });
    expect(onTitleChange).toHaveBeenCalled();
  });

  it("분할 버튼 클릭 시 콜백을 호출한다", () => {
    const onCutAuthorButtonClick = vi.fn();
    const onCutTitleButtonClick = vi.fn();
    render(
      <BookInfoView
        bookInfo={defaultBookInfo}
        isEditEnabled={true}
        onAuthorChange={vi.fn()}
        onTitleChange={vi.fn()}
        onCutAuthorButtonClick={onCutAuthorButtonClick}
        onCutTitleButtonClick={onCutTitleButtonClick}
      />,
    );
    const cutButtons = screen.getAllByText("분할");
    fireEvent.click(cutButtons[0]); // 저자 분할
    expect(onCutAuthorButtonClick).toHaveBeenCalled();
    fireEvent.click(cutButtons[1]); // 제목 분할
    expect(onCutTitleButtonClick).toHaveBeenCalled();
  });

  it("교환 버튼 클릭 시 콜백을 호출한다", () => {
    const onExchangeButtonClick = vi.fn();
    render(
      <BookInfoView
        bookInfo={defaultBookInfo}
        isEditEnabled={true}
        onAuthorChange={vi.fn()}
        onTitleChange={vi.fn()}
        onExchangeButtonClick={onExchangeButtonClick}
      />,
    );
    fireEvent.click(screen.getByText("교환"));
    expect(onExchangeButtonClick).toHaveBeenCalled();
  });

  it("복원 버튼 클릭 시 콜백을 호출한다", () => {
    const onResetButtonClick = vi.fn();
    render(
      <BookInfoView
        bookInfo={defaultBookInfo}
        isEditEnabled={true}
        onAuthorChange={vi.fn()}
        onTitleChange={vi.fn()}
        onResetButtonClick={onResetButtonClick}
      />,
    );
    fireEvent.click(screen.getByText("복원"));
    expect(onResetButtonClick).toHaveBeenCalled();
  });

  it("저자 IME compositionStart/compositionEnd 이벤트를 처리한다", () => {
    const onAuthorChange = vi.fn();
    render(
      <BookInfoView
        bookInfo={defaultBookInfo}
        isEditEnabled={true}
        onAuthorChange={onAuthorChange}
        onTitleChange={vi.fn()}
      />,
    );
    const authorInput = screen.getByDisplayValue("테스트 저자");

    // compositionStart 발생 → onChange 시 콜백 호출 안 함
    fireEvent.compositionStart(authorInput);
    onAuthorChange.mockClear();
    fireEvent.change(authorInput, { target: { value: "한글입력중" } });
    expect(onAuthorChange).not.toHaveBeenCalled();

    // compositionEnd 발생 → 콜백 호출
    fireEvent.compositionEnd(authorInput, { target: { value: "한글완성" } });
    expect(onAuthorChange).toHaveBeenCalled();
  });

  it("제목 IME compositionStart/compositionEnd 이벤트를 처리한다", () => {
    const onTitleChange = vi.fn();
    render(
      <BookInfoView
        bookInfo={defaultBookInfo}
        isEditEnabled={true}
        onAuthorChange={vi.fn()}
        onTitleChange={onTitleChange}
      />,
    );
    const titleInput = screen.getByDisplayValue("테스트 제목");

    // compositionStart 발생 → onChange 시 콜백 호출 안 함
    fireEvent.compositionStart(titleInput);
    onTitleChange.mockClear();
    fireEvent.change(titleInput, { target: { value: "제목입력중" } });
    expect(onTitleChange).not.toHaveBeenCalled();

    // compositionEnd 발생 → 콜백 호출
    fireEvent.compositionEnd(titleInput, { target: { value: "제목완성" } });
    expect(onTitleChange).toHaveBeenCalled();
  });
});
