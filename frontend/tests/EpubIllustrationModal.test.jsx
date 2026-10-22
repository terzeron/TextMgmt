// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup, fireEvent, waitFor } from "@testing-library/react";

import EpubIllustrationModal from "../src/EpubIllustrationModal";

afterEach(cleanup);

describe("EpubIllustrationModal", () => {
  it("삽화 삽입용 제목·안내와 epub-illustration 접두어 id 를 그린다", () => {
    render(<EpubIllustrationModal onSubmit={vi.fn()} onClose={vi.fn()} />);

    expect(screen.getByText("이 쪽 앞에 삽화 삽입").id).toBe("epub-illustration-title");
    expect(screen.getByText(/표지는 바뀌지 않습니다/)).toBeTruthy();
    expect(screen.getByLabelText("이미지 URL").id).toBe("epub-illustration-url");
    expect(screen.getByLabelText("이미지 URL").value).toBe("");
  });

  it("initialUrl 을 입력창에 채운다", () => {
    render(
      <EpubIllustrationModal
        initialUrl="https://img.example.com/a.jpg"
        onSubmit={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByLabelText("이미지 URL").value).toBe("https://img.example.com/a.jpg");
  });

  it("URL 을 제출하면 onSubmit 에 그 URL 을 넘긴다", async () => {
    const onSubmit = vi.fn(() => Promise.resolve(""));
    render(<EpubIllustrationModal onSubmit={onSubmit} onClose={vi.fn()} />);

    fireEvent.change(screen.getByLabelText("이미지 URL"), {
      target: { value: "https://img.example.com/b.png" },
    });
    fireEvent.click(screen.getByRole("button", { name: "삽입 표시" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith("https://img.example.com/b.png"));
  });

  it("취소 버튼은 onClose 를 부른다", () => {
    const onClose = vi.fn();
    render(<EpubIllustrationModal onSubmit={vi.fn()} onClose={onClose} />);

    fireEvent.click(screen.getByRole("button", { name: "취소" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
