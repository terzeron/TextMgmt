import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
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
    <SplitLayout storageKey={KEY} defaultLeftRatio={0.28} {...props}>
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
    mockContainerWidth(1000);
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

  it("setPointerCapture가 예외를 던져도 드래그는 계속된다", () => {
    mockContainerWidth(1000);
    renderLayout();
    handle().setPointerCapture = () => {
      throw new DOMException("no active pointer", "NotFoundError");
    };
    fireEvent.pointerDown(handle(), { clientX: 300, pointerId: 9, button: 0 });
    fireEvent.pointerMove(handle(), { clientX: 340, pointerId: 9 });
    expect(widthOf()).toBe(320);
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

  it("저장된 너비가 컨테이너 최대값보다 크면 조작 없이도 최대값으로 보인다", () => {
    window.localStorage.setItem(KEY, "900");
    mockContainerWidth(1000);
    renderLayout();
    expect(widthOf()).toBe(600);
  });

  it("창이 줄면 보이는 너비와 값이 함께 줄고, 다시 커지면 원래 너비로 돌아온다", () => {
    window.localStorage.setItem(KEY, "700");
    const rect = mockContainerWidth(1600);
    renderLayout();
    expect(widthOf()).toBe(700);
    rect.mockReturnValue({ width: 1000, height: 600, top: 0, left: 0 });
    fireEvent(window, new Event("resize"));
    expect(widthOf()).toBe(600);
    // 줄어든 상태에서 첫 드래그가 바로 움직인다(죽은 구간 없음)
    fireEvent.pointerDown(handle(), { clientX: 600, pointerId: 1, button: 0 });
    fireEvent.pointerMove(handle(), { clientX: 560, pointerId: 1 });
    expect(widthOf()).toBe(560);
    fireEvent.pointerUp(handle(), { pointerId: 1 });
    rect.mockReturnValue({ width: 2000, height: 600, top: 0, left: 0 });
    fireEvent(window, new Event("resize"));
    expect(widthOf()).toBe(560);
  });

  it("컨테이너가 문서 위에서 떨어진 만큼 높이에서 뺀다", () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      width: 1000,
      height: 600,
      top: 38,
      left: 0,
    });
    const { container } = renderLayout();
    expect(
      container
        .querySelector(".split-layout")
        .style.getPropertyValue("--split-top"),
    ).toBe("38px");
  });

  it("scroll={false}면 위 간격을 측정해 쓰지 않는다", () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      width: 1000,
      height: 600,
      top: 38,
      left: 0,
    });
    const { container } = renderLayout({ scroll: false });
    expect(
      container
        .querySelector(".split-layout")
        .style.getPropertyValue("--split-top"),
    ).toBe("");
  });

  it("localStorage가 막히거나 값이 숫자가 아니어도 기본 너비로 동작한다", () => {
    window.localStorage.setItem(KEY, "abc");
    mockContainerWidth(1000);
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
    ).toContain("28%");
  });

  it("오른쪽만 있으면 핸들과 왼쪽 pane이 없다", () => {
    const { container } = renderLayout({}, { left: false });
    expect(handle()).toBeNull();
    expect(container.querySelector(".split-layout-left")).toBeNull();
    expect(container.querySelector(".split-layout-right")).toBeTruthy();
  });

  it("Left/Right가 아닌 자식도 grid 밖에 그대로 렌더한다", () => {
    const { container } = render(
      <SplitLayout storageKey={KEY}>
        <SplitLayout.Left>A</SplitLayout.Left>
        <SplitLayout.Right>B</SplitLayout.Right>
        <div data-testid="extra">EXTRA</div>
      </SplitLayout>,
    );
    const extra = screen.getByTestId("extra");
    expect(extra.textContent).toBe("EXTRA");
    expect(container.querySelector(".split-layout").contains(extra)).toBe(
      false,
    );
  });

  it("저장된 너비가 없으면 컨테이너 너비의 25%가 기본이고 창 크기를 따라간다", () => {
    const rect = mockContainerWidth(1000);
    render(
      <SplitLayout storageKey={KEY}>
        <SplitLayout.Left>A</SplitLayout.Left>
        <SplitLayout.Right>B</SplitLayout.Right>
      </SplitLayout>,
    );
    expect(widthOf()).toBe(250);
    rect.mockReturnValue({ width: 2000, height: 600, top: 0, left: 0 });
    fireEvent(window, new Event("resize"));
    expect(widthOf()).toBe(500);
  });

  it("기본 비율이 200px보다 작아지면 200px로 제한한다", () => {
    mockContainerWidth(600);
    render(
      <SplitLayout storageKey={KEY}>
        <SplitLayout.Left>A</SplitLayout.Left>
        <SplitLayout.Right>B</SplitLayout.Right>
      </SplitLayout>,
    );
    expect(widthOf()).toBe(200);
  });

  it("더블클릭하면 저장된 너비를 지우고 기본 비율로 돌아간다", () => {
    mockContainerWidth(1000);
    render(
      <SplitLayout storageKey={KEY}>
        <SplitLayout.Left>A</SplitLayout.Left>
        <SplitLayout.Right>B</SplitLayout.Right>
      </SplitLayout>,
    );
    fireEvent.keyDown(handle(), { key: "ArrowRight" });
    expect(window.localStorage.getItem(KEY)).toBe("266");
    fireEvent.doubleClick(handle());
    expect(window.localStorage.getItem(KEY)).toBeNull();
    expect(widthOf()).toBe(250);
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

  it("SplitLayout.Left/Right 는 자리 표시용이라 직접 호출하면 null 을 돌려준다", () => {
    expect(SplitLayout.Left()).toBeNull();
    expect(SplitLayout.Right()).toBeNull();
  });

  it("ResizeObserver 가 있으면 컨테이너와 body 를 관찰하고 크기 변화에 다시 잰다", () => {
    const instances = [];
    class MockResizeObserver {
      constructor(callback) {
        this.callback = callback;
        this.observe = vi.fn();
        this.disconnect = vi.fn();
        instances.push(this);
      }
    }
    vi.stubGlobal("ResizeObserver", MockResizeObserver);
    try {
      const spy = mockContainerWidth(1000);
      const { container, unmount } = renderLayout({ storageKey: undefined });
      expect(instances).toHaveLength(1);
      const [observer] = instances;
      expect(observer.observe).toHaveBeenCalledWith(container.querySelector(".split-layout"));
      expect(observer.observe).toHaveBeenCalledWith(document.body);
      expect(widthOf()).toBe(280);

      // 크기가 그대로면 다시 재도 값은 그대로다.
      act(() => observer.callback());
      expect(widthOf()).toBe(280);

      // 관찰 중인 요소 크기가 바뀌면 콜백이 너비를 다시 재서 반영한다.
      spy.mockReturnValue({ width: 2000, height: 600, top: 0, left: 0, right: 2000, bottom: 600 });
      act(() => observer.callback());
      expect(widthOf()).toBe(560);

      unmount();
      expect(observer.disconnect).toHaveBeenCalledTimes(1);
      // 언마운트 뒤 늦게 온 콜백은 컨테이너가 없어도 예외 없이 무시한다.
      expect(() => observer.callback()).not.toThrow();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("드래그 중이 아닐 때 pointerup 은 아무것도 저장하지 않는다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.pointerUp(handle(), { pointerId: 1 });
    expect(window.localStorage.getItem(KEY)).toBeNull();
    expect(widthOf()).toBe(280);
  });

  it("storageKey 가 없으면 더블클릭해도 기본 비율로 돌아오고 저장소를 건드리지 않는다", () => {
    mockContainerWidth(1000);
    const removeSpy = vi.spyOn(Storage.prototype, "removeItem");
    const setSpy = vi.spyOn(Storage.prototype, "setItem");
    renderLayout({ storageKey: undefined });
    fireEvent.keyDown(handle(), { key: "ArrowRight" });
    expect(widthOf()).toBe(296);
    fireEvent.doubleClick(handle());
    expect(widthOf()).toBe(280);
    expect(removeSpy).not.toHaveBeenCalled();
    expect(setSpy).not.toHaveBeenCalled();
  });

  it("화살표가 아닌 키는 너비를 바꾸지 않고 기본 동작도 막지 않는다", () => {
    mockContainerWidth(1000);
    renderLayout();
    const notPrevented = fireEvent.keyDown(handle(), { key: "Enter" });
    expect(notPrevented).toBe(true);
    expect(widthOf()).toBe(280);
    expect(window.localStorage.getItem(KEY)).toBeNull();
  });
});
