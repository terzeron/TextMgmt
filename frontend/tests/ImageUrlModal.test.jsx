// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup, fireEvent } from "@testing-library/react";

import ImageUrlModal from "../src/ImageUrlModal";
import PdfInsertImageModal from "../src/PdfInsertImageModal";

afterEach(cleanup);

const base = {
  idPrefix: "x",
  title: "제목",
  description: "안내",
};

describe("ImageUrlModal", () => {
  it("제목·안내·id 를 받은 값으로 그린다", () => {
    render(
      <ImageUrlModal {...base} onSubmit={vi.fn()} onClose={vi.fn()} />,
    );
    expect(screen.getByText("제목").id).toBe("x-title");
    expect(screen.getByText("안내")).toBeTruthy();
    expect(screen.getByLabelText("이미지 URL").id).toBe("x-url");
  });

  it("Esc 로 닫는다", () => {
    const onClose = vi.fn();
    render(<ImageUrlModal {...base} onSubmit={vi.fn()} onClose={onClose} />);

    fireEvent.keyDown(document, { key: "Escape", code: "Escape" });

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("머리글 닫기 버튼으로 닫는다", () => {
    const onClose = vi.fn();
    render(<ImageUrlModal {...base} onSubmit={vi.fn()} onClose={onClose} />);

    fireEvent.click(screen.getByRole("button", { name: /close/i }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("요청 중에는 Esc 로 닫히지 않는다", async () => {
    const onClose = vi.fn();
    const onSubmit = vi.fn(() => new Promise(() => {}));
    render(<ImageUrlModal {...base} onSubmit={onSubmit} onClose={onClose} />);
    fireEvent.change(screen.getByLabelText("이미지 URL"), {
      target: { value: "https://img.example.com/a.jpg" },
    });
    fireEvent.click(screen.getByRole("button", { name: "삽입 표시" }));
    await screen.findByText("불러오는 중…");

    fireEvent.keyDown(document, { key: "Escape", code: "Escape" });

    expect(onClose).not.toHaveBeenCalled();
  });
});

describe("PdfInsertImageModal", () => {
  it("쪽 번호를 제목과 안내에 넣는다", () => {
    render(
      <PdfInsertImageModal pageNum={3} onSubmit={vi.fn()} onClose={vi.fn()} />,
    );
    expect(screen.getByText("3쪽 앞에 이미지 삽입")).toBeTruthy();
    expect(screen.getByText(/3쪽 앞에 들어갈 쪽을 미리 보여 주고/)).toBeTruthy();
    expect(screen.getByLabelText("이미지 URL").id).toBe("pdf-insert-image-url");
  });

  it("initialUrl 을 입력창에 채운다", () => {
    render(
      <PdfInsertImageModal
        pageNum={1}
        initialUrl="https://img.example.com/a.jpg"
        onSubmit={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByLabelText("이미지 URL").value).toBe(
      "https://img.example.com/a.jpg",
    );
  });
});
