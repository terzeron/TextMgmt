import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import SplitLayout from "../src/SplitLayout";

const KEY = "split:test";

function setInnerWidth(value) {
  Object.defineProperty(window, "innerWidth", {
    writable: true,
    configurable: true,
    value,
  });
}

function mockContainerWidth(width) {
  return vi
    .spyOn(HTMLElement.prototype, "getBoundingClientRect")
    .mockReturnValue({
      width,
      height: 600,
      top: 0,
      left: 0,
      right: width,
      bottom: 600,
    });
}

function renderLayout(props = {}, { left = true, right = true } = {}) {
  return render(
    <SplitLayout storageKey={KEY} defaultLeftWidth={280} {...props}>
      {left && <SplitLayout.Left>LEFT</SplitLayout.Left>}
      {right && <SplitLayout.Right>RIGHT</SplitLayout.Right>}
    </SplitLayout>,
  );
}

const handle = () => screen.queryByRole("separator");
const widthOf = () => Number(handle().getAttribute("aria-valuenow"));

describe("SplitLayout", () => {
  beforeEach(() => {
    setInnerWidth(1024);
    window.localStorage.clear();
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("두 pane과 핸들을 렌더한다", () => {
    const { container } = renderLayout();
    expect(container.querySelector(".split-layout-left").textContent).toBe(
      "LEFT",
    );
    expect(container.querySelector(".split-layout-right").textContent).toBe(
      "RIGHT",
    );
    expect(handle()).toBeTruthy();
    expect(widthOf()).toBe(280);
  });

  it("768px 미만에서는 쌓기 레이아웃이고 핸들이 없다", () => {
    setInnerWidth(500);
    const { container } = renderLayout();
    expect(container.querySelector(".split-layout.is-stacked")).toBeTruthy();
    expect(handle()).toBeNull();
  });

  it("드래그한 만큼 왼쪽 너비가 바뀐다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.pointerDown(handle(), { clientX: 300, pointerId: 1, button: 0 });
    fireEvent.pointerMove(handle(), { clientX: 340, pointerId: 1 });
    expect(widthOf()).toBe(320);
    fireEvent.pointerUp(handle(), { clientX: 340, pointerId: 1 });
    expect(widthOf()).toBe(320);
  });

  it("최소 200px와 최대 60%로 제한한다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.pointerDown(handle(), { clientX: 300, pointerId: 1, button: 0 });
    fireEvent.pointerMove(handle(), { clientX: -500, pointerId: 1 });
    expect(widthOf()).toBe(200);
    fireEvent.pointerMove(handle(), { clientX: 5000, pointerId: 1 });
    expect(widthOf()).toBe(600);
  });

  it("pointerup 뒤에는 이동해도 너비가 바뀌지 않고 pointercancel도 드래그를 끝낸다", () => {
    mockContainerWidth(1000);
    const { container } = renderLayout();
    fireEvent.pointerDown(handle(), { clientX: 300, pointerId: 1, button: 0 });
    expect(handle().className).toContain("is-dragging");
    fireEvent.pointerCancel(handle(), { pointerId: 1 });
    expect(handle().className).not.toContain("is-dragging");
    fireEvent.pointerMove(handle(), { clientX: 400, pointerId: 1 });
    expect(widthOf()).toBe(280);
    expect(container.querySelector(".split-layout")).toBeTruthy();
  });

  it("마우스 보조 버튼은 드래그를 시작하지 않는다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.pointerDown(handle(), {
      clientX: 300,
      pointerId: 1,
      button: 2,
      pointerType: "mouse",
    });
    fireEvent.pointerMove(handle(), { clientX: 400, pointerId: 1 });
    expect(widthOf()).toBe(280);
  });

  it("화살표 키로 16px씩 조절한다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.keyDown(handle(), { key: "ArrowRight" });
    expect(widthOf()).toBe(296);
    fireEvent.keyDown(handle(), { key: "ArrowLeft" });
    fireEvent.keyDown(handle(), { key: "ArrowLeft" });
    expect(widthOf()).toBe(264);
  });

  it("더블클릭하면 기본 너비로 돌아온다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.keyDown(handle(), { key: "ArrowRight" });
    fireEvent.doubleClick(handle());
    expect(widthOf()).toBe(280);
  });

  it("드래그가 끝나면 너비를 저장하고 다시 마운트하면 복원한다", () => {
    mockContainerWidth(1000);
    const first = renderLayout();
    fireEvent.pointerDown(handle(), { clientX: 300, pointerId: 1, button: 0 });
    fireEvent.pointerMove(handle(), { clientX: 360, pointerId: 1 });
    fireEvent.pointerUp(handle(), { pointerId: 1 });
    expect(window.localStorage.getItem(KEY)).toBe("340");
    first.unmount();
    renderLayout();
    expect(widthOf()).toBe(340);
  });

  it("저장된 너비가 컨테이너 최대값보다 크면 첫 조작에서 최대값 기준으로 줄어든다", () => {
    window.localStorage.setItem(KEY, "900");
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.keyDown(handle(), { key: "ArrowLeft" });
    expect(widthOf()).toBe(600);
  });

  it("localStorage가 막히거나 값이 숫자가 아니어도 기본 너비로 동작한다", () => {
    window.localStorage.setItem(KEY, "abc");
    renderLayout();
    expect(widthOf()).toBe(280);
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const { unmount } = renderLayout();
    mockContainerWidth(1000);
    fireEvent.keyDown(screen.getAllByRole("separator")[1], {
      key: "ArrowRight",
    });
    unmount();
  });

  it("왼쪽만 있으면 핸들 없이 저장된 너비를 유지한다", () => {
    const { container } = renderLayout({}, { right: false });
    expect(handle()).toBeNull();
    expect(
      container.querySelector(".split-layout").getAttribute("style"),
    ).toContain("280px");
  });

  it("오른쪽만 있으면 핸들과 왼쪽 pane이 없다", () => {
    const { container } = renderLayout({}, { left: false });
    expect(handle()).toBeNull();
    expect(container.querySelector(".split-layout-left")).toBeNull();
    expect(container.querySelector(".split-layout-right")).toBeTruthy();
  });

  it("scroll={false}면 no-scroll 클래스를 단다", () => {
    const { container } = renderLayout({ scroll: false });
    expect(container.querySelector(".split-layout.no-scroll")).toBeTruthy();
  });

  it("pane과 컨테이너에 추가 className을 적용한다", () => {
    const { container } = render(
      <SplitLayout storageKey={KEY} className="extra">
        <SplitLayout.Left className="l">A</SplitLayout.Left>
        <SplitLayout.Right className="r">B</SplitLayout.Right>
      </SplitLayout>,
    );
    expect(container.querySelector(".split-layout.extra")).toBeTruthy();
    expect(container.querySelector(".split-layout-left.l")).toBeTruthy();
    expect(container.querySelector(".split-layout-right.r")).toBeTruthy();
  });
});
