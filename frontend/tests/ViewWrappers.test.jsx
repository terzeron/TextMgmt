// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";

afterEach(cleanup);

vi.mock("../src/View", () => ({
  default: (props) => (
    <div data-testid="view" data-props={JSON.stringify(props)} />
  ),
}));

import BookView from "../src/BookView";
import ComicsView from "../src/ComicsView";

const viewProps = () => JSON.parse(screen.getByTestId("view").dataset.props);

describe("책·만화 보기 탭", () => {
  it("책 보기는 소설한국을 기본 디렉토리로 연다", () => {
    render(<BookView />);
    expect(viewProps()).toEqual({
      basePath: "/book-view",
      defaultDirectory: "2_소설한국",
    });
  });

  it("만화 보기는 완결을 기본 디렉토리로 연다", () => {
    render(<ComicsView />);
    expect(viewProps()).toEqual({
      basePath: "/comics-view",
      apiPrefix: "/comics",
      defaultDirectory: "5_완결",
    });
  });
});
