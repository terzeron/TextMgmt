import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import CoverImagePreview from "../src/CoverImagePreview";

afterEach(cleanup);
const preview = { id: "a", title: "책", url: "blob:cover", width: 600, height: 900 };

describe("원본 크기 표지 확대", () => {
  it("이미 받은 URL을 원본 이미지로 표시하고 선택과 닫기를 제공한다", () => {
    const close = vi.fn(), select = vi.fn();
    render(<CoverImagePreview preview={preview} onClose={close} onSelect={select} />);
    expect(screen.getByText("원본 크기 · 600 × 900 px")).toBeTruthy();
    expect(screen.getByAltText("책 확대 표지").getAttribute("src")).toBe("blob:cover");
    fireEvent.click(screen.getByRole("button", { name: "책 확대 표지 선택" }));
    expect(select).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "확대 닫기" }));
    expect(close).toHaveBeenCalledOnce();
  });

  it("Escape로 닫고 unmount 시 키보드 listener를 제거한다", () => {
    const close = vi.fn();
    const { unmount } = render(<CoverImagePreview preview={preview} onClose={close} onSelect={vi.fn()} />);
    fireEvent.keyDown(document, { key: "a" });
    expect(close).not.toHaveBeenCalled();
    fireEvent.keyDown(document, { key: "Escape", keyCode: 27 });
    expect(close).toHaveBeenCalledOnce();
    unmount();
    fireEvent.keyDown(document, { key: "Escape", keyCode: 27 });
    expect(close).toHaveBeenCalledOnce();
  });

  it("마우스가 모달 밖으로 이동해도 확대를 유지한다", () => {
    const close = vi.fn();
    render(<><button data-cover-preview="a">원래 후보</button><CoverImagePreview preview={preview} onClose={close} onSelect={vi.fn()} /></>);
    fireEvent.mouseLeave(screen.getByRole("dialog"), { relatedTarget: screen.getByRole("button", { name: "원래 후보" }) });
    expect(close).not.toHaveBeenCalled();
    fireEvent.mouseLeave(screen.getByRole("dialog"), { relatedTarget: document.body });
    expect(close).not.toHaveBeenCalled();
  });

  it.each([{}, { width: 600 }])("크기 정보가 없거나 마우스가 창 밖으로 나가도 처리한다: %j", (size) => {
    const close = vi.fn();
    render(<CoverImagePreview preview={{ id: "a", title: "책", url: "blob:cover", ...size }} onClose={close} onSelect={vi.fn()} />);
    expect(screen.getByText("원본 크기")).toBeTruthy();
    fireEvent.mouseLeave(screen.getByRole("dialog"), { relatedTarget: null });
    expect(close).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "확대 닫기" }));
    expect(close).toHaveBeenCalledOnce();
  });
});
