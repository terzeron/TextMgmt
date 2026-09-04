// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import ErrorBoundary from "../src/ErrorBoundary";
import * as clientLogger from "../src/clientLogger";

afterEach(cleanup);

function ProblemChild({
  shouldThrow = false,
  errorMessage = "Boom!",
  throwValue,
}) {
  if (throwValue !== undefined) {
    throw throwValue;
  }
  if (shouldThrow) {
    throw new Error(errorMessage);
  }
  return <div>Normal Content</div>;
}

describe("ErrorBoundary", () => {
  let reportSpy;
  let consoleErrorSpy;

  beforeEach(() => {
    reportSpy = vi
      .spyOn(clientLogger, "reportClientError")
      .mockReturnValue(true);
    // React's internal console.error suppression during error boundary test
    consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    reportSpy.mockRestore();
    consoleErrorSpy.mockRestore();
  });

  it("에러가 없을 때 정상적으로 자식 컴포넌트를 렌더링한다", () => {
    render(
      <ErrorBoundary>
        <ProblemChild shouldThrow={false} />
      </ErrorBoundary>,
    );

    expect(screen.getByText("Normal Content")).toBeTruthy();
    expect(reportSpy).not.toHaveBeenCalled();
  });

  it("자식 컴포넌트 렌더링 에러 발생 시 fallback UI를 표시하고 reportClientError를 호출한다", () => {
    render(
      <ErrorBoundary>
        <ProblemChild shouldThrow={true} errorMessage="Component exploded" />
      </ErrorBoundary>,
    );

    expect(screen.getByTestId("error-boundary-fallback")).toBeTruthy();
    expect(screen.getByText("화면 표시 중 오류가 발생했습니다")).toBeTruthy();
    expect(reportSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        errorType: "REACT_RENDER_ERROR",
        message: "Component exploded",
      }),
    );
  });

  it("상세 정보 버튼 클릭 시 에러 메시지와 스택을 토글한다", () => {
    render(
      <ErrorBoundary>
        <ProblemChild shouldThrow={true} errorMessage="Secret crash message" />
      </ErrorBoundary>,
    );

    const toggleBtn = screen.getByText("오류 상세 정보");
    fireEvent.click(toggleBtn);

    expect(screen.getAllByText(/Secret crash message/).length).toBeGreaterThan(
      0,
    );
    expect(screen.getByText("상세 정보 숨기기")).toBeTruthy();

    fireEvent.click(screen.getByText("상세 정보 숨기기"));
    expect(screen.queryByText("상세 정보 숨기기")).toBeNull();
  });

  it("다시 시도 버튼 클릭 시 상태를 리셋한다", () => {
    const onResetMock = vi.fn();
    const { rerender } = render(
      <ErrorBoundary onReset={onResetMock}>
        <ProblemChild shouldThrow={true} />
      </ErrorBoundary>,
    );

    expect(screen.getByTestId("error-boundary-fallback")).toBeTruthy();

    // Re-render with fixed child
    rerender(
      <ErrorBoundary onReset={onResetMock}>
        <ProblemChild shouldThrow={false} />
      </ErrorBoundary>,
    );

    fireEvent.click(screen.getByText("다시 시도"));
    expect(onResetMock).toHaveBeenCalled();
    expect(screen.getByText("Normal Content")).toBeTruthy();
  });

  it("onReset이 없으면 다시 시도 버튼 클릭 시 별도 콜백 없이 상태만 리셋한다", () => {
    const { rerender } = render(
      <ErrorBoundary>
        <ProblemChild shouldThrow={true} />
      </ErrorBoundary>,
    );

    expect(screen.getByTestId("error-boundary-fallback")).toBeTruthy();

    rerender(
      <ErrorBoundary>
        <ProblemChild shouldThrow={false} />
      </ErrorBoundary>,
    );

    expect(() => fireEvent.click(screen.getByText("다시 시도"))).not.toThrow();
    expect(screen.getByText("Normal Content")).toBeTruthy();
  });

  it("window.location이 없으면 새로고침/홈으로 이동 액션이 아무 동작도 하지 않는다", () => {
    const originalLocation = window.location;
    delete window.location;

    render(
      <ErrorBoundary>
        <ProblemChild shouldThrow={true} />
      </ErrorBoundary>,
    );

    expect(() => fireEvent.click(screen.getByText("새로고침"))).not.toThrow();
    expect(() =>
      fireEvent.click(screen.getByText("홈으로 이동")),
    ).not.toThrow();

    window.location = originalLocation;
  });

  it("커스텀 fallback 노드가 주어지면 기본 UI 대신 커스텀 fallback을 렌더링한다", () => {
    render(
      <ErrorBoundary fallback={<div>Custom Error View</div>}>
        <ProblemChild shouldThrow={true} />
      </ErrorBoundary>,
    );

    expect(screen.getByText("Custom Error View")).toBeTruthy();
  });

  it("커스텀 fallback 함수가 주어지면 함수 실행 결과를 렌더링한다", () => {
    const fallbackFn = (error) => <div>Fallback for: {error.message}</div>;
    render(
      <ErrorBoundary fallback={fallbackFn}>
        <ProblemChild shouldThrow={true} errorMessage="Custom Fn Error" />
      </ErrorBoundary>,
    );

    expect(screen.getByText("Fallback for: Custom Fn Error")).toBeTruthy();
  });

  it("onError 프로퍼티가 제공되면 에러 발생 시 호출되며 콜백 에러는 무음 처리된다", () => {
    const onErrorMock = vi.fn().mockImplementation(() => {
      throw new Error("callback error");
    });
    render(
      <ErrorBoundary onError={onErrorMock}>
        <ProblemChild shouldThrow={true} errorMessage="Trigger onError" />
      </ErrorBoundary>,
    );

    expect(onErrorMock).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("error-boundary-fallback")).toBeTruthy();
  });

  it("Error 인스턴스가 아닌 값을 throw하면 String(error)로 변환하고 상세정보는 기본값으로 표시한다", () => {
    render(
      <ErrorBoundary>
        <ProblemChild throwValue={{ code: 500 }} />
      </ErrorBoundary>,
    );

    expect(screen.getByTestId("error-boundary-fallback")).toBeTruthy();
    expect(reportSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        errorType: "REACT_RENDER_ERROR",
        message: String({ code: 500 }),
      }),
    );

    fireEvent.click(screen.getByText("오류 상세 정보"));
    // name/message가 없는 값이므로 "Error" / "Unknown error" 기본값이 표시된다
    expect(screen.getByText(/Error/).textContent).toContain("Unknown error");
  });

  it("onReset 프로퍼티 오류 시에도 안전하게 리셋 처리된다", () => {
    const onResetMock = vi.fn().mockImplementation(() => {
      throw new Error("reset callback error");
    });
    const { rerender } = render(
      <ErrorBoundary onReset={onResetMock}>
        <ProblemChild shouldThrow={true} />
      </ErrorBoundary>,
    );

    rerender(
      <ErrorBoundary onReset={onResetMock}>
        <ProblemChild shouldThrow={false} />
      </ErrorBoundary>,
    );

    fireEvent.click(screen.getByText("다시 시도"));
    expect(onResetMock).toHaveBeenCalled();
  });

  it("새로고침 및 홈으로 이동 버튼 클릭 시 정상 동작한다", () => {
    const reloadMock = vi.fn();
    delete window.location;
    window.location = { reload: reloadMock, href: "http://localhost:3000" };

    render(
      <ErrorBoundary>
        <ProblemChild shouldThrow={true} />
      </ErrorBoundary>,
    );

    fireEvent.click(screen.getByText("새로고침"));
    expect(reloadMock).toHaveBeenCalled();

    fireEvent.click(screen.getByText("홈으로 이동"));
    expect(window.location.href).toBe("/");
  });
});
