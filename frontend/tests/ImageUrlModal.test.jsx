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

describe("ImageUrlModal: 이 기기에서 이미지 선택", () => {
  const fileInput = () => screen.getByLabelText("이 기기에서 이미지 선택");
  const pick = (file) =>
    fireEvent.change(fileInput(), { target: { files: [file] } });
  const photo = () => new File(["x"], "photo.jpg", { type: "image/jpeg" });

  it("이미지 파일만 고르게 하고 id 에 접두어를 붙인다", () => {
    render(<ImageUrlModal {...base} onSubmit={vi.fn()} onClose={vi.fn()} />);

    expect(fileInput().type).toBe("file");
    expect(fileInput().accept).toBe("image/*");
    expect(fileInput().id).toBe("x-file");
  });

  it("파일을 고르면 곧바로 File 을 onSubmit 에 넘기고 요청 중에는 입력을 막는다", async () => {
    const file = photo();
    const onSubmit = vi.fn(() => new Promise(() => {}));
    render(<ImageUrlModal {...base} onSubmit={onSubmit} onClose={vi.fn()} />);

    pick(file);

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][0]).toBe(file);
    await screen.findByText("불러오는 중…");
    expect(fileInput().disabled).toBe(true);
  });

  it("실패 사유를 보여 주고 같은 파일을 다시 고를 수 있게 입력을 되돌린다", async () => {
    const onSubmit = vi
      .fn()
      .mockResolvedValueOnce("이미지 형식을 읽을 수 없습니다");
    render(<ImageUrlModal {...base} onSubmit={onSubmit} onClose={vi.fn()} />);

    pick(photo());

    expect(await screen.findByText("이미지 형식을 읽을 수 없습니다")).toBeTruthy();
    expect(fileInput().disabled).toBe(false);
    expect(fileInput().value).toBe("");
    pick(photo());
    expect(onSubmit).toHaveBeenCalledTimes(2);
  });

  it("고른 파일이 없으면 아무것도 보내지 않는다", () => {
    const onSubmit = vi.fn();
    render(<ImageUrlModal {...base} onSubmit={onSubmit} onClose={vi.fn()} />);

    fireEvent.change(fileInput(), { target: { files: [] } });

    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("URL 요청 중에 파일을 골라도 중복으로 보내지 않는다", async () => {
    const onSubmit = vi.fn(() => new Promise(() => {}));
    render(<ImageUrlModal {...base} onSubmit={onSubmit} onClose={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("이미지 URL"), {
      target: { value: "https://img.example.com/a.jpg" },
    });
    fireEvent.click(screen.getByRole("button", { name: "삽입 표시" }));
    await screen.findByText("불러오는 중…");

    pick(photo());

    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});
