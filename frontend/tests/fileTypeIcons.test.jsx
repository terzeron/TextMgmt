import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";

import { TreeNodeIcon } from "../src/fileTypeIcons";

/** react-file-icon이 그리는 파일 아이콘 SVG (MUI 아이콘과 viewBox로 구분). */
const fileIconOf = (container) =>
  container.querySelector('svg[viewBox="0 0 40 48"]');

/** MUI 아이콘 SVG. */
const muiIconOf = (container) => container.querySelector("svg.MuiSvgIcon-root");

describe("TreeNodeIcon", () => {
  // ── 디렉토리 / 가상 노드는 MUI 아이콘 ──

  it("하위 노드를 가진 노드는 폴더 아이콘으로 그린다", () => {
    const { container } = render(<TreeNodeIcon fileType="pdf" expandable />);
    expect(muiIconOf(container)).toBeTruthy();
    expect(fileIconOf(container)).toBeNull();
  });

  it("fileType이 folder면 폴더 아이콘으로 그린다", () => {
    const { container } = render(<TreeNodeIcon fileType="folder" />);
    expect(muiIconOf(container)).toBeTruthy();
  });

  it.each(["pinned", "trash", "placeholder", "more"])(
    "가상 노드 fileType %s은 MUI 아이콘으로 그린다",
    (fileType) => {
      const { container } = render(<TreeNodeIcon fileType={fileType} />);
      expect(muiIconOf(container)).toBeTruthy();
      expect(fileIconOf(container)).toBeNull();
    },
  );

  // ── 파일은 react-file-icon ──

  it.each(["pdf", "epub", "txt", "hwp", "docx"])(
    "%s 파일은 확장자 라벨이 있는 파일 아이콘으로 그린다",
    (fileType) => {
      const { container } = render(<TreeNodeIcon fileType={fileType} />);
      expect(fileIconOf(container)?.textContent).toBe(fileType);
    },
  );

  it("image/video 별칭 fileType을 확장자로 변환한다", () => {
    const image = render(<TreeNodeIcon fileType="image" />);
    expect(fileIconOf(image.container)?.textContent).toBe("png");

    const video = render(<TreeNodeIcon fileType="video" />);
    expect(fileIconOf(video.container)?.textContent).toBe("mp4");
  });

  it("defaultStyles에 없는 확장자도 라벨을 그린다", () => {
    const { container } = render(<TreeNodeIcon fileType="azw3" />);
    expect(fileIconOf(container)?.textContent).toBe("azw3");
  });

  // ── fileType이 unknown일 때의 라벨 기반 대체 ──

  it("fileType이 unknown이면 라벨의 확장자를 사용한다", () => {
    const { container } = render(
      <TreeNodeIcon fileType="unknown" label="어떤책.EPUB" />,
    );
    expect(fileIconOf(container)?.textContent).toBe("epub");
  });

  it("fileType이 없으면 라벨의 확장자를 사용한다", () => {
    const { container } = render(<TreeNodeIcon label="report.pdf" />);
    expect(fileIconOf(container)?.textContent).toBe("pdf");
  });

  it("확장자 없는 라벨은 빈 라벨로 그린다", () => {
    const { container } = render(<TreeNodeIcon label="README" />);
    expect(fileIconOf(container)?.textContent).toBe("");
  });

  it("fileType과 라벨이 모두 없어도 그린다", () => {
    const { container } = render(<TreeNodeIcon />);
    expect(fileIconOf(container)?.textContent).toBe("");
  });

  // ── muted ──

  it("muted 폴더는 흐린 색으로 그린다", () => {
    const { container } = render(<TreeNodeIcon fileType="folder" muted />);
    expect(muiIconOf(container)).toBeTruthy();
  });

  it("muted 파일은 투명도를 낮춰 그린다", () => {
    const { container } = render(<TreeNodeIcon fileType="pdf" muted />);
    const wrapper = container.querySelector(".labelIcon");
    expect(wrapper).toBeTruthy();
    expect(getComputedStyle(wrapper).opacity).toBe("0.5");
  });
});
