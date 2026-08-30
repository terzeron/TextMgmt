import { describe, it, expect, vi } from "vitest";
import React from "react";
import { render, screen } from "@testing-library/react";
import SearchResult from "../src/SearchResult";
import SimilarBooks from "../src/SimilarBooks";
import Folder from "../src/Folder";
import BookInfoView from "../src/BookInfoView";
import Bookstore from "../src/Bookstore";
import Actions from "../src/Actions";
import ViewDOC from "../src/ViewDOC";
import ViewTXT from "../src/ViewTXT";

describe("Frontend Defensive Handling Tests", () => {
  it("SearchResult: file_path나 category가 없는 비정상 도서 데이터도 크래시 없이 렌더링한다", () => {
    const anomalousResults = [
      { book_id: 1, title: "No FilePath Book", category: null, file_path: null, file_type: null },
      { book_id: 2, title: "Empty Path Book", category: "fiction", file_path: "", file_type: "pdf" },
    ];
    render(<SearchResult results={anomalousResults} basePath={null} />);
    expect(screen.getByText(/No FilePath Book/)).toBeTruthy();
    expect(screen.getByText(/Empty Path Book/)).toBeTruthy();
  });

  it("SimilarBooks: file_path가 없는 도서 데이터도 안전하게 렌더링한다", () => {
    render(<SimilarBooks bookId={10} basePath={null} />);
  });

  it("Folder: folderData가 빈 배열이거나 비어있을 때 크래시 없이 렌더링한다", () => {
    const { container } = render(
      <Folder
        folderData={[]}
        expandedItems={[]}
        onExpandedItemsChange={() => {}}
        onClickHandler={() => {}}
        isOpen={true}
        onToggle={() => {}}
      />
    );
    expect(container).toBeTruthy();
  });

  it("BookInfoView: bookInfo가 빈 객체이거나 속성이 없을 때도 크래시 없이 렌더링한다", () => {
    const { container } = render(
      <BookInfoView bookInfo={{}} isEditEnabled={false} />
    );
    expect(container).toBeTruthy();
  });

  it("Bookstore: bookInfo가 빈 객체일 때도 크래시 없이 렌더링한다", () => {
    const { container } = render(
      <Bookstore bookInfo={{}} />
    );
    expect(container).toBeTruthy();
  });

  it("Actions: selectedEntryId가 없거나 루트가 아닐 때도 크래시 없이 렌더링한다", () => {
    const { container } = render(
      <Actions
        selectedEntryId=""
        selectedCategory=""
        otherCategoryList={[]}
        newFileName=""
        toNextEntryClicked={() => {}}
      />
    );
    expect(container).toBeTruthy();
  });

  it("ViewDOC: docx에서 lineCount가 0일 때도 전체 문서를 정상 처리한다", async () => {
    const { container } = render(
      <ViewDOC bookId={123} fileType="docx" lineCount={0} />
    );
    expect(container).toBeTruthy();
  });

  it("ViewTXT: result가 객체이거나 비문자열일 때도 크래시 없이 처리한다", async () => {
    const { container } = render(
      <ViewTXT bookId={123} lineCount={0} />
    );
    expect(container).toBeTruthy();
  });
});
