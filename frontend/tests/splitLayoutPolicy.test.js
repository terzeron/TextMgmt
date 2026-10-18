// 디렉토리(Folder)를 쓰는 화면은 모두 SplitLayout의 왼쪽·오른쪽 컬럼이 따로 스크롤되고 폭을 조절할 수 있어야 한다.
// scroll={false}를 주면 문서 전체가 스크롤되어 이 성질이 깨지므로, 소스에서 막는다.
import { describe, it, expect } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const SRC = join(__dirname, "..", "src");
const sources = readdirSync(SRC)
  .filter((name) => name.endsWith(".jsx"))
  .map((name) => ({ name, text: readFileSync(join(SRC, name), "utf-8") }));
const folderScreens = sources.filter(
  ({ name, text }) =>
    name !== "Folder.jsx" &&
    /import Folder from ["']\.\/Folder(\.jsx)?["']/.test(text),
);

describe("디렉토리를 쓰는 화면의 분할 레이아웃", () => {
  it("디렉토리를 쓰는 화면을 찾는다", () => {
    expect(folderScreens.map(({ name }) => name).sort()).toEqual([
      "BookCoverAdmin.jsx",
      "Edit.jsx",
      "View.jsx",
    ]);
  });

  it.each(folderScreens.map(({ name, text }) => [name, text]))(
    "%s는 SplitLayout을 쓰고 컬럼별 스크롤을 끄지 않는다",
    (_name, text) => {
      expect(text).toMatch(/<SplitLayout\b/);
      expect(text).toMatch(/<SplitLayout\.Left\b/);
      expect(text).toMatch(/<SplitLayout\.Right\b/);
      expect(text).not.toMatch(/scroll=\{false\}/);
    },
  );

  it.each(folderScreens.map(({ name, text }) => [name, text]))(
    "%s는 폭 조절 값을 화면별로 저장한다",
    (_name, text) => {
      expect(text).toMatch(/storageKey="split:[a-z-]+"/);
    },
  );
});
