// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import {
  render,
  screen,
  waitFor,
  cleanup,
  fireEvent,
} from "@testing-library/react";

import EpubCoverModal from "../src/EpubCoverModal";

const URL_A = "https://img.example.com/a.jpg";

function setup(props = {}) {
  const onSubmit = vi.fn(() => Promise.resolve(null));
  const onClose = vi.fn();
  render(<EpubCoverModal onSubmit={onSubmit} onClose={onClose} {...props} />);
  return { onSubmit, onClose };
}

const typeUrl = (value) =>
  fireEvent.change(screen.getByLabelText("이미지 URL"), {
    target: { value },
  });
const submitButton = () =>
  screen.getByRole("button", { name: /삽입 표시|불러오는/ });

describe("EpubCoverModal", () => {
  afterEach(cleanup);

  it("URL 을 넣기 전에는 삽입 표시를 막는다", () => {
    setup();
    expect(submitButton().disabled).toBe(true);
  });

  it("표지 유무에 따라 제목이 다르다", () => {
    setup({ hasCover: true });
    expect(screen.getByText("표지 이미지 교체")).toBeTruthy();
    expect(screen.getByText(/원본은 0_trash에 보존됩니다/)).toBeTruthy();
    cleanup();
    setup({ hasCover: false });
    expect(screen.getByText("표지 이미지 추가")).toBeTruthy();
    expect(screen.queryByText(/원본은 0_trash에 보존됩니다/)).toBeNull();
  });

  it("initialUrl 이 있으면 입력창을 채운다", () => {
    setup({ initialUrl: URL_A });
    expect(screen.getByLabelText("이미지 URL").value).toBe(URL_A);
    expect(submitButton().disabled).toBe(false);
  });

  it("공백을 뺀 URL 로 onSubmit 을 부른다", async () => {
    const { onSubmit } = setup();
    typeUrl(`  ${URL_A}  `);
    fireEvent.click(submitButton());

    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith(URL_A));
  });

  it("실패 사유를 보여 주고 다시 시도할 수 있게 한다", async () => {
    const onSubmit = vi.fn(() => Promise.resolve("내부 주소는 사용할 수 없습니다"));
    setup({ onSubmit });
    typeUrl(URL_A);
    fireEvent.click(submitButton());

    expect(await screen.findByText("내부 주소는 사용할 수 없습니다")).toBeTruthy();
    expect(submitButton().disabled).toBe(false);
  });

  it("요청 중에는 버튼을 잠그고 재요청을 막는다", async () => {
    let finish;
    const onSubmit = vi.fn(
      () => new Promise((resolve) => (finish = () => resolve("끝"))),
    );
    setup({ onSubmit });
    typeUrl(URL_A);
    fireEvent.click(submitButton());

    expect(await screen.findByText("불러오는 중…")).toBeTruthy();
    fireEvent.submit(screen.getByLabelText("이미지 URL").closest("form"));
    expect(onSubmit).toHaveBeenCalledTimes(1);
    finish();
    await screen.findByText("끝");
  });

  it("URL 이 비면 submit 해도 요청하지 않는다", () => {
    const { onSubmit } = setup();
    fireEvent.submit(screen.getByLabelText("이미지 URL").closest("form"));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("취소를 누르면 onClose 를 부른다", () => {
    const { onClose } = setup();
    fireEvent.click(screen.getByRole("button", { name: "취소" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
