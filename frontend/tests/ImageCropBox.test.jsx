// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import ImageCropBox, { MarkCropBox } from "../src/ImageCropBox";
import { GRID_DIVISIONS, newMark } from "../src/cropRect";

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

describe("ImageCropBox: 표시", () => {
  it("박스 바깥을 흐리게 덮는 요소를 그리지 않는다(기울어도 마찬가지다)", () => {
    mockLayerRect(200, 100);
    const { container, rerender } = render(<ImageCropBox boxes={boxesOf()} onChange={vi.fn()} />);
    expect(container.querySelector(".image-crop-dim")).toBeNull();
    rerender(<ImageCropBox boxes={boxesOf({ ...RECT, angle: 30 })} onChange={vi.fn()} />);
    expect(container.querySelector(".image-crop-dim")).toBeNull();
    expect(screen.queryByTestId("image-crop-dim-rotated")).toBeNull();
  });

  it("박스마다 안쪽에 점선 격자(가로·세로 각각 GRID_DIVISIONS-1 줄)를 그린다", () => {
    render(
      <ImageCropBox
        boxes={[
          { id: "a", rect: RECT, color: "#00f" },
          { id: "b", rect: { ...RECT, angle: 20 }, color: "#f00" },
        ]}
        onChange={vi.fn()}
      />,
    );
    const grids = screen.getAllByTestId("image-crop-grid");
    expect(grids).toHaveLength(2);
    for (const grid of grids) {
      expect(grid.closest('[data-testid="image-crop-box"]')).toBeTruthy();
      expect(grid.querySelectorAll("line")).toHaveLength((GRID_DIVISIONS - 1) * 2);
    }
  });
});

describe("ImageCropBox: 포인터 조작", () => {
  it("드래그 중이 아니면 이동·종료 이벤트를 무시한다", () => {
    mockLayerRect(200, 100);
    const onChange = vi.fn();
    render(<ImageCropBox boxes={boxesOf()} onChange={onChange} />);

    fireEvent.pointerMove(handle("e"), { clientX: 50, clientY: 0, pointerId: 1 });
    fireEvent.pointerUp(handle("e"), { pointerId: 1 });
    fireEvent.pointerMove(screen.getByTestId("image-crop-rotate"), { clientX: 50, clientY: 0 });

    expect(onChange).not.toHaveBeenCalled();
  });

  it("레이어 크기를 아직 모르면(0x0) 드래그해도 onChange 를 부르지 않는다", () => {
    // jsdom 의 getBoundingClientRect 기본값은 0x0 이다.
    const onChange = vi.fn();
    render(<ImageCropBox boxes={boxesOf()} onChange={onChange} />);

    fireEvent.pointerDown(handle("e"), { clientX: 0, clientY: 0, pointerId: 1 });
    fireEvent.pointerMove(handle("e"), { clientX: 20, clientY: 0, pointerId: 1 });

    expect(onChange).not.toHaveBeenCalled();
  });

  it("크기 조절 핸들을 끌면 이동량만큼 변이 움직이고, 손을 떼면 더는 반응하지 않는다", () => {
    mockLayerRect(200, 100);
    const onChange = vi.fn();
    const setPointerCapture = vi.fn();
    const releasePointerCapture = vi.fn();
    render(<ImageCropBox boxes={boxesOf()} onChange={onChange} />);
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
    render(<ImageCropBox boxes={boxesOf()} onChange={onChange} />);
    const rotate = screen.getByTestId("image-crop-rotate");

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

describe("MarkCropBox", () => {
  it("크롭 표시는 박스 하나, 분할 표시는 앞·뒤 라벨이 붙은 박스 둘을 그린다", () => {
    const { rerender } = render(<MarkCropBox mark={newMark("crop")} onChange={vi.fn()} />);
    expect(screen.getAllByTestId("image-crop-box")).toHaveLength(1);

    rerender(<MarkCropBox mark={newMark("split")} onChange={vi.fn()} />);
    expect(screen.getAllByTestId("image-crop-box")).toHaveLength(2);
    expect(screen.getByText("앞")).toBeTruthy();
    expect(screen.getByText("뒤")).toBeTruthy();
  });

  it("삭제 표시나 표시 없음이면 아무것도 그리지 않는다", () => {
    const { container, rerender } = render(<MarkCropBox mark={newMark("page")} onChange={vi.fn()} />);
    expect(container.firstChild).toBeNull();
    rerender(<MarkCropBox mark={undefined} onChange={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });
});
