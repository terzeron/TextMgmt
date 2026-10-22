// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";

import PdfCropBox from "../src/PdfCropBox";

function mockLayerRect(width, height) {
  return vi
    .spyOn(HTMLElement.prototype, "getBoundingClientRect")
    .mockReturnValue({ width, height, top: 0, left: 0, right: width, bottom: height });
}

const RECT = { x0: 0.25, y0: 0.25, x1: 0.75, y1: 0.75 };
const boxesOf = (rect = RECT) => [{ id: "a", rect, color: "#00f" }];
const handle = (name) => document.querySelector(`[data-handle="${name}"]`);

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("PdfCropBox: 레이어 크기 측정", () => {
  it("ResizeObserver 가 있으면 레이어를 관찰하고 크기가 바뀌면 다시 재며 언마운트 때 해제한다", () => {
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
    const spy = mockLayerRect(200, 100);
    const rotated = boxesOf({ ...RECT, angle: 30 });
    const { unmount } = render(<PdfCropBox boxes={rotated} onChange={vi.fn()} />);

    const [observer] = instances;
    expect(instances).toHaveLength(1);
    expect(observer.observe).toHaveBeenCalledWith(screen.getByTestId("pdf-crop-layer"));
    // 크기를 알면 기울어진 박스는 clip-path 덮개를 쓴다.
    expect(screen.getByTestId("pdf-crop-dim-rotated")).toBeTruthy();

    // 같은 크기로 다시 재도 상태가 그대로이고 덮개도 유지된다.
    act(() => observer.callback());
    expect(screen.getByTestId("pdf-crop-dim-rotated")).toBeTruthy();

    // 크기가 0 이 되면 아직 못 잰 것으로 보고 조각 덮개로 돌아간다.
    spy.mockReturnValue({ width: 0, height: 0, top: 0, left: 0, right: 0, bottom: 0 });
    act(() => observer.callback());
    expect(screen.queryByTestId("pdf-crop-dim-rotated")).toBeNull();

    unmount();
    expect(observer.disconnect).toHaveBeenCalledTimes(1);
  });
});

describe("PdfCropBox: 포인터 조작", () => {
  it("드래그 중이 아니면 이동·종료 이벤트를 무시한다", () => {
    mockLayerRect(200, 100);
    const onChange = vi.fn();
    render(<PdfCropBox boxes={boxesOf()} onChange={onChange} />);

    fireEvent.pointerMove(handle("e"), { clientX: 50, clientY: 0, pointerId: 1 });
    fireEvent.pointerUp(handle("e"), { pointerId: 1 });
    fireEvent.pointerMove(screen.getByTestId("pdf-crop-rotate"), { clientX: 50, clientY: 0 });

    expect(onChange).not.toHaveBeenCalled();
  });

  it("레이어 크기를 아직 모르면(0x0) 드래그해도 onChange 를 부르지 않는다", () => {
    // jsdom 의 getBoundingClientRect 기본값은 0x0 이다.
    const onChange = vi.fn();
    render(<PdfCropBox boxes={boxesOf()} onChange={onChange} />);

    fireEvent.pointerDown(handle("e"), { clientX: 0, clientY: 0, pointerId: 1 });
    fireEvent.pointerMove(handle("e"), { clientX: 20, clientY: 0, pointerId: 1 });

    expect(onChange).not.toHaveBeenCalled();
  });

  it("크기 조절 핸들을 끌면 이동량만큼 변이 움직이고, 손을 떼면 더는 반응하지 않는다", () => {
    mockLayerRect(200, 100);
    const onChange = vi.fn();
    const setPointerCapture = vi.fn();
    const releasePointerCapture = vi.fn();
    render(<PdfCropBox boxes={boxesOf()} onChange={onChange} />);
    const east = handle("e");
    east.setPointerCapture = setPointerCapture;
    east.releasePointerCapture = releasePointerCapture;

    fireEvent.pointerDown(east, { clientX: 150, clientY: 50, pointerId: 7 });
    expect(setPointerCapture).toHaveBeenCalledWith(7);
    fireEvent.pointerMove(east, { clientX: 170, clientY: 50, pointerId: 7 });

    expect(onChange).toHaveBeenCalledTimes(1);
    const [id, next] = onChange.mock.calls[0];
    expect(id).toBe("a");
    // 오른쪽 변을 20px 끌면 폭 200px 기준 0.1 만큼 넓어진다.
    expect(next.x1).toBeCloseTo(0.85);
    expect(next.x0).toBeCloseTo(0.25);

    fireEvent.pointerUp(east, { pointerId: 7 });
    expect(releasePointerCapture).toHaveBeenCalledWith(7);
    fireEvent.pointerMove(east, { clientX: 190, clientY: 50, pointerId: 7 });
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("angle 이 없는 박스도 회전 핸들을 끌면 포인터 각도만큼 돌고, Shift 면 15도 단위로 맞춘다", () => {
    mockLayerRect(200, 200);
    const onChange = vi.fn();
    render(<PdfCropBox boxes={boxesOf()} onChange={onChange} />);
    const rotate = screen.getByTestId("pdf-crop-rotate");

    // 박스 중심(100,100)에서 오른쪽(0도)을 잡고 아래쪽(90도)으로 끈다.
    fireEvent.pointerDown(rotate, { clientX: 200, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(rotate, { clientX: 100, clientY: 200, pointerId: 1 });
    expect(onChange).toHaveBeenLastCalledWith("a", { ...RECT, angle: 90 });

    // Shift: 약 33.7도는 가장 가까운 15도 배수인 30도로 맞춘다.
    fireEvent.pointerUp(rotate, { pointerId: 1 });
    fireEvent.pointerDown(rotate, { clientX: 200, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(rotate, { clientX: 190, clientY: 160, pointerId: 1, shiftKey: true });
    const [, snapped] = onChange.mock.calls.at(-1);
    expect(snapped.angle).toBeCloseTo(30);
  });
});
